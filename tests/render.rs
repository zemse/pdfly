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
fn markdown_html_table_mode() {
    use pdf_rs::extract::{PdfMeta, Rect};
    use pdf_rs::model::{AnalyzedDoc, Cell, Element};
    let doc = AnalyzedDoc {
        meta: PdfMeta::default(),
        num_pages: 1,
        elements: vec![Element::Table {
            rows: vec![
                vec![Cell { text: "A".into(), col_span: 2, row_span: 1 }],
                vec![
                    Cell { text: "b".into(), col_span: 1, row_span: 1 },
                    Cell { text: "c".into(), col_span: 1, row_span: 1 },
                ],
            ],
            bbox: Rect::new(0.0, 0.0, 10.0, 10.0),
            page: 1,
        }],
    };
    let opts = RenderOptions { html_tables: true, ..Default::default() };
    let md = render::to_markdown(&doc, &opts);
    assert!(md.contains("<table>") && md.contains("colspan=\"2\""), "html table with span:\n{md}");

    let gfm = render::to_markdown(&doc, &RenderOptions::default());
    assert!(gfm.contains("| --- |"), "default is GFM pipe table");
}

#[test]
fn hidden_text_is_filtered_by_content_safety() {
    use pdf_rs::extract::{Document, Page, PdfMeta, Rect, TextRun};
    let run = |text: &str, hidden: bool| TextRun {
        text: text.into(),
        bbox: Rect::new(50.0, 700.0, 150.0, 712.0),
        font_size: 10.0,
        font_name: "F".into(),
        bold: false,
        italic: false,
        color: [0.0; 3],
        mcid: None,
        hidden,
    };
    let doc = Document {
        meta: PdfMeta::default(),
        structure: None,
        pages: vec![Page {
            number: 1,
            media_box: Rect::new(0.0, 0.0, 600.0, 800.0),
            runs: vec![run("Visible text here", false), run("inject hidden", true)],
            images: vec![],
            lines: vec![],
            image_data: Default::default(),
        }],
    };
    let on = analyze(&doc, &Options { content_safety: true, ..Default::default() });
    let md_on = render::to_markdown(&on, &RenderOptions::default());
    assert!(md_on.contains("Visible") && !md_on.contains("hidden"), "hidden dropped: {md_on}");

    let off = analyze(&doc, &Options { content_safety: false, ..Default::default() });
    let md_off = render::to_markdown(&off, &RenderOptions::default());
    assert!(md_off.contains("hidden"), "hidden kept when safety off");
}

#[test]
fn tagged_pdf_has_structure_tree() {
    let a = analyze_file("lorem.pdf");
    let tmp = std::env::temp_dir().join("pdfrs_tagged.pdf");
    pdf_rs::render::tagged::write_tagged_pdf(&corpus("lorem.pdf"), None, &a, &tmp).unwrap();
    let bytes = std::fs::read(&tmp).unwrap();
    assert!(bytes.windows(14).any(|w| w == b"StructTreeRoot"), "has struct tree root");
    // Re-loadable.
    let again = LopdfBackend::load(&tmp, None, None).unwrap();
    assert!(!again.pages.is_empty());
}

#[test]
fn annotated_pdf_is_written_and_loadable() {
    let a = analyze_file("lorem.pdf");
    let tmp = std::env::temp_dir().join("pdfrs_annotated.pdf");
    pdf_rs::render::annotate::write_annotated(&corpus("lorem.pdf"), None, &a, &tmp).unwrap();
    let again = LopdfBackend::load(&tmp, None, None).unwrap();
    assert_eq!(again.pages.len(), 1);
}

#[test]
fn html_and_text_render_without_panic() {
    let a = analyze_file("lorem.pdf");
    let html = render::to_html(&a);
    assert!(html.contains("<h1>") && html.contains("</body>"));
    let txt = render::to_text(&a);
    assert!(txt.contains("Lorem"));
}
