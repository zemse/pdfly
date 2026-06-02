use std::path::PathBuf;

use pdf_rs::extract::{LopdfBackend, PdfBackend};

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).map(PathBuf::from).expect("usage: pdf-rs <file.pdf>");
    let doc = LopdfBackend::load(&path, None, None)?;
    println!("meta: {:?}", doc.meta);
    for page in &doc.pages {
        println!(
            "--- page {} ({} runs, {} lines, {} images) media={:?}",
            page.number,
            page.runs.len(),
            page.lines.len(),
            page.images.len(),
            page.media_box
        );
        for r in page.runs.iter().take(12) {
            println!(
                "  [{:.0},{:.0},{:.0},{:.0}] sz={:.1} {}{}  {:?}",
                r.bbox.left,
                r.bbox.bottom,
                r.bbox.right,
                r.bbox.top,
                r.font_size,
                if r.bold { "B" } else { "" },
                if r.italic { "I" } else { "" },
                r.text
            );
        }
    }
    Ok(())
}
