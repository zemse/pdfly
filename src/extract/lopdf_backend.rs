//! `lopdf`-based extraction backend: a content-stream interpreter that tracks
//! the graphics + text state to recover positioned text runs, vector lines,
//! and placed images in PDF user space.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use anyhow::{Context, Result};
use lopdf::content::Content;
use lopdf::{Dictionary, Document, Object, ObjectId};

use super::fonts::{build_font, Font};
use super::matrix::Matrix;
use super::{ImageObj, LineSeg, Page, PdfMeta, Rect, TextRun};

pub struct LopdfBackend;

impl super::PdfBackend for LopdfBackend {
    fn load(
        path: &Path,
        password: Option<&str>,
        pages: Option<&BTreeSet<usize>>,
    ) -> Result<super::Document> {
        let mut doc = Document::load(path).with_context(|| format!("loading {}", path.display()))?;
        if doc.is_encrypted() {
            let pw = password.unwrap_or("");
            // Best-effort decryption with the supplied (or empty) password.
            let _ = doc.decrypt(pw);
        }

        let meta = read_meta(&doc);
        let page_index = page_number_map(&doc);
        let structure = parse_structure(&doc, &page_index);
        let mut out_pages = Vec::new();
        for (page_num, page_id) in doc.get_pages() {
            let n = page_num as usize;
            if let Some(sel) = pages {
                if !sel.contains(&n) {
                    continue;
                }
            }
            match extract_page(&doc, n, page_id) {
                Ok(p) => out_pages.push(p),
                Err(e) => {
                    eprintln!("warning: page {n} failed: {e}");
                    out_pages.push(Page {
                        number: n,
                        media_box: Rect::new(0.0, 0.0, 612.0, 792.0),
                        runs: vec![],
                        images: vec![],
                        lines: vec![],
                        image_data: HashMap::new(),
                    });
                }
            }
        }
        Ok(super::Document { meta, pages: out_pages, structure })
    }
}

/// Map each page's ObjectId to its 1-indexed page number.
fn page_number_map(doc: &Document) -> HashMap<ObjectId, usize> {
    doc.get_pages().into_iter().map(|(n, id)| (id, n as usize)).collect()
}

/// Parse the logical structure tree (StructTreeRoot) if present.
fn parse_structure(doc: &Document, page_index: &HashMap<ObjectId, usize>) -> Option<super::StructElem> {
    let root = doc.catalog().ok()?;
    let str_ref = root.get(b"StructTreeRoot").ok()?;
    let str_root = resolve(doc, str_ref).ok()?.as_dict().ok()?.clone();
    let mut kids = Vec::new();
    collect_kids(doc, &str_root, None, page_index, &mut kids, 0);
    Some(super::StructElem { tag: "Document".into(), alt: None, mcids: vec![], kids })
}

fn collect_kids(
    doc: &Document,
    parent: &Dictionary,
    inherited_pg: Option<ObjectId>,
    page_index: &HashMap<ObjectId, usize>,
    out: &mut Vec<super::StructElem>,
    depth: usize,
) {
    if depth > 50 {
        return;
    }
    let pg = parent
        .get(b"Pg")
        .ok()
        .and_then(|o| if let Object::Reference(id) = o { Some(*id) } else { None })
        .or(inherited_pg);
    let Ok(k) = parent.get(b"K") else { return };
    let items = flatten_k(doc, k);
    for item in items {
        match item {
            // Direct MCID integer -> belongs to the parent element.
            Object::Integer(_n) => { /* handled by build_elem reading parent's own mcids */ }
            Object::Dictionary(d) => {
                if let Ok(Object::Name(s)) = d.get(b"S") {
                    // Child structure element.
                    let tag = String::from_utf8_lossy(s).into_owned();
                    let alt = d
                        .get(b"Alt")
                        .ok()
                        .and_then(|o| if let Object::String(b, _) = o { Some(decode_pdf_text(b)) } else { None });
                    let mcids = gather_own_mcids(doc, &d, pg, page_index);
                    let mut child_kids = Vec::new();
                    collect_kids(doc, &d, pg, page_index, &mut child_kids, depth + 1);
                    out.push(super::StructElem { tag, alt, mcids, kids: child_kids });
                }
            }
            Object::Reference(id) => {
                if let Ok(d) = doc.get_dictionary(id).map(|d| d.clone()) {
                    if let Ok(Object::Name(s)) = d.get(b"S") {
                        let tag = String::from_utf8_lossy(s).into_owned();
                        let alt = d.get(b"Alt").ok().and_then(|o| {
                            if let Object::String(b, _) = o { Some(decode_pdf_text(b)) } else { None }
                        });
                        let mcids = gather_own_mcids(doc, &d, pg, page_index);
                        let mut child_kids = Vec::new();
                        collect_kids(doc, &d, pg, page_index, &mut child_kids, depth + 1);
                        out.push(super::StructElem { tag, alt, mcids, kids: child_kids });
                    }
                }
            }
            _ => {}
        }
    }
}

