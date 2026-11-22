# Spike: editor surface (M3, ADR 0022)

Question: for in-place editing, do we draw the caret, selection and IME composition ourselves
over the page, or put a native text field (a `contenteditable`) over the block? What matters:
caret fidelity against the layout the writer will save, Arabic input behaviour, effort.

What this measures: for five paragraphs (English, Arabic, mixed Arabic/English/numbers,
Arabic with full diacritics, justified Arabic) at 300 pt wide, the engine's layout
(`typeset::layout`, IBM Plex Sans / Plex Sans Arabic) against the webview's own layout of the
same text in a `contenteditable` with the same fonts, size and width. Per character, the
difference between the two layouts' horizontal extents. This is how far a native caret
would sit from where the saved text puts that character.

Run:

    cargo run -p spike-editor-surface            # writes out/layout.json
    # inline layout.json and the two fonts into out/data.js (base64) and open index.html
    # from a web server; the page prints the report (window.__report)

Results, 2026-11-22, Chromium in the app's browser pane (not WKWebView), fonts identical:

| Case | Lines (engine / browser) | Same breaks | Extent error, p95 | Extent error, max |
|---|---|---|---|---|
| English | 5 / 5 | yes | 0.008 pt | 0.008 pt (excluding hanging spaces) |
| Arabic | 3 / 3 | yes | 0.012 pt | 0.012 pt (excl. hanging spaces) |
| Mixed (digits, Latin, Arabic) | 3 / 3 | yes | 1.09 pt | 1.11 pt (excl. hanging spaces) |
| Arabic, full diacritics | 2 / 2 | yes | n/a | marks have zero advance in the engine; the browser reports its own box |
| Arabic, justified | 3 / 3 | yes | 0.010 pt | 0.010 pt (excl. hanging spaces) |

Outliers are not drift: a trailing space at a line end hangs in the engine (2.8 pt wide) and
has no box in the browser; a zero-advance mark has no width in the engine. The one real
difference is the mixed paragraph, where some Arabic letters sit 1.1 pt (0.09 em at 12 pt)
apart after a Latin or digit run. Line height differs too (browser 16 px "normal" vs the
engine's 15.6 pt for Plex): set it explicitly, as the engine's `LineHeight::Points`.

Not measured, and why: real Arabic input methods (there is no IME to drive here; Arabic
keyboards on macOS and Windows type final characters with no composition, while dead keys,
emoji, and CJK/Indic IMEs do compose); WKWebView (the shipping webview on macOS) rather than
Chromium; a custom surface's effort is estimated, not built.
