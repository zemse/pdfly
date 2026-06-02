//! End-to-end analysis + rendering invariants (oracle: invariants + structure).

use std::path::Path;

use pdf_rs::analyze::{analyze, Options};
use pdf_rs::extract::{LopdfBackend, PdfBackend};
use pdf_rs::render::{self, split, RenderOptions};

fn corpus(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus").join(name)
}

fn analyze_file(name: &str) -> pdf_rs::model::AnalyzedDoc {
    let doc = LopdfBackend::load(&corpus(name), None, None).unwrap();
    analyze(&doc, &Options::default())
}

#[test]
fn lorem_markdown_has_heading_and_paragraph() {
    let a = analyze_file("lorem.pdf");
    let md = render::to_markdown(&a, &RenderOptions::default());
    assert!(md.starts_with("# Lorem Ipsum"), "title heading first:\n{md}");
    assert!(md.to_lowercase().contains("dolor sit amet"), "body present");
    // Heading then paragraph as distinct blocks.
    assert!(md.contains("\n\nLorem ipsum dolor"), "paragraph separated from heading");
}

#[test]
fn lorem_json_is_valid_and_structured() {
    let a = analyze_file("lorem.pdf");
    let js = render::to_json(&a);
    let v: serde_json::Value = serde_json::from_str(&js).unwrap();
    assert_eq!(v["number of pages"], 1);
    let kids = v["kids"].as_array().unwrap();
    assert!(kids.iter().any(|k| k["type"] == "heading"));
    assert!(kids.iter().any(|k| k["type"] == "paragraph"));
    // every element has a 4-number bounding box
    for k in kids {
        let bb = k["bounding box"].as_array().unwrap();
        assert_eq!(bb.len(), 4, "bbox is [l,b,r,t]");
    }
}

#[test]
fn book_chapter_splits_into_multiple_files() {
    let a = analyze_file("pdfua-1-reference-suite-1-1/PDFUA-Ref-2-08_BookChapter.pdf");
    let chapters = split::split_markdown(&a, 1, &RenderOptions::default());
    assert!(chapters.len() >= 2, "expected multiple chapters, got {}", chapters.len());
    // filenames are NN-slug.md and unique
    let mut names: Vec<&str> = chapters.iter().map(|c| c.filename.as_str()).collect();
    let count = names.len();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), count, "chapter filenames unique");
    let index = split::build_index(&chapters, a.meta.title.as_deref());
    assert!(index.contains("](01-"), "index links to first chapter");
}

#[test]
fn images_extract_to_files() {
    use pdf_rs::render::images::{process_images, ImageMode};
    let doc = LopdfBackend::load(
        &corpus("pdfua-1-reference-suite-1-1/PDFUA-Ref-2-01_Magazine-danish.pdf"),
        None,
        None,
    )
    .unwrap();
    let mut a = analyze(&doc, &Options::default());
    let tmp = std::env::temp_dir().join("pdfrs_img_test");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let n = process_images(&doc, &mut a, ImageMode::External, "png", &tmp, &tmp, "mag").unwrap();
    assert!(n > 0, "expected images extracted");
    let files: Vec<_> = std::fs::read_dir(&tmp).unwrap().flatten().collect();
    assert!(!files.is_empty(), "image files written");
}

#[test]
fn html_and_text_render_without_panic() {
    let a = analyze_file("lorem.pdf");
    let html = render::to_html(&a);
    assert!(html.contains("<h1>") && html.contains("</body>"));
    let txt = render::to_text(&a);
    assert!(txt.contains("Lorem"));
}
