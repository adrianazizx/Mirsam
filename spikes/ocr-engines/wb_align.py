"""Word-box spike, step 2: align a VLM's text to detector lines, split lines into words, score the boxes.

    python wb_align.py <engine hyp dir name> [--verbose]

Inputs: spikes/out/ocr-engines/hyp/<engine>/<id>.txt (VLM text, no positions),
        wb/detect/<id>.json (wb_detect.py: PP-OCRv5 detector lines + the Arabic recogniser's text),
        word-boxes-truth.json (word_boxes_truth.mjs: Chrome's word rectangles).

Methods (each puts every VLM word into one detected line, then splits the line's ink extent):
  M0  proportional    lines by width, from the VLM text alone: no recogniser, no anchors
  M1  anchored        lines by character alignment of the VLM text to the recogniser's text
  M2  anchored+snap   M1, then every word boundary snapped to a real gap in the line's ink profile
  M3  width-fit+snap  no recogniser: cut the word sequence into one run per line by a dynamic
                      program that fits estimated word widths to line widths; then snap as M2
  M4  M3 + anchors    the same program, with the recogniser's text pinning words to lines
  M5  auto            M2 when the recogniser's text matches the VLM's well (>= 50% of characters), else M4

Words sit in a line in visual order: UAX#9 at word granularity (visual_order). A line's base
direction comes from the column margin it touches, else its first strong letter.

Scores, over the VLM words that equal a ground-truth word (the pairing is order-preserving):
  line      the assigned line contains the true word's centre
  IoU-x     horizontal overlap with the ink-tight true box (what selection and highlight need)
  IoU       full box overlap with the ink-tight true box
"""

from __future__ import annotations

import argparse
import difflib
import json
import os
import re
import statistics
import unicodedata

import numpy as np
from PIL import Image

from common import OUT, load_samples

ANCHOR_WEIGHT = float(os.environ.get("WB_ANCHOR_WEIGHT", "0.6"))
ARABIC = re.compile(r"[؀-ۿݐ-ݿࢠ-ࣿ]")
LATIN = re.compile(r"[A-Za-z]")


def norm(text: str) -> str:
    text = "".join(c for c in unicodedata.normalize("NFKD", text) if unicodedata.category(c) != "Mn" and c != "ـ")
    text = text.translate(str.maketrans("٠١٢٣٤٥٦٧٨٩۰۱۲۳۴۵۶۷۸۹", "01234567890123456789"))
    return text.lower()


def weight(word: str) -> float:
    w = 0.0
    for c in word:
        if ARABIC.match(c):
            w += 1.0
        elif c.isdigit():
            w += 0.95
        elif c.isalpha():
            w += 1.0 if c.isupper() else 0.85
        else:
            w += 0.45
    return max(w, 0.5)


# ---------- ink ----------

def otsu(gray: np.ndarray) -> float:
    hist, _ = np.histogram(gray, bins=256, range=(0, 256))
    total = gray.size
    sum_all = np.dot(np.arange(256), hist)
    best, bw, sb = 0.0, 0, 0.0
    thr = 128
    for t in range(256):
        bw += hist[t]
        if bw == 0 or bw == total:
            continue
        sb += t * hist[t]
        mf = sb / bw
        mb = (sum_all - sb) / (total - bw)
        between = bw * (total - bw) * (mf - mb) ** 2
        if between > best:
            best, thr = between, t
    return thr


class Page:
    def __init__(self, path):
        gray = np.asarray(Image.open(path).convert("L"), dtype=np.uint8)
        self.ink = gray < otsu(gray)

    def extent(self, box, pad=0):
        """Tight ink bbox inside box (x0, y0, x1, y1), or None."""
        x0, y0, x1, y1 = [int(round(v)) for v in box]
        x0, y0 = max(0, x0 - pad), max(0, y0 - pad)
        crop = self.ink[y0:y1 + pad, x0:x1 + pad]
        ys, xs = np.nonzero(crop)
        if len(xs) == 0:
            return None
        return [x0 + xs.min(), y0 + ys.min(), x0 + xs.max() + 1, y0 + ys.max() + 1]


# ---------- bidi order ----------

def para_dir(words: list[str]) -> str:
    for w in words:
        for c in w:
            if ARABIC.match(c):
                return "rtl"
            if LATIN.match(c):
                return "ltr"
    return "rtl"


