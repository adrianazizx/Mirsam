# 0052 — Find and replace

Status: Accepted (Milestone 6). Date: 2026-11-23.

## Context

Spec §5.3 (P1): "Find and replace text across the document". Spec §6.2 sets how search matches
Arabic (tashkeel and tatweel ignored, alef forms, ى/ي, optionally ة/ه, digit styles, presentation
forms), and the search panel already offers those options with case, whole words and regular
expressions (ADRs 0008, 0010).

A PDF has no text to replace, only glyphs drawn by operators. M3 built the only safe way this app
changes them: the in-place editor (ADRs 0023–0026) re-sets a whole paragraph from the editor's own
reading of the page, in the PDF's own fonts when they can set the new text and in a bundled
substitute when not, as an incremental update; PDFium never regenerates content (ADR 0005's
amendment).

Two readings of a page exist, and they differ: search runs on PDFium's text (ADR 0008), the
editor on its own interpreter (`blocks`). A match search finds may not be one the editor can
replace, and the user must be told which and why.

## Decision

### 1. What is replaced, and what is only listed (`safha_core::replace`)

- **Matches are found in the editor's paragraphs**: each paragraph's text as the editor shows it
  (logical order, NFKC, the text the paragraph field edits), with the search's own compiled query
  (`search::Query`), so the panel's options mean the same thing in both.
- **Each character of a paragraph knows where it is drawn** (`blocks::Block::places`, carried
  through logical reordering and NFKC folding a cluster at a time), so a match has boxes on the
  displayed page: the preview shows it there, and a search hit is matched to it by geometry
  (most of the hit's glyph centres inside the match's boxes).
- **Search hits no paragraph match covers are listed too, skipped**, with why:
  - `crossesParagraphs`: the hit's glyphs lie in two paragraphs, or in one and in text that is
    not one (a baseline shift search reads as one line, the editor as two paragraphs);
  - the paragraph's or the area's refusal (`shared`, `unreadable`, `font`, `rotated`,
    `encrypted`, `unsupported`; ADR 0023's amendment);
  - `inexact`: the editor reads that text differently from search.
- A match in a paragraph that cannot be edited is listed with that paragraph's refusal;
  `tooLong` when the replaced paragraph would pass the editor's 20,000 characters.

### 2. Replacement text

- A literal query's replacement is taken as typed.
- A regular expression's may name its groups: `$1` (two digits when that group exists), `${1}`,
  `${name}` or `$<name>`, `$0` or `$&`, `$$`. A group that took no part, or does not exist, gives
  nothing; any other `$` is kept.
- **Groups give the document's own text, not the folded text the pattern ran on**: `(\d+)` over
  `١٢٣٤` gives `١٢٣٤`, and a group over a vocalised word keeps its marks. Each group is extended
  over the marks on its last letter, as matches are.
- **A regular expression's Arabic is folded as the text is** (a fix to search): with
  normalisation on, `أجهزة` in a pattern found nothing, since the text had become `اجهزة`. Only
  the Arabic folds; case is still the regex flag's (folding `\D` to `\d` would change its meaning).

### 3. Setting each paragraph again, exactly or not at all

- **A paragraph is set again once, with all its replacements**, through `block_edit` as a user's
  edit would be (ADR 0024's styled runs: replaced text takes the style of what it replaced).
  Fonts: the page's own by their codes, else their embedded programs (ADR 0026), else a bundled
  substitute for the whole paragraph. The report names each substitution once (original font →
  face used) and counts the paragraphs; the UI says it as the editor's "Font substituted" does.
- **After each paragraph is set, the page is read again.** Unless a paragraph overlapping the old
  one's place holds the new text (compared folded, white space as single spaces), the edit is
  dropped and its matches are reported `inexact`. Nothing that would not read back as replaced is
  kept.
- The paragraph is found again by its text and place before its edit (setting another paragraph
  of the page renumbers them); a target whose paragraph or match moved is skipped `changed`.
- **All the edits are one incremental update** (`Change::Content`): one undo step.

### 4. Encrypted files

The in-place editor read nothing in encrypted files (ADR 0023). It now does what ADR 0051's
objects do: page content and fonts' `/ToUnicode` and program streams are decrypted under their own
numbers (`pdf::fontinfo::stream_data_in`), and new content streams and font objects are encrypted
under theirs (`pdf::crypt`). A page whose resources would carry strings into the page's own
dictionary is refused, as ADR 0021's added text is. `encrypted` remains only for handlers this
editor cannot write. Editing a paragraph in place works in encrypted files too.

### 5. Turned pages

Search could not find text upright on a page displayed turned a quarter (the editor could): in
page space that text runs up or down the page, and PDFium's extraction read it letter by letter
(ADR 0050 recorded the gap).

- Each character's baseline is read from PDFium (`FPDFText_GetCharAngle`, which is
  `atan2(c, a)` of its matrix and so turns **clockwise**) and snapped to a quarter turn when within
  0.2 of one; slanted text is read as before.
- Each quarter turn's characters are read in their own frame, where they run left to right, and
  their boxes turned back. A page whose text is all upright takes exactly the old path.
- Space marks (ADR 0008) carry the quarter turn of the text object that draws them, from its
  matrix, so the page's main text's marks cannot break words of text that crosses it.

### 6. Lines without letters

The editor read a wrapped line holding only "١٢٣٤." left to right (its direction was the majority
of its letters, and digits counted as left-to-right letters), so "رقم ١٢٣٤." read back as
"رقم .١٢٣٤". Digits and punctuation now give a line no direction (UAX #9: they are weak or
neutral); such a line takes its paragraph's, and does not vote on it.

### 7. The UI

- The search panel's options gain a **Replace** toggle; its row holds the replacement (placeholder
  naming `$1` when the query is a regular expression), **Replace** (the hit shown; Enter in the
  field) and **Replace all…**.
