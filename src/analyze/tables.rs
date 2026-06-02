//! Border-based table detection: build a grid from axis-aligned vector
//! segments, then drop text lines into cells. Simplified port of the
//! border-table idea (no row/col span inference yet — single cells).

use crate::extract::{LineSeg, Rect};
use crate::model::{Cell, Line};

pub struct DetectedTable {
    pub bbox: Rect,
    pub rows: Vec<Vec<Cell>>,
}

const TOL: f64 = 3.0;

/// Detect tables on a page from its line segments + text lines.
/// Returns detected tables and the set of line indices they consumed.
pub fn detect(lines: &[LineSeg], text_lines: &[Line]) -> (Vec<DetectedTable>, Vec<usize>) {
    // Cluster horizontal y's and vertical x's.
    let mut ys: Vec<f64> = lines.iter().filter(|l| l.is_horizontal()).map(|l| l.y0).collect();
    let mut xs: Vec<f64> = lines.iter().filter(|l| l.is_vertical()).map(|l| l.x0).collect();
    let ys = cluster(&mut ys);
    let xs = cluster(&mut xs);

    if ys.len() < 2 || xs.len() < 2 {
        return (vec![], vec![]);
    }

    let grid_box = Rect::new(
        *xs.first().unwrap(),
        *ys.first().unwrap(),
        *xs.last().unwrap(),
        *ys.last().unwrap(),
    );

    // Build cells: row r between ys[top]..ys[top-1] (ys sorted ascending → reverse for top-down).
    let mut ys_desc = ys.clone();
    ys_desc.sort_by(|a, b| b.partial_cmp(a).unwrap());
    let n_rows = ys_desc.len() - 1;
    let n_cols = xs.len() - 1;

    let mut rows: Vec<Vec<Cell>> = (0..n_rows)
        .map(|_| (0..n_cols).map(|_| Cell { col_span: 1, row_span: 1, ..Default::default() }).collect())
        .collect();

    let mut consumed = Vec::new();
    let mut any_text = false;
    for (li, line) in text_lines.iter().enumerate() {
        let (cx, cy) = (line.bbox.center_x(), line.bbox.center_y());
        if cx < grid_box.left - TOL || cx > grid_box.right + TOL || cy < grid_box.bottom - TOL || cy > grid_box.top + TOL {
            continue;
        }
        // Find row (top-down) and column.
        let mut row = None;
        for r in 0..n_rows {
            let top = ys_desc[r];
            let bot = ys_desc[r + 1];
            if cy <= top + TOL && cy >= bot - TOL {
                row = Some(r);
                break;
            }
        }
        let mut col = None;
        for c in 0..n_cols {
            if cx >= xs[c] - TOL && cx <= xs[c + 1] + TOL {
                col = Some(c);
                break;
            }
        }
        if let (Some(r), Some(c)) = (row, col) {
            let cell = &mut rows[r][c];
            if !cell.text.is_empty() {
                cell.text.push(' ');
            }
            cell.text.push_str(line.text.trim());
            consumed.push(li);
            any_text = true;
        }
    }

    // Span inference (lattice): merge cells across absent internal dividers.
    let h_segs: Vec<(f64, f64, f64)> = lines
        .iter()
        .filter(|l| l.is_horizontal())
        .map(|l| ((l.y0 + l.y1) / 2.0, l.x0.min(l.x1), l.x0.max(l.x1)))
        .collect();
    let v_segs: Vec<(f64, f64, f64)> = lines
        .iter()
        .filter(|l| l.is_vertical())
        .map(|l| ((l.x0 + l.x1) / 2.0, l.y0.min(l.y1), l.y0.max(l.y1)))
        .collect();
    // Is there a vertical divider at column boundary `xs[c]` spanning most of row r?
    let has_left = |r: usize, c: usize| -> bool {
        let (lo, hi) = (ys_desc[r + 1], ys_desc[r]);
        let need = (hi - lo) * 0.5;
        v_segs.iter().any(|&(x, y0, y1)| (x - xs[c]).abs() <= TOL && (y1.min(hi) - y0.max(lo)) >= need)
    };
    let has_top = |r: usize, c: usize| -> bool {
        let (lo, hi) = (xs[c], xs[c + 1]);
        let need = (hi - lo) * 0.5;
        h_segs.iter().any(|&(y, x0, x1)| (y - ys_desc[r]).abs() <= TOL && (x1.min(hi) - x0.max(lo)) >= need)
    };

    let mut covered = vec![vec![false; n_cols]; n_rows];
    // master_of[r][c] = (mr, mc) of the cell that owns this grid position.
    let mut master_of = vec![vec![(0usize, 0usize); n_cols]; n_rows];
    for r in 0..n_rows {
        for c in 0..n_cols {
            if covered[r][c] {
                continue;
            }
            let mut cs = 1;
            while c + cs < n_cols && !has_left(r, c + cs) {
                covered[r][c + cs] = true;
                master_of[r][c + cs] = (r, c);
                cs += 1;
            }
            let mut rs = 1;
            'down: while r + rs < n_rows {
                for cc in c..c + cs {
                    if has_top(r + rs, cc) {
                        break 'down;
                    }
                }
                for cc in c..c + cs {
                    covered[r + rs][cc] = true;
                    master_of[r + rs][cc] = (r, c);
                }
                rs += 1;
            }
            rows[r][c].col_span = cs;
            rows[r][c].row_span = rs;
        }
    }
    // Fold covered cells' text into their master and flag them.
    for r in 0..n_rows {
        for c in 0..n_cols {
            if !covered[r][c] {
                continue;
            }
            let (mr, mc) = master_of[r][c];
            let txt = std::mem::take(&mut rows[r][c].text);
            if !txt.trim().is_empty() {
                let m = &mut rows[mr][mc];
                if !m.text.is_empty() {
                    m.text.push(' ');
                }
                m.text.push_str(txt.trim());
            }
            rows[r][c].covered = true;
        }
    }

    // Require a real grid: >=2 rows and >=2 cols actually containing text,
    // and a decent fill, else treat as ordinary text (avoids fabricating
    // huge empty tables from stray rules / backgrounds).
    let filled = rows.iter().flatten().filter(|c| !c.text.trim().is_empty()).count();
    let rows_with_text = rows.iter().filter(|r| r.iter().any(|c| !c.text.trim().is_empty())).count();
    let cols_with_text = (0..n_cols)
        .filter(|&c| rows.iter().any(|r| r.get(c).map(|x| !x.text.trim().is_empty()).unwrap_or(false)))
        .count();
    if !any_text || filled < 4 || rows_with_text < 2 || cols_with_text < 2 {
        return (vec![], vec![]);
    }

    (vec![DetectedTable { bbox: grid_box, rows }], consumed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hline(y: f64, x0: f64, x1: f64) -> LineSeg {
        LineSeg { x0, y0: y, x1, y1: y }
    }
    fn vline(x: f64, y0: f64, y1: f64) -> LineSeg {
        LineSeg { x0: x, y0, x1: x, y1 }
    }
    fn line(text: &str, x: f64, y: f64) -> Line {
        Line {
            text: text.into(),
            bbox: Rect::new(x - 5.0, y - 5.0, x + 5.0, y + 5.0),
            font_size: 10.0,
            bold: false,
            italic: false,
            strike: false,
        }
    }

    #[test]
    fn cluster_detects_aligned_columns() {
        // Three rows, two columns aligned at x=10 and x=60, no ruling lines.
        let lines = vec![
            line("Name", 13.0, 100.0),
            line("Age", 63.0, 100.0),
            line("Alice", 13.0, 80.0),
            line("30", 63.0, 80.0),
            line("Bob", 13.0, 60.0),
            line("25", 63.0, 60.0),
        ];
        let (tables, consumed) = detect_cluster(&lines);
        assert_eq!(tables.len(), 1, "one borderless table");
        assert_eq!(tables[0].rows.len(), 3);
        assert_eq!(tables[0].rows[0].len(), 2);
        assert_eq!(consumed.len(), 6);
    }

    #[test]
    fn cluster_ignores_single_column_prose() {
        let lines = vec![
            line("This is a paragraph line one", 10.0, 100.0),
            line("and a second prose line here", 10.0, 85.0),
            line("and a third prose line again", 10.0, 70.0),
        ];
        let (tables, _) = detect_cluster(&lines);
        assert!(tables.is_empty(), "single-column prose is not a table");
    }

    #[test]
    fn colspan_from_missing_divider() {
        // 3 rows x 2 cols. The vertical divider at x=50 is absent in the top
        // band (y 70..100), so the header cell spans both columns; the two
        // data rows below are normal 2-cell rows (5 filled cells total).
        let segs = vec![
            hline(100.0, 0.0, 100.0),
            hline(70.0, 0.0, 100.0),
            hline(40.0, 0.0, 100.0),
            hline(0.0, 0.0, 100.0),
            vline(0.0, 0.0, 100.0),
            vline(100.0, 0.0, 100.0),
            vline(50.0, 0.0, 70.0), // present only below the header band
        ];
        let lines = vec![
            line("Header", 50.0, 85.0),
            line("a", 25.0, 55.0),
            line("b", 75.0, 55.0),
            line("c", 25.0, 20.0),
            line("d", 75.0, 20.0),
        ];
        let (tables, _consumed) = detect(&segs, &lines);
        assert_eq!(tables.len(), 1);
        let rows = &tables[0].rows;
        assert_eq!(rows[0][0].col_span, 2, "header spans both columns");
        assert_eq!(rows[0][0].text, "Header");
        assert!(rows[0][1].covered, "second header cell is covered");
        assert_eq!(rows[1][0].text, "a");
        assert_eq!(rows[1][1].text, "b");
        assert_eq!(rows[2][0].text, "c");
        assert_eq!(rows[2][1].text, "d");
    }
}

