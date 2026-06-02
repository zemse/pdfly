# Architecture of `opendataloader-pdf` (reference notes for the Rust port)

Notes from reading the source, to guide the Rust reimplementation in [PLAN.md](./PLAN.md).
Personal-use study notes — clean-room reimplement, don't copy code.

## Link conventions

Two upstream repos are referenced. Paths below are **repo-relative**; click to open on GitHub.

- **ODL** = `opendataloader-pdf` (Apache-2.0) — orchestration + output. Base:
  `https://github.com/opendataloader-project/opendataloader-pdf/blob/main/`
  Local clone used while writing this: `/tmp/odl-pdf/` (ephemeral — re-clone if gone).
- **WCAG** = `veraPDF-wcag-algs` (GPL-3.0/MPL-2.0) — the semantic engine. Base:
  `https://github.com/veraPDF/veraPDF-wcag-algs` (package `org.verapdf.wcag.algorithms.*`).
- **PARSER** = `veraPDF-parser` / `veraPDF-library` — the actual PDF/COS parser, fonts, chunk
  extraction. `https://github.com/veraPDF/veraPDF-parser`, `https://github.com/veraPDF/veraPDF-library`.

Line numbers (e.g. `:141`) refer to files read directly and may drift across versions.

---

## 1. The big picture: this repo is an orchestrator, not the engine

`opendataloader-pdf` is **two layers**:

1. **veraPDF (external dependency)** does the genuinely hard parts:
   - Parse the PDF/COS file, decrypt, decode streams, parse fonts/encodings/CMaps.
   - Extract **content chunks** per page: `TextChunk`, `ImageChunk`, `LineChunk`, `LineArtChunk`
     — each with a bounding box, and text chunks with font/size/weight/color.
   - Provide the **semantic object model** (`IObject` and subclasses) and several semantic
     algorithms (table-border detection from lines, cluster tables, contrast ratio, list-label
     parsing helpers, heading-probability helpers).
2. **This repo (`opendataloader-pdf-core`)** orchestrates a pipeline of **processors** that turn
   those chunks into a semantic tree, then **generators/serializers** that render the tree to
   Markdown / JSON / HTML / text / tagged-PDF.

> **Consequence for the Rust port:** the parser + the `IObject` model + a handful of veraPDF
> algorithms have **no Rust equivalent**. We rebuild them: `lopdf` + a custom content-stream
> interpreter (reference: pdf.js + ISO 32000) for layer 1, and clean-room Rust ports of the
> processors (reference: ODL + WCAG) for layer 2.

---

## 2. Entry points

| Layer | File | Notes |
|---|---|---|
| CLI | `java/opendataloader-pdf-cli/src/main/java/org/opendataloader/pdf/cli/CLIMain.java` | Parses args (Apache Commons CLI), expands directories recursively, loops files, calls the API, handles encrypted-PDF/password + error exit codes |
| CLI options schema | `java/opendataloader-pdf-core/src/main/java/org/opendataloader/pdf/api/cli/CLIOptions.java` + root `options.json` | Single source of truth for flags (mirror this in `clap`) |
| Public API | `…/pdf/api/OpenDataLoaderPDF.java:39` | `processFile(inputPdfName, config)` → delegates to `DocumentProcessor.processFile` |
| Config | `…/pdf/api/Config.java`, `…/pdf/api/FilterConfig.java` | All options + the sanitization regex rules (in `FilterConfig`) |
| Orchestrator | `…/pdf/processors/DocumentProcessor.java` | The whole pipeline lives here |
| Output facade | `…/pdf/api/OutputWriter.java` | Stable wrapper around `DocumentProcessor.generateOutputs` |

---

## 3. Top-level flow — `DocumentProcessor`

`processFileWithResult` (`DocumentProcessor.java:141`) is **two phases**:

```
Phase 1  extractContents(input, config)      → List<List<IObject>>  (per-page semantic objects)
Phase 2  generateOutputs(input, contents, …) → writes files
finally  closePdfResources()                 → close PDDocument, clear all static containers
```

State is passed via **veraPDF ThreadLocal static containers** (`StaticContainers`,
`StaticResources`, `StaticLayoutContainers`), not normal arguments — a Java-ism we will replace
with explicit `Document`/`Context` structs in Rust.

