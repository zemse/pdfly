//! Tagged-PDF writing: add a logical structure tree (`/StructTreeRoot`) to a
//! copy of the input PDF **with marked content**. Each page's content stream is
//! rewritten to wrap text/image operators in `/Tag <</MCID n>> BDC … EMC`
//! sequences, and the structure elements reference those MCIDs (with `/Pg`), so
//! the tags are associated with actual content.
//!
//! A trimmed content-stream interpreter maps each show-text / `Do` operator to
//! the detected element whose bounding box contains it (by anchor position).
//!
//! Scope: forward association (struct → content via MCID + `/Pg`) is written and
//! `/MarkInfo<</Marked true>>` is set. A `/ParentTree` (reverse map) and full
//! PDF/UA conformance validation are not yet produced.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, ObjectId, dictionary};

use crate::extract::Rect;
use crate::extract::matrix::Matrix;
use crate::model::{AnalyzedDoc, Element};

/// Structure tag for an element.
fn tag_of(el: &Element) -> String {
    match el {
        Element::Heading { level, .. } => format!("H{}", (*level).clamp(1, 6)),
        Element::Paragraph { .. } => "P".into(),
        Element::List { .. } => "L".into(),
        Element::Table { .. } => "Table".into(),
        Element::Image { .. } => "Figure".into(),
    }
}

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

    // Elements grouped by page, keeping their global index for stable identity.
    let mut by_page: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, el) in analyzed.elements.iter().enumerate() {
        by_page.entry(el.page()).or_default().push(i);
    }

    // element index -> (page, [mcids])
    let mut elem_mcids: BTreeMap<usize, (usize, Vec<i32>)> = BTreeMap::new();

    for (&num, &page_id) in &pages {
        let Some(elem_idxs) = by_page.get(&(num as usize)) else {
            continue;
        };
        let Ok(content_data) = doc.get_page_content(page_id) else {
            continue;
        };
        let Ok(content) = Content::decode(&content_data) else {
            continue;
        };

        let elem_boxes: Vec<(usize, Rect, String)> = elem_idxs
            .iter()
            .map(|&i| {
                (
                    i,
                    analyzed.elements[i].bbox(),
                    tag_of(&analyzed.elements[i]),
                )
            })
            .collect();

        let (new_ops, marks) = mark_operations(&content.operations, &elem_boxes);
        if marks == 0 {
            continue;
        }
        // Record MCIDs per element (assigned in mark_operations via the closure).
        // (mark_operations writes into `assigned` below.)
        let new_content = Content {
            operations: new_ops,
        };
        if let Ok(bytes) = new_content.encode() {
            // Replace the page content with a single new stream.
            let sid = doc.add_object(Object::Stream(lopdf::Stream::new(dictionary! {}, bytes)));
            if let Ok(page) = doc.get_dictionary_mut(page_id) {
                page.set("Contents", Object::Reference(sid));
                page.set("StructParents", Object::Integer(num as i64));
            }
        }
        // Pull the per-element MCIDs computed during marking.
        for (idx, mcid) in MARK_SCRATCH.with(|s| s.borrow_mut().drain(..).collect::<Vec<_>>()) {
            elem_mcids
                .entry(idx)
                .or_insert_with(|| (num as usize, Vec::new()))
                .1
                .push(mcid);
        }
    }

    // Build the structure tree referencing the MCIDs.
    let root_id = doc.new_object_id();
    let page_ref =
        |n: usize| -> Option<Object> { pages.get(&(n as u32)).map(|id| Object::Reference(*id)) };
    let mut kids: Vec<Object> = Vec::new();
    // page -> (mcid -> owning StructElem id), for the /ParentTree reverse map.
    let mut parent_map: BTreeMap<usize, BTreeMap<i32, ObjectId>> = BTreeMap::new();

    for (i, el) in analyzed.elements.iter().enumerate() {
        let Some((pg, mcids)) = elem_mcids.get(&i) else {
            continue;
        };
        if mcids.is_empty() {
            continue;
        }
        let k: Vec<Object> = mcids.iter().map(|m| Object::Integer(*m as i64)).collect();
        let mut d = dictionary! {
            "Type" => "StructElem",
            "S" => tag_of(el).as_str(),
            "P" => Object::Reference(root_id),
            "K" => Object::Array(k),
        };
        if let Some(p) = page_ref(*pg) {
            d.set("Pg", p);
        }
        if let Element::Image { alt, .. } = el {
            if !alt.is_empty() {
                d.set("Alt", Object::string_literal(alt.as_str()));
            }
        }
        let sid = doc.add_object(Object::Dictionary(d));
        kids.push(Object::Reference(sid));
        let page_slot = parent_map.entry(*pg).or_default();
        for &m in mcids {
            page_slot.insert(m, sid);
        }
    }

    // /ParentTree: number tree keyed by each page's /StructParents value (= page
    // number), mapping MCID index -> owning StructElem. Enables content->struct.
    let mut nums: Vec<Object> = Vec::new();
    for (pg, slot) in &parent_map {
        let max_mcid = slot.keys().copied().max().unwrap_or(0);
        let refs: Vec<Object> = (0..=max_mcid)
            .map(|m| {
                slot.get(&m)
                    .map(|id| Object::Reference(*id))
                    .unwrap_or(Object::Null)
            })
            .collect();
        nums.push(Object::Integer(*pg as i64));
        nums.push(Object::Reference(doc.add_object(Object::Array(refs))));
    }
    let parent_tree = doc.add_object(Object::Dictionary(
        dictionary! { "Nums" => Object::Array(nums) },
    ));
    let next_key = parent_map.keys().copied().max().map(|k| k + 1).unwrap_or(0);

    doc.set_object(
        root_id,
        Object::Dictionary(dictionary! {
            "Type" => "StructTreeRoot",
            "K" => Object::Array(kids),
            "ParentTree" => Object::Reference(parent_tree),
            "ParentTreeNextKey" => Object::Integer(next_key as i64),
        }),
    );

    // Catalog: structure tree + PDF/UA metadata.
    let title = analyzed
        .meta
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .or_else(|| {
            src.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        });
    if let Some(Object::Reference(cid)) = doc.trailer.get(b"Root").ok().cloned() {
        if let Ok(cat) = doc.get_dictionary_mut(cid) {
            cat.set("StructTreeRoot", Object::Reference(root_id));
            cat.set("MarkInfo", dictionary! { "Marked" => true });
            cat.set("Lang", Object::string_literal("en"));
            cat.set(
                "ViewerPreferences",
                dictionary! { "DisplayDocTitle" => true },
            );
        }
    }
    // Ensure a document title exists (PDF/UA needs one shown via DisplayDocTitle).
    if let Some(t) = &title {
        if let Ok(Object::Reference(info_id)) = doc.trailer.get(b"Info").cloned() {
            if let Ok(info) = doc.get_dictionary_mut(info_id) {
                info.set("Title", Object::string_literal(t.as_str()));
            }
        } else {
            let info =
                doc.add_object(dictionary! { "Title" => Object::string_literal(t.as_str()) });
            doc.trailer.set("Info", Object::Reference(info));
        }
    }

    doc.save(out).context("save tagged pdf")?;
    Ok(())
}

