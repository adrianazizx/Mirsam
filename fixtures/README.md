# PDF fixture corpus

Generated, never committed. Design and coverage: [docs/adr/0007-fixtures-and-self-test.md](../docs/adr/0007-fixtures-and-self-test.md).

```bash
node fixtures/generate.mjs                       # everything -> fixtures/out/ + manifest.json
node fixtures/generate.mjs --producer chrome     # one producer (does not rewrite the manifest)
node fixtures/generate.mjs --only mixed-inline   # one case
```

- `content/*.json` — one case each. **This is the ground truth**: every block's `text` is the
  logical-order string that extraction, search, and copy must reproduce. Edit with care; tests in
  later milestones assert against it.
- `fonts.json` — font keys → OFL files in `assets/fonts/`.
- `lib.mjs` — content → HTML layout, shared with `ocr-eval/generate.mjs`.
- `producers/coretext.swift` — the macOS CoreText/Quartz producer.
- `out/manifest.json` — what was generated, with checksums, page counts, and expected text.

Adding a case: add a content file covering the situation, regenerate, and look at the result with
`cargo run -p mirsam-core --example render_png`. Adding a producer: one entry in `PRODUCERS` in
`generate.mjs`. Needs Chrome (or `CHROME_BIN`), qpdf, and — for the CoreText producer — macOS.
