//! Markdown renderer (GFM). Idiomatic/RAG-oriented output.

use crate::model::{AnalyzedDoc, Cell, Element};

use super::RenderOptions;

pub fn to_markdown(doc: &AnalyzedDoc, opts: &RenderOptions) -> String {
    let mut out = String::new();
    render_elements(&doc.elements, opts, &mut out);
    out
}

/// Render a slice of elements (used by full-doc and chapter-split rendering).
pub fn render_elements(elements: &[Element], opts: &RenderOptions, out: &mut String) {
    let mut last_page = 0usize;
    for (i, el) in elements.iter().enumerate() {
        if i == 0 {
            last_page = el.page();
        } else if el.page() != last_page {
            if let Some(sep) = &opts.page_separator {
                let s = sep.replace("%page-number%", &el.page().to_string());
                out.push('\n');
                out.push_str(&s);
                out.push('\n');
            }
            last_page = el.page();
        }
        render_one(el, out);
        out.push('\n');
    }
    // collapse trailing blank lines to a single newline
    while out.ends_with("\n\n") {
        out.pop();
    }
}

fn render_one(el: &Element, out: &mut String) {
    match el {
        Element::Heading { level, text, .. } => {
            for _ in 0..(*level).clamp(1, 6) {
                out.push('#');
            }
            out.push(' ');
            out.push_str(&escape_inline(text));
            out.push('\n');
        }
        Element::Paragraph { text, .. } => {
            out.push_str(&escape_inline(text));
            out.push('\n');
        }
        Element::List { ordered, items, .. } => {
            for (i, item) in items.iter().enumerate() {
                if *ordered {
                    out.push_str(&format!("{}. ", i + 1));
                } else {
                    out.push_str("- ");
                }
                out.push_str(&escape_inline(&item.text));
                out.push('\n');
            }
        }
        Element::Table { rows, .. } => render_table(rows, out),
        Element::Image { name, alt, .. } => {
            out.push_str(&format!("![{}]({})\n", escape_inline(alt), name));
        }
    }
}

fn render_table(rows: &[Vec<Cell>], out: &mut String) {
    if rows.is_empty() {
        return;
    }
    let ncols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if ncols == 0 {
        return;
    }
    let cell = |row: &Vec<Cell>, c: usize| -> String {
        row.get(c).map(|x| escape_cell(&x.text)).unwrap_or_default()
    };
    // header row
    out.push('|');
    for c in 0..ncols {
        out.push(' ');
        out.push_str(&cell(&rows[0], c));
        out.push_str(" |");
    }
    out.push('\n');
    // separator
    out.push('|');
    for _ in 0..ncols {
        out.push_str(" --- |");
    }
    out.push('\n');
    // body
    for row in &rows[1..] {
        out.push('|');
        for c in 0..ncols {
            out.push(' ');
            out.push_str(&cell(row, c));
            out.push_str(" |");
        }
        out.push('\n');
    }
}

fn escape_inline(s: &str) -> String {
    s.replace('\\', "\\\\")
}

fn escape_cell(s: &str) -> String {
    s.replace('\\', "\\\\").replace('|', "\\|").replace('\n', " ")
}
