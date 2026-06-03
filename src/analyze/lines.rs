//! Assemble positioned [`TextRun`]s into [`Line`]s: group by baseline, order
//! left-to-right, insert spaces from horizontal gaps.

use crate::extract::{Rect, TextRun};
use crate::model::Line;

pub fn build_lines(runs: &[TextRun]) -> Vec<Line> {
    if runs.is_empty() {
        return vec![];
    }
    // Sort by descending center-y, then left.
    let mut idx: Vec<usize> = (0..runs.len()).collect();
    idx.sort_by(|&a, &b| {
        runs[b]
            .bbox
            .center_y()
            .partial_cmp(&runs[a].bbox.center_y())
            .unwrap()
            .then(runs[a].bbox.left.partial_cmp(&runs[b].bbox.left).unwrap())
    });

    // Greedy grouping into lines by baseline proximity.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    let mut cur_y = f64::NAN;
    let mut cur_size = 0.0;
    for &i in &idx {
        let cy = runs[i].bbox.center_y();
        let sz = runs[i].font_size.max(1.0);
        if cur.is_empty() {
            cur.push(i);
            cur_y = cy;
            cur_size = sz;
        } else {
            let tol = (cur_size.max(sz)) * 0.5;
            if (cy - cur_y).abs() <= tol {
                cur.push(i);
                // running average baseline
                cur_y = (cur_y * (cur.len() - 1) as f64 + cy) / cur.len() as f64;
                cur_size = cur_size.max(sz);
            } else {
                groups.push(std::mem::take(&mut cur));
                cur.push(i);
                cur_y = cy;
                cur_size = sz;
            }
        }
    }
    if !cur.is_empty() {
        groups.push(cur);
    }

    let mut lines = Vec::new();
    for g in groups {
        let mut g = g;
        g.sort_by(|&a, &b| runs[a].bbox.left.partial_cmp(&runs[b].bbox.left).unwrap());
        // Split a baseline group on large horizontal gaps (column gutters / tabs)
        // so two-column text on the same y doesn't merge into one line.
        let mut sub: Vec<usize> = Vec::new();
        let mut prev_right_split: Option<f64> = None;
        let mut segments: Vec<Vec<usize>> = Vec::new();
        for &i in &g {
            let fs = runs[i].font_size.max(1.0);
            if let Some(pr) = prev_right_split {
                let gap = runs[i].bbox.left - pr;
                if gap > (fs * 2.0).max(18.0) {
                    segments.push(std::mem::take(&mut sub));
                }
            }
            sub.push(i);
            prev_right_split = Some(runs[i].bbox.right);
        }
        if !sub.is_empty() {
            segments.push(sub);
        }
        for g in segments {
            build_one_line(runs, &g, &mut lines);
        }
    }
    lines
}

fn build_one_line(runs: &[TextRun], g: &[usize], lines: &mut Vec<Line>) {
    let mut text = String::new();
    let mut bbox = Rect::empty();
    let mut bold_chars = 0usize;
    let mut italic_chars = 0usize;
    let mut total_chars = 0usize;
    let mut sizes: Vec<f64> = Vec::new();
    let mut prev_right: Option<f64> = None;
    for &i in g {
        let r = &runs[i];
        let space_w = r.font_size * 0.25;
        if let Some(pr) = prev_right {
            let gap = r.bbox.left - pr;
            if gap > space_w && !text.ends_with(' ') && !r.text.starts_with(' ') && !text.is_empty()
            {
                text.push(' ');
            }
        }
        text.push_str(&r.text);
        bbox.union(&r.bbox);
        let n = r.text.chars().count();
        total_chars += n;
        if r.bold {
            bold_chars += n;
        }
        if r.italic {
            italic_chars += n;
        }
        sizes.push(r.font_size);
        prev_right = Some(r.bbox.right);
    }
    let text = collapse_spaces(&text);
    if text.trim().is_empty() {
        return;
    }
    let font_size = median(&mut sizes);
    lines.push(Line {
        text,
        bbox,
        font_size,
        bold: total_chars > 0 && bold_chars * 2 > total_chars,
        italic: total_chars > 0 && italic_chars * 2 > total_chars,
        strike: false,
    });
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 10.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn collapse_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = false;
    for c in s.chars() {
        let is_space = c == ' ' || c == '\u{a0}';
        if is_space {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(c);
            last_space = false;
        }
    }
    out.trim().to_string()
}
