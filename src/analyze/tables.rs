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

/// Sort + merge near-equal coordinates into representative grid lines.
fn cluster(vals: &mut [f64]) -> Vec<f64> {
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut out: Vec<f64> = Vec::new();
    for &v in vals.iter() {
        match out.last() {
            Some(&last) if (v - last).abs() <= TOL => {}
            _ => out.push(v),
        }
    }
    out
}
