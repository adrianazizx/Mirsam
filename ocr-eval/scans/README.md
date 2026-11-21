# Real scans

Synthetic pages cannot stand in for real paper. This folder holds real scanned
or photographed pages with **human-verified** ground truth; `generate.mjs`
merges them into the evaluation manifest.

Nothing is here yet. To add a scan:

1. Use only material we may redistribute: public-domain documents, or pages you
   created yourself. Never commit a copyrighted or confidential document
   (docs/SPEC.md section 0, rule 8).
2. Add three files with the same base name, e.g. `scan-gov-letter-001`:
   - `scan-gov-letter-001.png` — the page image exactly as scanned (no cleanup).
   - `scan-gov-letter-001.txt` — the full text in **logical reading order**,
     UTF-8, preserving tashkeel, digit style (Western vs Arabic-Indic), and
     punctuation exactly as printed. One paragraph or table cell per line.
   - `scan-gov-letter-001.json` — metadata:

     ```json
     {
       "categories": ["degraded-arabic"],
       "fields": ["12/03/2026", "١٢٣٤"],
       "source_note": "Where the page came from and why we may redistribute it",
       "dpi": 200,
       "verifiedBy": "Name of the person who proofread the .txt against the image",
       "verifiedOn": "2026-11-22"
     }
     ```
3. Bump `SET_VERSION` in `../generate.mjs`: results from different versions of
   the set are never compared.

`verifiedBy` is required. Text produced by an OCR engine and not proofread by a
person is not ground truth, however good it looks.
