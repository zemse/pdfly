//! Snapshot + determinism regression (self-consistent oracle).

use std::path::Path;

use pdfly::analyze::{Options, analyze};
use pdfly::extract::{LopdfBackend, PdfBackend};
use pdfly::render::{self, RenderOptions};

fn corpus(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/corpus")
        .join(name)
}

fn md_with(name: &str, threads: usize) -> String {
    let doc = LopdfBackend::load(&corpus(name), None, None).unwrap();
    let a = analyze(
        &doc,
        &Options {
            threads,
            ..Default::default()
        },
    );
    render::to_markdown(&a, &RenderOptions::default())
}

#[test]
fn lorem_markdown_snapshot() {
    insta::assert_snapshot!("lorem_md", md_with("lorem.pdf", 1));
}

#[test]
fn output_is_thread_independent() {
    for name in ["lorem.pdf", "2408.02509v1.pdf", "1901.03003.pdf"] {
        assert_eq!(
            md_with(name, 1),
            md_with(name, 4),
            "{name}: threads must not change output"
        );
    }
}
