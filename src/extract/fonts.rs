//! Pragmatic font decoding: map raw string bytes from content streams to
//! Unicode + glyph advance widths.
//!
//! Strategy (in order of reliability):
//!  1. `/ToUnicode` CMap — used when present (most modern PDFs have it).
//!  2. Simple-font `/Encoding` (WinAnsi base + `/Differences`).
//!  3. Raw byte fallback (Latin-1-ish).
//!
//! Widths come from `/Widths` (simple) or `/W`+`/DW` (Type0/CID).

use std::collections::HashMap;

use lopdf::{Dictionary, Document, Object};

#[derive(Clone, Debug)]
pub struct Font {
    pub base_font: String,
    pub bold: bool,
    pub italic: bool,
    /// Type0 (CID) fonts use 2-byte codes (Identity-H).
    pub two_byte: bool,
    to_unicode: Option<HashMap<u32, String>>,
    /// Simple-font byte -> char from base encoding + Differences.
    encoding: Option<HashMap<u8, char>>,
    /// CID/GID -> unicode recovered from an embedded font program (when there
    /// is no `/ToUnicode`). Used for Type0/Identity fonts.
    cid_unicode: Option<HashMap<u32, char>>,
    /// code/cid -> width in 1000-unit glyph space.
    widths: HashMap<u32, f64>,
    default_width: f64,
}

impl Default for Font {
    fn default() -> Self {
        Font {
            base_font: String::new(),
            bold: false,
            italic: false,
            two_byte: false,
            to_unicode: None,
            encoding: None,
            cid_unicode: None,
            widths: HashMap::new(),
            default_width: 500.0,
        }
    }
}

impl Font {
    /// Decode a raw PDF string into (unicode text, total advance in glyph units / 1000).
    /// `space_count` reports how many single-byte 0x20 codes were seen (for word spacing).
    pub fn decode(&self, bytes: &[u8]) -> DecodedRun {
        let mut text = String::new();
        let mut total_width = 0.0;
        let mut space_count = 0usize;

        let codes: Vec<u32> = if self.two_byte {
            bytes
                .chunks(2)
                .map(|c| if c.len() == 2 { ((c[0] as u32) << 8) | c[1] as u32 } else { c[0] as u32 })
                .collect()
        } else {
            bytes.iter().map(|&b| b as u32).collect()
        };

        for &code in &codes {
            // text
            if let Some(s) = self.to_unicode.as_ref().and_then(|tu| tu.get(&code)) {
                text.push_str(s);
            } else if let Some(&ch) = self.cid_unicode.as_ref().and_then(|m| m.get(&code)) {
                text.push(ch);
            } else {
                self.push_via_encoding(code, &mut text);
            }
            // width
            let w = self.widths.get(&code).copied().unwrap_or(self.default_width);
            total_width += w;
            if !self.two_byte && code == 0x20 {
                space_count += 1;
            }
        }

        DecodedRun { text, width_units: total_width, space_count }
    }

    fn push_via_encoding(&self, code: u32, text: &mut String) {
        if !self.two_byte && code <= 0xFF {
            if let Some(enc) = &self.encoding {
                if let Some(&ch) = enc.get(&(code as u8)) {
                    text.push(ch);
                    return;
                }
            }
            // Latin-1 fallback for printable range.
            if let Some(ch) = char::from_u32(code) {
                if !ch.is_control() {
                    text.push(ch);
                }
            }
        }
        // Unknown 2-byte code with no ToUnicode: skip (avoid garbage).
    }
}

pub struct DecodedRun {
    pub text: String,
    pub width_units: f64,
    pub space_count: usize,
}

