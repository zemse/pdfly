//! Semantic document model produced by the analysis pipeline and consumed by
//! the renderers. Independent of the PDF backend.

use crate::extract::Rect;

/// A text line assembled from runs on the same baseline.
#[derive(Clone, Debug)]
pub struct Line {
    pub text: String,
    pub bbox: Rect,
    pub font_size: f64,
    pub bold: bool,
    pub italic: bool,
    /// A horizontal rule crosses this line's vertical center (strikethrough).
    pub strike: bool,
}

/// A list item (possibly with nested children handled flatly for now).
#[derive(Clone, Debug)]
pub struct ListItem {
    pub text: String,
    pub bbox: Rect,
    /// Nesting depth (0 = top level), inferred from left indentation.
    pub level: usize,
    /// Original ordered-list marker as written (e.g. "34.", "52)"), preserved so
    /// renderers keep the document's real numbering instead of renumbering from 1.
    /// `None` for bullet lists or when no explicit marker was captured.
    pub marker: Option<String>,
}

/// A table cell.
#[derive(Clone, Debug, Default)]
pub struct Cell {
    pub text: String,
    pub col_span: usize,
    pub row_span: usize,
    /// True when this grid position is merged into a master cell above/left;
    /// span-aware renderers (HTML, tagged PDF) skip it.
    pub covered: bool,
}

/// A finished, render-ready semantic element, in reading order.
#[derive(Clone, Debug)]
pub enum Element {
    /// `size` is the source font size, used to rank levels; not rendered.
    Heading {
        level: u8,
        size: f64,
        text: String,
        bbox: Rect,
        page: usize,
    },
    Paragraph {
        text: String,
        bbox: Rect,
        page: usize,
    },
    List {
        ordered: bool,
        items: Vec<ListItem>,
        bbox: Rect,
        page: usize,
    },
    Table {
        rows: Vec<Vec<Cell>>,
        bbox: Rect,
        page: usize,
    },
    Image {
        name: String,
        alt: String,
        bbox: Rect,
        page: usize,
    },
}

impl Element {
    pub fn bbox(&self) -> Rect {
        match self {
            Element::Heading { bbox, .. }
            | Element::Paragraph { bbox, .. }
            | Element::List { bbox, .. }
            | Element::Table { bbox, .. }
            | Element::Image { bbox, .. } => *bbox,
        }
    }

    pub fn page(&self) -> usize {
        match self {
            Element::Heading { page, .. }
            | Element::Paragraph { page, .. }
            | Element::List { page, .. }
            | Element::Table { page, .. }
            | Element::Image { page, .. } => *page,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Element::Heading { .. } => "heading",
            Element::Paragraph { .. } => "paragraph",
            Element::List { .. } => "list",
            Element::Table { .. } => "table",
            Element::Image { .. } => "image",
        }
    }
}

/// The whole analyzed document.
#[derive(Clone, Debug)]
pub struct AnalyzedDoc {
    pub meta: crate::extract::PdfMeta,
    pub num_pages: usize,
    pub elements: Vec<Element>,
}
