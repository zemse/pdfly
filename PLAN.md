# pdf-rs — remaining work

The build is feature-complete and tested (26 tests; whole corpus converts across all output
modes with no panics). See [README.md](./README.md) and [ARCHITECTURE.md](./ARCHITECTURE.md) for
what shipped. This file lists only what's left.

## Open (tractable)

- [ ] **Dense academic multi-column edge cases** — clean two-column pages read correctly via gutter
  detection; a residual subset still interleaves columns (e.g. bench doc `01030000000012`).
  Re-validated against `opendataloader-bench`: the *catastrophic* NID failures are not reading-order
  bugs but extraction failures (scanned/vector/chart pages with little or no embedded text — docs 5,
  141, 27, 110, 200), which are OCR-blocked, not tunable. The genuine multi-column residual is small.
  *(A band-recursive rewrite was prototyped and reverted — naively treating full-width spanners as
  band separators fragments full-width code listings in two-column papers (`2408.02509v1`); still
  high-risk for marginal gain.)*

## Done — threshold tuning against opendataloader-bench

`opendataloader-bench` (200 PDFs w/ ground truth) cloned at `../opendataloader-bench`; scoring harness
wired up (see memory `opendataloader-bench-setup`). **Overall 0.717 → 0.785** (NID 0.847→0.879,
TEDS 0.309→0.394, MHS 0.501→0.663), all 26 tests passing. Changes:
- Numbered section headings ("4. Entropy") no longer eaten as single-item ordered lists.
- Borderless (column-aligned) table detection on by default (`--table-method ruled` for ruled-only),
  with precision guards: fill/regularity, prose-cell rejection, per-column ToC rejection.
- Finer line segmentation: split baseline runs on gaps > max(1.3·fs, 10pt) — recovers table columns,
  improves reading order and heading separation simultaneously.
- Reject sparse ruled grids with a paragraph-sized cell (bar-chart false positives).

Remaining gap to thresholds (nid 0.90 / teds 0.49 / mhs 0.74) is the hard tail: ~16 TEDS-zero docs
are image/vector tables (need OCR) or tight-gap multi-column/wide tables (need word-level table
reconstruction, not threshold tuning); ~4 MHS-zero docs have headings indistinct from body font
(lowering thresholds would cost precision on the 100+ docs that already score well).

## Blocked in this environment (need external assets)

- [ ] **Non-Latin OCR** — the OCR path works end-to-end (`--features ocr`), but the bundled `ocrs`
  models are Latin-script. Validate with language-specific `.rten` models for CJK/etc.
- [ ] **LaTeX formula extraction** — needs a local image→LaTeX vision model (e.g. pix2tex via
  `rten`). Feature scaffolding only; no model available here to build/verify against.
- [ ] **Chart / image descriptions** — needs a local VLM. Same constraint.
- [ ] **Korean special-form tables** — niche heuristic; no Korean test PDF available to verify
  against, so deliberately not implemented blind.

## Explicitly NOT doing

- Hybrid AI HTTP server (external server + network — breaks the self-contained local binary).

---

### Done in the latest pass (for reference)
Type1 (`/FontFile`) built-in `/Encoding` parser — symbolic Type1 fonts with a non-standard encoding
and no `/ToUnicode` now decode (verified on `1901.03003`: `†`, `‡`, `{ }`, `−`, `·` etc. recovered) ·
`--timing` flag (pages/sec per file + overall).

Earlier: tagged-PDF `/ParentTree` + PDF/UA metadata · table figure-vs-table discrimination · ToUnicode
CMap tokenizer fix (packed hex) · two-column gutter reading order · OCR end-to-end validation · publish
prep (Cargo metadata, MIT LICENSE, include whitelist) · span inference, borderless tables, glyph-name
expansion, tagged-PDF MCID association.