/// Build a [`Font`] from a font dictionary.
pub fn build_font(doc: &Document, font_dict: &Dictionary) -> Font {
    let mut font = Font::default();

    let subtype = font_dict.get(b"Subtype").ok().and_then(name_string).unwrap_or_default();
    font.base_font = font_dict.get(b"BaseFont").ok().and_then(name_string).unwrap_or_default();
    let bf_lower = font.base_font.to_lowercase();
    font.bold = bf_lower.contains("bold");
    font.italic = bf_lower.contains("italic") || bf_lower.contains("oblique");

    // ToUnicode (works for both simple and Type0).
    if let Ok(tu_ref) = font_dict.get(b"ToUnicode") {
        if let Some(data) = resolve_stream(doc, tu_ref) {
            font.to_unicode = Some(parse_to_unicode(&data));
        }
    }

    if subtype == "Type0" {
        font.two_byte = true; // assume Identity-H (the overwhelmingly common case)
        // Descendant font holds widths + descriptor.
        if let Some(desc) = font_dict
            .get(b"DescendantFonts")
            .ok()
            .and_then(|o| first_array_dict(doc, o))
        {
            font.default_width = desc.get(b"DW").ok().and_then(fnum).unwrap_or(1000.0);
            if let Ok(w) = desc.get(b"W") {
                if let Ok(arr) = resolve(doc, w).and_then(|o| o.as_array().map(|a| a.to_vec())) {
                    parse_cid_widths(&arr, &mut font.widths);
                }
            }
            apply_descriptor_flags(doc, &desc, &mut font);
            if font.to_unicode.is_none() {
                if let Some(prog) = descriptor_font_program(doc, &desc) {
                    if let Some(emb) = load_embedded(&prog) {
                        if !emb.gid_to_unicode.is_empty() {
                            font.cid_unicode = Some(emb.gid_to_unicode);
                        }
                    }
                }
            }
        }
    } else {
        // Simple font: byte codes.
        let first_char =
            font_dict.get(b"FirstChar").ok().and_then(|o| o.as_i64().ok()).unwrap_or(0) as u32;
        if let Ok(w) = font_dict.get(b"Widths") {
            if let Ok(arr) = resolve(doc, w).and_then(|o| o.as_array().map(|a| a.to_vec())) {
                for (i, item) in arr.iter().enumerate() {
                    if let Some(width) = resolve(doc, item).ok().and_then(fnum) {
                        font.widths.insert(first_char + i as u32, width);
                    }
                }
            }
        }
        let mut enc = build_simple_encoding(doc, font_dict);
        if let Ok(fd) = font_dict.get(b"FontDescriptor") {
            if let Ok(fd) = resolve(doc, fd).and_then(|o| o.as_dict().map(|d| d.clone())) {
                apply_descriptor_flags_dict(&fd, &mut font);
                // Recover code->unicode from the embedded program when there is
                // no ToUnicode and no explicit Differences for those codes.
                if font.to_unicode.is_none() {
                    if let Some(prog) = font_program(doc, &fd) {
                        if let Some(emb) = load_embedded(&prog) {
                            for (code, ch) in emb.code_to_unicode {
                                enc.entry(code).or_insert(ch);
                            }
                        }
                    }
                }
            }
        }
        font.encoding = Some(enc);
    }

    font
}

/// Embedded-font maps recovered via ttf-parser (TrueType / OpenType-CFF).
struct Embedded {
    gid_to_unicode: HashMap<u32, char>,
    code_to_unicode: HashMap<u8, char>,
}

fn descriptor_font_program(doc: &Document, descendant: &Dictionary) -> Option<Vec<u8>> {
    let fd = descendant.get(b"FontDescriptor").ok()?;
    let fd = resolve(doc, fd).ok()?.as_dict().ok()?.clone();
    font_program(doc, &fd)
}

fn font_program(doc: &Document, descriptor: &Dictionary) -> Option<Vec<u8>> {
    for key in [b"FontFile2".as_slice(), b"FontFile3".as_slice()] {
        if let Ok(r) = descriptor.get(key) {
            if let Some(data) = resolve_stream(doc, r) {
                return Some(data);
            }
        }
    }
    None
}

fn load_embedded(bytes: &[u8]) -> Option<Embedded> {
    let face = ttf_parser::Face::parse(bytes, 0).ok()?;
    let mut gid_to_unicode: HashMap<u32, char> = HashMap::new();
    let cmap = face.tables().cmap?;

    // Reverse map gid -> unicode from any Unicode subtable.
    for sub in cmap.subtables {
        if sub.is_unicode() {
            sub.codepoints(|cp| {
                if let Some(gid) = sub.glyph_index(cp) {
                    if let Some(ch) = char::from_u32(cp) {
                        gid_to_unicode.entry(gid.0 as u32).or_insert(ch);
                    }
                }
            });
        }
    }
    // Glyph-name fallback (post table -> AGL) for gids still unmapped.
    let num = face.number_of_glyphs();
    for gid in 0..num {
        if let std::collections::hash_map::Entry::Vacant(e) = gid_to_unicode.entry(gid as u32) {
            if let Some(name) = face.glyph_name(ttf_parser::GlyphId(gid)) {
                if let Some(ch) = glyph_name_to_char(name) {
                    e.insert(ch);
                }
            }
        }
    }

    // Simple-font code -> unicode via a builtin (symbol/mac) subtable.
    let mut code_to_unicode: HashMap<u8, char> = HashMap::new();
    for sub in cmap.subtables {
        for code in 0u32..=255 {
            let gid = sub.glyph_index(code).or_else(|| sub.glyph_index(0xF000 + code));
            if let Some(gid) = gid {
                if let Some(&ch) = gid_to_unicode.get(&(gid.0 as u32)) {
                    code_to_unicode.entry(code as u8).or_insert(ch);
                }
            }
        }
    }

    Some(Embedded { gid_to_unicode, code_to_unicode })
}

