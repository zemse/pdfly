//! Assemble positioned [`TextRun`]s into [`Line`]s: group by baseline, order
//! left-to-right, insert spaces from horizontal gaps.

use crate::extract::{Rect, TextRun};
use crate::model::Line;

pub fn build_lines(runs: &[TextRun]) -> Vec<Line> {
    if runs.is_empty() {
        return vec![];
    }
    // Rotated/vertical text (e.g. an arXiv side label) has a bbox far taller than
    // its font size. Such a run must not join horizontal baseline grouping: its
    // large font inflates the y-tolerance and its tall box spans many rows, pulling
    // unrelated body lines into one group and scrambling them. Emit each as its own
    // line and keep it out of the grouping below.
    let is_rotated = |i: usize| {
        let r = &runs[i];
        r.font_size > 0.0 && r.bbox.height() > r.font_size * 2.5
    };
    let mut lines = Vec::new();
    for i in 0..runs.len() {
        if is_rotated(i) {
            build_one_line(runs, &[i], &mut lines);
        }
    }
    // Page-level column gutter (if any): a central vertical band that almost no
    // run crosses. Used to split same-baseline runs from different columns even
    // when the gutter is narrower than the generic gap threshold (tight two-column
    // layouts otherwise merge "…left text.Right text…" into one scrambled line).
    let gutter = detect_gutter(runs);
    // Sort by descending center-y, then left (horizontal runs only).
    let mut idx: Vec<usize> = (0..runs.len()).filter(|&i| !is_rotated(i)).collect();
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
                // Split on a gap clearly wider than inter-word spacing (~0.25·fs):
                // column gutters, tab stops, and table-cell gaps. Tuned against
                // opendataloader-bench — finer segmentation here improves reading
                // order, heading separation, and borderless-table column recovery.
                let wide_gap = gap > (fs * 1.3).max(10.0);
                // Also split when consecutive runs sit on opposite sides of the
                // page column gutter, even if the raw gap is small (tight gutters).
                let crosses_gutter = gutter.is_some_and(|gx| pr <= gx && runs[i].bbox.left >= gx);
                if wide_gap || crosses_gutter {
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

/// Detect a single dominant column gutter: a central vertical band that almost no
/// text run crosses, with substantial text on both sides. Returns the gutter
/// center x. Conservative — returns `None` for single-column pages (no central
/// empty band) so their line assembly is unchanged.
fn detect_gutter(runs: &[TextRun]) -> Option<f64> {
    const N: usize = 100;
    let mut left = f64::MAX;
    let mut right = f64::MIN;
    for r in runs {
        left = left.min(r.bbox.left);
        right = right.max(r.bbox.right);
    }
    let w = right - left;
    if w <= 0.0 || runs.len() < 8 {
        return None;
    }
    // Coverage histogram: how many runs cover each x-bin. Full-width spanners
    // (titles, abstracts, full-width paragraphs) cross the gutter, so exclude them
    // — otherwise they fill the central band and hide the gutter.
    let mut cov = [0u32; N];
    for r in runs {
        if r.bbox.width() >= 0.55 * w {
            continue;
        }
        let a = (((r.bbox.left - left) / w) * N as f64)
            .floor()
            .clamp(0.0, N as f64) as usize;
        let b = (((r.bbox.right - left) / w) * N as f64)
            .ceil()
            .clamp(0.0, N as f64) as usize;
        for c in cov.iter_mut().take(b.min(N)).skip(a) {
            *c += 1;
        }
    }
    // Longest near-zero-coverage run of bins in the central region [0.30, 0.70].
    let max_cov = cov.iter().copied().max().unwrap_or(0);
    if max_cov < 4 {
        return None;
    }
    let floor = (max_cov as f64 * 0.03).ceil() as u32; // ≤3% of peak counts as "empty"
    let (lo_bin, hi_bin) = ((N as f64 * 0.30) as usize, (N as f64 * 0.70) as usize);
    let (mut best_start, mut best_len) = (0usize, 0usize);
    let (mut run_start, mut run_len) = (0usize, 0usize);
    for (k, &c) in cov.iter().enumerate().take(hi_bin).skip(lo_bin) {
        if c <= floor {
            if run_len == 0 {
                run_start = k;
            }
            run_len += 1;
            if run_len > best_len {
                best_len = run_len;
                best_start = run_start;
            }
        } else {
            run_len = 0;
        }
    }
    if best_len == 0 {
        return None;
    }
    let center_bin = best_start + best_len / 2;
    let gx = left + (center_bin as f64 / N as f64) * w;
    // Require real content on both sides of the gutter.
    let left_runs = runs.iter().filter(|r| r.bbox.right <= gx).count();
    let right_runs = runs.iter().filter(|r| r.bbox.left >= gx).count();
    if left_runs >= 3 && right_runs >= 3 {
        Some(gx)
    } else {
        None
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::Rect;

    fn run(text: &str, l: f64, r: f64, cy: f64, fs: f64) -> TextRun {
        TextRun {
            text: text.into(),
            bbox: Rect::new(l, cy - fs / 2.0, r, cy + fs / 2.0),
            font_size: fs,
            font_name: String::new(),
            bold: false,
            italic: false,
            color: [0.0; 3],
            mcid: None,
            hidden: false,
        }
    }

    #[test]
    fn rotated_side_label_does_not_scramble_body_lines() {
        // A tall vertical label (h >> font size) sits beside two body lines whose
        // baselines are ~10pt apart — within the label's inflated tolerance. Before
        // the rotated-text exclusion these three merged into one scrambled line.
        let mut label = run("arXiv:2002.05231", 17.0, 37.0, 452.0, 20.0);
        label.bbox = Rect::new(17.0, 252.0, 37.0, 652.0); // tall rotated box
        let runs = vec![
            label,
            run("first body line here", 49.0, 273.0, 458.0, 9.0),
            run("second body line here", 49.0, 296.0, 448.0, 9.0),
        ];
        let lines = build_lines(&runs);
        // The two body lines stay separate (not merged through the label).
        assert!(
            lines.iter().any(|l| l.text == "first body line here"),
            "got: {:?}",
            lines.iter().map(|l| &l.text).collect::<Vec<_>>()
        );
        assert!(lines.iter().any(|l| l.text == "second body line here"));
    }

    #[test]
    fn tight_gutter_splits_two_columns() {
        // Two columns separated by a ~12pt gutter (49..150 | 162..260), each with
        // several rows. Same-baseline left/right runs must not merge into one line.
        let mut runs = Vec::new();
        for k in 0..5 {
            let y = 600.0 - k as f64 * 12.0;
            runs.push(run("leftcol", 49.0, 150.0, y, 9.0));
            runs.push(run("rightcol", 162.0, 260.0, y, 9.0));
        }
        let lines = build_lines(&runs);
        // No line should contain both columns concatenated.
        assert!(
            lines
                .iter()
                .all(|l| l.text == "leftcol" || l.text == "rightcol"),
            "columns merged: {:?}",
            lines.iter().map(|l| &l.text).collect::<Vec<_>>()
        );
    }
}
