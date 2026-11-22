"""Apple Vision (the OCR built into macOS) through ocr-eval/engines/apple-vision.swift (ADR 0030).

    python run_apple_vision.py

The reference engine from ADR 0006, re-run on the current set so every engine is
measured on the same pages. Vision reports only a total time, so per-page seconds
are the average.
"""

from __future__ import annotations

import subprocess
import time

from common import MANIFEST, REPO, Run, load_samples


def main() -> None:
    samples = load_samples()
    run = Run("apple-vision", {"engine": "VNRecognizeTextRequest, accurate, language correction, ar-SA + en-US"})
    t0 = time.time()
    subprocess.run(
        ["swift", str(REPO / "ocr-eval" / "engines" / "apple-vision.swift"), str(MANIFEST), str(run.hyp_dir)],
        check=True,
    )
    seconds = (time.time() - t0) / len(samples)
    run.per_sample = {sample.id: round(seconds, 3) for sample in samples}
    run.finish(None, children=True)


if __name__ == "__main__":
    main()
