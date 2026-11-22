"""PaddleOCR PP-OCRv5 (server detector + Arabic recogniser) over the evaluation set (ADR 0030).

    python run_ppocr.py [--only <id substring>] [--det PP-OCRv5_server_det]

PP-OCRv5 has no layout analysis: it returns text lines with boxes. The lines are
put in reading order here with the simplest rule that respects Arabic: boxes on the
same text line (vertical overlap) form one line, read right to left when the line
holds Arabic letters and left to right otherwise; lines go top to bottom. There is
no column detection, so two-column pages and tables measure this rule, not the
recogniser. A real classical pipeline would add a layout model (PP-DocLayout).
"""

from __future__ import annotations

import argparse
import os
import re
import time

from common import CACHE, Run, dir_size_bytes, load_samples

os.environ.setdefault("PADDLE_PDX_CACHE_HOME", str(CACHE / "paddlex"))
os.environ.setdefault("PADDLE_PDX_DISABLE_MODEL_SOURCE_CHECK", "True")

ARABIC = re.compile(r"[؀-ۿ]")


def reading_order(boxes: list[tuple[float, float, float, float]], texts: list[str]) -> str:
    items = sorted(zip(boxes, texts), key=lambda item: (item[0][1] + item[0][3]) / 2)
    lines: list[list] = []
    for box, text in items:
        top, bottom = box[1], box[3]
        if lines:
            line_top = min(b[1] for b, _ in lines[-1])
            line_bottom = max(b[3] for b, _ in lines[-1])
            overlap = min(bottom, line_bottom) - max(top, line_top)
            if overlap > 0.5 * min(bottom - top, line_bottom - line_top):
                lines[-1].append((box, text))
                continue
        lines.append([(box, text)])
    out = []
    for line in lines:
        rtl = any(ARABIC.search(text) for _, text in line)
        line.sort(key=lambda item: item[0][0], reverse=rtl)
        out.append(" ".join(text for _, text in line))
    return "\n".join(out)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--only")
    parser.add_argument("--det", default="PP-OCRv5_server_det")
    parser.add_argument("--rec", default="arabic_PP-OCRv5_mobile_rec")
    args = parser.parse_args()

    import paddle
    import paddleocr
    from paddleocr import PaddleOCR

    started = time.time()
    ocr = PaddleOCR(
        text_detection_model_name=args.det,
        text_recognition_model_name=args.rec,
        use_doc_orientation_classify=False,
        use_doc_unwarping=False,
        use_textline_orientation=False,
        device="cpu",
    )
    engine = "ppocrv5-arabic" if args.det == "PP-OCRv5_server_det" else f"ppocrv5-arabic-{args.det}"
    run = Run(
        engine,
        {
            "det": args.det,
            "rec": args.rec,
            "paddleocr": paddleocr.__version__,
            "paddlepaddle": paddle.__version__,
            "device": "cpu",
            "assembly": "lines by vertical overlap, RTL within Arabic lines, no columns",
        },
    )
    run.load_seconds = time.time() - started

    for sample in load_samples(args.only):
        t0 = time.time()
        result = ocr.predict(str(sample.image))[0]
        boxes = [tuple(map(float, box)) for box in result["rec_boxes"]]
        text = reading_order(boxes, list(result["rec_texts"]))
        run.write(sample, text, time.time() - t0)

    models = CACHE / "paddlex" / "official_models"
    run.finish(sum(dir_size_bytes(models / name) for name in (args.det, args.rec) if (models / name).exists()))


if __name__ == "__main__":
    main()
