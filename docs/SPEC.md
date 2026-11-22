# PDF Viewer & Editor — Project Spec

> Name: **Mirsam** (Arabic مرسام), chosen 2026-11-22. The working name was Safha (صفحة); code
> identifiers still use `safha` until the rename tracked in `PROGRESS.md`.
> Audience for this file: Claude Code. Read it fully before writing any code.

---

## 0. How to work on this project (instructions for Claude Code)

1. **Start with Milestone 0** (section 12). Do not jump ahead to features.
2. On first run, create:
   - `CLAUDE.md` — short: build/test/lint commands, repo layout, coding conventions, and a pointer to this spec.
   - `docs/adr/` — one short Architecture Decision Record per significant decision (engine, IPC, layout engine, OCR engine).
   - `PROGRESS.md` — checklist mirroring the milestones; update it at the end of every session.
3. **Spikes before commitments.** Items marked **[SPIKE]** need a small throwaway prototype and a written ADR with measurements before the real implementation.
4. **Verify before adopting.** Library names and versions in this spec are starting points researched in late 2026. Before adding any dependency, check its current docs, maintenance status, and licence (section 10). If something here is outdated, say so and propose a replacement.
5. **Ask the user** at every item in section 3 (Open decisions) if it has not been answered yet and it blocks your work. Otherwise proceed on the stated assumption.
6. **Definition of done** for any task: code + tests passing + no regression in the performance budgets (section 11) + works in both English (LTR) and Arabic (RTL) UI.
7. Arabic/RTL is **not a later add-on**. Every text-handling component must be Unicode/bidi-correct from its first version.
8. Never commit copyrighted PDFs to the repo. Generate test fixtures or use public-domain documents.
9. **OCR is a headline feature and accuracy beats speed.** Whenever an OCR design choice trades recognition quality against processing time, model size, or implementation effort, choose quality (section 8). The speed budgets in section 11 apply to the interactive UI, not to OCR jobs.

---

## 1. Vision

A desktop PDF viewer and editor with the capability of Adobe Acrobat Pro, but:

- **Faster** — opens instantly, scrolls smoothly, never freezes on large files.
- **Better designed** — calm, modern UI where tools are easy to find and stay where they are.
- **First-class Arabic** — viewing, searching, editing, form filling, and OCR of Arabic and mixed Arabic/English documents work correctly. This is the main differentiator: mainstream editors still produce disconnected or reversed Arabic letters when editing, filling forms, or extracting text.
- **OCR you can trust** — the most accurate OCR we can deliver, especially for Arabic and mixed Arabic/English scans. A scanned document should come out searchable, copyable, and editable with errors rare enough that users stop double-checking. Accuracy is prioritized over processing speed.
- **Private by default** — everything works offline; no account, no telemetry without opt-in, no upsell prompts.

### Product principles

| Principle | What it means in practice |
|---|---|
| Speed is a feature (for the UI) | Hard performance budgets (section 11), enforced in CI |
| OCR: quality over speed | Use the most accurate pipeline the machine can run; slow is acceptable because OCR runs in the background, wrong is not |
| Direct manipulation | Click text to edit it. Minimal "modes". |
| Stable layout | Tools do not move between releases or hide behind menus |
| Everything undoable | Unlimited undo/redo across all operations, including page operations |
| Never lose or corrupt a file | Atomic saves, autosave recovery, original untouched until explicit save |
| Keyboard-first | Command palette + shortcut for every action |
| Bilingual from day one | Full UI in English and Arabic with proper RTL mirroring |

### What users dislike about Acrobat (design against these)

- Sluggish, laggy interaction and slow startup.
- Tools scattered or hidden; a redesign that moved everything and broke muscle memory.
- Cluttered navigation and constant feature promotion.
- Arabic: disjointed letters when typing in Fill & Sign and form fields, reversed text on copy/extract, weak Arabic OCR, damaged text after editing.

---

## 2. Target users and platforms

- **Users:** professionals handling contracts, government and business forms, reports, scanned documents — particularly in Arabic-speaking markets where bilingual documents are the norm.
- **Platforms (v1):** Windows 10/11 and macOS 13+. Linux as best-effort.
- **Later:** web version and mobile (the architecture should not block these — keep the core engine UI-independent).

---

## 3. Assumptions and open decisions

Proceed on the assumption unless the user says otherwise.

