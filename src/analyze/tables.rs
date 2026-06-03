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
    // Cluster horizontal y's and vertical x's. Only segments long enough to be
    // real rules count as grid lines — short ticks/underlines would otherwise
    // inflate the column/row count and manufacture sparse "tables".
    const MIN_RULE: f64 = 10.0;
    let mut ys: Vec<f64> = lines
        .iter()
        .filter(|l| l.is_horizontal() && (l.x1 - l.x0).abs() >= MIN_RULE)
        .map(|l| l.y0)
        .collect();
    let mut xs: Vec<f64> = lines
        .iter()
        .filter(|l| l.is_vertical() && (l.y1 - l.y0).abs() >= MIN_RULE)
        .map(|l| l.x0)
        .collect();
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
        .map(|_| {
            (0..n_cols)
                .map(|_| Cell {
                    col_span: 1,
                    row_span: 1,
                    ..Default::default()
                })
                .collect()
        })
        .collect();

    let mut consumed = Vec::new();
    let mut any_text = false;
    for (li, line) in text_lines.iter().enumerate() {
        let (cx, cy) = (line.bbox.center_x(), line.bbox.center_y());
        if cx < grid_box.left - TOL
            || cx > grid_box.right + TOL
            || cy < grid_box.bottom - TOL
            || cy > grid_box.top + TOL
        {
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
        v_segs
            .iter()
            .any(|&(x, y0, y1)| (x - xs[c]).abs() <= TOL && (y1.min(hi) - y0.max(lo)) >= need)
    };
    let has_top = |r: usize, c: usize| -> bool {
        let (lo, hi) = (xs[c], xs[c + 1]);
        let need = (hi - lo) * 0.5;
        h_segs
            .iter()
            .any(|&(y, x0, x1)| (y - ys_desc[r]).abs() <= TOL && (x1.min(hi) - x0.max(lo)) >= need)
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
    let filled = rows
        .iter()
        .flatten()
        .filter(|c| !c.text.trim().is_empty())
        .count();
    let cols_with_text = (0..n_cols)
        .filter(|&c| {
            rows.iter()
                .any(|r| r.get(c).map(|x| !x.text.trim().is_empty()).unwrap_or(false))
        })
        .count();
    // Genuinely tabular: at least two rows that each have >=2 filled cells, and
    // a non-trivial fill ratio (rejects sparse layout/figure grids).
    let rows_multi = rows
        .iter()
        .filter(|r| r.iter().filter(|c| !c.text.trim().is_empty()).count() >= 2)
        .count();
    let fill_ratio = filled as f64 / (n_rows * n_cols).max(1) as f64;
    // A sparse grid whose largest cell holds a whole paragraph is prose/figure
    // text that fell into the grid box (e.g. a bar chart whose bars read as
    // column rules, with the surrounding caption dropped into one cell). Dense
    // grids with a long cell are real tables (a wide description column), so
    // only reject when the grid is both sparse and carries a paragraph cell.
    let max_cell = rows
        .iter()
        .flatten()
        .map(|c| c.text.trim().chars().count())
        .max()
        .unwrap_or(0);
    let chart_like = max_cell > 300 && fill_ratio < 0.4;
    if !any_text
        || filled < 4
        || rows_multi < 2
        || cols_with_text < 2
        || fill_ratio < 0.15
        || chart_like
    {
        return (vec![], vec![]);
    }

    (
        vec![DetectedTable {
            bbox: grid_box,
            rows,
        }],
        consumed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hline(y: f64, x0: f64, x1: f64) -> LineSeg {
        LineSeg {
            x0,
            y0: y,
            x1,
            y1: y,
        }
    }
    fn vline(x: f64, y0: f64, y1: f64) -> LineSeg {
        LineSeg {
            x0: x,
            y0,
            x1: x,
            y1,
        }
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
        text_lines[b]
            .bbox
            .center_y()
            .partial_cmp(&text_lines[a].bbox.center_y())
            .unwrap()
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
            let mut lefts: Vec<f64> = block
                .iter()
                .flat_map(|&br| rows[br].iter().map(|&li| text_lines[li].bbox.left))
                .collect();
            let cols = cluster_with(&mut lefts, 12.0);
            if cols.len() >= 2 {
                let n_cols = cols.len();
                let mut out_rows: Vec<Vec<Cell>> = Vec::new();
                let mut bbox = Rect::empty();
                let mut block_consumed: Vec<usize> = Vec::new();
                for &br in &block {
                    let mut cells: Vec<Cell> = (0..n_cols)
                        .map(|_| Cell {
                            col_span: 1,
                            row_span: 1,
                            ..Default::default()
                        })
                        .collect();
                    for &li in &rows[br] {
                        let l = &text_lines[li];
                        // nearest column by left edge
                        let ci = cols
                            .iter()
                            .enumerate()
                            .min_by(|a, b| {
                                (a.1 - l.bbox.left)
                                    .abs()
                                    .partial_cmp(&(b.1 - l.bbox.left).abs())
                                    .unwrap()
                            })
                            .map(|(k, _)| k)
                            .unwrap_or(0);
                        if !cells[ci].text.is_empty() {
                            cells[ci].text.push(' ');
                        }
                        cells[ci].text.push_str(l.text.trim());
                        bbox.union(&l.bbox);
                        block_consumed.push(li);
                    }
                    out_rows.push(cells);
                }
                // Precision guard: a borderless block is only a table if it is
                // genuinely grid-like. Charts (scattered numbers) and prose with
                // accidental column alignment otherwise get fabricated into
                // tables, swallowing headings and wrecking reading order. Require
                // a dense, regular grid: most cells filled, and at least two
                // columns that are populated in a majority of rows.
                if is_grid_like(&out_rows, n_cols) {
                    consumed.extend_from_slice(&block_consumed);
                    tables.push(DetectedTable {
                        bbox,
                        rows: out_rows,
                    });
                }
            }
        }
        r = j.max(r + 1);
    }
    (tables, consumed)
}

/// Decide whether an assembled borderless block is a real table rather than a
/// chart or accidentally-aligned prose. Mirrors the density/regularity checks
/// the ruled-grid path applies, tuned for whitespace-aligned grids.
fn is_grid_like(out_rows: &[Vec<Cell>], n_cols: usize) -> bool {
    if out_rows.len() < 3 || n_cols < 2 {
        return false;
    }
    let n_rows = out_rows.len();
    let filled = out_rows
        .iter()
        .flatten()
        .filter(|c| !c.text.trim().is_empty())
        .count();
    let fill_ratio = filled as f64 / (n_rows * n_cols).max(1) as f64;
    // Rows that look tabular (>=2 populated cells).
    let rows_multi = out_rows
        .iter()
        .filter(|r| r.iter().filter(|c| !c.text.trim().is_empty()).count() >= 2)
        .count();
    // Columns populated in a majority of rows — a real table keeps the same
    // columns filled down the block; a chart populates a column in just one or
    // two rows.
    let strong_cols = (0..n_cols)
        .filter(|&c| {
            let cnt = out_rows
                .iter()
                .filter(|r| !r[c].text.trim().is_empty())
                .count();
            cnt * 2 >= n_rows
        })
        .count();
    if !(filled >= 6 && rows_multi * 2 >= n_rows && strong_cols >= 2 && fill_ratio >= 0.5) {
        return false;
    }
    // Cell-content check: real table cells are short tokens/values; multi-column
    // running prose (the dominant borderless false positive) produces cells that
    // are long sentence fragments. Reject blocks whose typical cell reads like
    // prose. Measured split on the benchmark: true tables ~1.8 words / ~5 chars
    // median per cell; two-column prose ~3.7-7 words / 18-48 chars.
    let mut lens: Vec<usize> = Vec::new();
    let mut total_words = 0usize;
    for cell in out_rows.iter().flatten() {
        let t = cell.text.trim();
        if t.is_empty() {
            continue;
        }
        lens.push(t.chars().count());
        total_words += t.split_whitespace().count();
    }
    lens.sort_unstable();
    let median_chars = lens[lens.len() / 2];
    let mean_words = total_words as f64 / lens.len() as f64;
    if median_chars > 12 || mean_words > 3.0 {
        return false;
    }
    // Per-column prose check: tables of contents / indexes pair a long-title
    // column with a short page-number column, so the overall median stays low.
    // Reject when any single column is itself prose-like (its own cells are long).
    let col_is_prose = (0..n_cols).any(|c| {
        let mut cl: Vec<usize> = out_rows
            .iter()
            .filter_map(|r| {
                let t = r[c].text.trim();
                (!t.is_empty()).then(|| t.chars().count())
            })
            .collect();
        if cl.len() < 3 {
            return false;
        }
        cl.sort_unstable();
        cl[cl.len() / 2] > 16
    });
    !col_is_prose
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