def visual_order(words: list[str], base: str) -> list[int]:
    """Indices of `words` (logical order) in left-to-right visual order: UAX#9 at word granularity.

    Each word gets one bidi type from its letters (Arabic R, Latin L, digits EN, else neutral),
    then the weak rule W7 (EN after L is L), neutral rule N1/N2, implicit levels I1/I2, and the
    reordering rule L2 (reverse every run at level >= k, from the highest level down to 1).
    """
    types = []
    for w in words:
        if ARABIC.search(w):
            types.append("R")
        elif LATIN.search(w):
            types.append("L")
        elif any(c.isdigit() for c in w):
            types.append("EN")
        else:
            types.append("N")
    base_t = "R" if base == "rtl" else "L"
    for i, t in enumerate(types):  # W7
        if t == "EN":
            prev = next((types[j] for j in range(i - 1, -1, -1) if types[j] in ("L", "R")), base_t)
            if prev == "L":
                types[i] = "L"
    strong = lambda t: "R" if t in ("R", "EN") else t  # numbers count as R for neutrals (N1)
    resolved = list(types)
    for i, t in enumerate(types):
        if t == "N":
            left = next((strong(types[j]) for j in range(i - 1, -1, -1) if types[j] != "N"), base_t)
            right = next((strong(types[j]) for j in range(i + 1, len(types)) if types[j] != "N"), base_t)
            resolved[i] = left if left == right else base_t
    base_level = 1 if base == "rtl" else 0
    levels = []
    for t in resolved:
        if base_level == 0:
            levels.append({"L": 0, "R": 1, "EN": 2}[t])
        else:
            levels.append({"L": 2, "R": 1, "EN": 2}[t])
    # the space between two words is a neutral too: it can break a run (e.g. "+966 11 234")
    def level_of(t):
        return {"L": 0, "R": 1, "EN": 2}[t] if base_level == 0 else {"L": 2, "R": 1, "EN": 2}[t]

    tokens = []  # (level, word index or None for a space)
    for i in range(len(words)):
        if i:
            left = next((strong(types[j]) for j in range(i - 1, -1, -1) if types[j] != "N"), base_t)
            right = next((strong(types[j]) for j in range(i, len(types)) if types[j] != "N"), base_t)
            tokens.append((level_of(left if left == right else base_t), None))
        tokens.append((levels[i], i))
    for k in range(max((t[0] for t in tokens), default=0), 0, -1):
        i = 0
        while i < len(tokens):
            if tokens[i][0] >= k:
                j = i
                while j < len(tokens) and tokens[j][0] >= k:
                    j += 1
                tokens[i:j] = tokens[i:j][::-1]
                i = j
            else:
                i += 1
    order = [t[1] for t in tokens if t[1] is not None]
    return order


# ---------- lines ----------

def rows(boxes):
    """Group boxes into visual rows by vertical overlap; returns list of lists of box indices."""
    order = sorted(range(len(boxes)), key=lambda i: (boxes[i][1] + boxes[i][3]) / 2)
    out = []
    for i in order:
        b = boxes[i]
        if out:
            top = min(boxes[j][1] for j in out[-1])
            bottom = max(boxes[j][3] for j in out[-1])
            overlap = min(b[3], bottom) - max(b[1], top)
            if overlap > 0.5 * min(b[3] - b[1], bottom - top):
                out[-1].append(i)
                continue
        out.append([i])
    return out


def segment_orders(boxes, page_w):
    """Candidate reading orders of the detector's boxes: by rows, or by columns."""
    cands = {}
    for rtl in (True, False):
        cands[f"rows-{'rtl' if rtl else 'ltr'}"] = [j for r in rows(boxes) for j in sorted(r, key=lambda i: boxes[i][0], reverse=rtl)]
    # columns: cluster x-centres; only when there is a wide empty vertical band
    centres = sorted(((b[0] + b[2]) / 2, i) for i, b in enumerate(boxes))
    cols = [[centres[0][1]]]
    for (c0, _), (c1, i) in zip(centres, centres[1:]):
        if c1 - c0 > 0.2 * page_w:
            cols.append([i])
        else:
            cols[-1].append(i)
    if len(cols) == 2:
        for first in (0, 1):
            ordered = cols if first == 0 else cols[::-1]
            cands[f"cols-{'ltr' if first == 0 else 'rtl'}"] = [j for col in ordered for j in sorted(col, key=lambda i: boxes[i][1])]
    return cands


