# pdf-rs

A fast, dependency-light **PDF → Markdown** command-line tool written in pure Rust.
It also emits JSON (with bounding boxes), HTML, and plain text, and can split a
document into one Markdown file per chapter.

Pure Rust, no native libraries, no GPU, no network — a single static binary.

## Install / build

```bash
cargo build --release
# binary at target/release/pdf-rs
```

## Usage

```bash
# PDF -> Markdown (written next to the input file)
pdf-rs report.pdf

# choose output dir and formats
pdf-rs report.pdf -o out/ -f markdown,json,html,text

# only some pages
pdf-rs report.pdf --pages 1,3,5-7

# encrypted PDF
pdf-rs secret.pdf -p mypassword

# split a book into one Markdown file per chapter (+ index.md) in out/<name>/
pdf-rs book.pdf -o out/ --split
pdf-rs book.pdf -o out/ --split --split-level 2   # split on H1 and H2

# images: extract to files (default), embed as base64, or drop
pdf-rs report.pdf --image-output external --image-format png
pdf-rs report.pdf --image-output embedded
pdf-rs report.pdf --image-output off

# use the PDF's own tags (tagged PDFs) instead of layout heuristics
pdf-rs tagged.pdf --use-struct-tree

# write a tagged PDF (adds a structure tree) / an annotated debug PDF
pdf-rs report.pdf --tagged-pdf
pdf-rs report.pdf --annotate

# redact sensitive data; detect strikethrough; HTML tables in Markdown
pdf-rs report.pdf --sanitize --detect-strikethrough --markdown-with-html

# faster on big PDFs (deterministic)
pdf-rs big.pdf --threads 8

# report processing time and throughput (pages/sec)
pdf-rs big.pdf --timing

# stream to stdout (single format); whole directory (recursive)
pdf-rs report.pdf -f markdown --to-stdout
pdf-rs ./pdfs/ -o out/
```

### OCR for scanned PDFs (optional)

OCR is a pure-Rust optional feature (no native deps). Build with it enabled and
point to [ocrs](https://github.com/robertknight/ocrs) `.rten` model files:

```bash
cargo build --release --features ocr
export PDFRS_OCR_DETECTION_MODEL=/path/to/text-detection.rten
export PDFRS_OCR_RECOGNITION_MODEL=/path/to/text-recognition.rten
pdf-rs scanned.pdf            # image-only pages are OCR'd automatically
```

The default build omits OCR entirely, keeping the binary small.

Run `pdf-rs --help` for all options.

## What it does

- **Text extraction**: a content-stream interpreter over `lopdf` recovers positioned
  text runs with fonts, sizes, weights, and colors (ToUnicode / encoding / CID width
  decoding).
- **Layout analysis**: line assembly, multi-column line splitting, body-font
  statistics, heading detection (relative font-size ranking → levels 1–6), list
  detection (bulleted/numbered), border-based table detection, and **XY-Cut++**
  reading order.
- **Header/footer** removal (repeated running content), **content-safety**
  filtering (tiny / off-page text), and optional **sanitization**.
- **Renderers**: GFM Markdown, schema-aligned JSON with bounding boxes, standalone
  HTML, plain text, and chapter-wise Markdown.

## Origins

A from-scratch Rust reimplementation of the data-extraction core of
[opendataloader-pdf](https://github.com/opendataloader-project/opendataloader-pdf)
(Apache-2.0). Algorithms were studied and reimplemented clean-room; no code was
copied. See [ARCHITECTURE.md](./ARCHITECTURE.md) for how the original works and
[PLAN.md](./PLAN.md) for the build plan. The XY-Cut++ reading order follows
opendataloader's `XYCutPlusPlusSorter`; layout heuristics are informed by
veraPDF's `wcag-algorithms`.

## Known limitations

- Dense multi-column academic papers (full-width abstract over a two-column body)
  can still interleave in reading order (improved, not perfect).
- Type1 (`FontFile`) subset fonts with non-standard built-in encodings and no
  `/ToUnicode` may still mis-decode (embedded TrueType/CFF and standard glyph
  names now decode).
- Bordered table detection can over-trigger on ruled figures; borderless
  detection (`--table-method cluster`) is conservative and opt-in.
- `--tagged-pdf` writes marked content + a structure tree (round-trips via
  `--use-struct-tree`) but does not yet emit a `/ParentTree` or run formal
  PDF/UA conformance validation.
- LaTeX formulas and chart/image descriptions need local ML models (not built).

## Tests

```bash
cargo test
```

Tests run against a committed corpus (`tests/corpus/`) using snapshot/invariant
checks (no external Java oracle required).