| # | Decision | Assumption | Why it matters |
|---|---|---|---|
| D1 | App shell | **Tauri 2** (Rust core + web UI) | Small installer, low memory, native-speed core. Alternative: Electron (heavier, but identical rendering on every OS). |
| D2 | Licence / business model | **Closed-source or permissive licence** | Rules out AGPL engines such as MuPDF unless a commercial licence is bought (section 10). |
| D3 | AI features | **Optional, off by default**, bring-your-own key or local model | Privacy promise; no backend to run. |
| D4 | Cloud / accounts / collaboration | **None in v1** | Keeps the app fully offline. |
| D5 | E-signature *requests* (send to others, track) | **Out of scope for v1** | Needs a server. Local signing is in scope. |
| D6 | UI languages at launch | **English + Arabic** | |
| D7 | OCR languages at launch | **Arabic + English** (others downloadable) | |

---

## 4. UX and design direction

### Layout

```
┌──────────────────────────────────────────────────────────────┐
│ Tabs: doc1.pdf │ doc2.pdf │ +                 ⌘K  Search  ⋯  │
├────┬─────────────────────────────────────────────┬───────────┤
│Tool│                                             │ Inspector │
│rail│              Page canvas                    │ (context- │
│    │   (floating toolbar appears on selection)   │  ual)     │
│    │                                             │           │
├────┴─────────────────────────────────────────────┴───────────┤
│ Page 12 / 240   ─────●─────  100%   Fit   Single ▾   RTL ▾   │
└──────────────────────────────────────────────────────────────┘
```

- **Tool rail** (one column of icons with labels on hover, max ~8 groups): Select, Edit, Comment, Pages, Forms, Sign, Protect, OCR/Convert.
- **Left panel** (toggle): thumbnails, outline/bookmarks, comments list, search results, attachments.
- **Inspector** (right): properties of whatever is selected — font, size, colour, alignment, direction for text; stroke/fill for shapes; field properties for forms. Empty state shows document info.
- **Floating toolbar** on text selection: highlight, underline, strike, comment, copy, edit, redact, (translate / ask AI if enabled).
- **Command palette** (`Ctrl/Cmd+K`): every action, fuzzy search in English and Arabic, shows shortcuts.
- **Page organizer** as a full-window grid view (drag to reorder, multi-select, drag in files to merge).
- In Arabic UI the entire layout mirrors (tool rail on the right, inspector on the left). Use CSS logical properties (`margin-inline-start`, etc.) everywhere; never hard-code left/right.

### Visual language

- Neutral, low-chrome interface; the document is the hero. Light, dark, and "follow system" themes, plus reading themes for the page itself (sepia, night).
- 4px spacing grid, one accent colour, generous hit targets (min 32px desktop).
- Typography: a clean UI sans for Latin + a matching Arabic UI face (e.g. IBM Plex Sans Arabic or Noto Sans Arabic). Verify font licences (OFL preferred).
- Design tokens in one place (colours, spacing, radii, shadows, motion). No hard-coded values in components.
- Motion: subtle and fast (120–200 ms); respect "reduce motion".
- Accessibility: WCAG 2.1 AA contrast, full keyboard navigation, screen-reader labels, focus rings.

### Interaction rules

- Single click selects, double click on text enters editing in place. `Esc` always exits.
- No modal dialogs for routine actions; use inline panels and undoable actions with a toast ("Deleted 3 pages — Undo").
- Long operations (OCR, conversion, compression) run in the background with progress and cancel; the document stays usable.
- Empty and error states written in plain language, in both languages.

---

## 5. Feature scope

**P0** = MVP · **P1** = v1.0 · **P2** = after v1.0

### 5.1 Viewing and navigation

| Feature | Priority |
|---|---|
| Fast tiled rendering, smooth zoom (pinch, Ctrl+scroll), HiDPI | P0 |
| Single / continuous / two-page / book view; **right-to-left page order** for Arabic books | P0 |
| Thumbnails, outline (bookmarks), page labels, go-to-page | P0 |
| Tabs, multiple windows, split view (same doc or two docs) | P0 (tabs), P1 (split) |
| Text selection and copy in correct logical order (bidi-aware) | P0 |
| Search: in document, whole-word, case, regex; Arabic-normalized search (section 6) | P0 |
| Recent files, reopen last session, remember position per file | P0 |
| Dark mode and reading themes | P0 |
| Presentation / full-screen mode | P1 |
| Reflow ("reader") mode for small windows | P2 |
| Read aloud (text-to-speech, Arabic + English) | P2 |
| Search across multiple files / folders | P2 |

### 5.2 Annotation and review

| Feature | Priority |
|---|---|
| Highlight, underline, strikethrough, squiggly | P0 |
| Sticky notes, text boxes, callouts (RTL-aware text entry) | P0 |
| Freehand ink (pen/tablet pressure), eraser | P0 |
| Shapes: rectangle, ellipse, line, arrow, polygon, cloud | P1 |
| Stamps (built-in + custom image; bilingual presets e.g. "Approved / معتمد") | P1 |
| Comments panel: filter, sort, reply, resolve | P1 |
| Measurement tools (distance, area, scale) | P2 |
| Import/export comments (XFDF), summarize comments, flatten annotations | P1 |

