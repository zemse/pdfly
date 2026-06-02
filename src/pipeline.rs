//! Orchestration: walk inputs, extract -> analyze -> render -> write.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::analyze::{self, Options};
use crate::cli::Cli;
use crate::extract::{LopdfBackend, PdfBackend};
use crate::render::{self, split, RenderOptions};

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Markdown,
    Json,
    Html,
    Text,
}

pub fn run(cli: &Cli) -> Result<()> {
    let formats = parse_formats(&cli.format)?;
    let pages = match &cli.pages {
        Some(s) => Some(parse_pages(s)?),
        None => None,
    };

    let files = collect_pdfs(&cli.inputs);
    if files.is_empty() {
        bail!("no PDF files found in the given input(s)");
    }

    if cli.to_stdout && (files.len() > 1 || formats.len() != 1) {
        bail!("--to-stdout requires a single input file and a single format");
    }

    for file in &files {
        if !cli.quiet {
            eprintln!("processing {}", file.display());
        }
        if let Err(e) = process_one(cli, file, &formats, pages.as_ref()) {
            eprintln!("error: {}: {e:#}", file.display());
        }
    }
    Ok(())
}

fn process_one(
    cli: &Cli,
    file: &Path,
    formats: &[Format],
    pages: Option<&BTreeSet<usize>>,
) -> Result<()> {
    let mut doc = LopdfBackend::load(file, cli.password.as_deref(), pages)
        .with_context(|| format!("extracting {}", file.display()))?;
    // Optional OCR for scanned pages (no-op unless built with --features ocr).
    match crate::ocr::augment(&mut doc) {
        Ok(n) if n > 0 && !cli.quiet => eprintln!("  OCR added {n} text line(s)"),
        Err(e) => eprintln!("  ocr: {e:#}"),
        _ => {}
    }
    let mut analyzed = analyze::analyze(
        &doc,
        &Options {
            include_header_footer: cli.include_header_footer,
            content_safety: !cli.content_safety_off,
            sanitize: cli.sanitize,
            threads: cli.threads.max(1),
            use_struct_tree: cli.use_struct_tree,
            detect_strikethrough: cli.detect_strikethrough,
        },
    );

    let ropts = RenderOptions {
        page_separator: cli.page_separator.clone(),
        html_tables: cli.markdown_with_html,
    };
    let out_dir = cli.output_dir.clone().unwrap_or_else(|| {
        file.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
    });
    let base = file.file_stem().and_then(|s| s.to_str()).unwrap_or("output").to_string();

    // Resolve images (write files / embed / drop) before rendering.
    if !cli.to_stdout {
        let mode = render::images::parse_mode(&cli.image_output);
        let image_dir = cli
            .image_dir
            .clone()
            .unwrap_or_else(|| out_dir.join(format!("{base}_images")));
        std::fs::create_dir_all(&out_dir).ok();
        let n = render::images::process_images(
            &doc, &mut analyzed, mode, &cli.image_format, &out_dir, &image_dir, &base,
        )
        .unwrap_or(0);
        if n > 0 && !cli.quiet {
            eprintln!("  extracted {n} image(s)");
        }
    } else {
        // stdout: drop images (can't reference files).
        analyzed.elements.retain(|e| !matches!(e, crate::model::Element::Image { .. }));
    }

    // Tagged PDF (structure tree).
    if cli.tagged_pdf {
        std::fs::create_dir_all(&out_dir).ok();
        let path = out_dir.join(format!("{base}.tagged.pdf"));
        match crate::render::tagged::write_tagged_pdf(file, cli.password.as_deref(), &analyzed, &path) {
            Ok(()) if !cli.quiet => eprintln!("  wrote {}", path.display()),
            Err(e) => eprintln!("  tagged-pdf failed: {e:#}"),
            _ => {}
        }
    }

    // Annotated debug PDF.
    if cli.annotate {
        std::fs::create_dir_all(&out_dir).ok();
        let path = out_dir.join(format!("{base}.annotated.pdf"));
        match crate::render::annotate::write_annotated(file, cli.password.as_deref(), &analyzed, &path) {
            Ok(()) if !cli.quiet => eprintln!("  wrote {}", path.display()),
            Err(e) => eprintln!("  annotate failed: {e:#}"),
            _ => {}
        }
    }

    // Chapter split (Markdown only) takes a dedicated directory.
    if cli.split {
        let chapters = split::split_markdown(&analyzed, cli.split_level.max(1), &ropts);
        let dir = out_dir.join(&base);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating {}", dir.display()))?;
        for ch in &chapters {
            std::fs::write(dir.join(&ch.filename), &ch.content)?;
        }
        let index = split::build_index(&chapters, analyzed.meta.title.as_deref());
        std::fs::write(dir.join("index.md"), index)?;
        if !cli.quiet {
            eprintln!("  wrote {} chapter file(s) -> {}", chapters.len(), dir.display());
        }
        if formats == [Format::Markdown] {
            return Ok(());
        }
    }

    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("creating {}", out_dir.display()))?;

    for &fmt in formats {
        let (ext, content) = match fmt {
            Format::Markdown => ("md", render::to_markdown(&analyzed, &ropts)),
            Format::Json => ("json", render::to_json(&analyzed)),
            Format::Html => ("html", render::to_html(&analyzed)),
            Format::Text => ("txt", render::to_text(&analyzed)),
        };
        if cli.to_stdout {
            print!("{content}");
        } else {
            let path = out_dir.join(format!("{base}.{ext}"));
            std::fs::write(&path, content)
                .with_context(|| format!("writing {}", path.display()))?;
            if !cli.quiet {
                eprintln!("  wrote {}", path.display());
            }
        }
    }
    Ok(())
}

