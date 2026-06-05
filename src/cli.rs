//! Command-line interface (clap).

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "pdf",
    version,
    about = "Read PDF files and convert them to Markdown (and JSON/HTML/text)."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Read a PDF and convert it to Markdown/JSON/HTML/text (stdout by default).
    Read(ReadArgs),
}

#[derive(Args, Debug)]
pub struct ReadArgs {
    /// Input PDF file.
    pub input: PathBuf,

    /// Write output to a file instead of stdout. With --split this is the
    /// output directory. Format is inferred from the file extension.
    #[arg(short = 'o', long)]
    pub out: Option<PathBuf>,

    /// Output format: markdown, json, html, text. Overrides the format
    /// inferred from --out's extension. Default: markdown.
    #[arg(short = 'f', long)]
    pub format: Option<String>,

    /// Password for encrypted PDFs.
    #[arg(short = 'p', long)]
    pub password: Option<String>,

    /// Pages to extract, e.g. "1,3,5-7". Default: all.
    #[arg(long)]
    pub pages: Option<String>,

    /// Split Markdown into one file per chapter (heading). Requires --out <dir>.
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

    /// Directory for extracted images (default: <out-dir>/<name>_images).
    #[arg(long)]
    pub image_dir: Option<std::path::PathBuf>,

    /// Write an annotated debug PDF with a box drawn around each detected element.
    /// Requires --out.
    #[arg(long)]
    pub annotate: bool,

    /// Write a tagged PDF (adds a /StructTreeRoot structure tree to a copy).
    /// Requires --out.
    #[arg(long)]
    pub tagged_pdf: bool,

    /// Detect strikethrough text and wrap it in ~~ (Markdown).
    #[arg(long)]
    pub detect_strikethrough: bool,

    /// Table detection method: cluster (ruled borders + borderless,
    /// column-aligned tables; default) or ruled (ruled borders only).
    #[arg(long, default_value = "cluster", value_parser = ["cluster", "ruled", "default"])]
    pub table_method: String,

    /// Emit raw HTML <table> (with col/row spans) inside Markdown for complex tables.
    #[arg(long)]
    pub markdown_with_html: bool,

    /// Suppress progress logging.
    #[arg(short = 'q', long)]
    pub quiet: bool,

    /// Print processing time and throughput (pages/sec).
    #[arg(long)]
    pub timing: bool,
}
