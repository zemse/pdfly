# pdf-rs — remaining work

The build is feature-complete and tested (23 tests; whole corpus converts across all output
modes with no panics). See [README.md](./README.md) and [ARCHITECTURE.md](./ARCHITECTURE.md) for
what shipped. This file lists only what's left.

## Open (tractable)

- [ ] **Type1 (`FontFile`) charstring fonts** without `/ToUnicode` and with a non-standard built-in
  encoding still mis-decode. *(Note: the common gibberish case — `issue-336` — was a ToUnicode CMap
  bug and is now fixed; embedded TrueType/CFF and standard glyph names decode. This is the rarer
  residue.)* Would need a Type1 charstring/`/Encoding` parser.
- [ ] **Dense academic multi-column edge cases** — clean two-column pages now read correctly via
  gutter detection; pages with a vertical margin stamp + an abstract that lives *inside* a column
  can still mis-order. Would need per-region column detection (recurse gutter detection into bands).
- [ ] **Benchmarks** — add a pages/sec measurement (criterion or a `--timing` flag).
- [ ] **Threshold tuning** — pull `opendataloader-bench` (200 PDFs w/ ground truth) and tune the
  heading/list/table heuristics against it. (Large download; not done here.)

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
Tagged-PDF `/ParentTree` + PDF/UA metadata · table figure-vs-table discrimination · ToUnicode CMap
tokenizer fix (packed hex) · two-column gutter reading order · OCR end-to-end validation · publish
prep (Cargo metadata, MIT LICENSE, include whitelist). Earlier: span inference, borderless tables,
glyph-name expansion, tagged-PDF MCID association.
