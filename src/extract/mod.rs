//! PDF extraction layer: turns a PDF file into a [`Document`] of positioned
//! content (text runs, images, vector line segments) per page.
//!
//! Everything above this layer (analysis, rendering) works only on these
//! types, so the backend is swappable behind [`PdfBackend`].

pub mod fonts;
pub mod lopdf_backend;
pub mod matrix;

pub use lopdf_backend::LopdfBackend;

/// Axis-aligned rectangle in PDF user space: origin bottom-left, points.
/// Stored as `[left, bottom, right, top]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub left: f64,
    pub bottom: f64,
    pub right: f64,
    pub top: f64,
}

impl Rect {
    pub fn new(left: f64, bottom: f64, right: f64, top: f64) -> Self {
        Rect { left, bottom, right, top }
    }

    pub fn empty() -> Self {
        Rect { left: f64::MAX, bottom: f64::MAX, right: f64::MIN, top: f64::MIN }
    }

    pub fn is_empty(&self) -> bool {
        self.right < self.left || self.top < self.bottom
    }

    pub fn width(&self) -> f64 {
        (self.right - self.left).max(0.0)
    }

    pub fn height(&self) -> f64 {
        (self.top - self.bottom).max(0.0)
    }

    pub fn center_x(&self) -> f64 {
        (self.left + self.right) / 2.0
    }

    pub fn center_y(&self) -> f64 {
        (self.bottom + self.top) / 2.0
    }

    pub fn area(&self) -> f64 {
        self.width() * self.height()
    }

    /// Expand to include `p` (a point).
    pub fn include_point(&mut self, x: f64, y: f64) {
        self.left = self.left.min(x);
        self.right = self.right.max(x);
        self.bottom = self.bottom.min(y);
        self.top = self.top.max(y);
    }

    /// Expand to include another rect.
    pub fn union(&mut self, o: &Rect) {
        if o.is_empty() {
            return;
        }
        self.left = self.left.min(o.left);
        self.right = self.right.max(o.right);
        self.bottom = self.bottom.min(o.bottom);
        self.top = self.top.max(o.top);
    }
}

/// A contiguous run of text produced by one show-text operation.
#[derive(Clone, Debug)]
pub struct TextRun {
    pub text: String,
    pub bbox: Rect,
    pub font_size: f64,
    pub font_name: String,
    pub bold: bool,
    pub italic: bool,
    /// RGB in 0..=1.
    pub color: [f64; 3],
    /// Marked-content id of the enclosing BDC, if any (for tagged PDFs).
    pub mcid: Option<i32>,
}

/// A placed image XObject.
#[derive(Clone, Debug)]
pub struct ImageObj {
    pub bbox: Rect,
    pub name: String,
}

/// An axis-aligned vector line segment (for table borders / strikethrough).
#[derive(Clone, Copy, Debug)]
pub struct LineSeg {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl LineSeg {
    pub fn is_horizontal(&self) -> bool {
        (self.y1 - self.y0).abs() <= 1.0 && (self.x1 - self.x0).abs() > 1.0
    }
    pub fn is_vertical(&self) -> bool {
        (self.x1 - self.x0).abs() <= 1.0 && (self.y1 - self.y0).abs() > 1.0
    }
}

#[derive(Clone, Debug)]
pub struct Page {
    pub number: usize, // 1-indexed
    pub media_box: Rect,
    pub runs: Vec<TextRun>,
    pub images: Vec<ImageObj>,
    pub lines: Vec<LineSeg>,
}

#[derive(Clone, Debug, Default)]
pub struct PdfMeta {
    pub title: Option<String>,
    pub author: Option<String>,
    pub creation_date: Option<String>,
    pub modification_date: Option<String>,
}

/// A node in the PDF logical structure tree (tagged PDFs).
#[derive(Clone, Debug)]
pub struct StructElem {
    /// Structure type tag, e.g. "H1", "P", "L", "LI", "Table", "Figure".
    pub tag: String,
    pub alt: Option<String>,
    /// (page 1-indexed, mcid) for marked-content this element owns directly.
    pub mcids: Vec<(usize, i32)>,
    pub kids: Vec<StructElem>,
}

#[derive(Clone, Debug)]
pub struct Document {
    pub meta: PdfMeta,
    pub pages: Vec<Page>,
    /// Root of the logical structure tree, if the PDF is tagged.
    pub structure: Option<StructElem>,
}

/// A pluggable PDF extraction backend.
pub trait PdfBackend {
    /// Load `path`, optionally with a `password`, extracting `pages`
    /// (1-indexed). `None` means all pages.
    fn load(
        path: &std::path::Path,
        password: Option<&str>,
        pages: Option<&std::collections::BTreeSet<usize>>,
    ) -> anyhow::Result<Document>;
}
