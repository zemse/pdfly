//! Annotated debug PDF: overlay a colored rectangle around each detected
//! element on top of the original page content. Pure `lopdf` overlay.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use lopdf::{dictionary, Document, Object, Stream};

use crate::model::{AnalyzedDoc, Element};

fn color(el: &Element) -> (f64, f64, f64) {
    match el {
        Element::Heading { .. } => (0.85, 0.10, 0.10),
        Element::Paragraph { .. } => (0.10, 0.35, 0.85),
        Element::List { .. } => (0.10, 0.65, 0.20),
        Element::Table { .. } => (0.95, 0.55, 0.10),
        Element::Image { .. } => (0.60, 0.20, 0.80),
    }
}

pub fn write_annotated(
    src: &Path,
    password: Option<&str>,
    analyzed: &AnalyzedDoc,
    out: &Path,
) -> Result<()> {
    let mut doc = Document::load(src).context("reload for annotation")?;
    if doc.is_encrypted() {
        let _ = doc.decrypt(password.unwrap_or(""));
    }

    // Group element bboxes by page number.
    let mut by_page: BTreeMap<usize, Vec<&Element>> = BTreeMap::new();
    for el in &analyzed.elements {
        by_page.entry(el.page()).or_default().push(el);
    }

    let pages: BTreeMap<u32, lopdf::ObjectId> = doc.get_pages();
    for (num, page_id) in pages {
        let Some(els) = by_page.get(&(num as usize)) else { continue };
        let mut content = String::from("q\n");
        for el in els {
            let b = el.bbox();
            if b.is_empty() {
                continue;
            }
            let (r, g, bl) = color(el);
            content.push_str(&format!(
                "{r:.2} {g:.2} {bl:.2} RG 1 w {:.2} {:.2} {:.2} {:.2} re S\n",
                b.left,
                b.bottom,
                b.width(),
                b.height()
            ));
        }
        content.push_str("Q\n");

        let stream = Stream::new(dictionary! {}, content.into_bytes());
        let sid = doc.add_object(Object::Stream(stream));

        // Append the overlay to the page's /Contents.
        if let Ok(page) = doc.get_dictionary_mut(page_id) {
            let new_contents = match page.get(b"Contents") {
                Ok(Object::Reference(r)) => vec![Object::Reference(*r), Object::Reference(sid)],
                Ok(Object::Array(a)) => {
                    let mut v = a.clone();
                    v.push(Object::Reference(sid));
                    v
                }
                _ => vec![Object::Reference(sid)],
            };
            page.set("Contents", Object::Array(new_contents));
        }
    }

    doc.save(out).context("save annotated pdf")?;
    Ok(())
}
