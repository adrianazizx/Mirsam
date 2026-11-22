"""Reconcile spike, step 1: cut each page into text-line strips and read every strip
with Nuqta 4B and 0.8B (ADR 0033).

    python reconcile_lines.py [--only <id part>]

A line strip is read at the page's own resolution (crops are not scaled up or down,
per the model card), so glyphs are about twice as large as in the 2.4 MP page read.
Writes spikes/out/ocr-engines/reconcile/lines/<id>.json:
  [{"box": [x0, y0, x1, y1], "4b": text, "0.8b": text}, ...]   top to bottom
"""

from __future__ import annotations

import argparse
import base64
import io
import json
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).parent))
from common import CACHE, OUT, load_samples  # noqa: E402
from run_vlm import NUQTA_PROMPT, nuqta_prepare  # noqa: E402
from to_text import to_text  # noqa: E402

MODELS = CACHE / "models"


def otsu(gray: np.ndarray) -> int:
    hist = np.bincount(gray.ravel(), minlength=256).astype(np.float64)
    total = gray.size
    cum = np.cumsum(hist)
    cum_mean = np.cumsum(hist * np.arange(256))
    best, thr = -1.0, 128
    for t in range(255):
        w0, w1 = cum[t], total - cum[t]
        if w0 == 0 or w1 == 0:
            continue
        m0, m1 = cum_mean[t] / w0, (cum_mean[-1] - cum_mean[t]) / w1
        between = w0 * w1 * (m0 - m1) ** 2
        if between > best:
            best, thr = between, t
    return thr + 1


def skew_angle(ink: np.ndarray) -> float:
    """Degrees to rotate the page (counter-clockwise) so its lines run level: the
    angle whose row profile is sharpest (largest variance), searched over ±4°."""
    ys, xs = np.nonzero(ink)
    if len(xs) == 0:
        return 0.0
    if len(xs) > 400_000:
        pick = np.random.default_rng(0).choice(len(xs), 400_000, replace=False)
        ys, xs = ys[pick], xs[pick]
    h = ink.shape[0]
    best, best_angle = -1.0, 0.0
    for angle in np.arange(-4.0, 4.01, 0.25):
        t = np.tan(np.radians(angle))
        rows = np.round(ys + xs * t).astype(np.int64)
        rows = rows[(rows >= 0) & (rows < h)]
        profile = np.bincount(rows, minlength=h).astype(np.float64)
        score = profile.var()
        if score > best:
            best, best_angle = score, angle
    return float(np.degrees(np.arctan(np.tan(np.radians(best_angle)))))


def box_sum(mask: np.ndarray, r: int) -> np.ndarray:
    """Count of set pixels in the (2r+1)² window around each pixel (integral image)."""
    m = np.pad(mask.astype(np.int32), r + 1)
    c = m.cumsum(0).cumsum(1)
    k = 2 * r + 1
    return c[k:, k:] - c[:-k, k:] - c[k:, :-k] + c[:-k, :-k]


def ink_mask(gray: np.ndarray) -> np.ndarray:
    """Ink, without isolated specks: a pixel stays if its 5x5 window holds at least 9
    ink pixels (strokes do, scattered specks do not)."""
    ink = gray < otsu(gray)
    return ink & (box_sum(ink, 2)[: ink.shape[0], : ink.shape[1]] >= 9)


