//! XY-Cut++ reading order, ported (clean-room) from opendataloader's
//! `XYCutPlusPlusSorter` (Apache-2.0). Pure geometric recursive projection
//! cuts with cross-layout handling. Operates on anything with a [`Rect`].

use crate::extract::Rect;

const BETA: f64 = 2.0;
const DENSITY_THRESHOLD: f64 = 0.9;
const OVERLAP_THRESHOLD: f64 = 0.1;
const MIN_OVERLAP_COUNT: usize = 2;
const MIN_GAP: f64 = 5.0;
const NARROW_RATIO: f64 = 0.1;

/// Sort indices of `boxes` into reading order.
pub fn order(boxes: &[Rect]) -> Vec<usize> {
    let idx: Vec<usize> = (0..boxes.len()).collect();
    if idx.len() <= 1 {
        return idx;
    }
    let cross = identify_cross_layout(boxes, &idx);
    let main: Vec<usize> = idx.iter().copied().filter(|i| !cross.contains(i)).collect();
    if main.is_empty() {
        return sort_y_then_x(boxes, &idx);
    }
    let density = density_ratio(boxes, &main);
    let prefer_h = density > DENSITY_THRESHOLD;
    let sorted_main = segment(boxes, &main, prefer_h);
    merge_cross(boxes, sorted_main, cross)
}

fn identify_cross_layout(boxes: &[Rect], idx: &[usize]) -> Vec<usize> {
    let mut cross = Vec::new();
    if idx.len() < 3 {
        return cross;
    }
    let max_w = idx.iter().map(|&i| boxes[i].width()).fold(0.0, f64::max);
    let threshold = BETA * max_w;
    for &i in idx {
        if boxes[i].width() >= threshold && overlaps_at_least(boxes, idx, i, MIN_OVERLAP_COUNT) {
            cross.push(i);
        }
    }
    cross
}

fn overlaps_at_least(boxes: &[Rect], idx: &[usize], i: usize, min: usize) -> bool {
    let mut count = 0;
    for &j in idx {
        if j == i {
            continue;
        }
        if h_overlap_ratio(&boxes[i], &boxes[j]) >= OVERLAP_THRESHOLD {
            count += 1;
            if count >= min {
                return true;
            }
        }
    }
    false
}

fn h_overlap_ratio(a: &Rect, b: &Rect) -> f64 {
    let left = a.left.max(b.left);
    let right = a.right.min(b.right);
    let w = (right - left).max(0.0);
    if w <= 0.0 {
        return 0.0;
    }
    let smaller = a.width().min(b.width());
    if smaller > 0.0 {
        w / smaller
    } else {
        0.0
    }
}

fn density_ratio(boxes: &[Rect], idx: &[usize]) -> f64 {
    if idx.is_empty() {
        return 1.0;
    }
    let mut region = Rect::empty();
    let mut content = 0.0;
    for &i in idx {
        region.union(&boxes[i]);
        content += boxes[i].area();
    }
    let ra = region.area();
    if ra <= 0.0 {
        1.0
    } else {
        (content / ra).min(1.0)
    }
}

fn segment(boxes: &[Rect], idx: &[usize], prefer_h: bool) -> Vec<usize> {
    if idx.len() <= 1 {
        return idx.to_vec();
    }
    let (h_pos, h_gap) = best_horizontal_cut(boxes, idx);
    let (v_pos, v_gap) = best_vertical_cut(boxes, idx);
    let h_ok = h_gap >= MIN_GAP;
    let v_ok = v_gap >= MIN_GAP;

    let use_h = if h_ok && v_ok {
        h_gap > v_gap
    } else if h_ok {
        true
    } else if v_ok {
        false
    } else {
        return sort_y_then_x(boxes, idx);
    };

    let groups = if use_h {
        split_h(boxes, idx, h_pos)
    } else {
        split_v(boxes, idx, v_pos)
    };
    if groups.len() <= 1 {
        return sort_y_then_x(boxes, idx);
    }
    let mut out = Vec::new();
    for g in groups {
        out.extend(segment(boxes, &g, prefer_h));
    }
    out
}

fn best_vertical_cut(boxes: &[Rect], idx: &[usize]) -> (f64, f64) {
    let edge = vertical_cut_by_edges(boxes, idx);
    if edge.1 >= MIN_GAP {
        return edge;
    }
    // Retry without narrow outliers that may bridge a column gap.
    if idx.len() >= 3 {
        let mut region = Rect::empty();
        for &i in idx {
            region.union(&boxes[i]);
        }
        let narrow = region.width() * NARROW_RATIO;
        let filtered: Vec<usize> = idx.iter().copied().filter(|&i| boxes[i].width() >= narrow).collect();
        if filtered.len() >= 2 && filtered.len() < idx.len() {
            let f = vertical_cut_by_edges(boxes, &filtered);
            if f.1 > edge.1 && f.1 >= MIN_GAP {
                return f;
            }
        }
    }
    edge
}

