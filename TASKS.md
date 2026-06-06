# pdfly — tasks & known issues

Tracks open bugs and remaining work. Corpus-testing issues are at the top; the
carried-over roadmap items (from the old `plan.md`) are at the bottom.

## Issues found in corpus testing (2026-06-06)

Ran `pdfly read` over 32 PDFs in `~/Documents/PDFs` and `~/Documents/reading`
(academic crypto papers, Rust books, legal text, trade paperbacks). No panics —
all exited 0 — but the following correctness/quality bugs surfaced. Most were
fixed on 2026-06-06 (see below); the remainder are open.

### Fixed (2026-06-06)

- [x] **Pathological slowness (~100×) → fixed (196× speedup).** `reading/Williams
  E. Design Patterns…2026.pdf` ran at 3.8 pages/s (~128s); now **690 pages/s
  (~0.65s)**. Root cause: 54 image XObjects in *inherited* page resources were
  Flate+PNG-predictor-decoded on **every one of 450 pages**. Fix: memoize decoded
  images by `ObjectId` (`Arc`), and fonts likewise (`Rc`), so each shared object is
  built once. (`src/extract/lopdf_backend.rs`)

- [x] **Silent empty output → now warns.** A PDF that extracts to zero text now
  prints a stderr warning (suggesting OCR / unmapped fonts) instead of writing a
  silent 0-byte file. (`src/pipeline.rs::text_len`)
  - Note: the trigger doc `crypto-papers/Hash-Functions/sha256-384-512.pdf` uses
    **Type3 fonts with opaque subset glyph names** ("B7","BA",…) and no
    `/ToUnicode`. The glyphs are vector procedures with no Unicode mapping, so this
    file is genuinely **OCR-only** — not fixable by decoding. Hence the warning is
    the correct outcome here, not a decode path.

- [x] **Ordered-list numbering destroyed → fixed.** Real markers are preserved
  (IPC §230 renders as `230.`, not `1.`). `ListItem` gained a `marker` field
  captured in analysis and emitted by the Markdown renderer.
  (`src/model.rs`, `src/analyze/mod.rs::ordered_marker`, `src/render/md.rs`)

- [x] **Empty image references `![]()` → fixed.** Undecodable image XObjects
  (unsupported filter/colorspace) are now dropped instead of emitting a broken
  `![]()`. Maleki *Deep Learning* went 8 → 0 empty refs. (`src/render/images.rs`)

- [x] **Bare page numbers promoted to headings → fixed.** `###### 193`, `###### xii`
  etc. are no longer emitted as headings (a page-number/roman-numeral guard demotes
  them). Heading-size→level bucketing also coarsened to whole points so near-equal
  sizes don't fragment the outline. h6 counts dropped (e.g. Rust for Rustaceans
  690→624, with the bogus bare-number headings eliminated).
  (`src/analyze/mod.rs::{is_page_number_like, assign_heading_levels}`)

### Still open

- [ ] **Two-column reading order scrambled.** `crypto-papers/2002.05231.pdf`:
  left/right columns are interleaved mid-line, e.g. "shufﬂed sequence with
  probability better than guessing. Itfor shufﬂing data encrypted under an
  additively homomorphiallowsc multi-party computation". Gutter detection fails on
  this layout. (Related to the multi-column roadmap item below; high-risk to retune
  — see the reverted band-recursive note.)

- [ ] **Heading *hierarchy* still imperfect (deeper than the page-number fix).**
  Real chapter/section titles in some books still land on `######` instead of
  h1/h2 because the size→level ranking maps the smallest heading size to the
  deepest level. The bare-number flood is fixed; a proper hierarchy needs more than
  font-size ranking (e.g. style/numbering cues, capping level count). Also: the
  document title isn't always promoted to `#` (Thaler → `###`), author-affiliation
  superscripts attach to names as headings, and HTML `<title>`/`lang` aren't
  populated from detected metadata. Higher-risk; deferred.

- [ ] **Algorithm/pseudocode mis-detected as tables.** `crypto-papers/pairing-friendly-eliptic-curves-2005-133.pdf`
  "Algorithm 1" pseudocode is shredded into garbled 4-column Markdown tables
  (column-aligned line numbers + assignments trip the borderless-table heuristic).
  A guard risks regressing real tables (the cluster detector already has several
  precision guards); needs careful tuning against the benchmark. Deferred.

- [ ] **Math super/subscripts flattened** (known limitation). `y2 = x3+b` instead
  of `y² = x³+b`. Inherent without formula handling; see LaTeX-extraction below.

## Open roadmap (tractable)

- [ ] **Dense academic multi-column edge cases.** Clean two-column pages read
  correctly via gutter detection; a residual subset still interleaves columns
  (see `2002.05231.pdf` above, and bench doc `01030000000012`). The *catastrophic*
  NID failures in `opendataloader-bench` are extraction failures (scanned/vector/
  chart pages with little/no embedded text — docs 5, 141, 27, 110, 200), which are
  OCR-blocked, not reading-order bugs. The genuine multi-column residual is small.
  *(A band-recursive rewrite was prototyped and reverted — naively treating
  full-width spanners as band separators fragments full-width code listings in
  two-column papers, e.g. `2408.02509v1`; high-risk for marginal gain.)*

## Blocked in this environment (need external assets)

- [ ] **Non-Latin OCR.** The OCR path works end-to-end (`--features ocr`), but the
  bundled `ocrs` models are Latin-script. Validate with language-specific `.rten`
  models for CJK/etc.
- [ ] **LaTeX formula extraction.** Needs a local image→LaTeX vision model (e.g.
  pix2tex via `rten`). Feature scaffolding only; no model available here to verify.
- [ ] **Chart / image descriptions.** Needs a local VLM. Same constraint.
- [ ] **Korean special-form tables.** Niche heuristic; no Korean test PDF available
  to verify against, so deliberately not implemented blind.

## Explicitly NOT doing

- Hybrid AI HTTP server (external server + network — breaks the self-contained
  local binary).
