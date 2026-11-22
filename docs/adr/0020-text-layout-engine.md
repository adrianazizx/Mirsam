# 0020 — Text layout engine: itemize → shape → break → position (`safha_core::typeset`)

Status: Accepted (M3 groundwork). Date: 2026-11-22.

## Context

Every piece of text Safha writes — new text, in-place edits, form appearances, stamps, typed
signatures — has to be laid out by our own engine (spec §6.3, §6.4, §7). ADR 0005 put that
pipeline in `safha-core` and, as amended for M3, has our own writer emit it. The same engine
will drive the editor surface, because spec §7 requires that what the user sees while typing is
exactly what is saved.

Requirements, from the spec:

- English and Arabic together from the first version: mixed-direction lines, Arabic joining,
  marks and vocalization, digits and brackets inside Arabic text;
- paragraph direction detected from the first strong character, overridable; alignment
  defaults follow direction (§6.3);
- line breaking per UAX #14; justification by word spacing (kashida is P2);
- output that a PDF writer can turn into glyph IDs with a correct ToUnicode mapping.

## Libraries (verified 2026-11-22)

| Need | Crate | Version, release | Licence | Why this one |
|---|---|---|---|---|
| Shaping | `harfrust` | 0.16.1, 2026-11-19 | MIT | The HarfBuzz organisation's Rust port, released monthly. `rustybuzz` has had no release since 0.20.1 (2024-11). |
| Bidi (UAX #9) | `unicode-bidi` | 0.3.18, 2024-12 | MIT / Apache-2.0 | Already used by the extractor (ADR 0008); implements bracket pairs (N0) and per-line reordering (L1–L2). Few releases, but stable and very widely used. |
| Line breaks (UAX #14), grapheme clusters | `icu_segmenter` | 2.4.0, 2026-10 | Unicode-3.0 | ICU4X, maintained. `unicode-linebreak` (0.1.5) was last released in 2023-07. |
| Script and Script_Extensions, bracket pairs | `icu_properties` | 2.4.0, 2026-10 | Unicode-3.0 | Already in the tree through `harfrust` and Tauri. |
| Font parsing and metrics | `read-fonts` (through `harfrust::Font`) | 0.47.0 | MIT / Apache-2.0 | `harfrust` re-exports it; no second font parser. |

`icu_segmenter` is built without its LSTM and dictionary models (`default-features = false,
features = ["compiled_data"]`): we use the rule-based segmenter, which covers Arabic and Latin.
New transitive crates: `icu_segmenter_data`, `icu_locale_fallback(_data)` (Unicode-3.0),
`core_maths`, `libm` (MIT / Apache-2.0). Unicode-3.0 is on the allowlist (PROGRESS, owner
item 4).

## Decision

A module `safha_core::typeset`, no UI and no PDF, with one entry point:

```rust
layout(text: &str, spans: &[Span], fonts: &[FontFace], style: &ParagraphStyle)
    -> Result<TextLayout, TypesetError>
```

- **Input.** The text; spans in logical order that cover it (font index, size, BCP 47
  language); the font list the spans index into, which fallback also searches; and the
  paragraph style (direction auto / LTR / RTL; alignment start, end, left, right, center,
  justify; width or unbounded; line height normal, a multiple, or fixed points).
- **Itemize.**
  - Bidi paragraphs and levels from `unicode-bidi`. Each paragraph after a separator detects
    its own direction.
  - A script per character from ICU. Common and Inherited characters take their context: a
    mark takes its letter's script; Script_Extensions keep the Arabic comma and Arabic-Indic
    digits Arabic; a closing bracket takes its opening bracket's script; leading neutrals take
    the first real script.
  - A font per grapheme cluster, so a mark never leaves its letter's font. The span's font is
    used if it has every character, otherwise the first font in the list that does. A space or
    punctuation cluster keeps the previous cluster's font when that font has it.
  - An item is a maximal run with one level, script, font and span. Line-break characters are
    never shaped.
- **Shape.** `harfrust`, one item per call, in the item's direction, with the whole paragraph
  as pre- and post-context, so Arabic letters join across item edges. Cluster level
  `MonotoneCharacters`: the finest glyph↔text mapping the font allows, which is what a
  ToUnicode map needs.
- **Break.**
  - Break opportunities from ICU's UAX #14 segmenter. BK, CR, LF and NL are forced breaks.
  - Greedy fill against the widths measured in the shaping pass. Trailing spaces hang and do
    not count.
  - A word wider than the line is broken between grapheme clusters, at least one per line.
- **Position.**
  - Per line, `unicode-bidi`'s visual runs, with rule L1 applied to that line.
  - Each run's part of each item is reshaped exactly, in the run's direction.
  - Alignment: `Start` and `End` follow the paragraph's direction. Hanging spaces sit on the
    paragraph's end side.
  - Justification widens spaces on every line but a paragraph's last and lines ended by a
    forced break.
  - Line height splits the leading half above and half below the text.
- **Output.**
  - `TextLayout` holds lines in logical order.
  - Each `Line` has its text range, its paragraph's direction, its vertical metrics, and its
    glyph runs in visual order.
  - Each `GlyphRun` has a font, size, direction, script, text range, and glyphs left to right.
    Each glyph has an id, x, a y offset and an advance. `clusters` map glyph ranges to text
    ranges.
  - Points throughout: x from the box's left edge, `baseline` down from the box's top,
    `y_offset` up from the baseline, as in PDF text space.
- **The output is backend-neutral.**
  - The own writer (ADR 0005 amendment) can place each glyph with `Tm`/`TJ`, or use `Ts` for
    a mark's offset.
  - PDFium's CID route (`SetCharcodes` + `SetPositions`, one object per distinct y offset)
    can consume the same runs.

## Verified

`cargo test -p safha-core --test typeset --test typeset_pdf`, plus unit tests in
`typeset/itemize.rs`. Fonts: Amiri, IBM Plex Sans (variable), IBM Plex Sans Arabic, Noto Naskh
Arabic (variable), all OFL, from `assets/fonts/`.

- **Every layout** is checked for:
  - lines that cover the text in order;
  - clusters that cover each run's text exactly once;
  - every drawn character in exactly one run;
  - runs that do not overlap;
  - lines that fit the box;
  - baselines that go down.
- **LTR:**
  - Amiri's `ffi`, `ff` and `fi` ligatures come out as single-glyph clusters.
  - Glyphs are placed left to right.
  - Wrapping breaks at spaces only, never inside `$4,500,000` or `12.5%`.
  - A variable font's `wght` 700 instance sets wider than its default.
- **RTL:**
  - Every letter of `سلم` takes a joined form, not its isolated one, and the last letter is
    drawn leftmost.
  - Lam-alef is one glyph in IBM Plex Sans Arabic.
  - Every mark of a vocalized phrase has a zero-advance glyph. Noto Naskh also moves marks
    vertically.
  - Wrapped lines end at the right edge, with each line's logically first character
    rightmost.
- **Mixed:**
  - `اشتريت 3 أجهزة iPhone 15 Pro بسعر 4,500 ريال.` reads in the right order, with
    `4,500`, `iPhone 15 Pro` and `3` as left-to-right runs.
  - Western and Arabic-Indic numbers read left to right between Arabic words.
  - Brackets are mirrored in RTL.
  - An Arabic phrase inside an English sentence stays inside its brackets.
  - Wrapped mixed paragraphs keep each run's order.
  - Direction is detected per paragraph, and can be forced.
  - Arabic falls back from IBM Plex Sans (no Arabic) to Noto Naskh, with no `.notdef`.
- **Lines:**
  - U+2028 and `\n` break lines.
  - Empty text, and text ending in a newline, end with an empty line for the caret.
  - An overlong word breaks between graphemes and never strands a mark.
  - Justified lines fill the width except the last, in English and in Arabic.
  - Line heights: normal, a multiple, and fixed points.
- **End to end (`tests/typeset_pdf.rs`):**
  - The layout is written to a PDF by a small test writer: Type 0 / CIDFontType2,
    Identity-H, ToUnicode from the clusters.
  - It passes `qpdf --check`, PDFium draws it, and `safha-core`'s extractor reads back
    exactly the input text.
  - Covered: a wrapped English paragraph, a wrapped Arabic paragraph, the mixed sentence, and
    a justified Arabic paragraph with an English phrase and Noto Naskh fallback.

**Cost.** Measured with `SAFHA_TYPESET_BENCH=<n> cargo run --release -p safha-core --example
typeset -- "<text>" 300`, on an Apple-silicon MacBook with other builds running, so the numbers
are noisy:

| Paragraph (300 pt wide, Amiri 18 pt) | Per layout |
|---|---|
| 21 characters, Arabic, 1 line | 0.02 ms |
| 306 characters: Arabic, vocalized, mixed with English and numbers; 6 lines | 0.24–0.34 ms |
| The same, IBM Plex Sans with Noto Naskh fallback; 7 lines | 0.17 ms |
| 1,227 characters, 24 lines | 0.96 ms |

The spec's typing budget is 32 ms per keystroke (§11). Laying out a whole paragraph again on
every keystroke takes about 3% of it at 1,200 characters.

## Not done, and known limits

- **Each piece is shaped twice:** once whole, to measure, and once per line, exactly. The
  second pass could reuse the first where harfrust marks the glyphs safe to break.
- **Greedy line filling.** No hyphenation, tabs, letter- or word-spacing input, kashida (P2),
  or vertical text.
- **Fonts come from the caller.** Matching and discovery are the next step (spec §7, step 4).
- **Complex-script line breaking** (Thai, Lao, Khmer, Myanmar) needs ICU4X's dictionary or
  LSTM data. That data is not built in: a size decision for when those scripts matter.
- **Bidi data is `unicode-bidi`'s own tables** (an older Unicode version than ICU 2.4's). The
  `icu_properties` adapter can feed newer data if a difference ever matters.
- **For the writer:** clusters of several glyphs need a ToUnicode recipe.
  - Noto Naskh draws a letter's dots as separate glyphs, and the pair shadda + fatha as two
    glyphs in one cluster.
  - The test writer's `/ActualText` around such a cluster round-trips single letters, but the
    two-mark cluster extracts twice through PDFium.
  - ADR 0005's extraction findings apply.
- **Variable fonts** are shaped at an instance. Embedding needs that instance as a static font
  (the subsetting step).

## Consequences

- The M3 writer, the add-new-text tool, and the editor-surface spike all start from
  `TextLayout`. A caret maps to text through `clusters`.
- Form appearances (M5) and stamps reuse the engine unchanged.