Annotations must be standard PDF annotations so they open correctly in other viewers.

### 5.3 Content editing

| Feature | Priority |
|---|---|
| **Edit existing text in place** with paragraph reflow (section 7) | P0 (core), refined in P1 |
| Add new text (any font, size, colour, alignment, direction) | P0 |
| Images: add, move, resize, rotate, crop, replace, delete, extract | P1 |
| Links: add/edit web and internal links | P1 |
| Move/resize/delete any page object; align and distribute | P1 |
| Headers/footers, page numbers (Western or Arabic-Indic digits), watermarks, backgrounds | P1 |
| Bates numbering | P2 |
| Find and replace text across the document | P1 |
| Edit bookmarks/outline; auto-generate from headings | P1 |

### 5.4 Page organization

| Feature | Priority |
|---|---|
| Reorder, rotate, delete, duplicate, insert blank | P0 |
| Merge files, insert pages from another PDF, extract, split (by range, size, bookmarks) | P0 |
| Crop, resize pages, change page boxes | P1 |
| Replace pages, n-up / booklet imposition | P2 |

### 5.5 Forms

| Feature | Priority |
|---|---|
| Fill AcroForms (text, checkbox, radio, dropdown, list, date) with **correct Arabic shaping in generated appearances** | P0 |
| "Fill & Sign" on flat (non-interactive) forms: click anywhere to type, tick, date | P0 |
| Create/edit form fields; properties, tab order, validation, simple calculations | P1 |
| Auto-detect fields on a flat form | P2 |
| Import/export form data (FDF/XFDF/CSV), flatten | P1 |
| XFA forms | Out of scope (show a clear message) |

### 5.6 Signatures

| Feature | Priority |
|---|---|
| Draw / type / image signature and initials, saved locally | P0 |
| Certificate-based digital signatures (PAdES), visible and invisible | P1 |
| Validate existing signatures; show what changed after signing | P1 |
| Trusted timestamps, long-term validation (LTV) | P2 |
| Incremental save so existing signatures are not invalidated | P1 |

### 5.7 Security and privacy

| Feature | Priority |
|---|---|
| Open password-protected PDFs | P0 |
| Encrypt (AES-256), permissions (print/copy/edit) | P1 |
| **True redaction**: mark → review → apply; removes text, image pixels, and vector content under the area | P1 |
| Search-and-redact by text or pattern (phone, email, IBAN, ID numbers), Arabic-aware | P1 |
| Sanitize: metadata, hidden layers, attachments, scripts, comments | P1 |
| No JavaScript execution from PDFs by default | P0 |

### 5.8 OCR (details in section 8)

| Feature | Priority |
|---|---|
| Make scanned PDF searchable (invisible text layer), Arabic + English + mixed, using the high-accuracy pipeline | P0 |
| Image cleanup: deskew, auto-rotate, dewarp, despeckle, contrast, upscaling of low-resolution scans | P0 |
| Layout analysis: columns, headers/footers, tables, reading order (RTL and mixed) | P0 |
| Per-page confidence score and "pages needing review" list | P0 |
| Review mode: step through low-confidence words beside the image crop and correct them | P1 |
| Re-run OCR on a page or selected region with different settings/engine | P1 |
| Detect and replace a poor existing text layer (e.g. from another tool) | P1 |
| Convert scan to editable text (real text objects with matched fonts) | P1 |
| Table recognition → export to spreadsheet | P2 |
| Import from scanner / camera images → PDF | P2 |
| Handwriting (best effort, clearly labelled) | P2 |

### 5.9 Convert, create, optimize

| Feature | Priority |
|---|---|
| Images ↔ PDF; PDF → PNG/JPEG per page | P0 |
| Compress / optimize (image downsampling, font subsetting, object cleanup) with size preview | P1 |
| PDF → Word / Excel / PowerPoint / Markdown / plain text (RTL preserved) | P1 (text, Markdown), P2 (Office formats) |
| Office → PDF | P2 **[SPIKE]** |
| PDF/A export and validation | P2 |
| Print with page scaling, booklet, selected pages | P0 |

### 5.10 Compare

| Feature | Priority |
|---|---|
| Side-by-side text diff of two PDFs with synced scrolling | P2 |
| Visual (pixel) overlay diff | P2 |

### 5.11 AI assist (optional, off by default)

All AI features sit behind one provider interface (cloud API key or local model). Show a clear notice before any document content leaves the device.

| Feature | Priority |
|---|---|
| Summarize document / selection; Q&A with answers linked to page regions | P2 |
| Translate Arabic ↔ English, side-by-side or in place preserving layout | P2 |
| Natural-language commands mapped onto the command palette API ("delete pages 3–5", "redact all phone numbers") | P2 |
| Suggest redactions (personal data detection) | P2 |
| Auto-generate bookmarks and alt text | P2 |

