# Spike: the in-place editor's surface (Milestone 3)

Question, method, results, and decision: [docs/adr/0022-editor-surface.md](../../docs/adr/0022-editor-surface.md).

- **Harness** (runs inside the real webview): `apps/desktop/src/spikes/editorSurface.ts`.
- **Engine command** (`devtools` builds only): `apps/desktop/src-tauri/src/spike_editor.rs`, over
  `safha_core::typeset::{caret_stops, glyph_outline}`. Those two stay as the real editor's
  building blocks.
- **Raw report behind the ADR's tables:** `results-2026-11-22-macos-arm64.json`.

Re-run (also needed on Windows, with WebView2, and on Linux, with WebKitGTK):

```bash
cargo build -p safha-desktop --features devtools
SAFHA_SELFTEST_SPIKE=editor-surface scripts/selftest.sh fixtures/out/chrome/en-paragraphs.pdf /tmp/spike en light
```

The report is in `/tmp/spike/report.json`. Keep the window visible: typing latency is timed to
the next animation frame.