# ---------- assignment ----------

def assign_by_width(words, seg_boxes):
    """M0: words to segments by cumulative width, from nothing but the text."""
    widths = [max(1, b[2] - b[0]) for b in seg_boxes]
    total_w, total_c = sum(widths), sum(weight(w) + 0.5 for w in words)
    out, seg, used = [], 0, 0.0
    for w in words:
        c = weight(w) + 0.5
        cap = widths[seg] / total_w * total_c
        if used >= cap * 0.92 and seg < len(seg_boxes) - 1:
            seg, used = seg + 1, 0.0
        out.append(seg)
        used += c
    return out


def assign_by_guide(words, seg_boxes, seg_texts):
    """M1: align the VLM's characters to the recogniser's, vote per word, fill gaps by capacity."""
    v_chars, v_word = [], []
    for wi, w in enumerate(words):
        for c in norm(w):
            if not c.isspace():
                v_chars.append(c)
                v_word.append(wi)
    g_chars, g_seg = [], []
    for si, t in enumerate(seg_texts):
        for c in norm(t):
            if not c.isspace():
                g_chars.append(c)
                g_seg.append(si)
    matcher = difflib.SequenceMatcher(None, v_chars, g_chars, autojunk=False)
    votes = [dict() for _ in words]
    matched = 0
    for a, b, n in matcher.get_matching_blocks():
        for k in range(n):
            matched += 1
            d = votes[v_word[a + k]]
            d[g_seg[b + k]] = d.get(g_seg[b + k], 0) + 1
    seg_of = [max(d, key=d.get) if d else None for d in votes]
    anchors = list(seg_of)
    # enforce monotone (non-decreasing) segments among the anchors
    last = 0
    for i, s in enumerate(seg_of):
        if s is None:
            continue
        if s < last:
            seg_of[i] = None
        else:
            last = s
    widths = [max(1, b[2] - b[0]) for b in seg_boxes]
    cw = sum(widths) / max(1.0, sum(weight(w) + 0.5 for w in words))
    cap = [wd / cw for wd in widths]
    used = [0.0] * len(seg_boxes)
    for i, s in enumerate(seg_of):
        if s is not None:
            used[s] += weight(words[i]) + 0.5
    i = 0
    prev = 0
    while i < len(words):
        if seg_of[i] is not None:
            prev = seg_of[i]
            i += 1
            continue
        j = i
        while j < len(words) and seg_of[j] is None:
            j += 1
        nxt = seg_of[j] if j < len(words) else len(seg_boxes) - 1
        cur = prev
        for k in range(i, j):
            c = weight(words[k]) + 0.5
            if cur < nxt and used[cur] >= cap[cur] * 0.92:
                cur += 1
            seg_of[k] = cur
            used[cur] += c
        i = j
    return seg_of, matched / max(1, len(v_chars)), anchors



def assign_by_dp(words, para_ids, seg_boxes, anchors=None):
    """M3/M4: cut the word sequence into one run per line so that each run's estimated width fits
    its line (a global px-per-weight scale, under-filled lines cost less than overfilled ones;
    a line never spans two paragraphs). With `anchors` (M4), words the recogniser's text pins to a
    line pay a penalty for leaving it. Returns (segment index per word, total cost)."""
    n, m = len(words), len(seg_boxes)
    ws = [weight(w) for w in words]
    L = [max(1.0, b[2] - b[0]) for b in seg_boxes]
    pre = [0.0]
    for w in ws:
        pre.append(pre[-1] + w)
    gap = 0.5

    def width_units(c, d):
        return pre[d] - pre[c] + gap * max(0, d - c - 1)

    scale = sum(L) / max(1e-9, width_units(0, n) * 1.0 + 1e-9)
    INF = 1e18
    for _ in range(3):
        f = [[INF] * (n + 1) for _ in range(m + 1)]
        back = [[0] * (n + 1) for _ in range(m + 1)]
        f[0][0] = 0.0
        for i in range(1, m + 1):
            li = L[i - 1]
            for d in range(n + 1):
                best, arg = INF, 0
                for c in range(d + 1):
                    if f[i - 1][c] >= INF:
                        continue
                    if c < d and para_ids[c] != para_ids[d - 1]:
                        continue
                    if c == d:
                        cost = 2.0
                    else:
                        est = scale * width_units(c, d)
                        r = (est - li) / li
                        cost = 4 * r * r if r > 0 else 0.5 * r * r
                        if anchors:
                            cost += ANCHOR_WEIGHT * sum(1 for k in range(c, d) if anchors[k] is not None and anchors[k] != i - 1)
                    if f[i - 1][c] + cost < best:
                        best, arg = f[i - 1][c] + cost, c
                f[i][d], back[i][d] = best, arg
        cuts, d = [n], n
        for i in range(m, 0, -1):
            d = back[i][d]
            cuts.append(d)
        cuts = cuts[::-1]
        out = []
        for i in range(m):
            out += [i] * (cuts[i + 1] - cuts[i])
        # re-estimate the scale from the lines that got a word, ignoring each paragraph's last line
        ratios = [L[i] / width_units(cuts[i], cuts[i + 1]) for i in range(m)
                  if cuts[i + 1] - cuts[i] >= 2 and (cuts[i + 1] == n or para_ids[cuts[i + 1] - 1] == para_ids[cuts[i + 1]])]
        if ratios:
            scale = statistics.median(ratios)
    return out, f[m][n]


