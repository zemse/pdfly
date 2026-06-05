//! Orchestration: extract -> analyze -> render -> write (single PDF).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};

use crate::analyze::{self, Options};
use crate::cli::ReadArgs;
use crate::extract::{LopdfBackend, PdfBackend};
use crate::render::{self, RenderOptions, split};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Format {
    Markdown,
    Json,
    Html,
    Text,
}

/// Run the `read` subcommand: convert one PDF to stdout (or `--out`).
pub fn run_read(args: &ReadArgs) -> Result<()> {
    let format = resolve_format(args.out.as_deref(), args.format.as_deref())?;
    let pages = match &args.pages {
        Some(s) => Some(parse_pages(s)?),
        None => None,
    };

    let file = &args.input;
    if !file.is_file() {
        bail!("input is not a file: {}", file.display());
    }

    let started = args.timing.then(Instant::now);
    let n = process_one(args, file, format, pages.as_ref())?;
    if let Some(started) = started {
        let secs = started.elapsed().as_secs_f64();
        eprintln!(
            "timing: {n} page(s) in {secs:.3}s ({:.1} pages/s)",
            pps(n, secs)
        );
    }
    Ok(())
}

/// Pages per second, guarding against a zero/negative elapsed time.
fn pps(pages: usize, secs: f64) -> f64 {
    if secs > 0.0 { pages as f64 / secs } else { 0.0 }
}

/// Process a single PDF; returns the number of pages analyzed (for throughput).
fn process_one(
    cli: &ReadArgs,
    file: &Path,
    format: Format,
    pages: Option<&BTreeSet<usize>>,
) -> Result<usize> {
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
            cluster_tables: cli.table_method == "cluster",
        },
    );

    let ropts = RenderOptions {
        page_separator: cli.page_separator.clone(),
        html_tables: cli.markdown_with_html,
    };

    let to_stdout = cli.out.is_none();

    // Side outputs need a destination on disk.
    if to_stdout {
        if cli.split {
            bail!("--split requires --out <dir>");
        }
        if cli.annotate {
            bail!("--annotate requires --out");
        }
        if cli.tagged_pdf {
            bail!("--tagged-pdf requires --out");
        }
    }
    if cli.split && format != Format::Markdown {
        bail!("--split only supports markdown output");
    }

    // Output directory + base name for images and side-car artifacts. With
    // --split, --out is itself the chapter directory; otherwise it is a file
    // whose parent/stem drive the naming. In stdout mode nothing is written,
    // but we still derive a sensible base from the input file.
    let input_base = file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output")
        .to_string();
    let (out_dir, base) = match (&cli.out, cli.split) {
        (Some(out), true) => (out.clone(), input_base),
        (Some(out), false) => {
            let dir = out
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            let base = out
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("output")
                .to_string();
            (dir, base)
        }
        (None, _) => (PathBuf::from("."), input_base),
    };

    // Resolve images (write files / embed / drop) before rendering.
    if !to_stdout {
        let mode = render::images::parse_mode(&cli.image_output);
        let image_dir = cli
            .image_dir
            .clone()
            .unwrap_or_else(|| out_dir.join(format!("{base}_images")));
        std::fs::create_dir_all(&out_dir).ok();
        let n = render::images::process_images(
            &doc,
            &mut analyzed,
            mode,
            &cli.image_format,
            &out_dir,
            &image_dir,
            &base,
        )
        .unwrap_or(0);
        if n > 0 && !cli.quiet {
            eprintln!("  extracted {n} image(s)");
        }
    } else {
        // stdout: drop images (can't reference files).
        analyzed
            .elements
            .retain(|e| !matches!(e, crate::model::Element::Image { .. }));
    }

    // Tagged PDF (structure tree).
    if cli.tagged_pdf {
        std::fs::create_dir_all(&out_dir).ok();
        let path = out_dir.join(format!("{base}.tagged.pdf"));
        match crate::render::tagged::write_tagged_pdf(
            file,
            cli.password.as_deref(),
            &analyzed,
            &path,
        ) {
            Ok(()) if !cli.quiet => eprintln!("  wrote {}", path.display()),
            Err(e) => eprintln!("  tagged-pdf failed: {e:#}"),
            _ => {}
        }
    }

    // Annotated debug PDF.
    if cli.annotate {
        std::fs::create_dir_all(&out_dir).ok();
        let path = out_dir.join(format!("{base}.annotated.pdf"));
        match crate::render::annotate::write_annotated(
            file,
            cli.password.as_deref(),
            &analyzed,
            &path,
        ) {
            Ok(()) if !cli.quiet => eprintln!("  wrote {}", path.display()),
            Err(e) => eprintln!("  annotate failed: {e:#}"),
            _ => {}
        }
    }

    // Chapter split (Markdown only): --out is the chapter directory.
    if cli.split {
        let dir = cli.out.as_ref().expect("--split requires --out (validated)");
        let chapters = split::split_markdown(&analyzed, cli.split_level.max(1), &ropts);
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        for ch in &chapters {
            std::fs::write(dir.join(&ch.filename), &ch.content)?;
        }
        let index = split::build_index(&chapters, analyzed.meta.title.as_deref());
        std::fs::write(dir.join("index.md"), index)?;
        if !cli.quiet {
            eprintln!(
                "wrote {} chapter file(s) -> {}",
                chapters.len(),
                dir.display()
            );
        }
        return Ok(analyzed.num_pages);
    }

    let content = match format {
        Format::Markdown => render::to_markdown(&analyzed, &ropts),
        Format::Json => render::to_json(&analyzed),
        Format::Html => render::to_html(&analyzed),
        Format::Text => render::to_text(&analyzed),
    };
    match &cli.out {
        None => print!("{content}"),
        Some(out) => {
            if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            std::fs::write(out, content).with_context(|| format!("writing {}", out.display()))?;
            if !cli.quiet {
                eprintln!("wrote {}", out.display());
            }
        }
    }
    Ok(analyzed.num_pages)
}