### 5.12 Productivity

| Feature | Priority |
|---|---|
| Autosave + crash recovery | P0 |
| Batch actions on many files (OCR, compress, watermark, convert) | P2 |
| Saved "workflows" (chain of actions) | P2 |
| CLI for the core engine | P2 |
| Auto-update, file associations, "Open with", drag-and-drop | P1 |

### 5.13 Explicitly out of scope (for now)

Real-time collaboration, cloud storage, sending documents for e-signature, XFA, 3D/multimedia content, prepress/preflight tools, accessibility tag *editor* (reading existing tags is fine; a PDF/UA checker may come later).

---

## 6. Arabic and RTL requirements

These are acceptance requirements, not nice-to-haves. Persian and Urdu share the script and should work through the same pipeline; Hebrew gets bidi support for free.

### 6.1 Reading existing PDFs

- Render Arabic identically to reference viewers (PDFium handles this).
- **Text extraction in logical order.** Many PDFs store glyphs in visual (left-to-right paint) order. Rebuild reading order from glyph positions and apply the Unicode Bidirectional Algorithm (UAX #9); do not trust content-stream order. Never "fix" already-logical text by reversing it again.
- Map Arabic presentation forms (U+FB50–FDFF, U+FE70–FEFF) back to base letters (NFKC) for copy and search.
- Handle missing or wrong `ToUnicode` maps gracefully: fall back to glyph names, and offer OCR for that page as a last resort.
- Selection moves visually but copies logically; mixed Arabic/English/number lines select correctly.

### 6.2 Search

Normalized matching (toggleable): ignore tashkeel (diacritics) and tatweel; unify alef variants (ا أ إ آ); ى/ي; optional ة/ه; treat Arabic-Indic (٠–٩) and Western digits as equal; presentation forms equal to base letters.

### 6.3 Writing and editing

- Shape all text with HarfBuzz-class shaping (contextual forms, lam-alef and other ligatures, mark positioning for tashkeel, kerning).
- Paragraph direction auto-detected from the first strong character, overridable in the inspector (RTL / LTR / auto). Alignment defaults follow direction.
- Line breaking per UAX #14; justification using kashida where the font supports it (P2), otherwise word spacing.
- Write text as glyph IDs with an Identity-H CID font, an embedded **subset** of the font, a correct `ToUnicode` CMap, and `/ActualText` on spans where glyph order or ligatures would otherwise break extraction — so the output copies and searches correctly in *other* viewers too.
- Digits: user choice of Western or Arabic-Indic for page numbers, dates, Bates numbers.
- Bundle a small set of open-licence Arabic fonts covering Naskh, Kufi-style sans, and a serif (for example Noto Naskh Arabic, Noto Sans Arabic, Amiri, IBM Plex Sans Arabic — confirm licences). Use system fonts too.

### 6.4 Forms and annotations

- Generate field appearance streams ourselves with full shaping; never rely on `NeedAppearances` and another viewer's renderer.
- Text boxes, notes, stamps, and typed signatures all go through the same shaping pipeline.

### 6.5 UI

- Full Arabic localization; layout mirrors; icons with direction (arrows, undo/redo) mirror appropriately while media-style icons do not.
- Document direction is independent of UI direction (an English UI can read an RTL book and vice versa).
- Set/read `ViewerPreferences /Direction` for right-to-left binding in two-page view.
- Optional Hijri calendar for date stamps and date fields (P2).

### 6.6 Test fixtures (create early, in Milestone 0)

Generate Arabic and mixed-direction PDFs from several producers (Word, LibreOffice, Chrome print, XeLaTeX, InDesign if available, a Tesseract-OCR'd scan) because each stores text differently. Include: fully vocalized text, lam-alef ligatures, numbers inside Arabic sentences, parentheses and punctuation at direction boundaries, tables, and a bilingual two-column contract.

---

## 7. Text editing engine (the hardest part)

PDF has no paragraphs — only positioned glyph runs. "Editing text" means reconstructing structure, re-laying it out, and rewriting the content stream.

### Pipeline

1. **Analyse page:** read text objects with font, size, matrix, colour → cluster into lines → blocks (paragraphs) using geometry; detect direction, alignment, line spacing, indentation, lists.
2. **Edit model:** each block becomes an editable rich-text model (styled runs in logical order).
3. **Layout:** own layout engine — itemize (script, direction, font) → shape → line break → position. Same engine for LTR and RTL.
4. **Fonts:**
   - Use the embedded font if it contains the glyphs needed (subset fonts usually do not contain letters that were not in the original).
   - Otherwise find the closest installed/bundled font by family name, then by classification and metrics; re-set the *whole paragraph* in the substitute for visual consistency, and show an unobtrusive indicator ("Font substituted: …") with a picker.
5. **Write back:** remove the original text objects for that block; emit new ones (section 6.3 rules); keep z-order and clipping; update font resources.
6. **Reflow limits (v1):** a block reflows inside its own box and may grow/shrink vertically. No automatic reflow across columns or pages — PDF is not a flow format; be honest about this in the UI.

### In-place editing UI

- The editing caret, selection, and IME composition are drawn by our own editor surface over the canvas using the same layout engine as the writer, so what the user sees while typing is exactly what gets saved. **[SPIKE]**: custom editor surface vs. a transparent native `contenteditable` overlay synchronized with the layout engine — evaluate caret fidelity, Arabic IME behaviour, and effort.

### Acceptance tests

- Fix a typo in an English paragraph: surrounding lines keep their positions; output opens identically in Chrome and Acrobat Reader.
- Insert a word in the middle of an Arabic paragraph: letters join correctly, line re-wraps right-to-left, copy-paste from the saved file in another viewer yields correct logical text.
- Edit a mixed sentence (Arabic with an English product name and a number): order remains correct.
- Round-trip: open → edit → save → reopen → edit again, with no drift or duplicate fonts.

---

## 8. OCR

### Why this matters

OCR is one of the product's headline features, not a checkbox. A large share of real-world Arabic documents (government letters, contracts, older records, anything that passed through a fax or scanner) exist only as images, and existing tools handle them badly. If our OCR is clearly better, that alone is a reason to switch. If it is mediocre, search, copy, redaction, translation, and "edit scanned text" all inherit its errors.

**Priority order for every OCR decision: accuracy → correct reading order and layout → robustness on bad scans → speed → model size.**

Consequences of "quality over speed":

- The **default** mode is the most accurate pipeline the user's hardware can run. A faster, lighter mode exists only as an explicit user choice ("Quick"), never the default.
- Taking many seconds per page is acceptable. Multi-pass processing, larger models, and higher render resolution are all acceptable costs.
- Larger model downloads (hundreds of MB to several GB) are acceptable for the best-quality tier; make the download resumable and explain the size.
- Use GPU / hardware acceleration when present (DirectML, Core ML / Metal, CUDA) to make the heavy pipeline practical, but it must still run correctly on CPU-only machines, just slower.
- Because it is slow, OCR must never block the app: background jobs, per-page progress, pause/resume, survives app restart, document remains usable meanwhile.
- **Progressive results:** run a quick pass first so the document becomes searchable within moments, then replace each page's text layer with the high-accuracy result as it completes. Mark pages as "draft" until the quality pass finishes.

### Requirements

- Fully **local** by default. Models are downloaded on demand per language, not bundled in the installer.
- Output: invisible text layer precisely aligned to the image (word-level boxes at minimum), stored in logical order with the rules from section 6.3, so search/copy work everywhere. Known pitfall to avoid: OCR text layers that other viewers read as reversed Arabic.
- Never destroy the original image. OCR adds a layer; the user can remove or redo it.
- Preserve what is on the page: tashkeel (diacritics) when present, Arabic-Indic vs. Western digits as printed, punctuation (، ؛ ؟), Latin words inside Arabic lines, numbers and dates exactly.
- **No silent invention.** If a language-model correction step is used, it may only choose between candidates supported by the image evidence; it must never rewrite names, numbers, IDs, or amounts from "what seems likely". Uncertain words are flagged for review, not guessed confidently.
- Confidence per word and per page, surfaced in the UI (section 5.8), and stored so the review mode can use it.

### Pipeline (quality mode)

1. **Render** the page at high resolution (≥ 300 DPI equivalent; higher for small print).
2. **Assess** image quality (resolution, skew, noise, blur) and choose preprocessing accordingly.
3. **Preprocess:** orientation detection, deskew, dewarp (curved pages / phone photos), denoise, background removal, contrast normalization, upscaling for low-resolution scans. Keep both the original and cleaned image; recognition may try both.
4. **Layout analysis:** regions (text, table, figure, header/footer, stamp/signature), columns, and reading order — right-to-left column order for Arabic pages, correct handling of bilingual side-by-side layouts.
5. **Script / language identification** per line or region (Arabic, Latin, mixed).
6. **Recognition** with the best available engine; for low-confidence lines, **re-recognize** with alternative preprocessing and/or a second engine and reconcile (ensemble / voting).
7. **Verification pass:** dictionary and language-model checks used to *flag or choose between candidates* (see "No silent invention").
8. **Assemble** logical-order text with word boxes; write the text layer; record confidence.

### Engine strategy **[SPIKE — benchmark before choosing; start the evaluation set in M0]**

Build an `OcrEngine` trait with pluggable back-ends, then measure on real Arabic samples. The winner on accuracy becomes the default, regardless of speed, provided it can run on a typical laptop (even slowly).

| Role | Candidates | Notes |
|---|---|---|
| **Best quality (default where hardware allows)** | Arabic-tuned vision-language OCR models such as QARI-OCR or Baseer; other current document-OCR models found at build time | Heavy (multi-GB, built on the Qwen-VL family). Check licences, quantized variants, and CPU feasibility. These models output text without precise boxes, so pair them with a detector/aligner to get word positions. |
| **Strong classical pipeline (default on weak hardware; second opinion in the ensemble)** | PaddleOCR PP-OCRv5 detection + the **Arabic** recognition model via ONNX Runtime (server-size models preferred over mobile ones) | As of late 2026 the newer PP-OCRv6 unified model does not include Arabic, so Arabic still uses the PP-OCRv5 Arabic model — re-check. Gives reliable boxes for alignment. |
| **Quick pass / fallback** | Tesseract (`ara`, `eng`, best-quality traineddata) or the mobile PaddleOCR models | Used for the instant draft layer and as a third voter. |
| **Cloud (optional, opt-in)** | Any provider via the AI provider interface | Only with explicit consent per document; offered when local hardware cannot run the best tier. |

Also evaluate fine-tuning the chosen recognizer on our own data if the benchmark shows a consistent weakness (specific fonts, diacritics, low resolution).

### Evaluation set and quality gates

Build `ocr-eval/`: a versioned set of page images with human-verified ground truth, plus a script that reports character error rate (CER) and word error rate (WER) per category. Include synthetic pages (known text rendered in many Arabic fonts, then degraded: blur, noise, skew, low DPI, fax artefacts, photocopy shadows) and real public-domain scans.

Categories: clean printed Naskh; other typefaces (Kufi-style sans, typewriter); low-resolution and fax-quality scans; phone photos; mixed Arabic/English lines; tables and forms; diacritized text; numbers, dates, and IDs; stamps and letterheads; handwriting (tracked separately — expected to be poor, set expectations in the UI).

Initial targets (calibrate after the first benchmark, then only ever tighten):

| Category | Target |
|---|---|
| Clean printed Arabic, 300 DPI | CER ≤ 1%, WER ≤ 3% |
| Clean printed English, 300 DPI | CER ≤ 0.5% |
| Mixed Arabic/English | CER ≤ 2%, and zero reading-order errors on the fixture set |
| Degraded / low-resolution Arabic scans | CER ≤ 5% |
| Numbers, dates, IDs | ≥ 99% exact-match per field |

Ship gates:

- Our default pipeline must **beat** plain Tesseract and the OCR built into the leading commercial editors on this set, in every Arabic category. Record the comparison in the ADR.
- CI runs the benchmark on every change to the OCR pipeline and fails on any regression in CER/WER.
- No speed target for quality mode. Track pages/minute for information only.

The KITAB-Bench benchmark is a useful public reference for Arabic OCR and document understanding; published results show Arabic remains well behind English even for strong models, so measure rather than assume.

---

## 9. Architecture and tech stack

### Shape

```
┌────────────────────────── UI (web tech) ──────────────────────────┐
│ React + TypeScript · design tokens · i18n (en, ar) · canvas view  │
│ state store · command registry (every action = a command)         │
└───────────────▲───────────────────────────────▲───────────────────┘
                │ typed IPC (commands/events)   │ binary tile channel
┌───────────────┴───────────────────────────────┴───────────────────┐
│ Core engine (Rust library crate, UI-independent, also drives CLI) │
│  document model · undo/redo journal · render scheduler + cache    │
│  text analysis · layout/shaping · content-stream writer           │
│  annotations · forms · signatures · redaction · OCR · conversion  │
└───────────────▲───────────────────────────────────────────────────┘
                │ FFI
        PDFium (render, parse, text, basic edit) + low-level PDF writer
```

### Starting choices (verify each; record an ADR)

| Concern | Starting choice | Notes |
|---|---|---|
| Shell | Tauri 2 | See D1. System webview differs per OS — test all three. |
| UI | React + TypeScript + Vite; headless component primitives; CSS variables for tokens | Must support RTL via logical properties. |
| Render/parse engine | **PDFium** via the `pdfium-render` Rust crate | Permissively licensed; same engine as Chrome. The crate supports rendering, text extraction, page-object creation/editing, and can also compile to WASM (helps a future web version). |
| Low-level PDF writing | `lopdf` or direct content-stream writing | **[SPIKE]**: confirm whether PDFium's edit API can place pre-shaped glyph IDs with custom fonts; if not, write text objects ourselves. |
| Shaping / bidi / fonts | HarfBuzz-compatible Rust shaper (`rustybuzz` or `harfrust`), `unicode-bidi`, a font database crate, a font subsetter | Pick the actively maintained options. |
| Repair / linearize / validate | `qpdf` (as a library or bundled tool) | Also used in tests to validate every saved file. |
| OCR runtime | ONNX Runtime (`ort` crate) + optional Tesseract | Section 8. |
| Crypto / signatures | Established Rust crypto crates; PAdES implementation | **[SPIKE]** — do not hand-roll cryptography. |
| Storage | SQLite for recent files, per-file view state, settings | |

### Rendering design

- Core renders **tiles** at the needed zoom on a worker pool; cache by (page, zoom, tile, document revision); cancel stale requests on fast scroll.
- Tiles reach the UI over a binary channel (custom protocol or shared buffer) — never base64 in JSON. **[SPIKE]**: measure Rust-side PDFium + binary transfer vs. PDFium-WASM inside the webview; choose by scroll smoothness and memory.
- Show a low-resolution preview immediately, then sharpen.
- Text selection layer built from PDFium character boxes, reordered per section 6.1.

### Document model and saving

- All mutations go through commands recorded in an **undo journal** (command + inverse), so undo works for text, annotations, and page operations alike.
- Original file is opened read-only. Save = write to temp file → validate (reopen + `qpdf --check`) → atomic rename. "Save As" always available.
- Incremental-update save when the document has digital signatures; full rewrite (with cleanup) otherwise.
- Autosave to a recovery file in the app data directory every N seconds while dirty.

### Security

- Treat every PDF as untrusted input: parsing happens in the core with strict limits (object counts, stream sizes, recursion); consider a separate sandboxed process for parsing/rendering (P1).
- PDF JavaScript and external actions disabled by default; links open only after showing the target.
- Tauri allow-list locked down: UI gets no direct filesystem or shell access.

---

## 10. Licensing rules

- **Allowed without asking:** MIT, BSD, Apache-2.0, ISC, Zlib, MPL-2.0 (dynamic use), OFL (fonts).
- **Ask the user first:** LGPL (linking constraints), any "community" or non-commercial licence, model weights with custom terms.
- **Do not use** (under assumption D2): AGPL/GPL components. This specifically includes **MuPDF / PyMuPDF** (AGPL or paid commercial licence from Artifex), Ghostscript, Poppler, and iText. If the user decides to open-source under AGPL or buy a commercial licence, MuPDF becomes a strong option for editing and should be re-evaluated.
- Keep `THIRD_PARTY_LICENSES.md` generated automatically in CI; fail the build on a disallowed licence.

---

## 11. Quality bars

### Performance budgets (targets on a mid-range laptop; enforce in CI benchmarks)

| Metric | Target |
|---|---|
| Cold start to usable window | ≤ 1.5 s |
| First page visible after opening a 100-page text PDF | ≤ 300 ms |
| Scrolling / zooming | 60 fps, no blank pages longer than 100 ms |
| Typing latency while editing text | ≤ 32 ms per keystroke |
| 1,000-page document | Opens without loading all pages; memory stays bounded (the app's own process < 500 MB; the system webview's processes are measured but not budgeted) |
| In-document search, 500 pages | ≤ 1 s after first index |
| Installer size (without OCR models) | ≤ 60 MB |

These budgets cover interactive use. **OCR jobs are exempt from speed targets** — they are judged on the accuracy gates in section 8. The only OCR performance rule is that a running job must not degrade UI responsiveness below the budgets above.

### Testing

- **Unit tests** for text analysis, bidi reordering, search normalization, layout, content-stream writing.
- **Golden-image tests:** render before/after for every editing operation; compare our saved output rendered by PDFium against expected images with a small tolerance.
- **Round-trip tests:** every write is reopened, validated with `qpdf --check`, and text-extracted to assert logical-order strings (essential for Arabic).
- **Redaction tests:** after applying, the redacted string must be absent from extracted text, raw streams, and metadata.
- **Fuzzing** of the open/parse path with malformed PDFs.
- **OCR benchmark:** the `ocr-eval/` set and CER/WER gates from section 8, run in CI on every OCR change.
- **Corpus:** the fixtures from section 6.6 plus encrypted, linearized, damaged, huge, form-heavy, and scanned files.
- **UI tests:** end-to-end smoke tests in both English and Arabic UI; visual regression snapshots in LTR and RTL, light and dark.

---

## 12. Milestones

Each milestone ends with: tests green, budgets met, `PROGRESS.md` updated, a short demo script in `docs/demos/`.

**M0 — Foundations**
Repo scaffold, CI on Windows/macOS/Linux, Tauri shell with empty window, PDFium bound and rendering one page to a canvas, test corpus and fixture generators, design tokens and base components in LTR + RTL, licence check in CI. Spikes: tile transport; PDFium edit API vs. own content-stream writer. Also start the `ocr-eval/` ground-truth set and its CER/WER scoring script now, so data accumulates before M4.

**M1 — Viewer (P0 viewing)**
Tiled rendering, zoom, view modes incl. RTL page order, thumbnails, outline, tabs, bidi-correct selection/copy, normalized search, recents and session restore, themes, command palette, printing. *Exit test:* a 1,000-page bilingual PDF scrolls at 60 fps; Arabic copy-paste and search are correct on all section 6.6 fixtures.

**M2 — Annotate and organize**
Markup, notes, text boxes, ink; comments panel; page organizer (reorder/rotate/delete/merge/split/extract); undo journal; safe save pipeline; autosave/recovery.

**M3 — Text engine**
Layout/shaping engine, font matching and subsetting, content-stream writer, add-new-text tool, then in-place editing of existing text with reflow. English and Arabic developed together. *Exit test:* all acceptance tests in section 7.

**M4 — OCR (quality-first; allow this milestone extra time)**
Engine benchmark and ADR, full quality pipeline (assessment, preprocessing, layout analysis, recognition, re-recognition/ensemble, verification), background job system with progressive draft-then-quality results, word-level confidence, pages-needing-review list, searchable-PDF output, model download manager, hardware acceleration. *Exit test:* the accuracy targets and ship gates in section 8 are met on `ocr-eval/`; a scanned bilingual document becomes searchable; copied Arabic text is in correct order in our app, Chrome, and Acrobat Reader. Do not close this milestone on a pipeline that merely works — it must win the benchmark. Review mode and editable-text conversion follow in M6.

**M5 — Forms, sign, protect**
Form filling with shaped appearances, Fill & Sign, drawn/typed/image signatures, open encrypted files, encryption, true redaction with pattern search, sanitize. Then form creation and certificate signatures.

**M6 — Edit everything else + convert**
Images, links, object manipulation, headers/footers/watermarks/page numbers, find & replace, compression, image export, text/Markdown export. OCR review mode, region re-OCR, replacing poor existing text layers, and scan-to-editable-text.

**M7 — v1.0 polish**
Accessibility pass, full Arabic localization review by a native speaker, onboarding, auto-update, installers and code signing, crash reporting (opt-in), performance hardening.

**Post-1.0 (P2)**
Compare, Office conversion, OCR table extraction, handwriting, AI assist, batch workflows and CLI, PDF/A, kashida justification, Hijri dates, web and mobile versions.

---

## 13. Main risks

| Risk | Mitigation |
|---|---|
| Text editing fidelity (font subsets, odd encodings, fragmented text) | Paragraph-level re-set with font substitution; large fixture corpus; ship editing as "beta" until golden tests are broad |
| Arabic OCR accuracy below expectations | Quality-first pipeline with ensemble re-recognition, own evaluation set with CI gates, fine-tuning if needed, review mode for flagged words, optional cloud tier with consent |
| Best OCR models too heavy for ordinary laptops | Quantized variants, hardware acceleration, progressive draft-then-quality results, strong classical pipeline as the fallback default |
| Language-model "corrections" introducing confident errors | Corrections limited to image-supported candidates; names and numbers never rewritten; uncertain words flagged |
| Webview differences across OSes (Tauri) | Keep page rendering in the core (bitmaps), test UI on all three; Electron remains a fallback (D1) |
| PDFium's editing API too limited | Own content-stream writer (spiked in M0) |
| Corrupting user files | Atomic validated saves, never overwrite without validation, recovery files |
| Scope creep (Acrobat has 70+ tools) | Priorities in section 5; P2 does not start before v1.0 ships |

---

## 14. References

- pdfium-render (Rust bindings for PDFium): https://docs.rs/pdfium-render
- MuPDF licensing (AGPL / commercial): https://mupdf.readthedocs.io/en/1.27.0/license.html
- KITAB-Bench — Arabic OCR and document understanding benchmark: https://arxiv.org/abs/2502.14949
- QARI-OCR — Arabic OCR via multimodal LLM adaptation: https://arxiv.org/abs/2506.02295
- Baseer — Arabic document-to-Markdown OCR model: https://arxiv.org/abs/2509.18174
- PaddleOCR (PP-OCRv5 language models, ONNX exports): https://github.com/PaddlePaddle/PaddleOCR
- Unicode Bidirectional Algorithm (UAX #9): https://unicode.org/reports/tr9/
- Unicode Line Breaking (UAX #14): https://unicode.org/reports/tr14/
- Acrobat desktop release notes (feature reference): https://helpx.adobe.com/acrobat/desktop/whats-new/release-notes.html
- Stirling-PDF (open-source feature reference): https://github.com/Stirling-Tools/Stirling-PDF
- Tauri 2: https://v2.tauri.app
