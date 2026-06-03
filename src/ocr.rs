//! Optional OCR for scanned PDFs (`--features ocr`, pure-Rust via ocrs/rten).
//!
//! Scanned pages are typically a single full-page image XObject, so we OCR the
//! decoded image directly (no page rasterizer needed) and inject the recognized
//! words back as positioned [`TextRun`]s, mapping image pixels to PDF points
//! via the image's placement box.
//!
//! Without the `ocr` feature this is a no-op. With it, model files are located
//! via `$PDFRS_OCR_DETECTION_MODEL` / `$PDFRS_OCR_RECOGNITION_MODEL` (`.rten`
//! files); if unset/missing, OCR is skipped with a warning.

use crate::extract::Document;

/// A page is considered "needs OCR" when it has almost no extractable text but
/// contains a large image.
#[cfg_attr(not(feature = "ocr"), allow(dead_code))]
fn page_needs_ocr(page: &crate::extract::Page) -> Option<String> {
    let text_chars: usize = page.runs.iter().map(|r| r.text.trim().len()).sum();
    if text_chars > 40 {
        return None;
    }
    let page_area = page.media_box.area().max(1.0);
    // Largest image covering >= 40% of the page.
    page.images
        .iter()
        .filter(|im| im.bbox.area() / page_area >= 0.40)
        .max_by(|a, b| a.bbox.area().partial_cmp(&b.bbox.area()).unwrap())
        .map(|im| im.name.clone())
}

#[cfg(not(feature = "ocr"))]
pub fn augment(_doc: &mut Document) -> anyhow::Result<usize> {
    Ok(0)
}

#[cfg(feature = "ocr")]
pub fn augment(doc: &mut Document) -> anyhow::Result<usize> {
    use crate::extract::{ImageData, Rect, TextRun};
    use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};

    let (Some(det_path), Some(rec_path)) = (
        std::env::var("PDFRS_OCR_DETECTION_MODEL").ok(),
        std::env::var("PDFRS_OCR_RECOGNITION_MODEL").ok(),
    ) else {
        eprintln!(
            "ocr: set PDFRS_OCR_DETECTION_MODEL and PDFRS_OCR_RECOGNITION_MODEL to .rten files; skipping OCR"
        );
        return Ok(0);
    };

    let detection_model = rten::Model::load_file(&det_path)?;
    let recognition_model = rten::Model::load_file(&rec_path)?;
    let engine = OcrEngine::new(OcrEngineParams {
        detection_model: Some(detection_model),
        recognition_model: Some(recognition_model),
        ..Default::default()
    })?;

    let mut added = 0usize;
    for page in &mut doc.pages {
        let Some(name) = page_needs_ocr(page) else {
            continue;
        };
        let Some(data) = page.image_data.get(&name).cloned() else {
            continue;
        };
        // Decode to RGB pixels.
        let rgb = match data {
            ImageData::Rgba {
                width,
                height,
                data,
            } => image::RgbaImage::from_raw(width, height, data).map(|i| {
                let d = image::DynamicImage::ImageRgba8(i).to_rgb8();
                (d.width(), d.height(), d.into_raw())
            }),
            ImageData::Jpeg(bytes) => image::load_from_memory(&bytes).ok().map(|i| {
                let d = i.to_rgb8();
                (d.width(), d.height(), d.into_raw())
            }),
        };
        let Some((iw, ih, pixels)) = rgb else {
            continue;
        };

        let src = ImageSource::from_bytes(&pixels, (iw, ih))?;
        let input = engine.prepare_input(src)?;
        let words = engine.detect_words(&input)?;
        let lines = engine.find_text_lines(&input, &words);
        let texts = engine.recognize_text(&input, &lines)?;

        // Placement box of this image in PDF user space.
        let pb = page
            .images
            .iter()
            .find(|im| im.name == name)
            .map(|im| im.bbox)
            .unwrap_or(page.media_box);
        let sx = pb.width() / iw as f64;
        let sy = pb.height() / ih as f64;

        for line in texts.into_iter().flatten() {
            let s = line.to_string();
            if s.trim().is_empty() {
                continue;
            }
            let r = line.bounding_rect();
            // image pixels: y grows downward; PDF: y grows upward.
            let left = pb.left + r.left() as f64 * sx;
            let right = pb.left + r.right() as f64 * sx;
            let top = pb.top - r.top() as f64 * sy;
            let bottom = pb.top - r.bottom() as f64 * sy;
            let bbox = Rect::new(left, bottom.min(top), right, bottom.max(top));
            page.runs.push(TextRun {
                text: s,
                font_size: (bbox.height()).max(6.0),
                bbox,
                font_name: "OCR".into(),
                bold: false,
                italic: false,
                color: [0.0, 0.0, 0.0],
                mcid: None,
                hidden: false,
            });
            added += 1;
        }
    }
    Ok(added)
}
