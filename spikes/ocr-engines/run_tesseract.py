"""Plain Tesseract over the evaluation set (ADR 0030): the spec's floor to beat.

    python run_tesseract.py [--only <id substring>] [--psm 3]

`tesseract` from Homebrew, with the tessdata_best `ara` and `eng` models from
SAFHA_OCR_BENCH_CACHE/tessdata. LSTM engine, automatic page segmentation (its own
layout analysis decides the reading order), the sample's true resolution.
"""

from __future__ import annotations

import argparse
import subprocess
import time

from common import CACHE, Run, load_samples

TESSDATA = CACHE / "tessdata"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--only")
    parser.add_argument("--psm", default="3")
    parser.add_argument("--langs", default="ara+eng")
    args = parser.parse_args()

    version = subprocess.run(["tesseract", "--version"], capture_output=True, text=True).stdout.splitlines()[0]
    engine = "tesseract" + ("" if args.langs == "ara+eng" else f"-{args.langs}") + ("" if args.psm == "3" else f"-psm{args.psm}")
    run = Run(engine, {"version": version, "langs": args.langs, "psm": args.psm, "oem": "1 (LSTM)", "tessdata": "tessdata_best"})

    for sample in load_samples(args.only):
        t0 = time.time()
        command = ["tesseract", str(sample.image), "stdout", "--tessdata-dir", str(TESSDATA)]
        command += ["-l", args.langs, "--oem", "1", "--psm", args.psm]
        if sample.dpi:
            command += ["--dpi", str(sample.dpi)]
        result = subprocess.run(command, capture_output=True, text=True, check=True)
        run.write(sample, result.stdout, time.time() - t0)

    model_bytes = sum((TESSDATA / f"{lang}.traineddata").stat().st_size for lang in args.langs.split("+"))
    run.finish(model_bytes, children=True)


if __name__ == "__main__":
    main()
