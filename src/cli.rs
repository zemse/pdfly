//! Command-line interface (clap).

use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "pdf-rs",
    version,
    about = "Convert PDF files to Markdown (and JSON/HTML/text), optionally split by chapter."
)]
pub struct Cli {
    /// Input PDF file(s) or directories (directories are searched recursively).
    #[arg(required = true)]
    pub inputs: Vec<PathBuf>,

    /// Directory to write output files. Default: alongside each input file.
    #[arg(short = 'o', long)]
    pub output_dir: Option<PathBuf>,

    /// Output formats, comma-separated: markdown, json, html, text.
    #[arg(short = 'f', long, default_value = "markdown")]
    pub format: String,

    /// Password for encrypted PDFs.
    #[arg(short = 'p', long)]
    pub password: Option<String>,

    /// Pages to extract, e.g. "1,3,5-7". Default: all.
    #[arg(long)]
    pub pages: Option<String>,

    /// Split Markdown into one file per chapter (heading) inside a directory.
    #[arg(long)]
    pub split: bool,

    /// Heading level to split on (with --split). 1 = top-level headings.
    #[arg(long, default_value_t = 1)]
    pub split_level: u8,

    /// Separator inserted between pages; `%page-number%` is substituted.
    #[arg(long)]
    pub page_separator: Option<String>,

    /// Include repeated page headers/footers in output.
    #[arg(long)]
    pub include_header_footer: bool,

    /// Disable content-safety filtering (tiny / off-page text).
    #[arg(long)]
    pub content_safety_off: bool,

    /// Redact emails, URLs, phone numbers, IPs, and card-like numbers.
    #[arg(long)]
    pub sanitize: bool,

    /// Worker threads for per-page processing (default 1). Output is deterministic.
    #[arg(long, default_value_t = 1)]
    pub threads: usize,

    /// Use the PDF's own tag tree (tagged PDFs) instead of layout heuristics.
    #[arg(long)]
    pub use_struct_tree: bool,

    /// Image handling: off (omit), embedded (base64 data URI), external (files).
    #[arg(long, default_value = "external")]
    pub image_output: String,

    /// Format for extracted raster images.
    #[arg(long, default_value = "png", value_parser = ["png", "jpeg"])]
    pub image_format: String,

    /// Directory for extracted images (default: <output-dir>/<name>_images).
    #[arg(long)]
    pub image_dir: Option<std::path::PathBuf>,

    /// Write an annotated debug PDF with a box drawn around each detected element.
    #[arg(long)]
    pub annotate: bool,

    /// Detect strikethrough text and wrap it in ~~ (Markdown).
    #[arg(long)]
    pub detect_strikethrough: bool,

    /// Emit raw HTML <table> (with col/row spans) inside Markdown for complex tables.
    #[arg(long)]
    pub markdown_with_html: bool,

    /// Write the single requested format to stdout instead of files.
    #[arg(long)]
    pub to_stdout: bool,

    /// Suppress progress logging.
    #[arg(short = 'q', long)]
    pub quiet: bool,
}