/// MCIDs owned directly by an element (integer kids, or MCR dicts).
fn gather_own_mcids(
    doc: &Document,
    elem: &Dictionary,
    inherited_pg: Option<ObjectId>,
    page_index: &HashMap<ObjectId, usize>,
) -> Vec<(usize, i32)> {
    let mut out = Vec::new();
    // Prefer this element's own /Pg over the inherited one.
    let pg = elem
        .get(b"Pg")
        .ok()
        .and_then(|o| if let Object::Reference(id) = o { Some(*id) } else { None })
        .or(inherited_pg);
    let Ok(k) = elem.get(b"K") else { return out };
    for item in flatten_k(doc, k) {
        match item {
            Object::Integer(n) => {
                if let Some(p) = pg.and_then(|id| page_index.get(&id)) {
                    out.push((*p, n as i32));
                }
            }
            Object::Dictionary(d) => {
                if d.get(b"Type").ok().and_then(name_of) == Some("MCR".to_string()) {
                    let mpg = d
                        .get(b"Pg")
                        .ok()
                        .and_then(|o| if let Object::Reference(id) = o { Some(*id) } else { None })
                        .or(pg);
                    if let (Ok(Object::Integer(n)), Some(p)) =
                        (d.get(b"MCID"), mpg.and_then(|id| page_index.get(&id)))
                    {
                        out.push((*p, *n as i32));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn name_of(o: &Object) -> Option<String> {
    if let Object::Name(n) = o {
        Some(String::from_utf8_lossy(n).into_owned())
    } else {
        None
    }
}

/// Normalize /K into a flat list of owned `Object`s (resolving arrays).
fn flatten_k(doc: &Document, k: &Object) -> Vec<Object> {
    match k {
        Object::Array(a) => a.iter().map(|o| resolve(doc, o).cloned().unwrap_or(Object::Null)).collect(),
        Object::Reference(_) => vec![resolve(doc, k).cloned().unwrap_or(Object::Null)],
        other => vec![other.clone()],
    }
}

fn read_meta(doc: &Document) -> PdfMeta {
    let mut meta = PdfMeta::default();
    if let Ok(info_ref) = doc.trailer.get(b"Info") {
        if let Ok(info) = resolve(doc, info_ref).and_then(|o| o.as_dict().map(|d| d.clone())) {
            meta.title = text_field(&info, b"Title");
            meta.author = text_field(&info, b"Author");
            meta.creation_date = text_field(&info, b"CreationDate");
            meta.modification_date = text_field(&info, b"ModDate");
        }
    }
    meta
}

fn text_field(d: &Dictionary, key: &[u8]) -> Option<String> {
    d.get(key).ok().and_then(|o| match o {
        Object::String(bytes, _) => Some(decode_pdf_text(bytes)),
        _ => None,
    })
}

/// Decode a PDF text string (UTF-16BE if BOM present, else PdfDocEncoding≈Latin1).
fn decode_pdf_text(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let units: Vec<u16> = bytes[2..]
            .chunks(2)
            .map(|c| if c.len() == 2 { ((c[0] as u16) << 8) | c[1] as u16 } else { c[0] as u16 })
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        bytes.iter().map(|&b| b as char).collect()
    }
}

fn extract_page(doc: &Document, number: usize, page_id: ObjectId) -> Result<Page> {
    let media_box = page_media_box(doc, page_id);
    let fonts = page_fonts(doc, page_id);
    let (xobjects, image_data) = page_images(doc, page_id);

    let content_data = doc.get_page_content(page_id).context("get_page_content")?;
    let content = Content::decode(&content_data).context("decode content stream")?;

    let mut interp = Interp::new(&fonts, &xobjects);
    interp.run(&content.operations);

    Ok(Page {
        number,
        media_box,
        runs: interp.runs,
        images: interp.images,
        lines: interp.lines,
        image_data,
    })
}

/// Graphics + text state, mutated as we walk operations.
struct Interp<'a> {
    fonts: &'a HashMap<String, Font>,
    xobjects: &'a HashMap<String, ()>,
    // graphics
    ctm: Matrix,
    fill_color: [f64; 3],
    gs_stack: Vec<(Matrix, [f64; 3])>,
    // text
    tm: Matrix,
    tlm: Matrix,
    font: Option<Font>,
    font_name: String,
    font_size: f64,
    char_spacing: f64,
    word_spacing: f64,
    h_scale: f64, // 1.0 == 100%
    leading: f64,
    rise: f64,
    render_mode: i64,
    // marked content (tagged PDFs)
    mcid_stack: Vec<Option<i32>>,
    // path
    cur: (f64, f64),
    subpath_start: (f64, f64),
    pending_lines: Vec<LineSeg>,
    // output
    runs: Vec<TextRun>,
    images: Vec<ImageObj>,
    lines: Vec<LineSeg>,
}

impl<'a> Interp<'a> {
    fn new(fonts: &'a HashMap<String, Font>, xobjects: &'a HashMap<String, ()>) -> Self {
        Interp {
            fonts,
            xobjects,
            ctm: Matrix::identity(),
            fill_color: [0.0, 0.0, 0.0],
            gs_stack: Vec::new(),
            tm: Matrix::identity(),
            tlm: Matrix::identity(),
            font: None,
            font_name: String::new(),
            font_size: 0.0,
            char_spacing: 0.0,
            word_spacing: 0.0,
            h_scale: 1.0,
            leading: 0.0,
            rise: 0.0,
            render_mode: 0,
            mcid_stack: Vec::new(),
            cur: (0.0, 0.0),
            subpath_start: (0.0, 0.0),
            pending_lines: Vec::new(),
            runs: Vec::new(),
            images: Vec::new(),
            lines: Vec::new(),
        }
    }

    fn run(&mut self, ops: &[lopdf::content::Operation]) {
        for op in ops {
            let o = op.operator.as_str();
            let a = &op.operands;
            match o {
                "q" => self.gs_stack.push((self.ctm, self.fill_color)),
                "Q" => {
                    if let Some((ctm, fc)) = self.gs_stack.pop() {
                        self.ctm = ctm;
                        self.fill_color = fc;
                    }
                }
                "cm" => {
                    if let Some(m) = matrix6(a) {
                        self.ctm = m.mul(&self.ctm);
                    }
                }
                // text object
                "BT" => {
                    self.tm = Matrix::identity();
                    self.tlm = Matrix::identity();
                }
                "ET" => {}
                "Td" => {
                    if let (Some(x), Some(y)) = (num(a, 0), num(a, 1)) {
                        self.tlm = Matrix::translation(x, y).mul(&self.tlm);
                        self.tm = self.tlm;
                    }
                }
                "TD" => {
                    if let (Some(x), Some(y)) = (num(a, 0), num(a, 1)) {
                        self.leading = -y;
                        self.tlm = Matrix::translation(x, y).mul(&self.tlm);
                        self.tm = self.tlm;
                    }
                }
                "Tm" => {
                    if let Some(m) = matrix6(a) {
                        self.tm = m;
                        self.tlm = m;
                    }
                }
                "T*" => self.next_line(),
                "Tc" => self.char_spacing = num(a, 0).unwrap_or(0.0),
                "Tw" => self.word_spacing = num(a, 0).unwrap_or(0.0),
                "Tz" => self.h_scale = num(a, 0).unwrap_or(100.0) / 100.0,
                "TL" => self.leading = num(a, 0).unwrap_or(0.0),
                "Ts" => self.rise = num(a, 0).unwrap_or(0.0),
                "Tr" => self.render_mode = num(a, 0).unwrap_or(0.0) as i64,
                "Tf" => self.set_font(a),
                "Tj" => {
                    if let Some(Object::String(bytes, _)) = a.first() {
                        self.show_text(bytes);
                    }
                }
                "'" => {
                    self.next_line();
                    if let Some(Object::String(bytes, _)) = a.first() {
                        self.show_text(bytes);
                    }
                }
                "\"" => {
                    self.word_spacing = num(a, 0).unwrap_or(0.0);
                    self.char_spacing = num(a, 1).unwrap_or(0.0);
                    self.next_line();
                    if let Some(Object::String(bytes, _)) = a.get(2) {
                        self.show_text(bytes);
                    }
                }
                "TJ" => {
                    if let Some(Object::Array(arr)) = a.first() {
                        self.show_tj(arr);
                    }
                }
                // color (fill)
                "rg" => {
                    if let (Some(r), Some(g), Some(b)) = (num(a, 0), num(a, 1), num(a, 2)) {
                        self.fill_color = [r, g, b];
                    }
                }
                "g" => {
                    if let Some(v) = num(a, 0) {
                        self.fill_color = [v, v, v];
                    }
                }
                "k" => {
                    if let (Some(c), Some(m), Some(y), Some(kk)) =
                        (num(a, 0), num(a, 1), num(a, 2), num(a, 3))
                    {
                        self.fill_color = cmyk_to_rgb(c, m, y, kk);
                    }
                }
                "sc" | "scn" => self.set_color_generic(a),
                // path construction
                "m" => {
                    if let (Some(x), Some(y)) = (num(a, 0), num(a, 1)) {
                        let p = self.ctm.apply(x, y);
                        self.cur = p;
                        self.subpath_start = p;
                    }
                }
                "l" => {
                    if let (Some(x), Some(y)) = (num(a, 0), num(a, 1)) {
                        let p = self.ctm.apply(x, y);
                        self.pending_lines.push(seg(self.cur, p));
                        self.cur = p;
                    }
                }
                "re" => {
                    if let (Some(x), Some(y), Some(w), Some(h)) =
                        (num(a, 0), num(a, 1), num(a, 2), num(a, 3))
                    {
                        let p0 = self.ctm.apply(x, y);
                        let p1 = self.ctm.apply(x + w, y);
                        let p2 = self.ctm.apply(x + w, y + h);
                        let p3 = self.ctm.apply(x, y + h);
                        self.pending_lines.push(seg(p0, p1));
                        self.pending_lines.push(seg(p1, p2));
                        self.pending_lines.push(seg(p2, p3));
                        self.pending_lines.push(seg(p3, p0));
                        self.cur = p0;
                        self.subpath_start = p0;
                    }
                }
                "h" => {
                    self.pending_lines.push(seg(self.cur, self.subpath_start));
                    self.cur = self.subpath_start;
                }
                "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" => {
                    // path painted -> commit thin axis-aligned segments as lines
                    self.commit_path();
                }
                "n" => self.pending_lines.clear(),
                // marked content
                "BDC" | "BMC" => self.mcid_stack.push(extract_mcid(a)),
                "EMC" => {
                    self.mcid_stack.pop();
                }
                // images
                "Do" => self.do_xobject(a),
                _ => {}
            }
        }
    }

    fn next_line(&mut self) {
        self.tlm = Matrix::translation(0.0, -self.leading).mul(&self.tlm);
        self.tm = self.tlm;
    }

    fn set_font(&mut self, a: &[Object]) {
        if let Some(Object::Name(name)) = a.first() {
            self.font_name = String::from_utf8_lossy(name).into_owned();
            self.font = self.fonts.get(&self.font_name).cloned();
        }
        self.font_size = num(a, 1).unwrap_or(self.font_size);
    }

    fn set_color_generic(&mut self, a: &[Object]) {
        let nums: Vec<f64> = a.iter().filter_map(super::fonts::fnum).collect();
        match nums.len() {
            1 => self.fill_color = [nums[0], nums[0], nums[0]],
            3 => self.fill_color = [nums[0], nums[1], nums[2]],
            4 => self.fill_color = cmyk_to_rgb(nums[0], nums[1], nums[2], nums[3]),
            _ => {}
        }
    }

    fn show_tj(&mut self, arr: &[Object]) {
        // Accumulate the whole TJ array into one run.
        let font = match &self.font {
            Some(f) => f.clone(),
            None => return,
        };
        let tfs = self.font_size;
        let th = self.h_scale;
        let start_tm = self.tm;
        let mut tx_total = 0.0;
        let mut text = String::new();

        // A large negative TJ adjustment is a positioning-based word space.
        const SPACE_ADJ: f64 = -120.0;
        for item in arr {
            match item {
                Object::String(bytes, _) => {
                    let dec = font.decode(bytes);
                    text.push_str(&dec.text);
                    let w = dec.width_units / 1000.0 * tfs;
                    let spaces = dec.space_count as f64 * self.word_spacing;
                    let chars = num_chars(&dec.text) as f64 * self.char_spacing;
                    tx_total += (w + chars + spaces) * th;
                }
                Object::Integer(n) => {
                    if (*n as f64) < SPACE_ADJ && !text.ends_with(' ') && !text.is_empty() {
                        text.push(' ');
                    }
                    tx_total -= (*n as f64 / 1000.0) * tfs * th;
                }
                Object::Real(n) => {
                    if (*n as f64) < SPACE_ADJ && !text.ends_with(' ') && !text.is_empty() {
                        text.push(' ');
                    }
                    tx_total -= (*n as f64 / 1000.0) * tfs * th;
                }
                _ => {}
            }
        }
        self.emit_run(&font, &text, start_tm, tx_total, tfs, th);
        self.tm = Matrix::translation(tx_total, 0.0).mul(&start_tm);
    }

    fn show_text(&mut self, bytes: &[u8]) {
        let font = match &self.font {
            Some(f) => f.clone(),
            None => return,
        };
        let tfs = self.font_size;
        let th = self.h_scale;
        let start_tm = self.tm;
        let dec = font.decode(bytes);
        let w = dec.width_units / 1000.0 * tfs;
        let spaces = dec.space_count as f64 * self.word_spacing;
        let chars = num_chars(&dec.text) as f64 * self.char_spacing;
        let tx_total = (w + chars + spaces) * th;
        self.emit_run(&font, &dec.text, start_tm, tx_total, tfs, th);
        self.tm = Matrix::translation(tx_total, 0.0).mul(&start_tm);
    }

    fn emit_run(&mut self, font: &Font, text: &str, start_tm: Matrix, tx: f64, tfs: f64, _th: f64) {
        let trimmed = text.trim_end_matches('\u{0}');
        if trimmed.trim().is_empty() {
            return;
        }
        let m = start_tm.mul(&self.ctm);
        let ascent = tfs * 0.75 + self.rise;
        let descent = -tfs * 0.25 + self.rise;
        let mut bbox = Rect::empty();
        for (x, y) in [(0.0, descent), (tx, descent), (tx, ascent), (0.0, ascent)] {
            let (px, py) = m.apply(x, y);
            bbox.include_point(px, py);
        }
        let font_size_user = tfs * m.scale_y();
        self.runs.push(TextRun {
            text: text.to_string(),
            bbox,
            font_size: font_size_user,
            font_name: font.base_font.clone(),
            bold: font.bold,
            italic: font.italic,
            color: self.fill_color,
            mcid: self.mcid_stack.iter().rev().find_map(|m| *m),
            hidden: self.render_mode == 3 || self.render_mode == 7,
        });
    }

    fn commit_path(&mut self) {
        for s in self.pending_lines.drain(..) {
            if s.is_horizontal() || s.is_vertical() {
                self.lines.push(s);
            }
        }
    }

    fn do_xobject(&mut self, a: &[Object]) {
        if let Some(Object::Name(name)) = a.first() {
            let nm = String::from_utf8_lossy(name).into_owned();
            if self.xobjects.contains_key(&nm) {
                // Image XObjects are drawn in the unit square under the CTM.
                let mut bbox = Rect::empty();
                for (x, y) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
                    let (px, py) = self.ctm.apply(x, y);
                    bbox.include_point(px, py);
                }
                if bbox.area() > 1.0 {
                    self.images.push(ImageObj { bbox, name: nm });
                }
            }
        }
    }
}

/// Extract /MCID from BDC operands `[/Tag <</MCID n ...>>]`.
fn extract_mcid(a: &[Object]) -> Option<i32> {
    if let Some(Object::Dictionary(d)) = a.get(1) {
        if let Ok(Object::Integer(n)) = d.get(b"MCID") {
            return Some(*n as i32);
        }
    }
    None
}

fn seg(p0: (f64, f64), p1: (f64, f64)) -> LineSeg {
    LineSeg { x0: p0.0, y0: p0.1, x1: p1.0, y1: p1.1 }
}

fn num(a: &[Object], i: usize) -> Option<f64> {
    a.get(i).and_then(super::fonts::fnum)
}

fn matrix6(a: &[Object]) -> Option<Matrix> {
    Some(Matrix::new(num(a, 0)?, num(a, 1)?, num(a, 2)?, num(a, 3)?, num(a, 4)?, num(a, 5)?))
}

fn num_chars(s: &str) -> usize {
    s.chars().count()
}

fn cmyk_to_rgb(c: f64, m: f64, y: f64, k: f64) -> [f64; 3] {
    [(1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)]
}

// ---- page resource helpers ----

fn resolve<'a>(doc: &'a Document, obj: &'a Object) -> Result<&'a Object, lopdf::Error> {
    match obj {
        Object::Reference(id) => doc.get_object(*id),
        other => Ok(other),
    }
}

/// Look up a key on the page dict, walking the `/Parent` chain (inherited attrs).
fn inherited<'a>(doc: &'a Document, mut id: ObjectId, key: &[u8]) -> Option<Object> {
    for _ in 0..32 {
        let dict = doc.get_dictionary(id).ok()?;
        if let Ok(v) = dict.get(key) {
            return resolve(doc, v).ok().cloned();
        }
        match dict.get(b"Parent") {
            Ok(Object::Reference(pid)) => id = *pid,
            _ => break,
        }
    }
    None
}

fn page_media_box(doc: &Document, page_id: ObjectId) -> Rect {
    if let Some(Object::Array(arr)) = inherited(doc, page_id, b"MediaBox") {
        let v: Vec<f64> = arr.iter().filter_map(super::fonts::fnum).collect();
        if v.len() == 4 {
            return Rect::new(v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3]));
        }
    }
    Rect::new(0.0, 0.0, 612.0, 792.0)
}