- **Replace all…** opens the preview in the panel: every match, its page, its context in its own
  direction with the match struck through and the replacement after it (each isolated, so an old
  number and its replacement do not run together in right-to-left text), ticked; skipped ones with
  their reason. **Replace N** applies the ticked ones; a changed option or replacement makes the
  preview again; Escape or Cancel closes it.
- After replacing, the search runs again: what is left is what was not replaced. A notice says how
  many were replaced, how many not, and any substitution; the hit after a replaced one is shown.
- Commands: `search.replaceOpen` (Ctrl+H; **⌥⌘F on macOS**, where ⌘H hides the app),
  `search.replaceClose`, `search.setReplacement`, `search.replace`, `search.replaceAll`,
  `search.replaceConfirm`, `search.replaceCancel`, `search.replaceToggle`, `search.replaceShow`.
  `Shortcut` gains `alt` (matched by the physical key, as Option changes the character) and a
  command may have a `macShortcut`. Replacing is gated by the file's `modify` permission.
- Shell: `replace_preview` (on the shared search index) and `replace_text` (scope: the preview's
  targets, or the hit shown by its page and boxes).

## Consequences

- **Honest limits:**
  - A match that runs from one paragraph into another is not replaced; there is no reflow across
    paragraphs (spec §7).
  - Every limit of in-place editing applies: Type 3 fonts, CoreText's Arabic and rotated text are
    not replaced (Chrome's fixtures draw in Type 3, so nothing in them is), reused fonts lose
    kerning, a paragraph that grows can overlap the one below, one character the paragraph's fonts
    lack substitutes the whole paragraph.
  - The preview can list more matches than search: a paragraph's match may run across a line
    break, which search's line-by-line matching does not see.
  - Case is not carried over ("Contractor" and "contractor" both become what was typed).
  - Each paragraph costs two page reads (to set it, and to check it); there is no progress bar or
    cancel for a long replace-all.
- **Budgets:** typing in the paragraph field is not touched (17 ms a keystroke in the self-test).
  Extraction reads one more property per character (its angle): 1,000 pages index in 1.12–1.21 s
  and are searched in 17–19 ms (release, under a load average of 30; M2 measured 1.11 s and
  17–18 ms).

## Verified

- `tests/replace.rs` (PDFium renders and extracts, our reader reads back, `qpdf --check` passes):
  - English, two pages, Helvetica: the preview's three matches with context and boxes where search
    sees them; replace-all as one update on top of the file, nothing embedded, read back in our
    extractor; saved, reopened, nothing left to replace; only the chosen match replaced; a search
    hit names its match; stale targets skipped `changed`.
  - A baseline shift: one line to search, two paragraphs to the editor: listed, skipped
    `crossesParagraphs`. Chrome's Type 3 text: every match listed, skipped `font`.
  - Arabic: "الى" as a whole word finds "إلى" and "الى", replaced in one paragraph set once, read
    back in logical order by both readers. Arabic into a Helvetica paragraph: a reported substitute.
  - Mixed: an English product name inside Arabic replaced with an Arabic one; a regular expression
    with two groups over Western and Arabic-Indic digits, kept as written; read back in logical
    order by both readers.
  - Turned pages (0°, 90°, 180°, 270°): each page's paragraph found where search sees it, replaced,
    read back.
  - Encrypted (AES-256): replaced, no text in the clear in the update, `qpdf --check` with the
    password; saved, opens only with its password, nothing left to replace.
- `tests/block_edit.rs`: an encrypted file's text is read and edited in place, saved, reopened with
  its password, and edited again.
- Unit tests: groups from the original text (Arabic-Indic digits, vocalised words, named groups,
  `$$`, missing groups, `$10`), literal templates, folded regex patterns; boxes over a line break;
  skip reasons by geometry; read-back; quarter turns from PDFium's angles; turned text read as
  lines; a line of digits and punctuation in its paragraph's direction; places in logical order.
- UI (`commands/replace.test.tsx`): shortcuts per platform; replacing the hit shown and the next
  shown; a skipped hit explained; the preview, a match left out, one undo; a changed replacement
  previewed again; the `modify` gate; the panel in English and Arabic (Arabic-Indic counts, each
  context in its own direction).
- The shell self-test replaces Arabic text written into the page (normalised search, preview,
  confirm, read back, one undo) in the real app, in English and Arabic.
- Checked in the browser preview (mock backend) in both directions and both themes.
