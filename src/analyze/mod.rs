//! Analysis pipeline: extracted [`Page`]s -> ordered semantic [`Element`]s.
//!
//! Stages (per page): line assembly -> table detection -> reading order ->
//! block classification (headings / paragraphs / lists). Then a global pass
//! maps heading font sizes to levels 1..=6.

pub mod reading_order;
pub mod sanitize;
pub mod structured;
pub mod tables;

use crate::extract::{Document, Rect, TextRun};
use crate::model::{AnalyzedDoc, Element, Line, ListItem};

mod lines;
use lines::build_lines;

pub struct Options {
    /// Keep repeated page headers/footers in output.
    pub include_header_footer: bool,
    /// Content-safety filters (tiny / off-page text). On by default.
    pub content_safety: bool,
    /// Redact emails/URLs/phones/etc.
    pub sanitize: bool,
    /// Worker threads for per-page line assembly (>=1). Output is identical
    /// regardless of count (each page is processed independently, in order).
    pub threads: usize,
    /// Use the PDF's own structure tree (tagged PDFs) instead of heuristics.
    pub use_struct_tree: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            include_header_footer: false,
            content_safety: true,
            sanitize: false,
            threads: 1,
            use_struct_tree: false,
        }
    }
}

pub fn analyze(doc: &Document, opts: &Options) -> AnalyzedDoc {
    // Tagged-PDF path: trust the author's structure tree when asked and available.
    if opts.use_struct_tree {
        if let Some(elements) = structured::structured_elements(doc) {
            let mut analyzed =
                AnalyzedDoc { meta: doc.meta.clone(), num_pages: doc.pages.len(), elements };
            if opts.sanitize {
                sanitize::sanitize_doc(&mut analyzed);
            }
            return analyzed;
        }
        eprintln!("note: no usable structure tree; falling back to heuristic analysis");
    }

    let mut elements: Vec<Element> = Vec::new();
    let mut heading_sizes: Vec<f64> = Vec::new();

    // Phase A: per-page filtered lines. Independent per page -> parallelizable
    // while preserving order (collect keeps input order).
    let build_page = |page: &crate::extract::Page| {
        let runs = filter_runs(&page.runs, page.media_box, opts.content_safety);
        build_lines(&runs)
    };
    let page_lines: Vec<Vec<Line>> = if opts.threads > 1 {
        use rayon::prelude::*;
        doc.pages.par_iter().map(build_page).collect()
    } else {
        doc.pages.iter().map(build_page).collect()
    };

    // Phase B: detect repeating headers/footers across pages.
    let drop_set = if opts.include_header_footer {
        std::collections::HashSet::new()
    } else {
        detect_headers_footers(&doc.pages, &page_lines)
    };

    for (pi, page) in doc.pages.iter().enumerate() {
        let page_no = page.number;
        let mut text_lines: Vec<Line> = page_lines[pi]
            .iter()
            .enumerate()
            .filter(|(li, _)| !drop_set.contains(&(pi, *li)))
            .map(|(_, l)| l.clone())
            .collect();

        // Tables consume the lines that fall inside them.
        let (detected, consumed) = tables::detect(&page.lines, &text_lines);
        let consumed: std::collections::HashSet<usize> = consumed.into_iter().collect();
        let remaining: Vec<Line> = text_lines
            .drain(..)
            .enumerate()
            .filter(|(i, _)| !consumed.contains(i))
            .map(|(_, l)| l)
            .collect();

        // Body font size = length-weighted mode of remaining line sizes.
        let body_size = body_font_size(&remaining);

        // Build ordered units: each table + each text line, sorted by reading order.
        #[derive(Clone)]
        enum Unit {
            Table(usize),
            Line(usize),
        }
        let mut unit_boxes: Vec<Rect> = Vec::new();
        let mut units: Vec<Unit> = Vec::new();
        for (ti, t) in detected.iter().enumerate() {
            unit_boxes.push(t.bbox);
            units.push(Unit::Table(ti));
        }
        for (li, l) in remaining.iter().enumerate() {
            unit_boxes.push(l.bbox);
            units.push(Unit::Line(li));
        }
        let ordering = reading_order::order(&unit_boxes);

        // Walk ordered units, classifying line runs into blocks.
        let mut pending_lines: Vec<usize> = Vec::new();
        let mut page_elements: Vec<Element> = Vec::new();

        let flush = |pending: &mut Vec<usize>,
                     out: &mut Vec<Element>,
                     hsizes: &mut Vec<f64>| {
            if pending.is_empty() {
                return;
            }
            let lines_slice: Vec<&Line> = pending.iter().map(|&i| &remaining[i]).collect();
            classify_block(&lines_slice, body_size, page_no, out, hsizes);
            pending.clear();
        };

        for &u in &ordering {
            match &units[u] {
                Unit::Table(ti) => {
                    flush(&mut pending_lines, &mut page_elements, &mut heading_sizes);
                    let t = &detected[*ti];
                    page_elements.push(Element::Table {
                        rows: t.rows.clone(),
                        bbox: t.bbox,
                        page: page_no,
                    });
                }
                Unit::Line(li) => pending_lines.push(*li),
            }
        }
        flush(&mut pending_lines, &mut page_elements, &mut heading_sizes);

        // Images as standalone elements, with caption association: a short,
        // caption-like paragraph just below/above the image becomes its alt
        // text and is removed from the flow.
        for img in &page.images {
            let mut alt = String::new();
            if let Some(idx) = find_caption(&page_elements, &img.bbox) {
                if let Element::Paragraph { text, .. } = &page_elements[idx] {
                    alt = text.clone();
                }
                page_elements.remove(idx);
            }
            page_elements.push(Element::Image {
                name: img.name.clone(),
                alt,
                bbox: img.bbox,
                page: page_no,
            });
        }

        elements.extend(page_elements);
    }

    assign_heading_levels(&mut elements, &mut heading_sizes);

    let mut analyzed = AnalyzedDoc { meta: doc.meta.clone(), num_pages: doc.pages.len(), elements };
    if opts.sanitize {
        sanitize::sanitize_doc(&mut analyzed);
    }
    analyzed
}