// Per-page scratch: (element index, mcid) pairs produced while marking.
thread_local! {
    static MARK_SCRATCH: std::cell::RefCell<Vec<(usize, i32)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Rewrite operations, wrapping text/image-painting ops in BDC/EMC with a fresh
/// per-page MCID when the op's anchor falls inside a detected element. Returns
/// the new operation list and the number of marks inserted. Per-element MCIDs
/// are pushed into MARK_SCRATCH.
fn mark_operations(ops: &[Operation], elems: &[(usize, Rect, String)]) -> (Vec<Operation>, usize) {
    let mut out: Vec<Operation> = Vec::with_capacity(ops.len() + 16);
    let mut ctm_stack: Vec<Matrix> = Vec::new();
    let mut ctm = Matrix::identity();
    let mut tm = Matrix::identity();
    let mut tlm = Matrix::identity();
    let mut leading = 0.0;
    let mut next_mcid = 0i32;
    let mut marks = 0usize;

    let num = |a: &[Object], i: usize| -> Option<f64> {
        a.get(i).and_then(|o| match o {
            Object::Integer(n) => Some(*n as f64),
            Object::Real(r) => Some(*r as f64),
            _ => None,
        })
    };
    let mat = |a: &[Object]| -> Option<Matrix> {
        Some(Matrix::new(
            num(a, 0)?,
            num(a, 1)?,
            num(a, 2)?,
            num(a, 3)?,
            num(a, 4)?,
            num(a, 5)?,
        ))
    };

    for op in ops {
        let o = op.operator.as_str();
        let a = &op.operands;
        // Update state (mirrors the extractor, position-only).
        match o {
            "q" => ctm_stack.push(ctm),
            "Q" => {
                if let Some(m) = ctm_stack.pop() {
                    ctm = m;
                }
            }
            "cm" => {
                if let Some(m) = mat(a) {
                    ctm = m.mul(&ctm);
                }
            }
            "BT" => {
                tm = Matrix::identity();
                tlm = Matrix::identity();
            }
            "Td" | "TD" => {
                if let (Some(x), Some(y)) = (num(a, 0), num(a, 1)) {
                    if o == "TD" {
                        leading = -y;
                    }
                    tlm = Matrix::translation(x, y).mul(&tlm);
                    tm = tlm;
                }
            }
            "Tm" => {
                if let Some(m) = mat(a) {
                    tm = m;
                    tlm = m;
                }
            }
            "T*" => {
                tlm = Matrix::translation(0.0, -leading).mul(&tlm);
                tm = tlm;
            }
            "TL" => leading = num(a, 0).unwrap_or(0.0),
            _ => {}
        }

        // Decide whether this op paints content we should tag.
        let anchor: Option<(f64, f64)> = match o {
            "Tj" | "TJ" | "'" | "\"" => {
                let m = tm.mul(&ctm);
                Some(m.apply(0.0, 0.0))
            }
            "Do" => {
                // image/XObject placed in the unit square under CTM
                Some(ctm.apply(0.5, 0.5))
            }
            _ => None,
        };

        if let Some((px, py)) = anchor {
            if let Some((idx, _b, tag)) = elems.iter().find(|(_, b, _)| contains(b, px, py)) {
                let mcid = next_mcid;
                next_mcid += 1;
                marks += 1;
                MARK_SCRATCH.with(|s| s.borrow_mut().push((*idx, mcid)));
                out.push(Operation::new(
                    "BDC",
                    vec![Object::Name(tag.clone().into_bytes()), bdc_props(mcid)],
                ));
                out.push(op.clone());
                out.push(Operation::new("EMC", vec![]));
                continue;
            }
        }
        out.push(op.clone());
    }
    (out, marks)
}

fn bdc_props(mcid: i32) -> Object {
    Object::Dictionary(dictionary! { "MCID" => Object::Integer(mcid as i64) })
}

fn contains(b: &Rect, x: f64, y: f64) -> bool {
    let pad = 1.0;
    x >= b.left - pad && x <= b.right + pad && y >= b.bottom - pad && y <= b.top + pad
}
