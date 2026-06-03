//! Chapter-wise Markdown splitting: one file per heading at or above a chosen
//! level, plus an `index.md` table of contents.

use crate::model::{AnalyzedDoc, Element};

use super::{RenderOptions, md};

#[derive(Clone, Debug)]
pub struct SplitChapter {
    pub filename: String,
    pub title: String,
    pub content: String,
}

/// Split into chapters at headings whose level <= `split_level`.
pub fn split_markdown(
    doc: &AnalyzedDoc,
    split_level: u8,
    opts: &RenderOptions,
) -> Vec<SplitChapter> {
    // Partition element indices into chapters.
    let mut chapters: Vec<(Option<String>, Vec<&Element>)> = Vec::new();
    let mut current: Vec<&Element> = Vec::new();
    let mut current_title: Option<String> = None;

    for el in &doc.elements {
        let is_boundary = matches!(el, Element::Heading { level, .. } if *level <= split_level);
        if is_boundary {
            if !current.is_empty() || current_title.is_some() {
                chapters.push((current_title.take(), std::mem::take(&mut current)));
            }
            if let Element::Heading { text, .. } = el {
                current_title = Some(text.clone());
            }
        }
        current.push(el);
    }
    if !current.is_empty() {
        chapters.push((current_title.take(), current));
    }

    // Front matter (before first heading) gets a default title.
    let mut used = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (i, (title, els)) in chapters.iter().enumerate() {
        let title = title.clone().unwrap_or_else(|| {
            doc.meta
                .title
                .clone()
                .unwrap_or_else(|| "Front Matter".to_string())
        });
        let slug = unique_slug(&slugify(&title), &mut used);
        let filename = format!("{:02}-{}.md", i + 1, slug);
        let owned: Vec<Element> = els.iter().map(|e| (*e).clone()).collect();
        let mut content = String::new();
        md::render_elements(&owned, opts, &mut content);
        content.push('\n');
        out.push(SplitChapter {
            filename,
            title,
            content,
        });
    }
    out
}

/// Build an `index.md` linking to each chapter.
pub fn build_index(chapters: &[SplitChapter], doc_title: Option<&str>) -> String {
    let mut s = String::new();
    s.push_str(&format!("# {}\n\n", doc_title.unwrap_or("Contents")));
    for ch in chapters {
        s.push_str(&format!("- [{}]({})\n", ch.title, ch.filename));
    }
    s
}

fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in s.chars().flat_map(|c| c.to_lowercase()) {
        if c.is_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
        if out.chars().count() >= 50 {
            break;
        }
    }
    let s = out.trim_matches('-').to_string();
    if s.is_empty() {
        "section".to_string()
    } else {
        s
    }
}

fn unique_slug(base: &str, used: &mut std::collections::HashSet<String>) -> String {
    if used.insert(base.to_string()) {
        return base.to_string();
    }
    for n in 2.. {
        let cand = format!("{base}-{n}");
        if used.insert(cand.clone()) {
            return cand;
        }
    }
    base.to_string()
}
