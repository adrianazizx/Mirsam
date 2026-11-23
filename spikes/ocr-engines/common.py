"""Shared plumbing for the M4 engine benchmark (throwaway spike, ADR 0030).

Every runner reads the evaluation manifest, writes `<out>/hyp/<engine>/<sample id>.txt`
(plain text in reading order, which is all the scorer needs), and records timing and
memory in `<out>/hyp/<engine>/run.json`. Timings are noisy: other sessions build on
the same Mac while this runs.
"""

from __future__ import annotations

import json
import os
import platform
import resource
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
MANIFEST = REPO / "ocr-eval" / "out" / "manifest.json"
OUT = Path(os.environ.get("MIRSAM_OCR_BENCH_OUT", REPO / "spikes" / "out" / "ocr-engines"))
# Models, Python environments and caches live outside the repo and outside OneDrive.
CACHE = Path(os.environ.get("MIRSAM_OCR_BENCH_CACHE", Path.home() / "Library" / "Caches" / "mirsam-dev" / "ocr-bench"))


@dataclass
class Sample:
    id: str
    image: Path
    dpi: int | None
    categories: list[str]


def load_samples(only: str | None = None) -> list[Sample]:
    if not MANIFEST.exists():
        sys.exit(f"{MANIFEST} is missing: run `node ocr-eval/generate.mjs` first")
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    samples = [
        Sample(s["id"], (MANIFEST.parent / s["image"]).resolve(), s.get("dpi"), s["categories"])
        for s in manifest["samples"]
    ]
    if only:
        samples = [s for s in samples if only in s.id]
    return samples


def peak_rss_bytes(children: bool = False) -> int:
    usage = resource.getrusage(resource.RUSAGE_CHILDREN if children else resource.RUSAGE_SELF)
    # ru_maxrss is bytes on macOS, kilobytes on Linux.
    return usage.ru_maxrss if platform.system() == "Darwin" else usage.ru_maxrss * 1024


def dir_size_bytes(path: Path) -> int:
    return sum(p.stat().st_size for p in path.rglob("*") if p.is_file() and not p.is_symlink())


@dataclass
class Run:
    engine: str
    settings: dict
    hyp_dir: Path = field(init=False)
    per_sample: dict = field(default_factory=dict)
    flags: dict = field(default_factory=dict)
    load_seconds: float = 0.0
    extra_peak_bytes: int = 0  # e.g. Metal (MPS) memory, which RSS does not show
    started: float = field(default_factory=time.time)

    def __post_init__(self) -> None:
        self.hyp_dir = OUT / "hyp" / self.engine
        self.hyp_dir.mkdir(parents=True, exist_ok=True)

    def write(self, sample: Sample, text: str, seconds: float, **flags) -> None:
        (self.hyp_dir / f"{sample.id}.txt").write_text(text.strip() + "\n", encoding="utf-8")
        self.per_sample[sample.id] = round(seconds, 3)
        if flags:
            self.flags[sample.id] = flags
        print(f"{seconds:7.1f} s  {sample.id}{'  ' + str(flags) if flags else ''}", flush=True)

    def finish(self, model_bytes: int | None, children: bool = False) -> None:
        recognised = sum(self.per_sample.values())
        record = {
            "engine": self.engine,
            "settings": self.settings,
            "machine": f"{platform.machine()} {platform.mac_ver()[0] or platform.system()}",
            "samples": len(self.per_sample),
            "load_seconds": round(self.load_seconds, 1),
            "recognition_seconds": round(recognised, 1),
            "seconds_per_page": round(recognised / max(1, len(self.per_sample)), 2),
            "pages_per_minute": round(60 * len(self.per_sample) / recognised, 2) if recognised else None,
            "peak_rss_mib": round(peak_rss_bytes(children) / 2**20),
            "peak_accelerator_mib": round(self.extra_peak_bytes / 2**20) if self.extra_peak_bytes else None,
            "model_mib": round(model_bytes / 2**20, 1) if model_bytes else None,
            "per_sample_seconds": self.per_sample,
            "flags": self.flags,
            "note": "Timings are noisy: other sessions were building on this Mac during the run.",
        }
        (self.hyp_dir / "run.json").write_text(json.dumps(record, indent=2, ensure_ascii=False) + "\n")
        print(json.dumps({k: v for k, v in record.items() if k not in ("per_sample_seconds", "flags")}, indent=2))
