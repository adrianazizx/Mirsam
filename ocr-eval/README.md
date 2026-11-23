# OCR evaluation set

Method, gates, and the first baseline: [docs/adr/0006-ocr-evaluation.md](../docs/adr/0006-ocr-evaluation.md).
Set v2, table-cell scoring, and the M4 engine benchmark: [docs/adr/0030-ocr-engines.md](../docs/adr/0030-ocr-engines.md).
The rule from the spec: **accuracy → reading order → robustness → speed → model size**, and the
default pipeline must beat plain Tesseract and the leading commercial editors in every Arabic
category before M4 can close.

```bash
node ocr-eval/generate.mjs      # synthetic samples -> ocr-eval/out/ (images, truth, manifest.json)

# Score an engine: it must write <out-dir>/<sample id>.txt, plain text in reading order.
cargo run -p safha-ocr-eval -- \
  --manifest ocr-eval/out/manifest.json --hyp <out-dir> --engine <name> \
  --targets ocr-eval/targets.json [--baseline ocr-eval/baselines/<name>.set-v1.json] [--out report.json]
```

Exit status 1 means a gate failed or a category regressed against the baseline.

## The benchmark gate (ADR 0036)

CI cannot run the models (6.6 GB, minutes a page on a hosted runner's CPU), so it replays
their recorded answers through the current pipeline code and scores the result:

```bash
# CI's run, no models needed (release builds need SAFHA_PDFIUM_DIR, see CLAUDE.md):
cargo run --release -p safha-core --example ocr_bench -- --out <dir> --replay ocr-eval/recordings/set-v2
cargo run --release -p safha-ocr-eval -- --manifest ocr-eval/out/manifest.json --hyp <dir>/text \
  --engine pipeline-nuqta-4b+0.8b --targets ocr-eval/targets.json \
  --baseline ocr-eval/baselines/pipeline-nuqta-4b+0.8b.set-v2.json
cargo run --release -p safha-ocr-eval -- --manifest ocr-eval/out/manifest.json --hyp <dir>/layer \
  --engine pipeline-layer-nuqta-4b+0.8b --baseline ocr-eval/baselines/pipeline-layer-nuqta-4b+0.8b.set-v2.json
```

`<dir>/text` is the pipeline's text; `<dir>/layer` is the searchable PDF's text, read back by
the app's extractor after the text layer is written into a one-image PDF of the page (ADR 0035).

**Record again** when a change makes the pipeline ask the models something new (the replay
then fails with "no recorded answer … record again"), when the models' pins change, or when
the set's version changes. It needs the models and a GPU (about 25 minutes on an M3 Max):

```bash
cargo run --release -p safha-core --example ocr_bench -- --out <dir> \
  --record ocr-eval/recordings/set-v2 --model-dir <GGUF folder> [--truth-boxes word-boxes-truth.json]
```

then score both sets with `--out` into `baselines/` and commit the recordings and baselines
together. `--truth-boxes` (from `spikes/ocr-engines/word_boxes_truth.mjs`) also scores the word
boxes against Chrome's layout (ADR 0031's measures).

| Path | What |
|---|---|
| `generate.mjs` | Sample matrix: content × Arabic face × scan condition → category. Bump `SET_VERSION` when samples or truth change. |
| `content/` | OCR-only pages (forms), in the same format as `fixtures/content/` but kept out of the PDF fixture corpus. |
| `fields.json` | Numbers, dates, and IDs per content file that must be recognized exactly. |
| `targets.json` | Gates per category (the spec's initial targets; only ever tightened). |
| `scans/` | Real scans with human-verified truth. **Empty — this is the set's biggest gap.** |
| `engines/apple-vision.swift` | Reference engine: the OCR built into macOS. Other engines' runners are in `spikes/ocr-engines/`. |
| `baselines/` | Stored reports, one per engine and set version (`<engine>.set-v<N>.json`), for regression checks and the M4 comparison. |
| `baselines/word-boxes/` | Word-box spike reports (ADR 0031): line assignment and box overlap per page and method. |
| `recordings/set-v<N>/` | Every model answer of a full pipeline run, per sample, with the draft reader's lines: what the CI gate replays (ADR 0036). |
| `out/` | Generated; not committed. |

Categories in set v2 (47 samples): `clean-arabic-300dpi`, `clean-english-300dpi`,
`mixed-arabic-english`, `degraded-arabic`, `numbers-dates-ids` (gated); `other-typefaces`,
`diacritized`, `tables-forms`, `degraded-mixed`, `degraded-arabic-severe` (tracked).

Set v2 added, without changing any v1 sample:

- **Severe scan conditions**, built from SVG filters in Chrome: a 100 dpi fax with thinned,
  broken strokes and specks; a second-generation photocopy (ink bleed, mottled grey paper, 2°
  skew); faded toner; and a phone photo of a curved page (perspective, wavy lines, uneven light).
  Arabic prose under these is the `degraded-arabic-severe` category; mixed lines and tables get
  them too. v1's `degraded-arabic` stays as it was, and so does its gate.
- **An Arabic-only form** (`content/form-leave-request.json`): a label/value table with
  Arabic-Indic IDs and dates.
- **Table cells.** Table samples list their cells (`cells` in the manifest, row by row in reading
  order). The scorer reports the share of cells read exactly and the number read but out of order,
  beside page CER, which cannot tell a misread table from a misordered one.
- **Either column first** on side-by-side bilingual pages. An Arabic reader finishes the Arabic
  column first and an English reader the English one; both orders are right as long as each column
  stays whole. The manifest lists the swapped order as `truth_alternatives`, and the scorer keeps
  whichever truth fits the output better. Interleaving the columns line by line still fails both.

Still missing from the spec's list: real phone photos, stamps and letterheads, typewriter faces,
handwriting, all of which need real images. The severe conditions are synthetic too.
