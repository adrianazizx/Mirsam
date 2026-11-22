"""Scores every engine's hypotheses and prints the comparison tables for ADR 0030.

    python summarize.py [engine ...]     # default: every directory under <out>/hyp

Runs `safha-ocr-eval` once per engine (reports land in <out>/reports/), then prints
Markdown: one table per metric (rows = categories, columns = engines), plus the
recorded speed, memory and model size.
"""

from __future__ import annotations

import json
import subprocess
import sys

from common import MANIFEST, OUT, REPO

CATEGORIES = [
    "clean-arabic-300dpi",
    "other-typefaces",
    "diacritized",
    "degraded-arabic",
    "degraded-arabic-severe",
    "mixed-arabic-english",
    "degraded-mixed",
    "numbers-dates-ids",
    "tables-forms",
    "clean-english-300dpi",
    "all",
]


def score(engine: str) -> dict:
    reports = OUT / "reports"
    reports.mkdir(parents=True, exist_ok=True)
    path = reports / f"{engine}.json"
    command = ["cargo", "run", "-q", "-p", "safha-ocr-eval", "--", "--manifest", str(MANIFEST)]
    command += ["--hyp", str(OUT / "hyp" / engine), "--engine", engine, "--out", str(path)]
    subprocess.run(command, cwd=REPO, check=True, stdout=subprocess.DEVNULL)
    return json.loads(path.read_text())


def table(title: str, engines: list[str], reports: dict, cell) -> None:
    print(f"\n**{title}**\n")
    print("| Category | n | " + " | ".join(engines) + " |")
    print("|---|---|" + "---|" * len(engines))
    for category in CATEGORIES:
        first = reports[engines[0]]["categories"].get(category)
        if not first:
            continue
        values = [cell(reports[e]["categories"][category]) for e in engines]
        print(f"| {category} | {first['samples']} | " + " | ".join(values) + " |")


def main() -> None:
    engines = sys.argv[1:] or sorted(p.name for p in (OUT / "hyp").iterdir() if p.is_dir())
    reports = {engine: score(engine) for engine in engines}
    pct = lambda x: f"{100 * x:.2f}%"  # noqa: E731
    table("CER (strict)", engines, reports, lambda c: pct(c["cer"]))
    table("WER (strict)", engines, reports, lambda c: pct(c["wer"]))
    table("CER ignoring tashkeel and letter variants (informational)", engines, reports, lambda c: pct(c["cer_base"]))
    table(
        "Fields exact",
        engines,
        reports,
        lambda c: pct(c["field_accuracy"]) if c["fields_total"] else "–",
    )
    table("Reading-order errors", engines, reports, lambda c: str(c["order_errors"]))
    table(
        "Table cells read exactly / misordered",
        engines,
        reports,
        lambda c: f"{pct(c.get('cell_accuracy', 1.0))} / {c.get('cell_order_errors', 0)}" if c.get("cells_total") else "–",
    )

    print("\n**Speed, memory, size (recorded, not gated; noisy)**\n")
    print("| Engine | s/page | pages/min | load s | peak RSS MiB | peak Metal MiB | model MiB | flags |")
    print("|---|---|---|---|---|---|---|---|")
    for engine in engines:
        path = OUT / "hyp" / engine / "run.json"
        if not path.exists():
            print(f"| {engine} | – | – | – | – | – | – | no run.json |")
            continue
        r = json.loads(path.read_text())
        flags = ", ".join(f"{k}: {v}" for k, v in r.get("flags", {}).items()) or ""
        print(
            f"| {engine} | {r['seconds_per_page']} | {r['pages_per_minute']} | {r['load_seconds']} | "
            f"{r['peak_rss_mib']} | {r['peak_accelerator_mib'] or '–'} | {r['model_mib'] or '–'} | {flags} |"
        )


if __name__ == "__main__":
    main()