/// Resolve the single output format: explicit `--format` wins, else infer from
/// the `--out` file extension, else default to Markdown (stdout).
fn resolve_format(out: Option<&Path>, flag: Option<&str>) -> Result<Format> {
    if let Some(f) = flag {
        return parse_format(f);
    }
    if let Some(ext) = out.and_then(|p| p.extension()).and_then(|e| e.to_str()) {
        return parse_format(ext);
    }
    Ok(Format::Markdown)
}

fn parse_format(s: &str) -> Result<Format> {
    match s.trim().to_lowercase().as_str() {
        "markdown" | "md" => Ok(Format::Markdown),
        "json" => Ok(Format::Json),
        "html" | "htm" => Ok(Format::Html),
        "text" | "txt" => Ok(Format::Text),
        other => bail!("unknown format '{other}' (use markdown, json, html, text)"),
    }
}

/// Parse a page spec like "1,3,5-7" into a 1-indexed set.
pub fn parse_pages(s: &str) -> Result<BTreeSet<usize>> {
    let mut set = BTreeSet::new();
    for part in s.split(',').map(|p| p.trim()).filter(|p| !p.is_empty()) {
        if let Some((a, b)) = part.split_once('-') {
            let a: usize = a
                .trim()
                .parse()
                .with_context(|| format!("bad page '{part}'"))?;
            let b: usize = b
                .trim()
                .parse()
                .with_context(|| format!("bad page '{part}'"))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn pages_parse() {
        let s = parse_pages("1,3,5-7").unwrap();
        assert_eq!(s.iter().copied().collect::<Vec<_>>(), vec![1, 3, 5, 6, 7]);
        assert!(parse_pages("0").is_err());
        assert!(parse_pages("5-2").is_err());
        assert!(parse_pages("x").is_err());
    }

    #[test]
    fn format_parse() {
        assert!(parse_format("markdown").is_ok());
        assert!(parse_format("json").is_ok());
        assert!(parse_format("bogus").is_err());
    }

    #[test]
    fn format_resolution() {
        // explicit flag wins over extension
        assert_eq!(
            resolve_format(Some(Path::new("a.md")), Some("json")).unwrap(),
            Format::Json
        );
        // inferred from extension
        assert_eq!(
            resolve_format(Some(Path::new("a.json")), None).unwrap(),
            Format::Json
        );
        // default to markdown (stdout, no flag)
        assert_eq!(resolve_format(None, None).unwrap(), Format::Markdown);
    }
}