def line_directions(seg_boxes, words_by_seg, page_w):
    """Per segment: 'rtl' or 'ltr' from alignment to its column's margins, else its first strong letter."""
    out = []
    for i, b in enumerate(seg_boxes):
        mates = [o for o in seg_boxes if min(o[2], b[2]) - max(o[0], b[0]) > 0.3 * min(o[2] - o[0], b[2] - b[0])]
        right, left = max(o[2] for o in mates), min(o[0] for o in mates)
        tol = 0.02 * page_w
        tr, tl = b[2] >= right - tol, b[0] <= left + tol
        if tr and not tl:
            out.append("rtl")
        elif tl and not tr:
            out.append("ltr")
        else:
            out.append(para_dir(words_by_seg.get(i, [])))
    return out


# ---------- splitting ----------

def gaps_in(profile, x0, height):
    """Gaps (centre, width) between ink runs of a column profile, inside [x0, x0+len)."""
    thresh = max(1, int(0.02 * height))
    empty = profile < thresh
    gaps, i, n = [], 0, len(profile)
    while i < n:
        if empty[i]:
            j = i
            while j < n and empty[j]:
                j += 1
            if i > 0 and j < n:  # interior only
                gaps.append((x0 + (i + j) / 2, j - i))
            i = j
        else:
            i += 1
    return gaps


def snap_boundaries(expected, gaps, length, height):
    """Choose one gap per expected boundary (increasing), or keep the expected position."""
    if not expected:
        return []
    wref = 0.3 * height
    k, g = len(expected), len(gaps)
    INF = 1e18
    # dp[j][last+1]: best cost for first j boundaries, last chosen gap index `last` (-1 none)
    dp = [[INF] * (g + 1) for _ in range(k + 1)]
    back = [[None] * (g + 1) for _ in range(k + 1)]
    dp[0][0] = 0.0
    for j in range(k):
        for last in range(-1, g):
            cur = dp[j][last + 1]
            if cur >= INF:
                continue
            # keep the proportional position
            if cur + 1.0 < dp[j + 1][last + 1]:
                dp[j + 1][last + 1] = cur + 1.0
                back[j + 1][last + 1] = (last, None)
            for q in range(last + 1, g):
                cx, w = gaps[q]
                cost = 60 * ((cx - expected[j]) / length) ** 2 + 0.4 * (1 - min(1.0, w / wref))
                if cur + cost < dp[j + 1][q + 1]:
                    dp[j + 1][q + 1] = cur + cost
                    back[j + 1][q + 1] = (last, q)
    end = min(range(g + 1), key=lambda s: dp[k][s])
    out = [None] * k
    state = end
    for j in range(k, 0, -1):
        last, q = back[j][state]
        out[j - 1] = gaps[q][0] if q is not None else expected[j - 1]
        state = last + 1
    return sorted(out)


