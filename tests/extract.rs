//! Extraction-layer invariants against the committed corpus (oracle: snapshot + invariants).

use std::collections::BTreeSet;
use std::path::Path;

use pdf_rs::extract::{LopdfBackend, PdfBackend};

fn corpus(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/corpus")
        .join(name)
}

#[test]
fn lorem_extracts_title_and_body() {
    let doc = LopdfBackend::load(&corpus("lorem.pdf"), None, None).unwrap();
    assert_eq!(doc.pages.len(), 1);
    let page = &doc.pages[0];
    assert!(
        page.runs.len() > 20,
        "expected many runs, got {}",
        page.runs.len()
    );

    let all_text: String = page.runs.iter().map(|r| r.text.as_str()).collect();
    assert!(
        all_text.contains("Lorem Ipsum") || all_text.contains("Lorem Ipsum"),
        "title text present"
    );
    assert!(
        all_text.to_lowercase().contains("dolor sit amet"),
        "body text present"
    );

    // The title must be in a clearly larger font than the body (heading signal).
    let max_size = page.runs.iter().map(|r| r.font_size).fold(0.0, f64::max);
    let body_size = page
        .runs
        .iter()
        .map(|r| r.font_size)
        .fold(f64::MAX, f64::min);
    assert!(
        max_size > body_size * 2.0,
        "title font {max_size} should dwarf body {body_size}"
    );
}

#[test]
fn page_selection_limits_pages() {
    // Multi-page arXiv paper: ask for only page 1.
    let mut sel = BTreeSet::new();
    sel.insert(1usize);
    let doc = LopdfBackend::load(&corpus("2408.02509v1.pdf"), None, Some(&sel)).unwrap();
    assert_eq!(doc.pages.len(), 1);
    assert_eq!(doc.pages[0].number, 1);
}

#[test]
fn all_corpus_pdfs_extract_without_panic() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");
    let mut count = 0;
    for entry in walk(&dir) {
        if entry.extension().and_then(|e| e.to_str()) == Some("pdf") {
            count += 1;
            let res = LopdfBackend::load(&entry, None, None);
            assert!(
                res.is_ok(),
                "failed to load {}: {:?}",
                entry.display(),
                res.err()
            );
        }
    }
    assert!(
        count >= 10,
        "expected the bundled corpus, found {count} pdfs"
    );
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}
