"""Word-box spike, step 1: PP-OCRv5 detector lines (and its recogniser's text as a guide).

    python wb_detect.py [--threads 4]

For every sample that has ground-truth word boxes (word_boxes_truth.mjs) this writes
spikes/out/ocr-engines/wb/detect/<id>.json:
  lines   [{box:[x0,y0,x1,y1], poly, text}]  in the detector's order
  det_seconds      TextDetection only (PP-OCRv5_server_det, long side capped at 2048 px)
  guide_seconds    full pipeline: detector + Arabic recogniser on every crop

PaddlePaddle's CPU backend on macOS, like run_ppocr.py. ONNX Runtime (the spec's plan) is not used.
"""

from __future__ import annotations

import argparse
import json
import os
import time

from common import CACHE, OUT, load_samples

os.environ.setdefault("PADDLE_PDX_CACHE_HOME", str(CACHE / "paddlex"))
os.environ.setdefault("PADDLE_PDX_DISABLE_MODEL_SOURCE_CHECK", "True")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--threads", type=int, default=4)
    parser.add_argument("--only")
    args = parser.parse_args()
    from paddleocr import PaddleOCR, TextDetection

    truth = json.loads((OUT / "word-boxes-truth.json").read_text())
    det = TextDetection(model_name="PP-OCRv5_server_det", limit_type="max", limit_side_len=2048, device="cpu", cpu_threads=args.threads)
    ocr = PaddleOCR(
        text_detection_model_name="PP-OCRv5_server_det",
        text_recognition_model_name="arabic_PP-OCRv5_mobile_rec",
        text_det_limit_type="max",
        text_det_limit_side_len=2048,
        use_doc_orientation_classify=False,
        use_doc_unwarping=False,
        use_textline_orientation=False,
        device="cpu",
        cpu_threads=args.threads,
    )
    out_dir = OUT / "wb" / "detect"
    out_dir.mkdir(parents=True, exist_ok=True)
    for sample in load_samples(args.only):
        if sample.id not in truth:
            continue
        t0 = time.time()
        d = det.predict(str(sample.image))[0]
        det_seconds = time.time() - t0
        t0 = time.time()
        r = ocr.predict(str(sample.image))[0]
        guide_seconds = time.time() - t0
        lines = [
            {"box": [int(v) for v in box], "poly": [[int(x), int(y)] for x, y in poly], "text": text}
            for box, poly, text in zip(r["rec_boxes"], r["rec_polys"], r["rec_texts"])
        ]
        record = {
            "lines": lines,
            "det_boxes": [[int(v) for v in p] for p in [[min(x for x, _ in poly), min(y for _, y in poly), max(x for x, _ in poly), max(y for _, y in poly)] for poly in d["dt_polys"]]],
            "det_seconds": round(det_seconds, 2),
            "guide_seconds": round(guide_seconds, 2),
        }
        (out_dir / f"{sample.id}.json").write_text(json.dumps(record, ensure_ascii=False))
        print(f"det {det_seconds:5.1f}s  full {guide_seconds:5.1f}s  {len(lines):3d} lines  {sample.id}", flush=True)


if __name__ == "__main__":
    main()
