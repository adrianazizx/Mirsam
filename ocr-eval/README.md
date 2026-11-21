# OCR evaluation set

Method, gates, and the first baseline: [docs/adr/0006-ocr-evaluation.md](../docs/adr/0006-ocr-evaluation.md).
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

| Path | What |
|---|---|
| `generate.mjs` | Sample matrix: content × Arabic face × scan condition → category. Bump `SET_VERSION` when samples or truth change. |
| `fields.json` | Numbers, dates, and IDs per content file that must be recognized exactly. |
| `targets.json` | Gates per category (the spec's initial targets; only ever tightened). |
| `scans/` | Real scans with human-verified truth. **Empty — this is the set's biggest gap.** |
| `engines/apple-vision.swift` | Reference engine: the OCR built into macOS. |
| `baselines/` | Stored reports, for regression checks and for the M4 comparison. |
| `out/` | Generated; not committed. |

Categories in set v1: `clean-arabic-300dpi`, `clean-english-300dpi`, `mixed-arabic-english`,
`degraded-arabic`, `numbers-dates-ids` (gated); `other-typefaces`, `diacritized`, `tables-forms`,
`degraded-mixed` (tracked). Still missing from the spec's list: phone photos, stamps and
letterheads, typewriter faces, handwriting — all of which need real images.
