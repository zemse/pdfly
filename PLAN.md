# pdf-rs — PDF → Markdown CLI (Rust)

A Rust CLI that converts PDFs into clean Markdown (and JSON/HTML/text), with optional
chapter-wise splitting into multiple `.md` files. This is a from-scratch Rust
reimplementation of the data-extraction core of
[opendataloader-pdf](https://github.com/opendataloader-project/opendataloader-pdf)
(Apache-2.0, Java). This document records what that project does, names the reference
sources we port from, then orders the work as tick-box milestones — each task carries a
way to test it.

> **Status (implemented): M0–M14 all built and tested.** Pure-Rust extraction (incl. embedded-font
> decoding), analysis (lines, headings, nested lists, border tables, XY-Cut++ w/ cross-layout,
> header/footer, content-safety incl. hidden-text, sanitize, strikethrough), renderers
> (Markdown/GFM+HTML-tables, JSON, HTML, text), CLI, chapter split, image extraction
> (external/embedded/off), `--use-struct-tree` (tagged-PDF reading), `--tagged-pdf` (structure-tree
> writing), `--annotate` (debug PDF), `--threads`, and optional OCR (`--features ocr`). Whole corpus
> converts to all formats with no panics; 14 tests pass.
>
> Plus, from the follow-up backlog: **table row/col-span inference** (lattice cell merging),
> **borderless tables** (`--table-method cluster`), **expanded glyph-name decoding** (Type1/Differences
> with standard names), and **tagged-PDF marked-content (MCID) association** (`--tagged-pdf` now
> writes BDC/EMC marks + a struct tree that round-trips via `--use-struct-tree`).
>
> **Still partial/deferred (documented):** dense multi-column reading order (improved, not perfect);
> Type1 (FontFile) charstring programs with non-standard built-in encodings; tagged-PDF `/ParentTree`
> (reverse map) + formal PDF/UA conformance validation; bordered detection can over-trigger on ruled
> figures; Korean special-form tables; ML stretch (LaTeX formulas, chart descriptions). The hybrid AI
> server is intentionally dropped.

> **Companion doc:** [ARCHITECTURE.md](./ARCHITECTURE.md) is the detailed map of *how the source
> works* — the exact pipeline order, per-processor purpose, the XY-Cut++ algorithm, and clickable
> file/line links into the upstream repos. PLAN.md = *what to build & in what order*;
> ARCHITECTURE.md = *where to look while building each piece*. Milestones below cite the relevant
> `ARCHITECTURE.md §` to port from.

---

## 1. What the source project actually is

`opendataloader-pdf` is a **PDF parser for AI-ready data + a PDF accessibility (auto-tagging) tool**.

- **Core**: Java (`opendataloader-pdf-core`) + a thin CLI (`opendataloader-pdf-cli`, Apache Commons CLI).
- **Wrappers**: Python (`pip`), Node.js (`npm`) — both just shell out to the JVM.
- **Heavy lifting is NOT in this repo.** Actual PDF parsing + most layout algorithms come from
  **veraPDF** libraries: `validation-model`, `wcag-validation`, and especially
  **`wcag-algorithms`** (`org.verapdf.wcag.algorithms.*`). The repo's `processors/` mostly
  *orchestrate and post-process* a semantic tree veraPDF produces.
- **Two run modes**: deterministic local (pure Java, no GPU, ~0.02s/page) and **hybrid** (routes
  complex pages to an external AI HTTP backend — Docling or Hancom-AI — for OCR, formulas, complex
  tables, chart descriptions). Hybrid is out of scope for the Rust port.

→ Full orchestrator-vs-engine breakdown and entry points: [ARCHITECTURE.md §1–2](./ARCHITECTURE.md).

---

## 2. Decisions: backend + reference sources

### Backend — pure Rust (confirmed)
`lopdf` for low-level object/stream access + a **custom content-stream interpreter** we write.
No native deps, fully permissive, single static binary. Trade-off accepted: bigger extraction
effort and **no page rasterization** (so hidden-text detection uses declared-state heuristics, not
WCAG contrast — see Milestone 7). Extraction sits behind a `PdfBackend` trait so a `pdfium` impl
can be swapped in later if pure Rust proves insufficient on some PDFs.

### We port from references — we do NOT guess the hard parts
Reading the proven implementations and reimplementing cleanly in Rust removes the "get it wrong"
risk. **Clean-room rule: read for algorithm/logic, then write original Rust — no literal
line-by-line translation, no copied comments. Record provenance in `NOTICE`.**

| Layer | Primary reference | License | How we use it |
|---|---|---|---|
| Content-stream parser, fonts, encodings, CMaps | **Mozilla pdf.js** (`src/core/`) + **ISO 32000-1** spec | Apache-2.0 / free spec | Study closely — Apache is safe. Mirror operator handling & font-width logic |
| Same, Rust prior art | `lopdf`, `pdf` crate | MIT/Apache | Reuse where possible, study otherwise |
| Layout/semantic heuristics (headings, lists, table borders, reading order, contrast) | **`veraPDF/veraPDF-wcag-algs`** | **GPL-3.0 / MPL-2.0** | Read for *logic only*; clean-room reimplement. Do NOT translate verbatim |
| PDF parser logic, secondary | `veraPDF/veraPDF-parser` | GPL-3.0 / MPL-2.0 | Read for logic only (prefer pdf.js for copy-safety) |
| Output mapping (md/json/html), thresholds | **opendataloader-pdf** Java source (cloned at `/tmp/odl-pdf`) | Apache-2.0 | Match field-for-field; safe to follow closely |

> Licensing note: literal Java→Rust translation of veraPDF code would make derivative work bound by
> MPL-2.0 (file-level copyleft) or GPL-3.0. A clean reimplementation referencing the algorithm is
> fine and is the chosen approach.

---

## 3. Complete feature inventory (from source)

### 3.1 Input handling
- Single PDF, multiple PDFs, **directories** (recursive).
- **Encrypted PDFs** via `--password` (RC4/AES).
- **Page selection** `--pages "1,3,5-7"` (default all). Magic-number (`%PDF-`) validation.
- Tagged-PDF: if `/StructTreeRoot` exists, optionally use it (`--use-struct-tree`).

### 3.2 Extraction primitives (per page)
- **Text chunks**: value, bbox `[llx,lly,urx,ury]` (PDF points, origin bottom-left), font name/size/
  weight, bold/italic, text/background/stroke color.
- **Image chunks**: bytes + bbox + format. **Line/line-art chunks**: vector lines & shapes (for table
  borders + strikethrough).

### 3.3 Layout pipeline (feature list — for the *exact* 18-stage order & file/line links see [ARCHITECTURE.md §5](./ARCHITECTURE.md))
> Note: in the real pipeline **reading order (step 11) runs last, after all detection**, sorting the
> semantic objects — not the raw chunks ([ARCHITECTURE.md §3.1, §6](./ARCHITECTURE.md)). Structured
> detection is always on internally, even for text-only output.
1. **Content filter**: dedupe, drop decoration glyphs, drop tiny text (<~1pt), drop off-page (outside
   Media/CropBox), merge adjacent chunks, trim/split whitespace.
2. **Hidden-text filter** (contrast in source; declared-state heuristics for us). AI-safety guard.
3. **Tables**: border-based (vector-line grid → cells, row/col span), cluster-based
   (`--table-method cluster`), Korean special-form tables.
4. **Strikethrough** (`--detect-strikethrough`): horizontal line through text center.
5. **Text-line assembly**: merge chunks → lines, insert spaces from gaps, sort.
6. **Header/footer**: repeating top/bottom content across pages (excluded unless `--include-header-footer`).
7. **Lists**: bullets (•◦■–-), numbered (`1.` `1)` `a)` `i)` roman, Korean), indentation nesting,
   cross-page continuation.
8. **Paragraphs**: group lines by alignment (L/C/R/justify), spacing, indentation, font consistency.
9. **Headings**: probability from font-size/weight *rarity* vs. body → threshold; levels 1–6 by
   clustering distinct heading sizes; build outline.
10. **Captions**: associate nearby small text with images/tables.
11. **Reading order** (**XY-Cut++**, `--reading-order xycut|off`): recursive XY projection cuts,
    cross-layout full-width element handling, density heuristic for columns vs. newspaper.
12. **Level/nesting + stable ID** assignment for cross-refs.

### 3.4 Semantic element types
`heading`(level), `paragraph`, `list`+`list item`(numbering), `table`+`row`+`cell`(span), `image`,
`picture`(image+caption), `caption`, `formula`(LaTeX, hybrid), `footnote`, `header`/`footer`,
`text chunk`/`text line`.

### 3.5 Output formats (`--format json,markdown,html,text,pdf,tagged-pdf`)
- **Markdown**: `#`×N (1–6); paragraphs plain; `- `/nested lists; GFM pipe tables; `![alt](<path>)`;
  `$$…$$`; `~~…~~`.
- **Markdown+HTML** (`--markdown-with-html`): raw `<table>` with `colspan`/`rowspan` for merged cells.
- **HTML**: full doc; `<h1-6>`,`<p>`,`<ul><li>`,`<table border="1">` spans, `<img>`,`<figure>`,
  `<div class="math-display">\[…\]</div>`, inline `<span style>` (font-size pt×4/3→px, `rgb()` color,
  weight, italic, line-through).
- **JSON** (`schema.json`): root `file name`,`number of pages`,`author`,`title`,`creation/modification
  date`,`kids[]`; every element has `type`,`id`,`page number`,`bounding box [l,b,r,t]` + type-specific
  fields.
- **Plain text**: values only; lists indented; table rows tab-separated.
- **Tagged PDF / PDF-UA / annotated debug PDF**: write tags back. **Out of scope** for the Rust port.

### 3.6 Output controls
`--output-dir`, `--to-stdout`; images `--image-output off|embedded|external`, `--image-format png|jpeg`,
`--image-dir`; page separators `--{markdown,text,html}-page-separator` (support `%page-number%`);
`--keep-line-breaks`, `--replace-invalid-chars`, `--quiet`.

### 3.7 Safety / privacy
`--content-safety-off all|hidden-text|off-page|tiny|hidden-ocg` (filters on by default);
`--sanitize` (regex → placeholders: email, intl phone, passport-ish IDs, credit cards, long digit
runs, IPv4/IPv6/MAC, 15-digit, URLs — rules in `FilterConfig.java`).

### 3.8 Perf / misc
`--threads N` (experimental per-page parallelism); deterministic output is a design goal.
Hybrid AI backend (`--hybrid*`): OCR 80+ langs, LaTeX, chart descriptions, complex tables — these
were one **external HTTP server** in the source. We **drop the server**; the underlying
capabilities (OCR, etc.) are reconsidered as *local pure-Rust* features in §4 / the backlog.

### 3.9 New feature (not in source)
**Chapter-wise split**: one `.md` per top-level heading (H1, optionally H2) into a directory, with an
index/TOC and `NN-title.md` filenames.

---

## 4. Scope for pdf-rs

**Shipped (v1, M0–M8):** local deterministic extraction → Markdown/JSON/HTML/text; headings,
paragraphs, lists, bordered tables; XY-Cut++ reading order; header/footer filtering; page selection;
encrypted PDFs; page separators; sanitization; content-safety (tiny/off-page); chapter split.

**Backlog (planned, M9–M14 below):** image-byte extraction, struct-tree path, threads, strikethrough,
captions, Markdown+HTML tables, better multi-column reading order, font-cmap decoding, nested lists,
borderless/span tables — **and**, re-scoped from the old "out of scope" list, the capabilities below.

**Re-evaluating the old "out of scope" list** (now that the hybrid server is dropped):

| Capability | Verdict | Why / pure-Rust path |
|---|---|---|
| **Hybrid AI backend** (`--hybrid*`, external Docling/Hancom HTTP server) | ❌ **Dropped** | Requires running a separate server + network; breaks the "single static binary, 100% local" promise. The *features* it provided are reconsidered below as local options. |
| **OCR** (scanned PDFs) | ✅ **Planned (optional)** — M12 | Feasible in pure Rust via `ocrs` + `rten` (ONNX, no native dep; model files downloaded once). Feature-gated so the core binary stays tiny. We have `chinese_scan.pdf` to test. |
| **Tagged-PDF / PDF-UA writing** | ✅ **Planned** — M13 | No ML; pure structure-tag writing with `lopdf`. This is the accessibility half of the source. Substantial but in-language. |
| **Annotated debug PDF** (draw bboxes) | ✅ **Planned** — M11 | Pure `lopdf` overlay; small; great for debugging layout. |
| **Korean special-form tables** | ✅ **Planned (low priority)** — M14 | Pure heuristic, niche; cheap to add later. |
| **LaTeX formula extraction** | ◐ **Stretch (optional)** — M14 | Needs a vision model (image→LaTeX, e.g. pix2tex) via ONNX/`rten`. Heavy + niche; optional feature only. |
| **Chart / image descriptions** | ◐ **Stretch (optional)** — M14 | Needs a local VLM. Heaviest; optional feature only, off by default. |

Net: **only the hybrid server is truly dropped.** Everything else moves into the backlog, with the
ML-dependent items gated behind optional Cargo features so the default binary stays dependency-light.

---

## 5. Test corpus & artifacts

Maintain `tests/corpus/` (gitignored large files; a small curated subset committed). Sources:

- **Bundled samples** (already at `/tmp/odl-pdf/samples/pdf/`, 14 PDFs) — copy these in. Variety:
  arXiv papers (`1901.03003`, `2408.02509v1`), Italian financial statement, `lorem.pdf`,
  `chinese_scan.pdf`, and the **PDF/UA reference suite** (magazine, invoice, academic abstract,
  presentation, brochure, multilingual book chapters, scanned, form). `samples/json/lorem.json` is a
  ready golden output.
- **opendataloader-bench** (`github.com/opendataloader-project/opendataloader-bench`) — 200 real-world
  PDFs **with ground truth**; the project's own accuracy benchmark. Best golden/regression set.
- **Mozilla pdf.js test corpus** (`github.com/mozilla/pdf.js`, `test/pdfs/`) — hundreds of tricky/edge
  PDFs (weird fonts, encodings, encryption, broken files). Apache-2.0. Great for parser robustness.
- **veraPDF / PDF Association test suites** — PDF/UA + PDF/A reference files.
- **arXiv** — bulk scientific PDFs (multi-column, formulas, dense tables) for stress testing.
- **GovDocs1 / Common Crawl PDFs / open-government portals** — large random real-world variety.
- **Layout datasets** DocLayNet, PubLayNet — PDFs/images **with element annotations** to score layout
  accuracy.
- **Synthetic, known-ground-truth**: generate PDFs from Typst / LaTeX / `printpdf` where we author the
  structure → exact expected Markdown for deterministic unit tests.

**How testing works per task:** each milestone task below has a `🧪` line. Tactics used:
- *Unit*: `cargo test` on synthetic/fixed inputs with asserted output.
- *Golden/snapshot*: run pdf-rs on a corpus PDF, diff against a committed `.md`/`.json` (use `insta`
  crate); update deliberately. Cross-check JSON against the Java tool's output on the same file.
- *Eyeball*: open generated `.md`/`.html` for a known sample and read it.
- *Corpus sweep*: run over the whole corpus, assert no panics + basic invariants (non-empty output,
  valid UTF-8, balanced markup).

---

## 6. Milestones (tick-box)

### ✅ Milestone 0 — Project skeleton
- [ ] `git init`; Cargo binary crate; add `lopdf`, `clap` (derive), `anyhow`/`thiserror`, `serde_json`, `image`, `insta` (dev).
  🧪 `cargo build` succeeds; `cargo test` runs (empty).
- [ ] Module layout: `extract/`, `model/`, `analyze/`, `render/`, `cli.rs`, `pipeline.rs` (canonical source→module mapping in [ARCHITECTURE.md §9](./ARCHITECTURE.md)).
  🧪 `cargo check` clean.
- [ ] `clap` CLI surface matching source flags (input(s), `-o`, `-f`, `-q`, `--pages`, `--password`, etc.).
  🧪 `pdf-rs --help` shows all flags; bad input → clean error, not panic.
- [ ] Copy `/tmp/odl-pdf/samples/pdf/*` into `tests/corpus/`; write a corpus-sweep test harness.
  🧪 Harness lists all corpus PDFs.

### ✅ Milestone 1 — Extraction layer (highest risk; reference pdf.js + spec)
↳ Port target: the veraPDF parse boundary — [ARCHITECTURE.md §4](./ARCHITECTURE.md) (`preprocessing`, `parseChunks`) + §8 (`IObject` model). Rust mapping in §9.
- [ ] `lopdf`: open doc, decrypt (`--password`), decompress streams, walk page tree, read resources & metadata, detect `/StructTreeRoot`.
  🧪 Unit: page count + title/author match `pdfinfo`/known values for each corpus PDF.
- [ ] `--pages` range parser (`"1,3,5-7"`).
  🧪 Unit: parse cases incl. malformed; out-of-range clamped/errored.
- [ ] Content-stream interpreter: graphics/text state (CTM, `Tm`, `Tf`, `Tc/Tw/Tz/TL`, color, render mode); `BT/ET`, `Td/TD/Tm/T*`, `Tj/TJ/'/"`.
  🧪 Unit on a hand-built minimal PDF with known glyph positions; assert bbox within tolerance.
- [ ] Font decoding: WinAnsi/MacRoman/`Differences`, CID/CMap (`ToUnicode`, Identity-H); glyph advances from width arrays.
  🧪 Golden: extracted text of `lorem.pdf` and `chinese_scan.pdf` (digital text) matches expected string.
- [ ] Vector lines (`m/l/re`+stroke → segments) and images (XObject + inline, decode via `image`).
  🧪 Eyeball: dump detected lines/images count for a table PDF and an image PDF.
- [ ] `PdfBackend` trait → `Page { text_runs, images, lines, media_box, crop_box }`; raw-chunk JSON dumper.
  🧪 **Golden**: chunk JSON geometry vs. Java tool's JSON on `lorem.pdf` (compare bboxes within tolerance).

### ✅ Milestone 2 — Minimal pipeline → Markdown / text / JSON
↳ Port targets: `ContentFilterProcessor`, `TextLineProcessor`, `ParagraphProcessor` ([ARCHITECTURE.md §5.1](./ARCHITECTURE.md)); renderers + JSON serializers ([§7](./ARCHITECTURE.md)).
- [ ] Content filtering (dedupe, tiny, off-page, merge, whitespace).
  🧪 Unit on synthetic chunks; corpus sweep: no panics.
- [ ] Text-line assembly (gap-based spacing, sort) + naive top→bottom-left→right order.
  🧪 Golden: `lorem.pdf` lines in correct order.
- [ ] Basic paragraph grouping (spacing/blank-line).
  🧪 Eyeball `lorem.pdf` paragraphs.
- [ ] Markdown, plain-text, JSON renderers for paragraphs/lines/chunks.
  🧪 **Milestone gate**: single-column PDF → readable Markdown; JSON validates against `schema.json` subset.

### ✅ Milestone 3 — Headings & lists
↳ Port targets: `HeadingProcessor` + `utils/TextNodeStatistics`, `ListProcessor` (+ WCAG `NodeUtils`/`ListLabelsUtils`) ([ARCHITECTURE.md §5.1, §8](./ARCHITECTURE.md)).
- [ ] Font-size/weight statistics (body mode + rarity scoring; port `TextNodeStatistics` logic).
  🧪 Unit: histogram + rarity on synthetic font sets.
- [ ] Heading probability + level clustering (1–6).
  🧪 Golden: `PDFUA-Ref-2-08_BookChapter.pdf` heading levels look right.
- [ ] List detection: bullets/numbers/roman/letters, indentation nesting, cross-page continuation.
  🧪 Golden: a list-heavy sample → correct `- `/`1.` and nesting.
- [ ] Wire to MD (`#`,`-`,`1.`) + JSON (`heading level`, `numbering style`).
  🧪 **Milestone gate**: snapshot tests on 3 corpus PDFs accepted.

### ✅ Milestone 4 — Tables
↳ Port targets: `LinesPreprocessingConsumer` (WCAG) + `TableBorderProcessor`/`AbstractTableProcessor`/`TableStructureNormalizer`; table serializers/renderers ([ARCHITECTURE.md §4, §5.1, §7](./ARCHITECTURE.md)).
- [ ] Border-based detection from vector lines → grid, cell assignment, row/col span.
  🧪 Unit on synthetic ruled table; golden on `issue-336-conto-economico-bialetti.pdf`.
- [ ] GFM pipe-table render + `--markdown-with-html` HTML-table path for spans.
  🧪 Eyeball merged-cell table renders correctly in both modes.
- [ ] JSON `table/row/cell` + HTML `<table border="1">`.
  🧪 **Milestone gate**: bordered tables render in MD/HTML/JSON; cell text complete.

### ✅ Milestone 5 — Reading order (XY-Cut++)
↳ Port target: `XYCutPlusPlusSorter` — full 4-phase algorithm with line refs & constants in [ARCHITECTURE.md §6](./ARCHITECTURE.md). Apache-2.0 + self-contained = most directly portable file.
- [ ] Recursive XY projection cuts; cross-layout (full-width) handling; density heuristic; narrow-outlier filter. `--reading-order xycut|off`.
  🧪 Unit on synthetic 2-column layout; **golden** on arXiv `2408.02509v1.pdf` (multi-column) — order correct.
  🧪 Compare `xycut` vs `off` output to confirm it changes column sequencing.

### ✅ Milestone 6 — Images, captions, headers/footers, separators, HTML
↳ Port targets: `CaptionProcessor`, `HeaderFooterProcessor`, `ImagesUtils`/`Base64ImageUtils`, `HtmlGenerator` ([ARCHITECTURE.md §5.1, §7](./ARCHITECTURE.md)).
- [ ] Image extraction modes `off|embedded|external`, `--image-format`, `--image-dir`; `![alt](<path>)`/Base64/`<img>`.
  🧪 Unit: external mode writes files + correct relative links; embedded mode emits valid data URI.
- [ ] Caption association → `picture` + `linked content id`.
  🧪 Golden on a figure+caption sample.
- [ ] Header/footer repeat-detection; `--include-header-footer`.
  🧪 Golden: magazine sample — running header excluded by default, included with flag.
- [ ] Page separators (`%page-number%`), `--keep-line-breaks`, `--replace-invalid-chars`.
  🧪 Unit: separator token substitution.
- [ ] Full HTML renderer with inline styles.
  🧪 **Milestone gate**: local-extraction feature parity across MD/HTML/JSON/text on corpus subset.

### ✅ Milestone 7 — Safety & privacy
↳ Port targets: `HiddenTextProcessor` (note rasterization caveat), `ContentSanitizer`+`FilterConfig` regexes, `StrikethroughProcessor` ([ARCHITECTURE.md §5.1, §10](./ARCHITECTURE.md)).
- [ ] Content-safety: hidden-text via declared-state heuristics (render mode 3, zero-size/transparent, color==bg, OCG off), tiny, off-page, hidden-OCG; `--content-safety-off`.
  🧪 Unit on a PDF with injected invisible text → excluded by default, present when disabled.
- [ ] `--sanitize` regex rules (port `FilterConfig` patterns).
  🧪 Unit: emails/phones/cards/IPs/URLs → placeholders; non-matches untouched.
- [ ] Strikethrough detection (`--detect-strikethrough`).
  🧪 Unit on a struck-text sample → `~~…~~`.

### ✅ Milestone 8 — Chapter-wise split (new feature)
- [ ] `--split` + `--split-by heading[:level]`; walk ordered elements, new file per split heading.
  🧪 Unit on synthetic doc with 3 H1s → 3 files.
- [ ] Slugified `NN-title.md` filenames; `index.md`/TOC with links; front-matter (pre-first-heading) handling.
  🧪 **Milestone gate**: `PDFUA-Ref-2-08_BookChapter.pdf` splits into per-section `.md` + index; links resolve.

### ✅ Milestone 9 — Robustness, perf, polish
↳ Port target: `TaggedDocumentProcessor` (struct-tree path) ([ARCHITECTURE.md §3.1, §5.1](./ARCHITECTURE.md)); threading model in `processDocument` ([§5](./ARCHITECTURE.md)).
- [x] Docs (README), `--help`, PLAN status. *(done)*
- [ ] Optional tagged-PDF reading (`--use-struct-tree`) when `/StructTreeRoot` present.
  🧪 Golden on a well-tagged PDF/UA sample vs. heuristic output.
- [ ] `--threads` per-page parallelism (rayon), behind flag.
  🧪 Output identical with 1 vs N threads on corpus subset; time improves.
- [ ] Corpus-wide regression with **insta snapshots** (self-consistent oracle).
  🧪 Full corpus sweep: zero panics; snapshots stable.
- [ ] `NOTICE`/`CREDITS` attributing opendataloader + veraPDF (clean-room provenance).
  🧪 File present; referenced from README.

### ✅ Milestone 10 — Extraction & layout quality (close the known gaps)
↳ The quality backlog from the v1 build. ([ARCHITECTURE.md §5.1, §6](./ARCHITECTURE.md))
- [ ] **Font cmap decoding** for embedded fonts without `/ToUnicode` (use the font program's
  cmap / glyph names) so custom-encoded PDFs stop producing gibberish.
  🧪 A known-bad corpus PDF (e.g. `issue-336-...`) decodes to real words.
- [ ] **Multi-column reading order**: band-then-column recursion so a full-width abstract over a
  two-column body doesn't interleave.
  🧪 Golden on arXiv `2408.02509v1.pdf` p1: abstract before body, columns in order.
- [ ] **Borderless / clustered tables** and **row/col span** inference.
  🧪 Golden on a borderless-table sample; spans render in HTML/JSON.
- [ ] **Nested lists** (indentation depth) instead of flattening.
  🧪 Nested sample → nested `-`/`1.` in Markdown.
- [ ] Heading-detection refinement (font-weight rarity, not just size; suppress false positives).
  🧪 Snapshot diff shows fewer spurious headings on corpus.

### ✅ Milestone 11 — Images & annotated output
- [ ] **Image-byte extraction**: decode image XObjects (`image` crate) and write files;
  `--image-output off|embedded|external`, `--image-format png|jpeg`, `--image-dir`.
  🧪 External mode writes files + correct `![](...)` links; embedded emits valid data URIs.
- [ ] **Caption association** (image/table ↔ nearby small text) → `picture` + `linked content id`.
  🧪 Golden on a figure+caption sample.
- [ ] **Annotated debug PDF**: draw element bboxes back onto the PDF (pure `lopdf` overlay).
  🧪 Output PDF opens; one rect per detected element.

### ✅ Milestone 12 — OCR for scanned PDFs (optional feature)
↳ Pure-Rust, feature-gated (`--features ocr`); no native dep. Crates: `ocrs` + `rten` (ONNX).
- [ ] Detect image-only / no-text pages; rasterize region and run OCR; merge OCR words as text runs.
  🧪 `chinese_scan.pdf` (and an English scan) yield non-empty, sensible text.
- [ ] Model files downloaded/cached on first use; core binary unaffected when feature is off.
  🧪 Default build has no ONNX dep; `--features ocr` build runs OCR.

### ✅ Milestone 13 — Tagged-PDF / PDF-UA writing
↳ Accessibility half of the source; pure structure-tag writing with `lopdf` (no ML).
- [ ] Write a `/StructTreeRoot` (headings, paragraphs, lists, tables, reading order) into a copy of the PDF.
  🧪 Output parses; structure tree present; spot-check tag tree matches detected elements.
- [ ] (Stretch) PDF/UA conformance pass.
  🧪 Validate against a checker if available.

### ◑ Milestone 14 (core done; ML stretch deferred) — Long tail
- [ ] **Strikethrough** detection (`--detect-strikethrough`) → `~~…~~` / `<del>`.
  🧪 Struck-text sample renders `~~…~~`.
- [ ] **Markdown+HTML table mode** (`--markdown-with-html`) for merged cells.
  🧪 Merged-cell table emits `<table>` with `colspan`/`rowspan`.
- [ ] **Hidden-text declared-state heuristics** (render mode 3, transparent, color==bg, OCG off)
  under `--content-safety-off` granularity (`hidden-text`, `hidden-ocg`).
  🧪 Injected invisible text excluded by default, present when disabled.
- [ ] **Korean special-form tables** heuristic.
  🧪 Korean form sample → table.
- [ ] ◐ **LaTeX formula** extraction (optional feature, vision model via `rten`).
- [ ] ◐ **Chart/image descriptions** (optional feature, local VLM; off by default).

---

## 7. Open questions
1. **Heuristic thresholds**: exact numbers live in veraPDF-wcag-algs (`NodeUtils`/`ListLabelsUtils`/
   `CaptionUtils`) — read them there ([ARCHITECTURE.md §5.1, §8](./ARCHITECTURE.md)), then tune
   against the corpus. (No longer guessing.)
2. **Determinism vs. fidelity**: match Java output closely, or optimize Markdown for LLM/RAG?
3. Chapter-split default level (H1 only vs. configurable) and front-matter handling.

## 8. Suggested next steps (M0–M8 shipped)
Highest impact first:
1. **M10 font-cmap decoding** + **multi-column reading order** — biggest correctness wins on real PDFs.
2. **M11 image extraction** — the most-requested missing output feature.
3. **M9 `NOTICE`/CREDITS** + insta snapshots — cheap, closes provenance + regression gaps.
4. **M12 OCR** (optional feature) — unlocks scanned PDFs without bloating the default binary.
5. **M13 tagged-PDF writing** — opens the accessibility use case.

The hybrid AI server is intentionally **not** on this list (dropped, §4).