fn parse_formats(s: &str) -> Result<Vec<Format>> {
    let mut out = Vec::new();
    for part in s.split(',').map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty()) {
        let f = match part.as_str() {
            "markdown" | "md" => Format::Markdown,
            "json" => Format::Json,
            "html" => Format::Html,
            "text" | "txt" => Format::Text,
            other => bail!("unknown format '{other}' (use markdown, json, html, text)"),
        };
        if !out.contains(&f) {
            out.push(f);
        }
    }
    if out.is_empty() {
        bail!("no output format specified");
    }
    Ok(out)
}

/// Parse a page spec like "1,3,5-7" into a 1-indexed set.
pub fn parse_pages(s: &str) -> Result<BTreeSet<usize>> {
    let mut set = BTreeSet::new();
    for part in s.split(',').map(|p| p.trim()).filter(|p| !p.is_empty()) {
        if let Some((a, b)) = part.split_once('-') {
            let a: usize = a.trim().parse().with_context(|| format!("bad page '{part}'"))?;
            let b: usize = b.trim().parse().with_context(|| format!("bad page '{part}'"))?;
            if a == 0 || b < a {
                bail!("invalid page range '{part}'");
            }
            for p in a..=b {
                set.insert(p);
            }
        } else {
            let p: usize = part.parse().with_context(|| format!("bad page '{part}'"))?;
            if p == 0 {
                bail!("page numbers are 1-indexed");
            }
            set.insert(p);
        }
    }
    if set.is_empty() {
        bail!("empty page selection");
    }
    Ok(set)
}

/// Expand inputs (files or dirs) into a list of PDF files.
fn collect_pdfs(inputs: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for inp in inputs {
        if inp.is_dir() {
            collect_dir(inp, &mut out);
        } else if is_pdf(inp) {
            out.push(inp.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

fn collect_dir(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_dir(&p, out);
            } else if is_pdf(&p) {
                out.push(p);
            }
        }
    }
}

fn is_pdf(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("pdf")).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_parse() {
        let s = parse_pages("1,3,5-7").unwrap();
        assert_eq!(s.iter().copied().collect::<Vec<_>>(), vec![1, 3, 5, 6, 7]);
        assert!(parse_pages("0").is_err());
        assert!(parse_pages("5-2").is_err());
        assert!(parse_pages("x").is_err());
    }

    #[test]
    fn formats_parse() {
        assert_eq!(parse_formats("markdown,json").unwrap().len(), 2);
        assert!(parse_formats("bogus").is_err());
    }
}