def split_line(page, seg_box, words, base, snap):
    """Boxes [x0,y0,x1,y1] for `words` (logical order) laid out in the segment, visual order via bidi."""
    ext = page.extent(seg_box, pad=2) or seg_box
    x0, y0, x1, y1 = ext
    order = visual_order(words, base)  # left-to-right
    ws = [weight(words[i]) for i in order]
    gap_w = 0.5
    total = sum(ws) + gap_w * (len(ws) - 1)
    L = x1 - x0
    # boundary j sits in the middle of the space between word j and j+1
    cum, expected = 0.0, []
    for j in range(len(ws) - 1):
        cum += ws[j]
        expected.append(x0 + L * (cum + gap_w / 2) / total)
        cum += gap_w
    bounds = expected
    if snap and expected:
        profile = page.ink[int(y0):int(y1), int(x0):int(x1)].sum(axis=0)
        bounds = snap_boundaries(expected, gaps_in(profile, x0, y1 - y0), L, y1 - y0)
    edges = [x0] + list(bounds) + [x1]
    out = {}
    for j, wi in enumerate(order):
        bx0, bx1 = edges[j], edges[j + 1]
        box = [bx0, y0, bx1, y1]
        tight = page.extent(box)
        out[wi] = tight or box
    return out


# ---------- scoring ----------

def iou_1d(a, b):
    inter = max(0.0, min(a[2], b[2]) - max(a[0], b[0]))
    union = (a[2] - a[0]) + (b[2] - b[0]) - inter
    return inter / union if union > 0 else 0.0


def iou_2d(a, b):
    iw = max(0.0, min(a[2], b[2]) - max(a[0], b[0]))
    ih = max(0.0, min(a[3], b[3]) - max(a[1], b[1]))
    inter = iw * ih
    union = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - inter
    return inter / union if union > 0 else 0.0


def pair_words(vlm: list[str], truth: list[dict], page_w: int):
    """Order-preserving pairs (vlm index, truth index) of words that are equal after normalisation."""
    best = []
    mid = page_w / 2
    cands = [list(range(len(truth)))]
    right = [i for i, t in enumerate(truth) if (t["box"][0] + t["box"][2]) / 2 >= mid]
    left = [i for i, t in enumerate(truth) if (t["box"][0] + t["box"][2]) / 2 < mid]
    cands += [right + left, left + right]
    for order in cands:
        a = [norm(w) for w in vlm]
        b = [norm(truth[i]["text"]) for i in order]
        sm = difflib.SequenceMatcher(None, a, b, autojunk=False)
        pairs = [(x + k, order[y + k]) for x, y, n in sm.get_matching_blocks() for k in range(n)]
        if len(pairs) > len(best):
            best = pairs
    return best


