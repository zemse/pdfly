# pdfly

A fast, dependency-light **PDF → Markdown** command-line tool written in pure Rust.
It also emits JSON (with bounding boxes), HTML, and plain text, and can split a
document into one Markdown file per chapter.

Pure Rust, no native libraries, no GPU, no network — a single static binary.

## Install / build

```bash
# install the `pdfly` binary from crates.io
cargo install pdfly

# ...or from git
cargo install --git https://github.com/zemse/pdfly

# ...or build locally
cargo build --release   # binary at target/release/pdfly
```

## Usage

`pdfly read <file>` converts a PDF and prints the result to **stdout** by default.
Pass `--out <path>` to write a file instead; the format is inferred from the
extension (`.md`, `.json`, `.html`, `.txt`) unless you override it with `--format`.

```bash
# PDF -> Markdown on stdout
pdfly read report.pdf

# write to a file (format inferred from the extension)
pdfly read report.pdf -o report.md
pdfly read report.pdf -o report.json

# pick a format explicitly (still stdout)
pdfly read report.pdf -f json

# only some pages
pdfly read report.pdf --pages 1,3,5-7

# encrypted PDF
pdfly read secret.pdf -p mypassword

# split a book into one Markdown file per chapter (+ index.md) in a directory
pdfly read book.pdf -o out/ --split
pdfly read book.pdf -o out/ --split --split-level 2   # split on H1 and H2

# images: extract to files (default), embed as base64, or drop
# (external images require --out; stdout output drops images)
pdfly read report.pdf -o report.md --image-output external --image-format png
pdfly read report.pdf -o report.md --image-output embedded
pdfly read report.pdf --image-output off

# use the PDF's own tags (tagged PDFs) instead of layout heuristics
pdfly read tagged.pdf --use-struct-tree

# write a tagged PDF (adds a structure tree) / an annotated debug PDF (need --out)
pdfly read report.pdf -o report.md --tagged-pdf
pdfly read report.pdf -o report.md --annotate

# redact sensitive data; detect strikethrough; HTML tables in Markdown
pdfly read report.pdf --sanitize --detect-strikethrough --markdown-with-html

# faster on big PDFs (deterministic)
pdfly read big.pdf --threads 8

# report processing time and throughput (pages/sec)
pdfly read big.pdf --timing
```

### OCR for scanned PDFs (optional)

OCR is a pure-Rust optional feature (no native deps). Build with it enabled and
point to [ocrs](https://github.com/robertknight/ocrs) `.rten` model files:

```bash
cargo build --release --features ocr
export PDFRS_OCR_DETECTION_MODEL=/path/to/text-detection.rten
export PDFRS_OCR_RECOGNITION_MODEL=/path/to/text-recognition.rten
pdfly read scanned.pdf          # image-only pages are OCR'd automatically
```

The default build omits OCR entirely, keeping the binary small.

Run `pdfly read --help` for all options.

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
[TASKS.md](./TASKS.md) for open issues and remaining work. The XY-Cut++ reading order follows
opendataloader's `XYCutPlusPlusSorter`; layout heuristics are informed by
veraPDF's `wcag-algorithms`.

## Known limitations

- Dense multi-column academic papers (full-width abstract over a two-column body)
  can still interleave in reading order (improved, not perfect).
- Type1 (`FontFile`) subset fonts with non-standard built-in encodings and no
  `/ToUnicode` may still mis-decode (embedded TrueType/CFF and standard glyph
  names now decode).
- Borderless (column-aligned) table detection is on by default; pass
  `--table-method ruled` to restrict detection to ruled-border tables only.
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
