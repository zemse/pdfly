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
  sizes don't fragment the outline. (`src/analyze/mod.rs::{is_page_number_like,
  assign_heading_levels}`)

- [x] **Bold body-paragraph text split into per-line h6 headings → fixed.** Book
  chapter intros set in a larger display font had every wrapped line promoted to a
  heading ("…teaches you and what", "is always wide. Over time…"). A line starting
  with a lowercase letter is now treated as prose, not a heading. h6 counts dropped
  sharply (Rust for Rustaceans 624→383, Williams 825→554); academic-paper headings
  unaffected. (`src/analyze/mod.rs`, `prose_fragment`)

- [x] **Algorithm/pseudocode mis-detected as tables → fixed.** `crypto-papers/pairing-friendly-eliptic-curves-2005-133.pdf`
  "Algorithm 1" no longer shreds into a 4-column table: a borderless block whose
  left column is mostly step labels ("1:", "2:", …) is rejected. Other docs' table
  counts unchanged (lookups 20, POSEIDON 45). (`src/analyze/tables.rs::is_step_label`)

All of the above were validated on `opendataloader-bench` (200 docs): overall
0.785 → **0.791**, mhs (heading hierarchy) 0.663 → **0.685**, nid/teds unchanged —
i.e. net improvement, no regression. (See memory `opendataloader-bench-setup`.)

### Still open — needs substantial work (not a quick/safe fix)

- [ ] **Two-column reading order scrambled on tight-gutter pages.**
  `crypto-papers/2002.05231.pdf` has a ~12pt gutter narrower than the
  benchmark-tuned line-split threshold (`max(1.3·fs, 10pt)` ≈ 12.3pt here), so
  left/right column runs merge into one line ("…guessing. Itfor shufﬂing…").
  Lowering the global threshold regresses the benchmark (already swept — 1.3 won);
  the real fix is **word-level column reconstruction** (use a detected gutter x to
  split lines), a larger rework. Confirmed against the bench, not guessed.

- [ ] **Heading *hierarchy* depth (the flood is fixed; depth isn't).** Real
  chapter/section titles in some books still land on `######` because the size→level
  ranking sends the smallest heading size to the deepest level. A faithful hierarchy
  needs more than font-size ranking (numbering/style cues, level-count capping).
  Also minor: document title not always `#` (Thaler → `###`), author-affiliation
  superscripts attach to names, HTML `<title>`/`lang` not populated. Higher-risk.

## Requires external assets — cannot be built or verified in this environment

These are not actionable here: each needs a model or test asset that doesn't exist
locally. Listed so they aren't lost, not as pending work for this machine.

- **Non-Latin OCR.** The OCR path works end-to-end (`--features ocr`), but the
  bundled `ocrs` models are Latin-script. Needs language-specific `.rten` models.
- **LaTeX formula extraction** (covers the math super/subscript limitation, e.g.
  `y2` vs `y²`). Needs a local image→LaTeX vision model (e.g. pix2tex via `rten`).
- **Chart / image descriptions.** Needs a local VLM.
- **Korean special-form tables.** No Korean test PDF available to verify against, so
  deliberately not implemented blind.

## Explicitly NOT doing

- Hybrid AI HTTP server (external server + network — breaks the self-contained
  local binary).
