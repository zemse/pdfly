//! Plain-text renderer: values only, lists indented, table rows tab-separated.

use crate::model::{AnalyzedDoc, Element};

pub fn to_text(doc: &AnalyzedDoc) -> String {
    let mut out = String::new();
    for el in &doc.elements {
        match el {
            Element::Heading { text, .. } | Element::Paragraph { text, .. } => {
                out.push_str(text);
                out.push_str("\n\n");
            }
            Element::List { items, .. } => {
                for item in items {
                    out.push_str("  ");
                    out.push_str(&item.text);
                    out.push('\n');
                }
                out.push('\n');
            }
            Element::Table { rows, .. } => {
                for row in rows {
                    let line: Vec<&str> = row.iter().map(|c| c.text.trim()).collect();
                    out.push_str(&line.join("\t"));
                    out.push('\n');
                }
                out.push('\n');
            }
            Element::Image { alt, .. } => {
                if !alt.is_empty() {
                    out.push_str(alt);
                    out.push_str("\n\n");
                }
            }
        }
    }
    out.trim_end().to_string() + "\n"
}