fn apply_descriptor_flags(doc: &Document, desc: &Dictionary, font: &mut Font) {
    if let Ok(fd) = desc.get(b"FontDescriptor") {
        if let Ok(fd) = resolve(doc, fd).and_then(|o| o.as_dict().map(|d| d.clone())) {
            apply_descriptor_flags_dict(&fd, font);
        }
    }
}

fn apply_descriptor_flags_dict(fd: &Dictionary, font: &mut Font) {
    if let Ok(flags) = fd.get(b"Flags").and_then(|o| o.as_i64().map(|v| v)) {
        // Bit 7 (0x40) = italic; bit 19 weight is not in flags but ForceBold bit 18 (0x40000).
        if flags & 0x40 != 0 {
            font.italic = true;
        }
        if flags & 0x4_0000 != 0 {
            font.bold = true;
        }
    }
    if let Some(stemv) = fd.get(b"StemV").ok().and_then(fnum) {
        if stemv >= 120.0 {
            font.bold = true;
        }
    }
}

fn parse_cid_widths(arr: &[Object], out: &mut HashMap<u32, f64>) {
    let mut i = 0;
    while i < arr.len() {
        let c = match arr[i].as_i64() {
            Ok(v) => v as u32,
            Err(_) => {
                i += 1;
                continue;
            }
        };
        if i + 1 < arr.len() {
            match &arr[i + 1] {
                // c [w1 w2 ...]
                Object::Array(list) => {
                    for (k, w) in list.iter().enumerate() {
                        if let Some(width) = fnum(w) {
                            out.insert(c + k as u32, width);
                        }
                    }
                    i += 2;
                }
                // c_first c_last w
                _ => {
                    if i + 2 < arr.len() {
                        if let (Ok(c_last), Some(width)) = (arr[i + 1].as_i64(), fnum(&arr[i + 2])) {
                            for cid in c..=(c_last as u32) {
                                out.insert(cid, width);
                            }
                        }
                        i += 3;
                    } else {
                        i += 1;
                    }
                }
            }
        } else {
            break;
        }
    }
}