/// Content-safety filtering: drop tiny text and content outside the page box.
fn filter_runs(runs: &[TextRun], media: Rect, content_safety: bool) -> Vec<TextRun> {
    if !content_safety {
        return runs.to_vec();
    }
    let margin = 2.0;
    runs.iter()
        .filter(|r| {
            // tiny text
            if r.font_size < 1.5 || r.bbox.height() < 1.0 {
                return false;
            }
            // off-page (center outside media box + margin)
            let cx = r.bbox.center_x();
            let cy = r.bbox.center_y();
            cx >= media.left - margin
                && cx <= media.right + margin
                && cy >= media.bottom - margin
                && cy <= media.top + margin
        })
        .cloned()
        .collect()
}

/// Find (page_index, line_index) of lines that repeat at the top/bottom of
/// pages (running headers/footers, page numbers). Needs >= 3 pages.
fn detect_headers_footers(
    pages: &[crate::extract::Page],
    page_lines: &[Vec<Line>],
) -> std::collections::HashSet<(usize, usize)> {
    use std::collections::HashMap;
    let mut drop = std::collections::HashSet::new();
    if pages.len() < 3 {
        return drop;
    }
    // Map normalized header/footer text -> list of (page, line) occurrences.
    let mut seen: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
    for (pi, page) in pages.iter().enumerate() {
        let h = page.media_box.height();
        if h <= 0.0 {
            continue;
        }
        let top_cut = page.media_box.top - h * 0.10;
        let bot_cut = page.media_box.bottom + h * 0.10;
        for (li, line) in page_lines[pi].iter().enumerate() {
            let cy = line.bbox.center_y();
            if cy >= top_cut || cy <= bot_cut {
                let key = normalize_running(&line.text);
                if key.len() >= 2 {
                    seen.entry(key).or_default().push((pi, li));
                }
            }
        }
    }
    let threshold = (pages.len() / 2).max(2);
    for (_, occ) in seen {
        // Distinct pages it appears on.
        let mut pset: Vec<usize> = occ.iter().map(|(p, _)| *p).collect();
        pset.sort_unstable();
        pset.dedup();
        if pset.len() >= threshold {
            for o in occ {
                drop.insert(o);
            }
        }
    }
    drop
}

