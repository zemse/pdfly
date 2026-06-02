//! Renderers: [`AnalyzedDoc`] -> Markdown / text / HTML / JSON, plus
//! chapter-wise Markdown splitting.

pub mod annotate;
pub mod html;
pub mod images;
pub mod json;
pub mod md;
pub mod split;
pub mod text;

#[derive(Clone, Debug)]
pub struct RenderOptions {
    /// Inserted between pages; `%page-number%` is replaced with the *next* page number.
    pub page_separator: Option<String>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions { page_separator: None }
    }
}

pub use html::to_html;
pub use json::to_json;
pub use md::to_markdown;
pub use split::{split_markdown, SplitChapter};
pub use text::to_text;
