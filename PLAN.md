# pdf-rs — remaining work

Everything in the original build plan (extraction → analysis → Markdown/JSON/HTML/text, chapter
split, images, reading order, tables w/ spans, content-safety, sanitize, threads, struct-tree
read/write w/ marked content, optional OCR) is **done and tested**. For how the code is organized
and what shipped, see [README.md](./README.md) and [ARCHITECTURE.md](./ARCHITECTURE.md).

This file is just the open to-do list.

## Correctness / quality

- [ ] **Type1 font decoding** — Type1 (`FontFile`) subset fonts with non-standard built-in
  encodings still mis-decode to gibberish (e.g. `issue-336-...pdf`). Need a Type1 charstring /
  built-in-encoding parser. *(Embedded TrueType/CFF and standard glyph names already decode.)*
  🧪 `issue-336` decodes to real words.
- [ ] **Dense multi-column reading order** — a full-width abstract over a two-column body can still
  interleave. Improve band-then-column recursion (cut horizontal bands before column V-cuts).
  🧪 arXiv `2408.02509v1.pdf` p1: abstract before body, each column in order.
- [ ] **Bordered-table over-triggering** — ruled figures/boxes get detected as tables. Add
  figure-vs-table discrimination (aspect ratio, text density, caption nearby).
  🧪 Corpus table counts drop to plausible numbers (BookChapter ≠ 21 tables).
- [ ] **Heading level for struct-tree path** — the `--use-struct-tree` path stores heading `size`
  as `0.0`; levels come only from the tag name. Fine for H1–H6, but verify deep nesting.

## Accessibility (tagged PDF → PDF/UA)

- [ ] **`/ParentTree`** — `--tagged-pdf` writes marked content + a `/StructTreeRoot` that
  round-trips, but no reverse map. Add a `/ParentTree` number tree keyed by `/StructParents`.
  🧪 A validator resolves content → structure (not just structure → content).
- [ ] **PDF/UA conformance pass** — set required metadata (`/Lang`, document title in XMP, `/ViewerPreferences`),
  then validate (e.g. veraPDF) and fix violations.
  🧪 veraPDF PDF/UA-1 check passes on a tagged sample.

## Optional / ML features (off by default, feature-gated)

- [ ] **Validate OCR end-to-end** — `--features ocr` compiles and is wired, but never run with real
  models here. Download `ocrs` `.rten` models, set `PDFRS_OCR_{DETECTION,RECOGNITION}_MODEL`, and
  confirm output on `chinese_scan.pdf`.
  🧪 Scanned PDF yields non-empty, sensible text.
- [ ] **LaTeX formula extraction** — vision model (image→LaTeX) via `rten`, behind a `formula`
  feature. Emit `$$…$$`.
- [ ] **Chart / image descriptions** — local VLM behind a `vlm` feature; fill image `alt`. Off by default.
- [ ] **Korean special-form tables** — niche heuristic; no test corpus yet.

## Polish

- [ ] Tune heuristic thresholds against a larger corpus (pull `opendataloader-bench`); the source's
  numbers live in veraPDF-wcag-algs (`NodeUtils`/`ListLabelsUtils`/`CaptionUtils`).
- [ ] Benchmarks (pages/sec) and a `--quiet`/log-level pass.
- [ ] Publish prep: crate metadata, `include` whitelist, `cargo package --list` check.

## Explicitly NOT doing

- Hybrid AI HTTP server (external server + network — breaks the self-contained local binary).