/// Normalize running text so "Page 3" and "Page 4" collapse together.
fn normalize_running(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().chars() {
        if c.is_ascii_digit() {
            out.push('#');
        } else {
            out.extend(c.to_lowercase());
        }
    }
    out
}

/// Length-weighted most common font size (rounded to 0.5pt).
fn body_font_size(lines: &[Line]) -> f64 {
    use std::collections::HashMap;
    let mut hist: HashMap<i64, usize> = HashMap::new();
    for l in lines {
        let key = (l.font_size * 2.0).round() as i64;
        *hist.entry(key).or_insert(0) += l.text.chars().count().max(1);
    }
    hist.into_iter()
        .max_by_key(|&(_, w)| w)
        .map(|(k, _)| k as f64 / 2.0)
        .unwrap_or(10.0)
}

/// Classify a run of consecutive lines (already separated by big gaps/tables)
/// into headings, list, or paragraph elements.
fn classify_block(
    lines: &[&Line],
    body_size: f64,
    page: usize,
    out: &mut Vec<Element>,
    heading_sizes: &mut Vec<f64>,
) {
    // Split into sub-blocks on large vertical gaps or heading/list boundaries.
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];

        // List item?
        if let Some((ordered, _marker)) = list_marker(&line.text) {
            // Gather consecutive list items.
            let mut raw: Vec<(String, Rect, bool)> = Vec::new();
            let mut bbox = Rect::empty();
            let mut ord = ordered;
            while i < lines.len() {
                if let Some((o2, _)) = list_marker(&lines[i].text) {
                    let txt = strip_marker(&lines[i].text);
                    bbox.union(&lines[i].bbox);
                    raw.push((txt, lines[i].bbox, o2));
                    ord = ord || o2;
                    i += 1;
                } else {
                    break;
                }
            }
            // Infer nesting depth from left indentation (relative to min left).
            let min_left = raw.iter().map(|(_, b, _)| b.left).fold(f64::MAX, f64::min);
            let items: Vec<ListItem> = raw
                .into_iter()
                .map(|(text, b, _)| {
                    let indent = (b.left - min_left).max(0.0);
                    let level = (indent / 18.0).round() as usize; // ~1 level per 18pt
                    ListItem { text, bbox: b, level: level.min(5) }
                })
                .collect();
            out.push(Element::List { ordered: ord, items, bbox, page });
            continue;
        }

        // Heading? Larger-than-body font, or bold + short. Guard against bold
        // running text: a bold line only counts as a heading if it's short and
        // doesn't read like a sentence (no terminal period, not too long).
        let chars = line.text.chars().count();
        let trimmed = line.text.trim();
        let is_larger = line.font_size >= body_size * 1.15;
        let sentence_like = chars > 60 || trimmed.ends_with('.') || trimmed.ends_with(';');
        let is_bold_short = line.bold && line.font_size >= body_size * 0.95 && chars <= 60 && !sentence_like;
        if (is_larger || is_bold_short) && !trimmed.is_empty() {
            heading_sizes.push(line.font_size);
            out.push(Element::Heading {
                level: 0, // filled in globally
                size: line.font_size,
                text: line.text.trim().to_string(),
                bbox: line.bbox,
                page,
            });
            i += 1;
            continue;
        }

        // Paragraph: merge following non-heading, non-list lines with small gaps.
        let mut text = line.text.trim_end().to_string();
        let mut bbox = line.bbox;
        let mut j = i + 1;
        while j < lines.len() {
            let next = lines[j];
            if list_marker(&next.text).is_some() {
                break;
            }
            if next.font_size >= body_size * 1.15 || (next.bold && next.text.chars().count() <= 80) {
                break;
            }
            let gap = bbox.bottom - next.bbox.top;
            let line_h = next.bbox.height().max(next.font_size);
            if gap > line_h * 1.0 {
                break; // paragraph break
            }
            if !text.ends_with(' ') {
                text.push(' ');
            }
            text.push_str(next.text.trim());
            bbox.union(&next.bbox);
            j += 1;
        }
        out.push(Element::Paragraph { text: normalize_ws(&text), bbox, page });
        i = j;
    }
}