/// Build byte->char map for a simple font from its `/Encoding`.
fn build_simple_encoding(doc: &Document, font_dict: &Dictionary) -> HashMap<u8, char> {
    let mut map = winansi_base();
    if let Ok(enc) = font_dict.get(b"Encoding") {
        match resolve(doc, enc) {
            Ok(Object::Name(_)) => { /* base name; WinAnsi base is fine for our fallback */ }
            Ok(Object::Dictionary(d)) => {
                if let Ok(diffs) = d.get(b"Differences").and_then(|o| o.as_array()) {
                    let mut code: u32 = 0;
                    for item in diffs {
                        match item {
                            Object::Integer(n) => code = *n as u32,
                            Object::Name(name) => {
                                if let Some(ch) = glyph_name_to_char(&String::from_utf8_lossy(name)) {
                                    if code <= 0xFF {
                                        map.insert(code as u8, ch);
                                    }
                                }
                                code += 1;
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }
    map
}

/// Minimal WinAnsi/Latin-1 base table for the printable range.
fn winansi_base() -> HashMap<u8, char> {
    let mut m = HashMap::new();
    for b in 0x20u8..=0x7E {
        m.insert(b, b as char);
    }
    for b in 0xA0u8..=0xFF {
        m.insert(b, b as char);
    }
    // A few common Windows-1252 high glyphs.
    for (b, ch) in [
        (0x91u8, '\u{2018}'),
        (0x92, '\u{2019}'),
        (0x93, '\u{201C}'),
        (0x94, '\u{201D}'),
        (0x95, '\u{2022}'),
        (0x96, '\u{2013}'),
        (0x97, '\u{2014}'),
        (0x85, '\u{2026}'),
    ] {
        m.insert(b, ch);
    }
    m
}

/// Map a handful of common glyph names + `uniXXXX` to chars.
fn glyph_name_to_char(name: &str) -> Option<char> {
    if let Some(hex) = name.strip_prefix("uni") {
        if let Ok(cp) = u32::from_str_radix(hex, 16) {
            return char::from_u32(cp);
        }
    }
    let c = match name {
        "space" => ' ',
        "period" => '.',
        "comma" => ',',
        "colon" => ':',
        "semicolon" => ';',
        "hyphen" => '-',
        "endash" => '\u{2013}',
        "emdash" => '\u{2014}',
        "quoteleft" => '\u{2018}',
        "quoteright" => '\u{2019}',
        "quotedblleft" => '\u{201C}',
        "quotedblright" => '\u{201D}',
        "bullet" => '\u{2022}',
        "parenleft" => '(',
        "parenright" => ')',
        "slash" => '/',
        "ellipsis" => '\u{2026}',
        "zero" => '0',
        "one" => '1',
        "two" => '2',
        "three" => '3',
        "four" => '4',
        "five" => '5',
        "six" => '6',
        "seven" => '7',
        "eight" => '8',
        "nine" => '9',
        _ => return None,
    };
    Some(c)
}

/// Parse a ToUnicode CMap into code -> unicode string.
fn parse_to_unicode(data: &[u8]) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    let s = String::from_utf8_lossy(data);
    let tokens: Vec<&str> = s.split_whitespace().collect();

    let mut i = 0;
    while i < tokens.len() {
        match tokens[i] {
            "beginbfchar" => {
                i += 1;
                while i < tokens.len() && tokens[i] != "endbfchar" {
                    if i + 1 < tokens.len() {
                        if let (Some(code), Some(dst)) =
                            (parse_hex_code(tokens[i]), parse_hex_utf16(tokens[i + 1]))
                        {
                            map.insert(code, dst);
                        }
                        i += 2;
                    } else {
                        break;
                    }
                }
            }
            "beginbfrange" => {
                i += 1;
                while i < tokens.len() && tokens[i] != "endbfrange" {
                    // <lo> <hi> <dst>  — array dst not handled (rare); skip those gracefully.
                    if i + 2 < tokens.len() {
                        let lo = parse_hex_code(tokens[i]);
                        let hi = parse_hex_code(tokens[i + 1]);
                        if tokens[i + 2].starts_with('[') {
                            // array form: advance past array
                            i += 2;
                            while i < tokens.len() && !tokens[i].ends_with(']') {
                                i += 1;
                            }
                            i += 1;
                            continue;
                        }
                        let dst = parse_hex_utf16(tokens[i + 2]);
                        if let (Some(lo), Some(hi), Some(dst)) = (lo, hi, dst) {
                            if let Some(first) = dst.chars().next() {
                                let base = first as u32;
                                for (k, code) in (lo..=hi).enumerate() {
                                    if let Some(ch) = char::from_u32(base + k as u32) {
                                        map.insert(code, ch.to_string());
                                    }
                                }
                            }
                        }
                        i += 3;
                    } else {
                        break;
                    }
                }
            }
            _ => i += 1,
        }
    }
    map
}

fn parse_hex_code(tok: &str) -> Option<u32> {
    let h = tok.trim_start_matches('<').trim_end_matches('>');
    u32::from_str_radix(h, 16).ok()
}

fn parse_hex_utf16(tok: &str) -> Option<String> {
    let h = tok.trim_start_matches('<').trim_end_matches('>');
    if h.len() % 2 != 0 {
        return None;
    }
    let bytes: Vec<u8> = (0..h.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&h[i..i + 2], 16).ok())
        .collect();
    let units: Vec<u16> =
        bytes.chunks(2).map(|c| if c.len() == 2 { ((c[0] as u16) << 8) | c[1] as u16 } else { c[0] as u16 }).collect();
    Some(String::from_utf16_lossy(&units))
}

// ---- lopdf helpers ----

fn resolve<'a>(doc: &'a Document, obj: &'a Object) -> Result<&'a Object, lopdf::Error> {
    match obj {
        Object::Reference(id) => doc.get_object(*id),
        other => Ok(other),
    }
}

fn resolve_stream(doc: &Document, obj: &Object) -> Option<Vec<u8>> {
    let resolved = resolve(doc, obj).ok()?;
    if let Object::Stream(stream) = resolved {
        let mut s = stream.clone();
        let _ = s.decompress();
        return Some(s.content);
    }
    None
}

fn first_array_dict(doc: &Document, obj: &Object) -> Option<Dictionary> {
    let arr = resolve(doc, obj).ok()?.as_array().ok()?;
    let first = arr.first()?;
    resolve(doc, first).ok()?.as_dict().ok().cloned()
}

/// Numeric value of an Object whether stored as Integer or Real.
pub(crate) fn fnum(o: &Object) -> Option<f64> {
    match o {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(*r as f64),
        _ => None,
    }
}

fn name_string(o: &Object) -> Option<String> {
    match o {
        Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
        _ => None,
    }
}
