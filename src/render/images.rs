//! Image extraction: decode image XObjects to files (external), base64 data
//! URIs (embedded), or drop them (off). Rewrites each `Element::Image.name`
//! to the resolved reference so the text renderers stay oblivious.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

use crate::extract::{Document, ImageData};
use crate::model::{AnalyzedDoc, Element};

#[derive(Clone, Copy, PartialEq)]
pub enum ImageMode {
    Off,
    Embedded,
    External,
}

pub fn parse_mode(s: &str) -> ImageMode {
    match s.to_lowercase().as_str() {
        "off" => ImageMode::Off,
        "embedded" => ImageMode::Embedded,
        _ => ImageMode::External,
    }
}

/// Resolve image references in `analyzed`. For `External`, files are written to
/// `image_dir` and links are made relative to `out_dir`.
pub fn process_images(
    doc: &Document,
    analyzed: &mut AnalyzedDoc,
    mode: ImageMode,
    format: &str,
    out_dir: &Path,
    image_dir: &Path,
    base: &str,
) -> Result<usize> {
    if mode == ImageMode::Off {
        analyzed
            .elements
            .retain(|e| !matches!(e, Element::Image { .. }));
        return Ok(0);
    }

    // page -> name -> ImageData lookup.
    let mut lookup: HashMap<(usize, &str), &ImageData> = HashMap::new();
    for page in &doc.pages {
        for (name, data) in &page.image_data {
            lookup.insert((page.number, name.as_str()), data);
        }
    }

    let mut resolved: HashMap<(usize, String), String> = HashMap::new();
    let mut count = 0usize;
    let mut written = false;

    for el in &mut analyzed.elements {
        if let Element::Image { name, page, .. } = el {
            let key = (*page, name.clone());
            if let Some(link) = resolved.get(&key) {
                *name = link.clone();
                continue;
            }
            let Some(data) = lookup.get(&(*page, name.as_str())).copied() else {
                // Image XObject couldn't be decoded (unsupported filter/colorspace,
                // e.g. JBIG2/CCITT/JPEG2000/Indexed). Mark it for removal rather
                // than emitting a broken `![]()` link.
                *name = String::new();
                continue;
            };
            count += 1;
            let (ext, bytes) = encode(data, format);
            let link = match mode {
                ImageMode::Embedded => {
                    let mime = if ext == "jpg" {
                        "image/jpeg"
                    } else {
                        "image/png"
                    };
                    format!("data:{};base64,{}", mime, base64(&bytes))
                }
                ImageMode::External => {
                    if !written {
                        std::fs::create_dir_all(image_dir)?;
                        written = true;
                    }
                    let fname = format!("{base}_img{count}.{ext}");
                    std::fs::write(image_dir.join(&fname), &bytes)?;
                    rel_link(out_dir, image_dir, &fname)
                }
                ImageMode::Off => unreachable!(),
            };
            resolved.insert(key, link.clone());
            *name = link;
        }
    }
    // Drop images that couldn't be resolved to a link (undecodable XObjects),
    // so renderers never emit an empty `![]()`.
    analyzed
        .elements
        .retain(|e| !matches!(e, Element::Image { name, .. } if name.is_empty()));
    Ok(count)
}

fn encode(data: &ImageData, format: &str) -> (&'static str, Vec<u8>) {
    match data {
        // Already JPEG: pass through regardless of requested format.
        ImageData::Jpeg(b) => ("jpg", b.clone()),
        ImageData::Rgba {
            width,
            height,
            data,
        } => {
            let img = image::RgbaImage::from_raw(*width, *height, data.clone());
            if let Some(img) = img {
                let mut buf = std::io::Cursor::new(Vec::new());
                if format == "jpeg" {
                    let rgb = image::DynamicImage::ImageRgba8(img).to_rgb8();
                    if rgb.write_to(&mut buf, image::ImageFormat::Jpeg).is_ok() {
                        return ("jpg", buf.into_inner());
                    }
                } else if img.write_to(&mut buf, image::ImageFormat::Png).is_ok() {
                    return ("png", buf.into_inner());
                }
            }
            ("png", Vec::new())
        }
    }
}

fn rel_link(out_dir: &Path, image_dir: &Path, fname: &str) -> String {
    // Relative path from out_dir to the image file, for portable Markdown links.
    let full = image_dir.join(fname);
    let rel = pathdiff(&full, out_dir).unwrap_or_else(|| full.clone());
    let s = rel.to_string_lossy().replace('\\', "/");
    if s.contains(' ') { format!("<{s}>") } else { s }
}

/// Minimal relative-path computation (no external crate).
fn pathdiff(target: &Path, base: &Path) -> Option<std::path::PathBuf> {
    let t: Vec<_> = target.components().collect();
    let b: Vec<_> = base.components().collect();
    let mut i = 0;
    while i < t.len() && i < b.len() && t[i] == b[i] {
        i += 1;
    }
    let mut out = std::path::PathBuf::new();
    for _ in i..b.len() {
        out.push("..");
    }
    for c in &t[i..] {
        out.push(c.as_os_str());
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Standard base64 (no line wrapping).
fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