### 3.1 `extractContents` (`DocumentProcessor.java:173`)
1. `preprocessing()` — see §4.
2. `calculateDocumentInfo()` (`:754`) — page count + author/title/creation/mod date from the COS
   trailer `/Info` (with XMP fallback).
3. `getValidPageNumbers()` (`:209`) — apply `--pages`, convert 1-based → 0-based, warn on missing.
4. Branch on mode:
   - `useStructTree` → `TaggedDocumentProcessor.processDocument` (read the PDF's own tag tree)
   - `hybridEnabled` → `HybridDocumentProcessor.processDocument` (out of scope for us)
   - else → **`processDocument()`** the local heuristic pipeline (see §5)
5. `sortContents()` (`:853`) — **reading-order sort runs HERE, after detection** (§6), per page.
6. `ContentSanitizer.sanitizeContents()` (`…/utils/ContentSanitizer.java`) — `--sanitize` regex
   redaction.

### 3.2 `generateOutputs` (`DocumentProcessor.java:520`)
- `--to-stdout`: only text or markdown supported.
- else `mkdirs`, then conditionally: images (`ImagesUtils.write`), tagged PDF
  (`AutoTaggingProcessor.createTaggedPDF`), updated PDF (`PDFWriter`), JSON (`JsonWriter`),
  Markdown (`MarkdownGeneratorFactory`), HTML (`HtmlGeneratorFactory`), text (`TextGenerator`).
  See §7.

---

## 4. Preprocessing = the veraPDF parse boundary (`DocumentProcessor.java:592`)

This is exactly where our Rust extraction layer plugs in. Steps:

1. `validatePdfMagicNumber()` (`:651`) — scan first 1024 bytes for `%PDF-` (ISO 32000-1 §7.5.2).
2. `new PDDocument(pdfName)` — **PARSER** parses the file; `InvalidPasswordException` bubbles up
   for the password flow.
3. `new GFSAPDFDocument(pdDocument)`, set flavour `WCAG_2_2_HUMAN`.
4. Flags into static storage: filter-invisible-layers (`--content-safety-off hidden-ocg`),
   data-loader mode, font-program parsing on, add-spaces-between-text-pieces on, ignore-MCIDs
   unless using struct tree.
5. If `--use-struct-tree`: `document.parseStructureTreeRoot()`; fall back to heuristics if no tree.
6. **`document.parseChunks()`** — the key call: **PARSER** produces the per-page chunk lists
   (`TextChunk`/`ImageChunk`/`LineChunk`/`LineArtChunk`).
7. `LinesPreprocessingConsumer.findTableBorders()` (**WCAG**) → `TableBordersCollection`
   (vector-line grid detection, consumed later by `TableBorderProcessor`).

**Rust mapping:** steps 1, 2, 6, 7 are the `PdfBackend` trait + content-stream interpreter +
line-grid detector (PLAN.md Milestone 1).

---

## 5. The local detection pipeline — `processDocument` (`DocumentProcessor.java:245`)

Runs on a `ForkJoinPool(threads)`. **Order matters** and alternates parallel-per-page with
sequential cross-page stages. Exact sequence:

| # | Stage | File / call | Mode |
|---|---|---|---|
| 1 | Content filtering | `ContentFilterProcessor.getFilteredContents` (`:301`) | ∥ per page |
| 2 | Hidden-text filter | `HiddenTextProcessor.findHiddenText` (`:318`) — renders pages for contrast | seq (can't ∥) |
| 3 | Cluster tables (opt) | `ClusterTableProcessor.processTables` (`:331`), `--table-method cluster` | seq, whole-doc |
| 4 | Strikethrough (opt) | `StrikethroughProcessor.processStrikethroughs` (`:344`) | ∥ per page |
| 5 | Border tables | `TableBorderProcessor.processTableBorders` (`:346`); then drop `LineChunk`s | ∥ per page |
| 6 | Special tables | `SpecialTableProcessor.detectSpecialTables` (`:348`) — Korean forms | ∥ per page |
| 7 | Text-line assembly | `TextLineProcessor.processTextLines` (`:350`) | ∥ per page |
| 8 | Header/footer | `HeaderFooterProcessor.processHeadersAndFooters` (`:357`) | seq cross-page |
| 9 | Lists | `ListProcessor.processLists` (`:358`) | seq cross-page |
| 10 | Paragraphs | `ParagraphProcessor.processParagraphs` (`:369`) — always on (text needs it) | ∥ per page |
| 11 | Lists from text nodes | `ListProcessor.processListsFromTextNodes` (`:371`) | ∥ per page |
| 12 | Headings | `HeadingProcessor.processHeadings` (`:372`) | ∥ per page |
| 13 | Assign IDs | `setIDs` (`:381`, def `:710`) — sequential, page order, before captions | seq |
| 14 | Captions | `CaptionProcessor.processCaptions` (`:390`) — links to figures/tables by id | seq |
| 15 | Neighbor lists | `ListProcessor.checkNeighborLists` (`:397`) — cross-page list continuation | seq |
| 16 | Neighbor tables | `TableBorderProcessor.checkNeighborTables` (`:398`) | seq |
| 17 | Heading levels | `HeadingProcessor.detectHeadingsLevels` (`:399`) — assign H1–H6 | seq |
| 18 | Nesting levels | `LevelProcessor.detectLevels` (`:400`) | seq |

Then back in `extractContents`: **reading-order sort (§6)** → sanitize.

All processor files live in `java/opendataloader-pdf-core/src/main/java/org/opendataloader/pdf/processors/`.

### 5.1 Per-processor purpose (clean-room targets)

| Processor file | What it does |
|---|---|
| `ContentFilterProcessor.java` | dedupe chunks, drop decoration glyphs, drop tiny text, drop off-page (outside Media/CropBox), merge adjacent chunks, whitespace handling |
| `HiddenTextProcessor.java` | WCAG contrast ratio of text vs. rendered background → mark/remove hidden text (AI-safety). *Needs rasterization — Rust uses declared-state heuristics instead* |
| `TableBorderProcessor.java`, `AbstractTableProcessor.java` | build grid from `TableBordersCollection`, assign text to cells, row/col span, cross-page joins |
| `ClusterTableProcessor.java` | borderless tables via WCAG `ClusterTableConsumer` (spatial clustering) |
| `TableStructureNormalizer.java` | repair under-segmented grids |
| `SpecialTableProcessor.java` | Korean form layouts (out of scope) |
| `StrikethroughProcessor.java` | horizontal line through text center → `~~` |
| `TextLineProcessor.java` | merge chunks → `TextLine`, insert spaces from gaps, sort |
| `HeaderFooterProcessor.java` | detect repeating top/bottom content across pages |
| `ListProcessor.java` | bullet/number/roman/letter detection, indentation nesting, cross-page continuation (uses WCAG `ListLabelsUtils`) |
| `ParagraphProcessor.java` | group lines into paragraphs by alignment/spacing/indent/font |
| `HeadingProcessor.java` | heading probability from font-size/weight **rarity** vs. body; level clustering (uses WCAG `NodeUtils.headingProbability`) |
| `CaptionProcessor.java` | associate nearby small text with image/table; set `linked content id` |
| `LevelProcessor.java` + `utils/levels/` | nesting/indent level assignment |
| `TaggedDocumentProcessor.java` | alternate path: read the PDF's `/StructTreeRoot` directly |
| `HybridDocumentProcessor.java` | alternate path: AI backend (out of scope) |

Supporting stats: `utils/TextNodeStatistics.java`, `TextNodeStatisticsConfig.java`,
`ModeWeightStatistics.java`, `BulletedParagraphUtils.java` — body-text mode + rarity scoring that
drives heading detection.

> Many numeric thresholds used by headings/lists/captions are **inside WCAG**
> (`org.verapdf.wcag.algorithms.semanticalgorithms.utils.NodeUtils`, `ListLabelsUtils`,
> `CaptionUtils`), not in this repo. Read them there; tune against the corpus.

---

## 6. Reading order — XY-Cut++ (`processors/readingorder/XYCutPlusPlusSorter.java`)

Apache-2.0, **fully readable/portable**. Based on arXiv:2504.10258. Pure-geometric (no semantic
priorities). `sort(objects)` (`:82`) is per-page and stateless. Constants (`:50-68`):
`BETA=2.0` (cross-layout width multiple; 2.0 effectively disables it), `DENSITY_THRESHOLD=0.9`,
`OVERLAP_THRESHOLD=0.1`, `MIN_OVERLAP_COUNT=2`, `MIN_GAP_THRESHOLD=5.0`pt, `NARROW_ELEMENT_WIDTH_RATIO=0.1`.

Four phases (`:110-128`):
1. **Identify cross-layout elements** (`identifyCrossLayoutElements:146`): width ≥ `beta*maxWidth`
   **and** horizontally overlaps ≥2 others (`hasMinimumOverlaps:196`, overlap ratio relative to the
   narrower box, `:233`). These (full-width titles/banners) are set aside.
2. **Density ratio** (`computeDensityRatio:260`) = total content area / region area;
   `> 0.9` ⇒ prefer horizontal cuts first.
3. **Recursive segmentation** (`recursiveSegment:331`): find the largest projection gap in each
   axis — horizontal (`findBestHorizontalCutWithProjection:484`, sort by `topY` desc) and vertical
   (`findBestVerticalCutWithProjection:406`, sort by `leftX`). The vertical finder retries after
   dropping narrow outliers (page numbers/footnote markers that bridge columns, `:421-441`). Pick
   the axis with the larger gap if both ≥ `MIN_GAP_THRESHOLD`; split by **center** coordinate
   (`splitByHorizontalCut:524` / `splitByVerticalCut:556`); recurse; base case = `sortByYThenX`
   (`:644`).
4. **Merge cross-layout back** (`mergeCrossLayoutElements:590`): interleave the set-aside elements
   into the sorted stream by `topY`.

This is a prime Milestone-5 port; the file is self-contained (depends only on `IObject` +
`BoundingBox` geometry).

---

## 7. Output generation

Generators consume `List<List<IObject>>` and walk the semantic tree.

| Format | Generator | Key file |
|---|---|---|
| Markdown | `MarkdownGenerator` (+ `MarkdownGeneratorFactory`, `MarkdownSyntax`) | `…/pdf/markdown/MarkdownGenerator.java` |
| Markdown+HTML tables | `MarkdownHTMLGenerator` (`--markdown-with-html`) | `…/pdf/markdown/MarkdownHTMLGenerator.java` |
| HTML | `HtmlGenerator` (+ factory, `HtmlSyntax`) | `…/pdf/html/` |
| JSON | `JsonWriter` + per-type serializers | `…/pdf/json/JsonWriter.java`, `…/pdf/json/serializers/*` |
| Text | `TextGenerator` | `…/pdf/text/TextGenerator.java` |
| Tagged PDF | `AutoTaggingProcessor` (+ `autotagging/`) | out of scope |
| Updated/annotated PDF | `PDFWriter` | `…/pdf/pdf/` (out of scope) |
| Images | `ImagesUtils`, `Base64ImageUtils` | `…/pdf/utils/` |

Element → Markdown mapping (from `MarkdownGenerator` / `MarkdownSyntax`):
- heading → `#`×min(6,level) + text; paragraph → text; list item → `- ` (nested by recursion);
  table → **GFM pipe table** (header row + `| --- |` separator; spanned cells emit blank);
  image → `![alt](<path>)` (path wrapped in `<>` + escaped) or Base64 data URI;
  formula → `$$\n…\n$$`; strikethrough chunk → `~~…~~`.
- `--markdown-with-html`: tables become raw `<table>` with `colspan`/`rowspan` (GFM can't express
  merged cells); everything else stays Markdown.

JSON: root `{ file name, number of pages, author, title, creation date, modification date, kids[] }`;
every element carries `type`, `id`, `page number`, `bounding box [left,bottom,right,top]` + per-type
fields. The committed contract is root-level `schema.json`, with a worked example at
`samples/json/lorem.json`. Each serializer in `json/serializers/` owns one element type
(`HeadingSerializer`, `TableSerializer`/`TableRowSerializer`/`TableCellSerializer`, `ListSerializer`,
`ImageSerializer`, `CaptionSerializer`, `FormulaSerializer`, etc.); shared bbox/color/metadata
logic in `SerializerUtil.java`.

HTML: full document, `<h1-6>`/`<p>`/`<ul><li>`/`<table border="1">` with spans/`<img>`/`<figure>`/
`<div class="math-display">\[…\]</div>`, plus inline `<span style>` carrying font-size (pt×4/3→px),
`rgb()` color, weight, italic, line-through.

---

## 8. The semantic object model (`IObject`)

Lives in **WCAG** (`org.verapdf.wcag.algorithms.entities.*`), extended by this repo's
`…/pdf/entities/` (`SemanticFormula`, `SemanticFootnote`, `SemanticPicture`, `EnrichedImageChunk`).
We reimplement this as a Rust enum/struct tree. Core types:

- Content chunks: `content.TextChunk`, `content.ImageChunk`, `content.LineChunk`,
  `content.LineArtChunk`, `content.TextLine`, `content.TextBlock`, `geometry.BoundingBox`.
- Semantic nodes: `SemanticTextNode` (+ `SemanticHeading`, paragraph), `SemanticCaption`,
  `SemanticHeaderOrFooter`, `SemanticFigure`.
- Lists: `lists.PDFList`, `lists.ListItem`.
- Tables: `tables.tableBorders.TableBorder` / `TableBorderRow` / `TableBorderCell`,
  `tables.TableBordersCollection`.
- Every `IObject` has a `BoundingBox` and a `recognizedStructureId` (the cross-ref `id`).

Useful WCAG entry points to read for algorithms:
`semanticalgorithms.consumers.{LinesPreprocessingConsumer, ClusterTableConsumer, ContrastRatioConsumer}`,
`semanticalgorithms.utils.{NodeUtils, ListLabelsUtils, CaptionUtils}`,
`semanticalgorithms.containers.StaticContainers`.

---

## 9. Architecture → Rust module mapping

| ODL/WCAG concept | Rust module (PLAN.md) | Milestone |
|---|---|---|
| `PDDocument.parseChunks`, fonts, encodings (PARSER) | `extract/` (`lopdf` + content-stream interpreter) | 1 |
| `IObject` model + `entities/` | `model/` | 1–2 |
| `ContentFilter`/`TextLine` processors | `analyze/filter`, `analyze/lines` | 2 |
| `Heading`/`List`/`Paragraph` + `TextNodeStatistics` | `analyze/blocks` | 3 |
| `TableBorder`/`ClusterTable` + `LinesPreprocessingConsumer` | `analyze/tables` | 4 |
| `XYCutPlusPlusSorter` | `analyze/reading_order` | 5 |
| Caption/Header-Footer/Images | `analyze/{caption,headerfooter}`, `render/images` | 6 |
| `HiddenText` (declared-state), `ContentSanitizer`, `Strikethrough` | `analyze/safety`, `sanitize` | 7 |
| `MarkdownGenerator`/`JsonWriter`/`HtmlGenerator`/`TextGenerator` | `render/{md,json,html,text}` | 2–6 |
| (new) chapter split | `render/split` | 8 |
| `TaggedDocumentProcessor` (`/StructTreeRoot`) | `extract/struct_tree` (optional) | 9 |
| `HybridDocumentProcessor`, OCR, formulas, tagged-PDF write | — (out of scope) | — |

---

## 10. Gotchas worth remembering

- **Coordinates**: PDF user space, origin bottom-left, bbox `[left, bottom, right, top]`; "higher
  Y = nearer top". The renderers and XY-Cut depend on this.
- **Reading order runs last** (after all detection), per page — sort the semantic objects, not raw
  chunks.
- **Structured processing is always on** internally (`structured=true`, `:327`) even for text-only
  output, because paragraphs/headings/lists/tables/captions feed everything.
- **State is global/ThreadLocal** in Java; in Rust pass an explicit context to keep it testable.
- **Heading detection is relative**, not absolute — it scores font size/weight *rarity* against the
  document's body mode, so a 12pt heading in an 8pt document still ranks. Port the statistics, not
  hard size cutoffs.
- **Hidden-text contrast** needs page rasterization we won't have — substitute declared-state
  heuristics (render mode 3, zero-size/transparent fonts, color == background, OCG off).