fn vertical_cut_by_edges(boxes: &[Rect], idx: &[usize]) -> (f64, f64) {
    let mut sorted = idx.to_vec();
    sorted.sort_by(|&a, &b| boxes[a].left.partial_cmp(&boxes[b].left).unwrap());
    let mut largest = 0.0;
    let mut pos = 0.0;
    let mut prev_right: Option<f64> = None;
    for &i in &sorted {
        let l = boxes[i].left;
        let r = boxes[i].right;
        if let Some(pr) = prev_right {
            if l > pr {
                let gap = l - pr;
                if gap > largest {
                    largest = gap;
                    pos = (pr + l) / 2.0;
                }
            }
        }
        prev_right = Some(prev_right.map_or(r, |pr| pr.max(r)));
    }
    (pos, largest)
}

fn best_horizontal_cut(boxes: &[Rect], idx: &[usize]) -> (f64, f64) {
    let mut sorted = idx.to_vec();
    sorted.sort_by(|&a, &b| boxes[b].top.partial_cmp(&boxes[a].top).unwrap());
    let mut largest = 0.0;
    let mut pos = 0.0;
    let mut prev_bottom: Option<f64> = None;
    for &i in &sorted {
        let top = boxes[i].top;
        let bottom = boxes[i].bottom;
        if let Some(pb) = prev_bottom {
            if pb > top {
                let gap = pb - top;
                if gap > largest {
                    largest = gap;
                    pos = (pb + top) / 2.0;
                }
            }
        }
        prev_bottom = Some(prev_bottom.map_or(bottom, |pb| pb.min(bottom)));
    }
    (pos, largest)
}

fn split_h(boxes: &[Rect], idx: &[usize], cut_y: f64) -> Vec<Vec<usize>> {
    let (mut above, mut below) = (Vec::new(), Vec::new());
    for &i in idx {
        if boxes[i].center_y() > cut_y {
            above.push(i);
        } else {
            below.push(i);
        }
    }
    [above, below].into_iter().filter(|g| !g.is_empty()).collect()
}

fn split_v(boxes: &[Rect], idx: &[usize], cut_x: f64) -> Vec<Vec<usize>> {
    let (mut left, mut right) = (Vec::new(), Vec::new());
    for &i in idx {
        if boxes[i].center_x() < cut_x {
            left.push(i);
        } else {
            right.push(i);
        }
    }
    [left, right].into_iter().filter(|g| !g.is_empty()).collect()
}

fn merge_cross(boxes: &[Rect], main: Vec<usize>, cross: Vec<usize>) -> Vec<usize> {
    if cross.is_empty() {
        return main;
    }
    let sorted_cross = sort_y_then_x(boxes, &cross);
    let mut out = Vec::with_capacity(main.len() + sorted_cross.len());
    let (mut mi, mut ci) = (0, 0);
    while mi < main.len() || ci < sorted_cross.len() {
        if ci >= sorted_cross.len() {
            out.push(main[mi]);
            mi += 1;
        } else if mi >= main.len() {
            out.push(sorted_cross[ci]);
            ci += 1;
        } else if boxes[sorted_cross[ci]].top >= boxes[main[mi]].top {
            out.push(sorted_cross[ci]);
            ci += 1;
        } else {
            out.push(main[mi]);
            mi += 1;
        }
    }
    out
}

fn sort_y_then_x(boxes: &[Rect], idx: &[usize]) -> Vec<usize> {
    let mut v = idx.to_vec();
    v.sort_by(|&a, &b| {
        boxes[b].top
            .partial_cmp(&boxes[a].top)
            .unwrap()
            .then(boxes[a].left.partial_cmp(&boxes[b].left).unwrap())
    });
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_columns_read_left_then_right() {
        // Left column (x 0..100) two blocks, right column (x 200..300) two blocks.
        let boxes = vec![
            Rect::new(0.0, 700.0, 100.0, 750.0),   // L top
            Rect::new(0.0, 600.0, 100.0, 650.0),   // L bottom
            Rect::new(200.0, 700.0, 300.0, 750.0), // R top
            Rect::new(200.0, 600.0, 300.0, 650.0), // R bottom
        ];
        let ord = order(&boxes);
        assert_eq!(ord, vec![0, 1, 2, 3], "left column fully before right");
    }

    #[test]
    fn full_width_header_comes_first() {
        let boxes = vec![
            Rect::new(0.0, 600.0, 100.0, 650.0),   // L body
            Rect::new(200.0, 600.0, 300.0, 650.0), // R body
            Rect::new(0.0, 760.0, 300.0, 790.0),   // full-width header
        ];
        let ord = order(&boxes);
        assert_eq!(ord[0], 2, "header first");
    }
}
