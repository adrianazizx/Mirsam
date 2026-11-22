"""Prints the compact comparison tables for ADR 0030 from ocr-eval/baselines/*.set-v2.json.

    python adr_tables.py
"""

from __future__ import annotations

import json

from common import MANIFEST, OUT, REPO

ENGINES = [
    ("apple-vision", "Apple Vision"),
    ("tesseract", "Tesseract ara+eng"),
    ("tesseract-ara", "Tesseract ara"),
    ("ppocrv5-arabic", "PP-OCRv5 Arabic"),
    ("qari-0.3", "QARI 0.3"),
    ("qari-0.4", "QARI 0.4"),
    ("paddleocr-vl-1.6", "PaddleOCR-VL 1.6"),
    ("nuqta-0.8b", "Nuqta 0.8B"),
    ("nuqta-4b", "Nuqta 4B"),
]
CATEGORIES = [
    ("clean-arabic-300dpi", "Clean Arabic, 300 dpi (gated)"),
    ("other-typefaces", "Other typefaces"),
    ("diacritized", "Diacritized"),
    ("degraded-arabic", "Degraded Arabic (gated)"),
    ("degraded-arabic-severe", "Severe scans (new)"),
    ("mixed-arabic-english", "Mixed Arabic/English (gated)"),
    ("degraded-mixed", "Degraded mixed"),
    ("numbers-dates-ids", "Numbers, dates, IDs"),
    ("tables-forms", "Tables and forms"),
    ("clean-english-300dpi", "Clean English (gated)"),
    ("all", "All pages"),
]


def main() -> None:
    reports, runs = {}, {}
    for key, _ in ENGINES:
        path = REPO / "ocr-eval" / "baselines" / f"{key}.set-v2.json"
        if path.exists():
            reports[key] = json.loads(path.read_text())
            run = OUT / "hyp" / key / "run.json"
            runs[key] = json.loads(run.read_text()) if run.exists() else {}
    engines = [(k, n) for k, n in ENGINES if k in reports]

    print("| Category | n | " + " | ".join(n for _, n in engines) + " |")
    print("|---|---|" + "---|" * len(engines))
    for cat, label in CATEGORIES:
        values = [reports[k]["categories"][cat]["cer"] for k, _ in engines]
        best = min(values)
        cells = [f"**{100 * v:.2f}%**" if v == best else f"{100 * v:.2f}%" for v in values]
        n = reports[engines[0][0]]["categories"][cat]["samples"]
        print(f"| {label} | {n} | " + " | ".join(cells) + " |")

    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    bit64 = [s["id"] for s in manifest["samples"] if "64-bit" in s["fields"] and "numbers-dates-ids" in s["categories"]]
    print()
    print("| | " + " | ".join(n for _, n in engines) + " |")
    print("|---|" + "---|" * len(engines))
    rows = {"fields": [], "no64": [], "order": [], "cells": [], "wer": []}
    for key, _ in engines:
        c = reports[key]["categories"]["numbers-dates-ids"]
        matched = round(c["field_accuracy"] * c["fields_total"])
        got64 = sum("64-bit" in (OUT / "hyp" / key / f"{i}.txt").read_text(encoding="utf-8") for i in bit64)
        rows["fields"].append(f"{100 * matched / c['fields_total']:.1f}%")
        rows["no64"].append(f"{100 * (matched - got64) / (c['fields_total'] - len(bit64)):.1f}%")
        a = reports[key]["categories"]["all"]
        rows["order"].append(str(a["order_errors"]))
        rows["cells"].append(f"{100 * a['cell_accuracy']:.1f}% / {a['cell_order_errors']}")
        rows["wer"].append(f"{100 * a['wer']:.2f}%")
    total = reports[engines[0][0]]["categories"]["numbers-dates-ids"]["fields_total"]
    cells_total = reports[engines[0][0]]["categories"]["all"]["cells_total"]
    print(f"| WER, all pages | " + " | ".join(rows["wer"]) + " |")
    print(f"| Fields exact ({total}) | " + " | ".join(rows["fields"]) + " |")
    print(f"| … without \"64-bit\" ({total - len(bit64)}) | " + " | ".join(rows["no64"]) + " |")
    print(f"| Reading-order errors, all pages | " + " | ".join(rows["order"]) + " |")
    print(f"| Table cells exact / misordered ({cells_total}) | " + " | ".join(rows["cells"]) + " |")

    print()
    print("| Engine | Seconds per page | Peak RSS | Peak Metal | Weights |")
    print("|---|---|---|---|---|")
    gib = lambda mib: f"{mib / 1024:.1f} GiB" if mib and mib >= 1024 else (f"{mib:.0f} MiB" if mib else "–")  # noqa: E731
    for key, name in engines:
        r = runs[key]
        print(f"| {name} | {r.get('seconds_per_page', '–')} | {gib(r.get('peak_rss_mib'))} | {gib(r.get('peak_accelerator_mib'))} | {gib(r.get('model_mib'))} |")


if __name__ == "__main__":
    main()