/// Borderless table detection: find runs of text rows that share aligned
/// columns (whitespace-separated), without any ruling lines. Conservative —
/// opt-in via `--table-method cluster`. Returns tables + consumed line indices.
pub fn detect_cluster(text_lines: &[Line]) -> (Vec<DetectedTable>, Vec<usize>) {
    // Group line indices into rows by baseline.
    let mut idx: Vec<usize> = (0..text_lines.len()).collect();
    idx.sort_by(|&a, &b| {
        text_lines[b].bbox.center_y().partial_cmp(&text_lines[a].bbox.center_y()).unwrap()
    });
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    let mut cur_y = f64::NAN;
    for &i in &idx {
        let cy = text_lines[i].bbox.center_y();
        let tol = (text_lines[i].font_size * 0.6).max(3.0);
        if cur.is_empty() || (cy - cur_y).abs() <= tol {
            if cur.is_empty() {
                cur_y = cy;
            }
            cur.push(i);
        } else {
            rows.push(std::mem::take(&mut cur));
            cur.push(i);
            cur_y = cy;
        }
    }
    if !cur.is_empty() {
        rows.push(cur);
    }

    let mut tables = Vec::new();
    let mut consumed = Vec::new();

    // Scan consecutive multi-cell rows into blocks.
    let mut r = 0;
    while r < rows.len() {
        if rows[r].len() < 2 {
            r += 1;
            continue;
        }
        let mut block = vec![r];
        let mut j = r + 1;
        while j < rows.len() && rows[j].len() >= 2 {
            block.push(j);
            j += 1;
        }
        if block.len() >= 3 {
            // Column starts = clustered left edges across the block.
            let mut lefts: Vec<f64> =
                block.iter().flat_map(|&br| rows[br].iter().map(|&li| text_lines[li].bbox.left)).collect();
            let cols = cluster_with(&mut lefts, 12.0);
            if cols.len() >= 2 {
                let n_cols = cols.len();
                let mut out_rows: Vec<Vec<Cell>> = Vec::new();
                let mut bbox = Rect::empty();
                for &br in &block {
                    let mut cells: Vec<Cell> =
                        (0..n_cols).map(|_| Cell { col_span: 1, row_span: 1, ..Default::default() }).collect();
                    for &li in &rows[br] {
                        let l = &text_lines[li];
                        // nearest column by left edge
                        let ci = cols
                            .iter()
                            .enumerate()
                            .min_by(|a, b| {
                                (a.1 - l.bbox.left).abs().partial_cmp(&(b.1 - l.bbox.left).abs()).unwrap()
                            })
                            .map(|(k, _)| k)
                            .unwrap_or(0);
                        if !cells[ci].text.is_empty() {
                            cells[ci].text.push(' ');
                        }
                        cells[ci].text.push_str(l.text.trim());
                        bbox.union(&l.bbox);
                        consumed.push(li);
                    }
                    out_rows.push(cells);
                }
                tables.push(DetectedTable { bbox, rows: out_rows });
            }
        }
        r = j.max(r + 1);
    }
    (tables, consumed)
}

/// Sort + merge near-equal coordinates into representative grid lines.
fn cluster(vals: &mut [f64]) -> Vec<f64> {
    cluster_with(vals, TOL)
}

fn cluster_with(vals: &mut [f64], tol: f64) -> Vec<f64> {
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut out: Vec<f64> = Vec::new();
    for &v in vals.iter() {
        match out.last() {
            Some(&last) if (v - last).abs() <= tol => {}
            _ => out.push(v),
        }
    }
    out
}
