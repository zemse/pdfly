//! Pragmatic font decoding: map raw string bytes from content streams to
//! Unicode + glyph advance widths.
//!
//! Strategy (in order of reliability):
//!  1. `/ToUnicode` CMap — used when present (most modern PDFs have it).
//!  2. Simple-font `/Encoding` (WinAnsi base + `/Differences`).
//!  3. A Type1 (`/FontFile`) program's built-in `/Encoding`, for symbolic
//!     fonts with a non-standard encoding and no `/ToUnicode`.
//!  4. Raw byte fallback (Latin-1-ish).
//!
//! Widths come from `/Widths` (simple) or `/W`+`/DW` (Type0/CID).

use std::collections::HashMap;

use lopdf::{Dictionary, Document, Object};
use regex::Regex;

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
        let descriptor = font_dict
            .get(b"FontDescriptor")
            .ok()
            .and_then(|fd| resolve(doc, fd).ok())
            .and_then(|o| o.as_dict().ok().cloned());

        // A Type1 (`/FontFile`) program carries its own built-in `/Encoding`
        // (code -> glyph name) in its clear-text header. When the PDF doesn't
        // pin a base encoding by name, that built-in encoding is authoritative
        // — needed for symbolic Type1 fonts with a non-standard encoding and no
        // `/ToUnicode`.
        let type1_builtin = descriptor
            .as_ref()
            .and_then(|fd| type1_font_program(doc, fd))
            .map(|prog| parse_type1_builtin_encoding(&prog))
            .filter(|m| !m.is_empty());

        let mut enc = build_simple_encoding(doc, font_dict, type1_builtin.as_ref());

        if let Some(fd) = &descriptor {
            apply_descriptor_flags_dict(fd, &mut font);
            // Recover code->unicode from an embedded TrueType/CFF program when
            // there is no ToUnicode (fills remaining gaps only).
            if font.to_unicode.is_none() {
                if let Some(prog) = font_program(doc, fd) {
                    if let Some(emb) = load_embedded(&prog) {
                        for (code, ch) in emb.code_to_unicode {
                            enc.entry(code).or_insert(ch);
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

/// Build byte->char map for a simple font from its `/Encoding`, layering (low
/// to high priority): WinAnsi base, the Type1 program's built-in encoding (when
/// the PDF doesn't pin a base encoding by name), then `/Differences`.
fn build_simple_encoding(
    doc: &Document,
    font_dict: &Dictionary,
    type1_builtin: Option<&HashMap<u8, char>>,
) -> HashMap<u8, char> {
    let mut map = winansi_base();

    // Resolve the PDF /Encoding entry once (may be a base name and/or a dict
    // with /BaseEncoding and /Differences).
    let enc_obj = font_dict.get(b"Encoding").ok().and_then(|e| resolve(doc, e).ok().cloned());
    let base_name_pinned = match &enc_obj {
        Some(Object::Name(_)) => true,
        Some(Object::Dictionary(d)) => d.get(b"BaseEncoding").is_ok(),
        _ => false,
    };

    // Built-in Type1 encoding overrides the WinAnsi base, but only when the PDF
    // didn't pin a base encoding by name (a named base takes precedence).
    if !base_name_pinned {
        if let Some(builtin) = type1_builtin {
            for (&code, &ch) in builtin {
                map.insert(code, ch);
            }
        }
    }

    // /Differences always win.
    if let Some(Object::Dictionary(d)) = &enc_obj {
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
    map
}

/// The Type1 font program (`/FontFile`), distinct from FontFile2/3 (which are
/// TrueType/CFF and handled by [`font_program`] + ttf-parser).
fn type1_font_program(doc: &Document, descriptor: &Dictionary) -> Option<Vec<u8>> {
    let r = descriptor.get(b"FontFile").ok()?;
    resolve_stream(doc, r)
}

/// Parse a Type1 font program's built-in `/Encoding` into code -> char.
///
/// The encoding lives in the program's clear-text header (before the `eexec`
/// binary section) as either the keyword `StandardEncoding` (nothing custom to
/// record — the WinAnsi/Standard base table covers it) or a sequence of
/// `dup <code> /<glyphname> put` entries.
fn parse_type1_builtin_encoding(bytes: &[u8]) -> HashMap<u8, char> {
    let mut map = HashMap::new();
    let clear_end = find_subslice(bytes, b"eexec").unwrap_or(bytes.len());
    let text = String::from_utf8_lossy(&bytes[..clear_end]);

    if let Some(pos) = text.find("/Encoding") {
        let rest = text[pos + "/Encoding".len()..].trim_start();
        if rest.starts_with("StandardEncoding") {
            return map;
        }
    }

    let re = Regex::new(r"dup\s+(\d+)\s*/([^\s/(){}\[\]<>]+)\s+put").unwrap();
    for cap in re.captures_iter(&text) {
        if let Ok(code) = cap[1].parse::<u32>() {
            if code <= 0xFF {
                if let Some(ch) = glyph_name_to_char(&cap[2]) {
                    map.insert(code as u8, ch);
                }
            }
        }
    }
    map
}

/// Index of the first occurrence of `needle` in `hay`.
fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
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
    // uXXXX / uniXXXX handled above; also accept "gNN"/"cidNN" -> no unicode.
    let c = match name {
        // whitespace & punctuation
        "space" | "nbspace" => ' ',
        "period" => '.',
        "comma" => ',',
        "colon" => ':',
        "semicolon" => ';',
        "exclam" => '!',
        "question" => '?',
        "quotesingle" => '\'',
        "quotedbl" => '"',
        "grave" => '`',
        "asciitilde" => '~',
        "asciicircum" => '^',
        "underscore" => '_',
        "hyphen" | "sfthyphen" => '-',
        "endash" => '\u{2013}',
        "emdash" => '\u{2014}',
        "quoteleft" => '\u{2018}',
        "quoteright" => '\u{2019}',
        "quotedblleft" => '\u{201C}',
        "quotedblright" => '\u{201D}',
        "quotesinglbase" => '\u{201A}',
        "quotedblbase" => '\u{201E}',
        "bullet" => '\u{2022}',
        "dagger" => '\u{2020}',
        "daggerdbl" => '\u{2021}',
        "ellipsis" => '\u{2026}',
        "parenleft" => '(',
        "parenright" => ')',
        "bracketleft" => '[',
        "bracketright" => ']',
        "braceleft" => '{',
        "braceright" => '}',
        "slash" => '/',
        "backslash" => '\\',
        "bar" => '|',
        "at" => '@',
        "numbersign" => '#',
        "dollar" => '$',
        "percent" => '%',
        "ampersand" => '&',
        "asterisk" => '*',
        "plus" => '+',
        "equal" => '=',
        "less" => '<',
        "greater" => '>',
        "degree" => '\u{00B0}',
        "euro" => '\u{20AC}',
        "sterling" => '\u{00A3}',
        "cent" => '\u{00A2}',
        "yen" => '\u{00A5}',
        "section" => '\u{00A7}',
        "paragraph" => '\u{00B6}',
        "copyright" => '\u{00A9}',
        "registered" => '\u{00AE}',
        "trademark" => '\u{2122}',
        "periodcentered" => '\u{00B7}',
        "guillemotleft" => '\u{00AB}',
        "guillemotright" => '\u{00BB}',
        "guilsinglleft" => '\u{2039}',
        "guilsinglright" => '\u{203A}',
        "minus" => '\u{2212}',
        "fraction" => '\u{2044}',
        // ligatures
        "fi" => '\u{FB01}',
        "fl" => '\u{FB02}',
        "ff" => '\u{FB00}',
        "ffi" => '\u{FB03}',
        "ffl" => '\u{FB04}',
        // number words
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
        _ => return single_letter_glyph(name),
    };
    Some(c)
}

/// Adobe convention: a single ASCII letter glyph is named by that letter
/// ("A".."z"); accented letters like "eacute" map via a small table.
fn single_letter_glyph(name: &str) -> Option<char> {
    let mut chars = name.chars();
    let first = chars.next()?;
    if chars.next().is_none() && first.is_ascii_alphabetic() {
        return Some(first);
    }
    let c = match name {
        "aacute" => 'á', "agrave" => 'à', "acircumflex" => 'â', "atilde" => 'ã',
        "adieresis" => 'ä', "aring" => 'å', "ae" => 'æ', "ccedilla" => 'ç',
        "eacute" => 'é', "egrave" => 'è', "ecircumflex" => 'ê', "edieresis" => 'ë',
        "iacute" => 'í', "igrave" => 'ì', "icircumflex" => 'î', "idieresis" => 'ï',
        "ntilde" => 'ñ', "oacute" => 'ó', "ograve" => 'ò', "ocircumflex" => 'ô',
        "otilde" => 'õ', "odieresis" => 'ö', "oslash" => 'ø', "oe" => 'œ',
        "uacute" => 'ú', "ugrave" => 'ù', "ucircumflex" => 'û', "udieresis" => 'ü',
        "yacute" => 'ý', "ydieresis" => 'ÿ', "germandbls" => 'ß',
        "Aacute" => 'Á', "Agrave" => 'À', "Adieresis" => 'Ä', "Ccedilla" => 'Ç',
        "Eacute" => 'É', "Egrave" => 'È', "Edieresis" => 'Ë', "Ntilde" => 'Ñ',
        "Oacute" => 'Ó', "Odieresis" => 'Ö', "Oslash" => 'Ø', "Udieresis" => 'Ü',
        _ => return None,
    };
    Some(c)
}

/// A token from a CMap stream.
#[derive(Debug, Clone)]
enum CMapTok {
    Hex(String),
    Word(String),
    ArrayOpen,
    ArrayClose,
}

/// Tokenize a CMap: `<..>` hex strings (which may be packed with no spaces),
/// `[`/`]`, and bare keywords. Robust to missing whitespace between groups.
fn tokenize_cmap(data: &[u8]) -> Vec<CMapTok> {
    let s = String::from_utf8_lossy(data);
    let bytes = s.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'<' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b'>' {
                j += 1;
            }
            toks.push(CMapTok::Hex(s[i + 1..j.min(bytes.len())].to_string()));
            i = j + 1;
        } else if c == b'[' {
            toks.push(CMapTok::ArrayOpen);
            i += 1;
        } else if c == b']' {
            toks.push(CMapTok::ArrayClose);
            i += 1;
        } else if c.is_ascii_alphabetic() {
            let mut j = i;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric()) {
                j += 1;
            }
            toks.push(CMapTok::Word(s[i..j].to_string()));
            i = j;
        } else {
            i += 1;
        }
    }
    toks
}

/// Parse a ToUnicode CMap into code -> unicode string.
fn parse_to_unicode(data: &[u8]) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    let toks = tokenize_cmap(data);
    let mut i = 0;
    while i < toks.len() {
        match &toks[i] {
            CMapTok::Word(w) if w == "beginbfchar" => {
                i += 1;
                while i + 1 < toks.len() && !matches!(&toks[i], CMapTok::Word(w) if w == "endbfchar") {
                    if let (CMapTok::Hex(code), CMapTok::Hex(dst)) = (&toks[i], &toks[i + 1]) {
                        if let (Some(code), Some(dst)) = (hex_code(code), hex_utf16(dst)) {
                            map.insert(code, dst);
                        }
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
            }
            CMapTok::Word(w) if w == "beginbfrange" => {
                i += 1;
                while i < toks.len() && !matches!(&toks[i], CMapTok::Word(w) if w == "endbfrange") {
                    // <lo> <hi> ( <dst> | [ <d0> <d1> ... ] )
                    if i + 2 < toks.len() {
                        if let (CMapTok::Hex(lo), CMapTok::Hex(hi)) = (&toks[i], &toks[i + 1]) {
                            let (lo, hi) = (hex_code(lo), hex_code(hi));
                            match &toks[i + 2] {
                                CMapTok::Hex(dst) => {
                                    if let (Some(lo), Some(hi), Some(dst)) = (lo, hi, hex_utf16(dst)) {
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
                                }
                                CMapTok::ArrayOpen => {
                                    let mut code = lo.unwrap_or(0);
                                    i += 3;
                                    while i < toks.len() && !matches!(&toks[i], CMapTok::ArrayClose) {
                                        if let CMapTok::Hex(dst) = &toks[i] {
                                            if let Some(s) = hex_utf16(dst) {
                                                map.insert(code, s);
                                            }
                                            code += 1;
                                        }
                                        i += 1;
                                    }
                                    i += 1; // ArrayClose
                                }
                                _ => i += 1,
                            }
                        } else {
                            i += 1;
                        }
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

fn hex_code(h: &str) -> Option<u32> {
    u32::from_str_radix(h.trim(), 16).ok()
}

fn hex_utf16(h: &str) -> Option<String> {
    let h = h.trim();
    if h.is_empty() || h.len() % 2 != 0 {
        return None;
    }
    let bytes: Vec<u8> =
        (0..h.len()).step_by(2).filter_map(|i| u8::from_str_radix(&h[i..i + 2], 16).ok()).collect();
    let units: Vec<u16> = bytes
        .chunks(2)
        .map(|c| if c.len() == 2 { ((c[0] as u16) << 8) | c[1] as u16 } else { c[0] as u16 })
        .collect();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type1_builtin_encoding_parsed() {
        // Clear-text header of a Type1 program with a custom encoding array,
        // followed by the eexec binary section (which must be ignored).
        let prog: &[u8] = b"/Encoding 256 array\n\
            0 1 255 {1 index exch /.notdef put} for\n\
            dup 65 /A put\n\
            dup 97 /a put\n\
            dup 32 /space put\n\
            dup 200 /eacute put\n\
            readonly def\n\
            eexec\n\x80\x01\x02\x03 dup 1 /ignored put";
        let m = parse_type1_builtin_encoding(prog);
        assert_eq!(m.get(&65), Some(&'A'));
        assert_eq!(m.get(&97), Some(&'a'));
        assert_eq!(m.get(&32), Some(&' '));
        assert_eq!(m.get(&200), Some(&'é'));
        // Entries after `eexec` are not parsed.
        assert_eq!(m.get(&1), None);
        // `.notdef` has no Unicode mapping and is dropped.
        assert!(!m.values().any(|&c| c == '\u{0}'));
    }

    #[test]
    fn type1_standard_encoding_yields_empty() {
        let prog: &[u8] = b"/Encoding StandardEncoding def\neexec\n";
        assert!(parse_type1_builtin_encoding(prog).is_empty());
    }

    #[test]
    fn find_subslice_basic() {
        assert_eq!(find_subslice(b"abcdef", b"cd"), Some(2));
        assert_eq!(find_subslice(b"abcdef", b"xy"), None);
        assert_eq!(find_subslice(b"ab", b"abc"), None);
    }
}