fn page_resources(doc: &Document, page_id: ObjectId) -> Option<Dictionary> {
    match inherited(doc, page_id, b"Resources")? {
        Object::Dictionary(d) => Some(d),
        _ => None,
    }
}

fn subtype_is(dict: &Dictionary, want: &str) -> bool {
    matches!(dict.get(b"Subtype"), Ok(Object::Name(n)) if n.as_slice() == want.as_bytes())
}

fn as_dict_owned(doc: &Document, o: &Object) -> Option<Dictionary> {
    match o {
        Object::Reference(id) => doc.get_dictionary(*id).ok().cloned(),
        Object::Dictionary(d) => Some(d.clone()),
        _ => None,
    }
}

fn page_fonts(doc: &Document, page_id: ObjectId) -> HashMap<String, Font> {
    let mut map = HashMap::new();
    let Some(res) = page_resources(doc, page_id) else { return map };
    if let Some(font_dict) = res.get(b"Font").ok().and_then(|o| as_dict_owned(doc, o)) {
        for (name, val) in font_dict.iter() {
            if let Ok(fd) = resolve(doc, val).and_then(|o| o.as_dict().map(|d| d.clone())) {
                let key = String::from_utf8_lossy(name).into_owned();
                map.insert(key, build_font(doc, &fd));
            }
        }
    }
    map
}

