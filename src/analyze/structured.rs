//! Tagged-PDF path: build elements directly from the logical structure tree
//! (`--use-struct-tree`), resolving text via marked-content (MCID) mapping.
//! Falls back to `None` if the tree yields too little text.

use std::collections::HashMap;

use crate::extract::{Document, Rect, StructElem};
use crate::model::{Cell, Element, ListItem};

type McidText = HashMap<(usize, i32), (String, Rect)>;

pub fn structured_elements(doc: &Document) -> Option<Vec<Element>> {
    let root = doc.structure.as_ref()?;
    let map = build_mcid_text(doc);
    let mut out = Vec::new();
    for kid in &root.kids {
        emit(kid, &map, &mut out, 0);
    }
    let total: usize = out.iter().map(element_len).sum();
    if total < 20 {
        return None; // structure present but unmapped to text -> fall back
    }
    Some(out)
}

fn element_len(e: &Element) -> usize {
    match e {
        Element::Heading { text, .. } | Element::Paragraph { text, .. } => text.len(),
        Element::List { items, .. } => items.iter().map(|i| i.text.len()).sum(),
        Element::Table { rows, .. } => rows.iter().flatten().map(|c| c.text.len()).sum(),
        Element::Image { alt, .. } => alt.len(),
    }
}

/// Group runs by (page, mcid) -> ordered text + bbox.
fn build_mcid_text(doc: &Document) -> McidText {
    let mut groups: HashMap<(usize, i32), Vec<&crate::extract::TextRun>> = HashMap::new();
    for page in &doc.pages {
        for r in &page.runs {
            if let Some(m) = r.mcid {
                groups.entry((page.number, m)).or_default().push(r);
            }
        }
    }
    let mut out = McidText::new();
    for (key, mut runs) in groups {
        runs.sort_by(|a, b| {
            b.bbox
                .top
                .partial_cmp(&a.bbox.top)
                .unwrap()
                .then(a.bbox.left.partial_cmp(&b.bbox.left).unwrap())
        });
        let mut text = String::new();
        let mut bbox = Rect::empty();
        for r in runs {
            if !text.is_empty() && !text.ends_with(' ') && !r.text.starts_with(' ') {
                text.push(' ');
            }
            text.push_str(&r.text);
            bbox.union(&r.bbox);
        }
        out.insert(
            key,
            (text.split_whitespace().collect::<Vec<_>>().join(" "), bbox),
        );
    }
    out
}

/// All MCID text within an element subtree, in document order, with bbox/page.
fn gather(elem: &StructElem, map: &McidText) -> (String, Rect, usize) {
    let mut text = String::new();
    let mut bbox = Rect::empty();
    let mut page = 0usize;
    gather_into(elem, map, &mut text, &mut bbox, &mut page);
    (
        text.split_whitespace().collect::<Vec<_>>().join(" "),
        bbox,
        page,
    )
}

fn gather_into(
    elem: &StructElem,
    map: &McidText,
    text: &mut String,
    bbox: &mut Rect,
    page: &mut usize,
) {
    for key in &elem.mcids {
        if let Some((t, b)) = map.get(key) {
            if !t.is_empty() {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(t);
                bbox.union(b);
                if *page == 0 {
                    *page = key.0;
                }
            }
        }
    }
    for kid in &elem.kids {
        gather_into(kid, map, text, bbox, page);
    }
}

fn heading_level(tag: &str) -> Option<u8> {
    match tag {
        "Title" => Some(1),
        "H" | "H1" => Some(1),
        "H2" => Some(2),
        "H3" => Some(3),
        "H4" => Some(4),
        "H5" => Some(5),
        "H6" => Some(6),
        _ => None,
    }
}

fn emit(elem: &StructElem, map: &McidText, out: &mut Vec<Element>, depth: usize) {
    if depth > 60 {
        return;
    }
    let tag = elem.tag.as_str();

    if let Some(level) = heading_level(tag) {
        let (text, bbox, page) = gather(elem, map);
        if !text.is_empty() {
            out.push(Element::Heading {
                level,
                size: 0.0,
                text,
                bbox,
                page,
            });
        }
        return;
    }

    match tag {
        "P" | "Note" | "BlockQuote" | "Caption" | "Quote" | "Index" => {
            let (text, bbox, page) = gather(elem, map);
            if !text.is_empty() {
                out.push(Element::Paragraph { text, bbox, page });
            }
        }
        "L" => {
            let mut items = Vec::new();
            let mut bbox = Rect::empty();
            let mut page = 0;
            for li in &elem.kids {
                if li.tag == "LI" || li.tag == "LBody" {
                    let (text, b, p) = gather(li, map);
                    if !text.is_empty() {
                        bbox.union(&b);
                        if page == 0 {
                            page = p;
                        }
                        items.push(ListItem {
                            text,
                            bbox: b,
                            level: 0,
                            marker: None,
                        });
                    }
                }
            }
            if !items.is_empty() {
                let ordered = items
                    .first()
                    .map(|i| starts_ordered(&i.text))
                    .unwrap_or(false);
                out.push(Element::List {
                    ordered,
                    items,
                    bbox,
                    page,
                });
            }
        }
        "Table" => {
            let mut rows = Vec::new();
            let mut bbox = Rect::empty();
            let mut page = 0;
            for tr in descendants_with_tag(elem, "TR") {
                let mut cells = Vec::new();
                for cell in &tr.kids {
                    if cell.tag == "TD" || cell.tag == "TH" {
                        let (text, b, p) = gather(cell, map);
                        bbox.union(&b);
                        if page == 0 {
                            page = p;
                        }
                        cells.push(Cell {
                            text,
                            col_span: 1,
                            row_span: 1,
                            covered: false,
                        });
                    }
                }
                if cells.iter().any(|c| !c.text.trim().is_empty()) {
                    rows.push(cells);
                }
            }
            // Only emit if the structure tree actually mapped to cell text.
            if rows.len() >= 1
                && rows
                    .iter()
                    .flatten()
                    .filter(|c| !c.text.trim().is_empty())
                    .count()
                    >= 2
            {
                out.push(Element::Table { rows, bbox, page });
            }
        }
        "Figure" => {
            let (_t, bbox, page) = gather(elem, map);
            out.push(Element::Image {
                name: String::new(),
                alt: elem.alt.clone().unwrap_or_default(),
                bbox,
                page,
            });
        }
        // Containers: recurse.
        _ => {
            for kid in &elem.kids {
                emit(kid, map, out, depth + 1);
            }
        }
    }
}

fn descendants_with_tag<'a>(elem: &'a StructElem, tag: &str) -> Vec<&'a StructElem> {
    let mut out = Vec::new();
    for kid in &elem.kids {
        if kid.tag == tag {
            out.push(kid);
        } else {
            out.extend(descendants_with_tag(kid, tag));
        }
    }
    out
}

fn starts_ordered(text: &str) -> bool {
    let t = text.trim_start();
    let mut chars = t.chars();
    let mut saw_digit = false;
    for c in chars.by_ref() {
        if c.is_ascii_digit() {
            saw_digit = true;
        } else {
            return saw_digit && (c == '.' || c == ')');
        }
    }
    false
}