/// Map distinct heading font sizes (descending) to levels 1..=6 across the doc.
fn assign_heading_levels(elements: &mut [Element], sizes: &mut [f64]) {
    let mut distinct: Vec<i64> = sizes.iter().map(|s| (s * 2.0).round() as i64).collect();
    distinct.sort_unstable_by(|a, b| b.cmp(a));
    distinct.dedup();
    for el in elements.iter_mut() {
        if let Element::Heading { level, size, .. } = el {
            let key = (*size * 2.0).round() as i64;
            let rank = distinct.iter().position(|&d| d == key).unwrap_or(0);
            *level = (rank as u8 + 1).min(6);
        }
    }
}

/// Return (ordered, marker_len) if the line begins with a list marker.
fn list_marker(text: &str) -> Option<(bool, usize)> {
    let t = text.trim_start();
    let mut chars = t.chars();
    if let Some(first) = chars.next() {
        if matches!(first, '•' | '◦' | '▪' | '■' | '◆' | '‣' | '·') {
            return Some((false, 1));
        }
        if (first == '-' || first == '*' || first == '–' || first == '—')
            && chars.next().map(|c| c == ' ').unwrap_or(false)
        {
            return Some((false, 2));
        }
    }
    // numbered: "1." "1)" "12." "a)" "iv."
    let bytes = t.as_bytes();
    let mut k = 0;
    while k < bytes.len() && bytes[k].is_ascii_digit() {
        k += 1;
    }
    if k > 0 && k < bytes.len() && (bytes[k] == b'.' || bytes[k] == b')') {
        return Some((true, k + 1));
    }
    if !bytes.is_empty()
        && bytes[0].is_ascii_alphabetic()
        && bytes.len() > 1
        && (bytes[1] == b'.' || bytes[1] == b')')
    {
        return Some((true, 2));
    }
    None
}

fn strip_marker(text: &str) -> String {
    let t = text.trim_start();
    if let Some((_, len)) = list_marker(text) {
        let mut chars = t.char_indices();
        let mut byte = 0;
        for _ in 0..len {
            if let Some((b, _)) = chars.next() {
                byte = b;
            }
        }
        // advance one more char index to get end of marker
        let rest = if let Some((b, _)) = chars.next() { &t[b..] } else { t.get(byte + 1..).unwrap_or("") };
        return rest.trim_start().to_string();
    }
    t.to_string()
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Find a caption-like paragraph adjacent to an image bbox.
fn find_caption(elements: &[Element], img: &Rect) -> Option<usize> {
    let img_h = img.height().max(1.0);
    for (i, el) in elements.iter().enumerate() {
        if let Element::Paragraph { text, bbox, .. } = el {
            let chars = text.chars().count();
            if chars == 0 || chars > 160 {
                continue;
            }
            // horizontally overlapping
            let overlap = bbox.left.max(img.left) < bbox.right.min(img.right);
            if !overlap {
                continue;
            }
            let below = (img.bottom - bbox.top).abs() < img_h * 0.8 && bbox.top <= img.bottom + 2.0;
            let above = (bbox.bottom - img.top).abs() < img_h * 0.8 && bbox.bottom >= img.top - 2.0;
            let caption_word = {
                let l = text.to_lowercase();
                l.starts_with("fig") || l.starts_with("table") || l.starts_with("image") || l.starts_with("photo")
            };
            if (below || above) && (caption_word || chars <= 90) {
                return Some(i);
            }
        }
    }
    None
}