fn page_images(
    doc: &Document,
    page_id: ObjectId,
) -> (HashMap<String, ()>, HashMap<String, super::ImageData>) {
    let mut names = HashMap::new();
    let mut data = HashMap::new();
    let Some(res) = page_resources(doc, page_id) else { return (names, data) };
    if let Some(xo) = res.get(b"XObject").ok().and_then(|o| as_dict_owned(doc, o)) {
        for (name, val) in xo.iter() {
            if let Ok(Object::Stream(s)) = resolve(doc, val) {
                if subtype_is(&s.dict, "Image") {
                    let key = String::from_utf8_lossy(name).into_owned();
                    names.insert(key.clone(), ());
                    if let Some(img) = decode_image(doc, s) {
                        data.insert(key, img);
                    }
                }
            }
        }
    }
    (names, data)
}

/// Decode an image XObject to JPEG (DCT passthrough) or raw RGBA.
fn decode_image(doc: &Document, stream: &lopdf::Stream) -> Option<super::ImageData> {
    let dict = &stream.dict;
    let filters = image_filters(dict);
    let is_dct = filters.iter().any(|f| f == "DCTDecode");

    if is_dct {
        // The stream content is already a JPEG.
        return Some(super::ImageData::Jpeg(stream.content.clone()));
    }

    // Raw samples (Flate/none): reconstruct from colorspace, 8-bit only.
    let width = dict.get(b"Width").ok().and_then(super::fonts::fnum)? as u32;
    let height = dict.get(b"Height").ok().and_then(super::fonts::fnum)? as u32;
    let bpc = dict.get(b"BitsPerComponent").ok().and_then(super::fonts::fnum).unwrap_or(8.0) as u32;
    if bpc != 8 || width == 0 || height == 0 || width > 20000 || height > 20000 {
        return None;
    }
    let mut s = stream.clone();
    let _ = s.decompress();
    let bytes = &s.content;
    let cs = color_space_name(doc, dict);
    let px = (width as usize) * (height as usize);
    let mut rgba = Vec::with_capacity(px * 4);
    match cs.as_deref() {
        Some("DeviceGray") | Some("CalGray") | Some("G") => {
            if bytes.len() < px {
                return None;
            }
            for &g in &bytes[..px] {
                rgba.extend_from_slice(&[g, g, g, 255]);
            }
        }
        Some("DeviceRGB") | Some("CalRGB") | Some("RGB") => {
            if bytes.len() < px * 3 {
                return None;
            }
            for c in bytes[..px * 3].chunks(3) {
                rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        Some("DeviceCMYK") | Some("CMYK") => {
            if bytes.len() < px * 4 {
                return None;
            }
            for c in bytes[..px * 4].chunks(4) {
                let (cy, m, ye, k) = (c[0] as f64, c[1] as f64, c[2] as f64, c[3] as f64);
                let r = (255.0 - cy) * (255.0 - k) / 255.0;
                let g = (255.0 - m) * (255.0 - k) / 255.0;
                let b = (255.0 - ye) * (255.0 - k) / 255.0;
                rgba.extend_from_slice(&[r as u8, g as u8, b as u8, 255]);
            }
        }
        _ => return None, // Indexed / ICCBased / unsupported
    }
    Some(super::ImageData::Rgba { width, height, data: rgba })
}

fn image_filters(dict: &Dictionary) -> Vec<String> {
    match dict.get(b"Filter") {
        Ok(Object::Name(n)) => vec![String::from_utf8_lossy(n).into_owned()],
        Ok(Object::Array(a)) => a
            .iter()
            .filter_map(|o| if let Object::Name(n) = o { Some(String::from_utf8_lossy(n).into_owned()) } else { None })
            .collect(),
        _ => vec![],
    }
}

fn color_space_name(doc: &Document, dict: &Dictionary) -> Option<String> {
    let cs = dict.get(b"ColorSpace").or_else(|_| dict.get(b"CS")).ok()?;
    match resolve(doc, cs).ok()? {
        Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
        Object::Array(a) => {
            // e.g. [/ICCBased stream] -> peek /N; [/Indexed base ...] unsupported here
            if let Some(Object::Name(n)) = a.first() {
                let head = String::from_utf8_lossy(n).into_owned();
                if head == "ICCBased" {
                    if let Some(Object::Reference(id)) = a.get(1) {
                        if let Ok(s) = doc.get_object(*id).and_then(|o| o.as_stream()) {
                            let n = s.dict.get(b"N").ok().and_then(super::fonts::fnum).unwrap_or(3.0);
                            return Some(match n as i64 {
                                1 => "DeviceGray".into(),
                                4 => "DeviceCMYK".into(),
                                _ => "DeviceRGB".into(),
                            });
                        }
                    }
                    return Some("DeviceRGB".into());
                }
                return Some(head);
            }
            None
        }
        _ => None,
    }
}
