//! HTML renderer: a full standalone document.

use crate::model::{AnalyzedDoc, Cell, Element};

pub fn to_html(doc: &AnalyzedDoc) -> String {
    let title = doc.meta.title.clone().unwrap_or_default();
    let mut out = String::new();
    out.push_str("<!DOCTYPE html>\n<html lang=\"und\">\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str(&format!("<title>{}</title>\n</head>\n<body>\n", esc(&title)));
    for el in &doc.elements {
        match el {
            Element::Heading { level, text, .. } => {
                let l = (*level).clamp(1, 6);
                out.push_str(&format!("<h{l}>{}</h{l}>\n", esc(text)));
            }
            Element::Paragraph { text, .. } => {
                out.push_str(&format!("<p>{}</p>\n", esc(text)));
            }
            Element::List { ordered, items, .. } => {
                let tag = if *ordered { "ol" } else { "ul" };
                out.push_str(&format!("<{tag}>\n"));
                for item in items {
                    out.push_str(&format!("<li>{}</li>\n", esc(&item.text)));
                }
                out.push_str(&format!("</{tag}>\n"));
            }
            Element::Table { rows, .. } => render_table(rows, &mut out),
            Element::Image { name, alt, .. } => {
                out.push_str(&format!("<img src=\"{}\" alt=\"{}\">\n", esc(name), esc(alt)));
            }
        }
    }
    out.push_str("</body>\n</html>\n");
    out
}

fn render_table(rows: &[Vec<Cell>], out: &mut String) {
    out.push_str("<table border=\"1\">\n");
    for (r, row) in rows.iter().enumerate() {
        out.push_str("<tr>\n");
        let tag = if r == 0 { "th" } else { "td" };
        for cell in row {
            if cell.covered {
                continue;
            }
            let mut attrs = String::new();
            if cell.col_span > 1 {
                attrs.push_str(&format!(" colspan=\"{}\"", cell.col_span));
            }
            if cell.row_span > 1 {
                attrs.push_str(&format!(" rowspan=\"{}\"", cell.row_span));
            }
            out.push_str(&format!("<{tag}{attrs}>{}</{tag}>\n", esc(cell.text.trim())));
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</table>\n");
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}