def line_boxes(image: Image.Image) -> tuple[list[list[int]], Image.Image, float]:
    """Text-line strips from the horizontal ink profile of the deskewed page (no
    detector). Returns the boxes, the deskewed image they refer to, and the angle."""
    gray = np.asarray(image.convert("L"))
    angle = skew_angle(ink_mask(gray))
    if abs(angle) >= 0.25:
        image = image.rotate(-angle, resample=Image.BICUBIC, expand=False, fillcolor=(255, 255, 255))
        gray = np.asarray(image.convert("L"))
    h, w = gray.shape
    ink = ink_mask(gray)
    # Ignore page edges and columns that are mostly ink (scanner shadows).
    density = ink.mean(axis=0)
    usable = density < 0.5
    usable[: int(0.02 * w)] = False
    usable[int(0.98 * w):] = False
    rows = ink[:, usable].sum(axis=1)
    # Specks put a little ink in every row; text rows hold far more.
    # Rows mostly covered in ink are background around the paper or a shadow, not text.
    valid = rows < 0.7 * max(1, int(usable.sum()))
    if not valid.any():
        return [], image, angle
    strong = np.percentile(rows[valid], 95)
    floor = np.percentile(rows[valid], 30)  # rows between lines: specks, margins
    on = valid & (rows > max(2, int(0.002 * w), floor + 0.15 * (strong - floor)))
    bands, y = [], 0
    while y < h:
        if on[y]:
            start = y
            while y < h and on[y]:
                y += 1
            bands.append([start, y])
        else:
            y += 1
    if not bands:
        return [], image, angle
    # Bands of a few specks hold little ink; text lines hold much more.
    mass = [int(rows[b[0]:b[1]].sum()) for b in bands]
    big = np.percentile(mass, 75)
    bands = [b for b, m in zip(bands, mass) if m >= 0.1 * big]
    heights = sorted(b[1] - b[0] for b in bands)
    median = heights[len(heights) // 2]
    bands = [b for b in bands if b[1] - b[0] >= 0.3 * median]
    merged: list[list[int]] = []
    for b in bands:
        if merged and b[0] - merged[-1][1] < 0.35 * median:
            merged[-1][1] = b[1]
        else:
            merged.append(list(b))
    boxes = []
    for y0, y1 in merged:
        counts = ink[y0:y1].sum(axis=0) * usable
        cols = np.nonzero(counts > max(1, 0.03 * (y1 - y0)))[0]
        if len(cols) == 0:
            continue
        lh = y1 - y0
        pad_x, pad_y = int(0.6 * lh), int(0.35 * lh)
        boxes.append([max(0, int(cols.min()) - pad_x), max(0, y0 - pad_y), min(w, int(cols.max()) + 1 + pad_x), min(h, y1 + pad_y)])
    return boxes, image, angle


class Server:
    def __init__(self, size: str, port: int):
        self.port = port
        model = MODELS / f"nuqta-ocr-{size}-Q8_0.gguf"
        mmproj = MODELS / f"nuqta-ocr-{size}-mmproj-f16.gguf"
        self.proc = subprocess.Popen(
            ["llama-server", "-m", str(model), "--mmproj", str(mmproj), "-c", "8192", "--image-max-tokens", "4096",
             "--temp", "0", "--host", "127.0.0.1", "--port", str(port), "--no-webui", "--parallel", "1"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        while True:
            try:
                if urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=2).status == 200:
                    break
            except Exception:
                time.sleep(0.5)

    def read(self, image: Image.Image) -> str:
        buf = io.BytesIO()
        nuqta_prepare(image).save(buf, "PNG")
        body = {
            "messages": [{"role": "user", "content": [
                {"type": "image_url", "image_url": {"url": "data:image/png;base64," + base64.b64encode(buf.getvalue()).decode()}},
                {"type": "text", "text": NUQTA_PROMPT}]}],
            "temperature": 0, "max_tokens": 1024, "chat_template_kwargs": {"enable_thinking": False},
        }
        req = urllib.request.Request(f"http://127.0.0.1:{self.port}/v1/chat/completions", data=json.dumps(body).encode(),
                                     headers={"Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=600) as r:
            return to_text(json.load(r)["choices"][0]["message"]["content"] or "")

    def close(self):
        self.proc.terminate()
        self.proc.wait()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--only")
    args = parser.parse_args()
    out = OUT / "reconcile" / "lines"
    out.mkdir(parents=True, exist_ok=True)
    servers = {"4b": Server("4b", 8801), "0.8b": Server("0.8b", 8802)}
    try:
        for sample in load_samples(args.only):
            image = Image.open(sample.image).convert("RGB")
            boxes, image, angle = line_boxes(image)
            t0 = time.time()
            lines = []
            for box in boxes:
                crop = image.crop(tuple(box))
                lines.append({"box": box, **{k: s.read(crop) for k, s in servers.items()}})
            (out / f"{sample.id}.json").write_text(json.dumps({"skew": angle, "lines": lines}, ensure_ascii=False, indent=0))
            print(f"{time.time() - t0:6.1f} s  {len(boxes):3d} lines  {sample.id}", flush=True)
    finally:
        for s in servers.values():
            s.close()


if __name__ == "__main__":
    main()
