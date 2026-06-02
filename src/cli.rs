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

    /// Write the single requested format to stdout instead of files.
    #[arg(long)]
    pub to_stdout: bool,

    /// Suppress progress logging.
    #[arg(short = 'q', long)]
    pub quiet: bool,
}
