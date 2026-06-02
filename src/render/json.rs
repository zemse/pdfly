//! JSON renderer: structured elements with bounding boxes, modelled on the
//! opendataloader schema (`type`, `page number`, `bounding box [l,b,r,t]`).

use serde_json::{json, Value};

use crate::extract::Rect;
use crate::model::{AnalyzedDoc, Element};

pub fn to_json(doc: &AnalyzedDoc) -> String {
    let kids: Vec<Value> = doc.elements.iter().map(element_json).collect();
    let root = json!({
        "number of pages": doc.num_pages,
        "title": doc.meta.title,
        "author": doc.meta.author,
        "creation date": doc.meta.creation_date,
        "modification date": doc.meta.modification_date,
        "kids": kids,
    });
    serde_json::to_string_pretty(&root).unwrap_or_else(|_| "{}".to_string())
}

fn bbox(r: &Rect) -> Value {
    json!([round(r.left), round(r.bottom), round(r.right), round(r.top)])
}

fn round(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

fn element_json(el: &Element) -> Value {
    match el {
        Element::Heading { level, text, bbox: b, page, .. } => json!({
            "type": "heading",
            "heading level": level,
            "page number": page,
            "bounding box": bbox(b),
            "content": text,
        }),
        Element::Paragraph { text, bbox: b, page } => json!({
            "type": "paragraph",
            "page number": page,
            "bounding box": bbox(b),
            "content": text,
        }),
        Element::List { ordered, items, bbox: b, page } => json!({
            "type": "list",
            "numbering style": if *ordered { "ordered" } else { "unordered" },
            "page number": page,
            "bounding box": bbox(b),
            "number of list items": items.len(),
            "list items": items.iter().map(|it| json!({
                "type": "list item",
                "bounding box": bbox(&it.bbox),
                "content": it.text,
            })).collect::<Vec<_>>(),
        }),
        Element::Table { rows, bbox: b, page } => json!({
            "type": "table",
            "page number": page,
            "bounding box": bbox(b),
            "number of rows": rows.len(),
            "number of columns": rows.iter().map(|r| r.len()).max().unwrap_or(0),
            "rows": rows.iter().enumerate().map(|(ri, row)| json!({
                "type": "table row",
                "row number": ri + 1,
                "cells": row.iter().enumerate().map(|(ci, c)| json!({
                    "type": "table cell",
                    "row number": ri + 1,
                    "column number": ci + 1,
                    "row span": c.row_span.max(1),
                    "column span": c.col_span.max(1),
                    "content": c.text,
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        }),
        Element::Image { name, alt, bbox: b, page } => json!({
            "type": "image",
            "page number": page,
            "bounding box": bbox(b),
            "source": name,
            "alt": alt,
        }),
    }
}
