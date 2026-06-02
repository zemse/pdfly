//! Tagged-PDF writing: add a logical structure tree (`/StructTreeRoot`) to a
//! copy of the input PDF, reflecting the detected element order, tag types,
//! page references, and figure alt-text, and mark the document as tagged.
//!
//! Scope: this writes the structure *skeleton* (tag types + reading order +
//! `/Alt`), which screen readers and PDF tools can surface. Full marked-content
//! (MCID) association and PDF/UA conformance validation are out of scope.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use lopdf::{dictionary, Document, Object, ObjectId};

use crate::model::{AnalyzedDoc, Element};

pub fn write_tagged_pdf(
    src: &Path,
    password: Option<&str>,
    analyzed: &AnalyzedDoc,
    out: &Path,
) -> Result<()> {
    let mut doc = Document::load(src).context("reload for tagging")?;
    if doc.is_encrypted() {
        let _ = doc.decrypt(password.unwrap_or(""));
    }

    let pages: BTreeMap<u32, ObjectId> = doc.get_pages();
    let page_ref = |n: usize| -> Option<Object> { pages.get(&(n as u32)).map(|id| Object::Reference(*id)) };

    // Reserve the StructTreeRoot id so children can reference it as /P.
    let root_id = doc.new_object_id();

    let mut kids: Vec<Object> = Vec::new();
    for el in &analyzed.elements {
        let pg = page_ref(el.page());
        match el {
            Element::Heading { level, .. } => {
                let tag = format!("H{}", (*level).clamp(1, 6));
                kids.push(struct_elem(&mut doc, &tag, root_id, pg, None));
            }
            Element::Paragraph { .. } => {
                kids.push(struct_elem(&mut doc, "P", root_id, pg, None));
            }
            Element::Image { alt, .. } => {
                let alt = if alt.is_empty() { None } else { Some(alt.as_str()) };
                kids.push(struct_elem(&mut doc, "Figure", root_id, pg, alt));
            }
            Element::List { items, .. } => {
                let list_id = doc.new_object_id();
                let mut li_refs = Vec::new();
                for _ in items {
                    let li = child_elem(&mut doc, "LI", list_id, pg.clone());
                    li_refs.push(li);
                }
                doc.set_object(
                    list_id,
                    Object::Dictionary(dictionary! {
                        "Type" => "StructElem",
                        "S" => "L",
                        "P" => Object::Reference(root_id),
                        "K" => Object::Array(li_refs),
                    }),
                );
                if let Some(p) = &pg {
                    if let Ok(d) = doc.get_dictionary_mut(list_id) {
                        d.set("Pg", p.clone());
                    }
                }
                kids.push(Object::Reference(list_id));
            }
            Element::Table { rows, .. } => {
                let table_id = doc.new_object_id();
                let mut tr_refs = Vec::new();
                for row in rows {
                    let tr_id = doc.new_object_id();
                    let mut cell_refs = Vec::new();
                    for cell in row {
                        if cell.covered {
                            continue;
                        }
                        cell_refs.push(child_elem(&mut doc, "TD", tr_id, pg.clone()));
                    }
                    doc.set_object(
                        tr_id,
                        Object::Dictionary(dictionary! {
                            "Type" => "StructElem",
                            "S" => "TR",
                            "P" => Object::Reference(table_id),
                            "K" => Object::Array(cell_refs),
                        }),
                    );
                    tr_refs.push(Object::Reference(tr_id));
                }
                doc.set_object(
                    table_id,
                    Object::Dictionary(dictionary! {
                        "Type" => "StructElem",
                        "S" => "Table",
                        "P" => Object::Reference(root_id),
                        "K" => Object::Array(tr_refs),
                    }),
                );
                kids.push(Object::Reference(table_id));
            }
        }
    }

    doc.set_object(
        root_id,
        Object::Dictionary(dictionary! {
            "Type" => "StructTreeRoot",
            "K" => Object::Array(kids),
        }),
    );

    // Catalog: /StructTreeRoot + /MarkInfo<</Marked true>>.
    let catalog_id = doc.trailer.get(b"Root").ok().and_then(|o| {
        if let Object::Reference(id) = o { Some(*id) } else { None }
    });
    if let Some(cid) = catalog_id {
        if let Ok(cat) = doc.get_dictionary_mut(cid) {
            cat.set("StructTreeRoot", Object::Reference(root_id));
            cat.set("MarkInfo", dictionary! { "Marked" => true });
        }
    }

    doc.save(out).context("save tagged pdf")?;
    Ok(())
}

fn struct_elem(
    doc: &mut Document,
    tag: &str,
    parent: ObjectId,
    pg: Option<Object>,
    alt: Option<&str>,
) -> Object {
    let mut d = dictionary! {
        "Type" => "StructElem",
        "S" => tag,
        "P" => Object::Reference(parent),
    };
    if let Some(p) = pg {
        d.set("Pg", p);
    }
    if let Some(a) = alt {
        d.set("Alt", Object::string_literal(a));
    }
    Object::Reference(doc.add_object(Object::Dictionary(d)))
}

fn child_elem(doc: &mut Document, tag: &str, parent: ObjectId, pg: Option<Object>) -> Object {
    let mut d = dictionary! {
        "Type" => "StructElem",
        "S" => tag,
        "P" => Object::Reference(parent),
    };
    if let Some(p) = pg {
        d.set("Pg", p);
    }
    Object::Reference(doc.add_object(Object::Dictionary(d)))
}
