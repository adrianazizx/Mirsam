# 0026 — Setting edited text in the PDF's own embedded fonts

Status: Accepted (Milestone 3). Date: 2026-11-22.

## Context

Spec §7, step 3: "reuse the PDF's embedded font if it has the glyphs; otherwise substitute
for the whole paragraph". ADR 0023 reused a font only through its codes. A Latin character
needed a code in the font's `/ToUnicode`, so the font set the text as it is and nothing new
was embedded. Arabic always went to a bundled substitute: "embedded Arabic subsets hold
letter forms, not the letters".

What the fixtures' producers embed:

| Producer | Program | `cmap` | `GSUB` / `GPOS` | `OS/2` |
|---|---|---|---|---|
| Chrome (static TrueType web fonts) | `FontFile2` subsets | yes: 52 Arabic presentation forms, ligatures (U+FC94 yeh-noon) and the letters it drew isolated | no | yes, `fsType` 0 |
| CoreText | `FontFile2` subsets | Mac Roman only: no Arabic | no | no |
| Chrome (variable fonts) | Type 3 | — | — | — |

So a shaper given Chrome's subset finds no `GSUB` to choose a letter's joined form, and no
`cmap` entry for most bare letters. harfrust (the HarfBuzz port in `typeset`) gave `.notdef` for
them: it has no Arabic fallback shaper.

## Decision

### A third way to set an edited paragraph, between the two

1. **The page's own fonts by their codes** (ADR 0023): Latin characters the font has codes for.
   Nothing is embedded.
2. **New: the page's own fonts' embedded programs, laid out again** (`FontSource::Embedded`).
   - Each run's font (or the one of its family with the bold and italic it is to have,
     ADR 0025) must have a TrueType or OpenType program the layout engine can read.
   - The new text is laid out with those programs and embedded as new subsets under the
     font's own name: the same glyphs, in any script.
3. **Bundled faces** (ADRs 0023, 0024).

### Arabic from a subset with no `GSUB`: presentation forms

This is what HarfBuzz calls its Arabic fallback shaper (`typeset::presentation_forms_in`):

- **Each letter's joined form** (isolated, final, initial, medial) comes from its joining type:
  Unicode's `Joining_Type`, through ICU. Marks are skipped.
- **The letter is replaced by Unicode's presentation-form character** for that form,
  U+FB50–U+FEFF. The tables are generated from Unicode's compatibility decompositions.
- **Lam then alef** becomes its ligature, as Arabic requires.
- **Unicode's optional two- and three-letter ligatures** are used when the font has them. The
  producer's shaper drew them, so its subset has the ligature (yeh-noon U+FC94), not the
  separate forms.
- **Where the font lacks a letter's isolated form but has the letter,** the letter stays.
  Subsets map isolated forms by the letter.
- **What is written still says the letters:** `typeset::logical_text` folds presentation
  forms back for `/ToUnicode` and `/ActualText`. Copying, search and our extractor read
  letters in logical order.

Forms are used only when a chosen face has no `GSUB`. A complete embedded font shapes the
letters itself.

### When the embedded program is not used

The paragraph then goes to bundled faces, whole, as spec §7 asks:
- **a character the program lacks** (in its presentation form, for Arabic): spaces may fall
  back to a bundled face, since they have no glyph to differ;
- **marks with no `GPOS` to place them** (tashkeel in Chrome's subsets);
- **embedding permissions that forbid it** (`OS/2.fsType`): restricted, preview and print only,
  no subsetting, or bitmaps only. Installable and editable embedding are allowed. A subset with
  no `OS/2` (CoreText's) says nothing; the producer embedded it, and the new subset holds only
  glyphs it already held.
- **a Type 1 program or bare CFF**, which the layout engine does not read.

### Reported

`EditedBlock::source`, passed to the UI, says whether the text was set in the page's fonts, in
their embedded programs, or in bundled faces. A substitution is reported as before.

## Consequences

- **Fixed:** an Arabic paragraph from Chrome keeps its own font when it is edited with forms
  the paragraph already drew: words removed, moved or retyped. Before, it was always set in
  a bundled substitute.
- **The limit is the subset:**
  - a letter in a position the paragraph never drew it in has no glyph, so it falls back;
  - text whose letters are all in a typical body paragraph mostly succeeds;
  - a short heading often does not.
- **Kerning and mark positioning** are what the subset lacks: `GPOS`. Text set from it has
  none, so marks fall back to bundled faces.
- **Each edit embeds a new subset of the program.** The old one leaves the page when nothing
  uses it (ADR 0023's clean-up), and the file at the next save.
- **Not yet:**
  - Type 1 and bare CFF programs;
  - composite glyphs or `CIDToGIDMap`s the program's `cmap` does not reach;
  - choosing, per character, between the page's program and a bundled face (spec §7: the
    whole paragraph substitutes).
- **Seen separately:** main's paragraph analysis reads Chrome's tashkeel with stray spaces
  ("عل م ًا"). It is not caused by this change; it is a follow-up in `blocks`.

## Verified

- **Unit tests:**
  - joined forms (isolated, initial, medial, final; alef not joining forward; spaces);
  - marks skipped for joining;
  - lam-alef standing for two letters;
  - a font's isolated form by its letter;
  - an optional ligature the font has (yeh-noon);
  - presentation forms read back as letters;
  - embedding permissions (installable, editable, none; restricted, preview-only, no
    subsetting, bitmaps).
- **`tests/block_edit.rs`, on Chrome's Arabic fixture:**
  - **the heading re-set:** in its own program (IBM Plex Sans Arabic SemiBold from the PDF,
    re-subset); read back as letters; nothing changed outside it; its ink within 10% of the
    original's;
  - **a word taken away:** still its own program;
  - **an isolated beh the subset lacks:** bundled faces;
  - **a paragraph with tashkeel:** bundled faces (no `GPOS`);
  - **Latin in a page's own codes:** still embeds nothing.