def run(engine: str, verbose: bool) -> dict:
    truth = json.loads((OUT / "word-boxes-truth.json").read_text())
    hyp_dir = OUT / "hyp" / engine
    results = {m: [] for m in ("M0", "M1", "M2", "M3", "M4", "M5")}
    per_page = []
    det_recall = []
    for sample in load_samples():
        det_file = OUT / "wb" / "detect" / f"{sample.id}.json"
        hyp_file = hyp_dir / f"{sample.id}.txt"
        if sample.id not in truth or not det_file.exists() or not hyp_file.exists():
            continue
        det = json.loads(det_file.read_text())
        page = Page(sample.image)
        h, w = page.ink.shape
        t_words = truth[sample.id]
        # ink-tight true boxes
        for t in t_words:
            t["tight"] = page.extent(t["box"]) or t["box"]
        paras = [line.split() for line in hyp_file.read_text(encoding="utf-8").splitlines() if line.strip()]
        words = [(wd, pi) for pi, p in enumerate(paras) for wd in p]
        vlm = [wd for wd, _ in words]
        dirs = {pi: para_dir(p) for pi, p in enumerate(paras)}
        seg = det["lines"]
        boxes = [s["box"] for s in seg]
        texts = [s["text"] for s in seg]
        if not boxes or not vlm:
            continue
        # detector recall: true word centres inside some detected box
        hit = sum(
            any(b[0] <= (t["tight"][0] + t["tight"][2]) / 2 <= b[2] and b[1] <= (t["tight"][1] + t["tight"][3]) / 2 <= b[3] for b in boxes)
            for t in t_words
        )
        det_recall.append((sample.id, hit, len(t_words)))
        pairs = pair_words(vlm, t_words, w)
        para_ids = [pi for _, pi in words]
        para_ids = [pi for _, pi in words]
        # candidate segment orders: the guide's character match picks one for M0-M2, M4, M5;
        # the width-fit cost picks one for M3
        best_g = best_w = None
        for name, order in segment_orders(boxes, w).items():
            sb_ = [boxes[i] for i in order]
            seg_g, ratio, anchors_g = assign_by_guide(vlm, sb_, [texts[i] for i in order])
            if best_g is None or ratio > best_g[0]:
                best_g = (ratio, name, order, seg_g, anchors_g)
            seg_w, cost = assign_by_dp(vlm, para_ids, sb_)
            if best_w is None or cost < best_w[0]:
                best_w = (cost, name, order, seg_w)
        ratio, order_name, order, m1, anchors = best_g
        sb = [boxes[i] for i in order]
        sb_main = sb
        m0 = assign_by_width(vlm, sb)
        m4, _ = assign_by_dp(vlm, para_ids, sb, anchors)
        m5 = m1 if ratio >= 0.5 else m4
        sb_w = [boxes[i] for i in best_w[2]]
        page_stats = {"id": sample.id, "words": len(vlm), "paired": len(pairs), "order": order_name, "guide_match": round(ratio, 3)}
        for method, segs, snap in (("M0", m0, False), ("M1", m1, False), ("M2", m1, True),
                                   ("M3", best_w[3], True), ("M4", m4, True), ("M5", m5, True)):
            sb = sb_w if method == "M3" else sb_main
            laid = {}
            by_seg = {}
            for wi, s in enumerate(segs):
                by_seg.setdefault(s, []).append(wi)
            line_dir = line_directions(sb, {k: [vlm[i] for i in v] for k, v in by_seg.items()}, w)
            for s, idx in by_seg.items():
                boxes_s = split_line(page, sb[s], [vlm[i] for i in idx], line_dir[s], snap)
                for local, wi in enumerate(idx):
                    laid[wi] = (boxes_s[local], sb[s])
            line_ok = 0
            vals1, vals2, misses = [], [], []
            for vi, ti in pairs:
                box, seg_box = laid[vi]
                t = t_words[ti]["tight"]
                cx, cy = (t[0] + t[2]) / 2, (t[1] + t[3]) / 2
                pad = 0.1 * (seg_box[3] - seg_box[1])
                line_ok += seg_box[0] - pad <= cx <= seg_box[2] + pad and seg_box[1] - pad <= cy <= seg_box[3] + pad
                a, b = iou_1d(box, t), iou_2d(box, t)
                vals1.append(a)
                vals2.append(b)
                if a < 0.5:
                    misses.append([vlm[vi], round(a, 2)])
            results[method].append((sample.id, len(pairs), line_ok, vals1, vals2))
            page_stats[method] = {
                "line": round(line_ok / max(1, len(pairs)), 3),
                "iou_x": round(statistics.mean(vals1), 3) if vals1 else None,
                "misses": misses,
            }
        per_page.append(page_stats)
        if verbose:
            print(page_stats)
    summary = {}
    for method, rs in results.items():
        pairs = sum(r[1] for r in rs)
        v1 = [v for r in rs for v in r[3]]
        v2 = [v for r in rs for v in r[4]]
        summary[method] = {
            "pairs": pairs,
            "line_correct": round(sum(r[2] for r in rs) / pairs, 4),
            "iou_x_mean": round(statistics.mean(v1), 4),
            "iou_x_ge_0.5": round(sum(v >= 0.5 for v in v1) / len(v1), 4),
            "iou_x_ge_0.7": round(sum(v >= 0.7 for v in v1) / len(v1), 4),
            "iou_mean": round(statistics.mean(v2), 4),
            "iou_ge_0.5": round(sum(v >= 0.5 for v in v2) / len(v2), 4),
        }
    hits = sum(h for _, h, _ in det_recall)
    total = sum(n for _, _, n in det_recall)
    return {
        "engine": engine,
        "pages": len(per_page),
        "detector_word_centre_recall": round(hits / total, 4),
        "summary": summary,
        "per_page": per_page,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("engine")
    parser.add_argument("--verbose", action="store_true")
    args = parser.parse_args()
    report = run(args.engine, args.verbose)
    out = OUT / "wb" / f"align-{args.engine}.json"
    out.write_text(json.dumps(report, indent=2, ensure_ascii=False))
    print(json.dumps({k: v for k, v in report.items() if k != "per_page"}, indent=2))


if __name__ == "__main__":
    main()
