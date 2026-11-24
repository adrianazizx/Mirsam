"""Reconcile spike, step 2: try reconcile rules offline on recorded readings (ADR 0033).

    python3 reconcile_sim.py

Readings per page:
  P    Nuqta 4B, whole page (the primary), with word confidences
  S4   Nuqta 4B, line strips at full resolution (reconcile_lines.py)
  S08  Nuqta 0.8B, line strips
  V    Apple Vision, whole page (the platform OCR: third reader for numbers)

Writes one hypothesis directory per rule to spikes/out/ocr-engines/reconcile/<rule>/ and a
flags summary; score each with mirsam-ocr-eval.
"""

from __future__ import annotations

import difflib
import json
import os
import re
import sys
import unicodedata
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
OUT = REPO / "spikes" / "out" / "ocr-engines"
HYP = OUT / "hyp"
VISION = Path(os.environ.get("MIRSAM_VISION_HYP", REPO.parent / "m4-ocr-benchmark-f94419" / "spikes/out/ocr-engines/hyp/apple-vision"))

BIDI = re.compile("[؜‎‏‪-‮⁦-⁩]")
DIGIT = re.compile(r"[0-9٠-٩۰-۹]")
ARABIC = re.compile(r"[؀-ۿ]")
UNIT = re.compile(r"^(\W*)([A-Za-z]+)-([0-9]+)(\W*)$")
UNITS = {"bit", "byte", "core", "inch", "star", "day", "week", "month", "year", "hour", "minute",
         "second", "page", "point", "piece", "part", "step", "digit", "speed", "way", "door", "seat",
         "room", "lane", "track", "layer", "level", "sided", "fold", "storey", "story"}


def norm(w: str) -> str:
    return BIDI.sub("", unicodedata.normalize("NFC", w))


def fix_order(line: str) -> tuple[str, int]:
    """In a right-to-left line, `<unit>-<number>` as read visually is `<number>-<unit>`."""
    if not ARABIC.search(line):
        return line, 0
    out, n = [], 0
    for w in line.split(" "):
        m = UNIT.match(w)
        if m and m.group(2).lower() in UNITS:
            w = f"{m.group(1)}{m.group(3)}-{m.group(2)}{m.group(4)}"
            n += 1
        out.append(w)
    return " ".join(out), n


DIACRITICS = re.compile("[ً-ٰٟۖ-ۭـ]")
TO_ASCII = str.maketrans("٠١٢٣٤٥٦٧٨٩۰۱۲۳۴۵۶۷۸۹", "01234567890123456789")
TO_ARABIC = str.maketrans("0123456789", "٠١٢٣٤٥٦٧٨٩")


def key(w: str) -> str:
    """What a reading says, ignoring how it was written: digits by value, letters
    without tashkeel, outer punctuation dropped."""
    w = DIACRITICS.sub("", norm(w)).translate(TO_ASCII)
    return w.strip(".,،؛:;!?؟()[]\"'“”«»-–—")


def similar(a: str, b: str) -> bool:
    ka, kb = key(a), key(b)
    if not ka or not kb:
        return ka == kb
    return difflib.SequenceMatcher(None, ka, kb, autojunk=False).ratio() >= 0.5


def locate(page: list[str], i: int, other: list[str]) -> str | None:
    """The word of `other` standing where page[i] stands: best match of the context
    (three words either side, nearer ones weigh more), and similar to page[i]."""
    best, best_j = 0.0, None
    for j in range(len(other)):
        score = 0.0
        for d in (-3, -2, -1, 1, 2, 3):
            a, b = i + d, j + d
            if 0 <= a < len(page) and 0 <= b < len(other) and key(page[a]) == key(other[b]):
                score += 1.0 / abs(d)
        if score > best and similar(page[i], other[j]):
            best, best_j = score, j
    if best_j is None or best < 1.0:
        return None
    return other[best_j]


def in_style_of(word: str, model: str) -> str:
    """`word` with its digits written in the script `model` uses."""
    if re.search(r"[٠-٩]", model):
        return word.translate(TO_ARABIC)
    return word.translate(TO_ASCII)


def decide_strips_first(page_word: str, readings: dict[str, str | None]) -> tuple[str, str]:
    """The full-resolution strip reads (s4, s08) weigh more than whole-page reads (q, v):
    - every reading agrees with the page: confirmed;
    - both strip reads agree on another value: corrected to it;
    - one strip read and another reader agree on another value, and no strip read
      backs the page: corrected;
    - otherwise: uncertain (kept, flagged)."""
    pk = key(page_word)
    got = {name: r for name, r in readings.items() if r is not None}
    if not got:
        return "unchecked", page_word
    if all(key(r) == pk for r in got.values()):
        return "confirmed", page_word
    s4, s08 = got.get("s4"), got.get("s08")
    if s4 is not None and s08 is not None and key(s4) == key(s08) != pk:
        return "corrected", in_style_of(s4, page_word)
    strip_backs_page = any(key(got[n]) == pk for n in ("s4", "s08") if n in got)
    if not strip_backs_page:
        for n in ("s4", "s08"):
            if n in got and key(got[n]) != pk:
                support = [m for m in got if m != n and key(got[m]) == key(got[n])]
                if support:
                    return "corrected", in_style_of(got[n], page_word)
    return "uncertain", page_word


def decide(page_word: str, readings: dict[str, str | None]) -> tuple[str, str]:
    """('confirmed'|'corrected'|'uncertain', text)."""
    pk = key(page_word)
    got = {name: r for name, r in readings.items() if r is not None}
    if not got:
        return "unchecked", page_word
    if all(key(r) == pk for r in got.values()):
        return "confirmed", page_word
    if any(key(r) == pk for r in got.values()):
        return "uncertain", page_word
    votes: dict[str, list[str]] = {}
    for name, r in got.items():
        votes.setdefault(key(r), []).append(name)
    best_key, names = max(votes.items(), key=lambda kv: len(kv[1]))
    if len(names) >= 2 and sum(1 for v in votes.values() if len(v) == len(names)) == 1:
        source = "s4" if "s4" in names else names[0]
        return "corrected", in_style_of(got[source], page_word)
    return "uncertain", page_word


def words_of(path: Path) -> list[str]:
    return path.read_text(encoding="utf-8").split() if path.exists() else []


def run() -> None:
    manifest = json.loads((REPO / "ocr-eval/out/manifest.json").read_text())
    variants = {
        "order": dict(targets=None, readers=()),
        "A-strips": dict(targets="digits+lowconf", readers=("s4", "s08")),
        "B-strips+q": dict(targets="digits+lowconf", readers=("s4", "s08", "q")),
        "C-strips+q+v": dict(targets="digits+lowconf", readers=("s4", "s08", "q", "v")),
        "D-all-q-disagree": dict(targets="digits+lowconf+q", readers=("s4", "s08", "q", "v")),
        "E-strips-first": dict(targets="digits+lowconf", readers=("s4", "s08", "q"), rule="strips"),
        "F-strips-first+v": dict(targets="digits+lowconf", readers=("s4", "s08", "q", "v"), rule="strips"),
        "G-strips-first+v-fields": dict(targets="digits+lowconf", readers=("s4", "s08", "q", "v"), rule="strips", v_fields_only=True),
    }
    stats = {}
    for name, cfg in variants.items():
        (OUT / "reconcile" / name).mkdir(parents=True, exist_ok=True)
        st = stats[name] = {"corrected_fix": 0, "corrected_break": 0, "corrected_other": 0,
                            "flag_right": 0, "flag_wrong": 0, "silent_wrong": 0, "words": 0, "wrong": 0}
        for s in manifest["samples"]:
            sid = s["id"]
            page_text = (HYP / "rust-nuqta-4b-q8_0-metal" / f"{sid}.txt").read_text(encoding="utf-8")
            conf = json.loads((HYP / "rust-nuqta-4b-q8_0-metal" / f"{sid}.words.json").read_text())
            strips = json.loads((OUT / "reconcile/lines" / f"{sid}.json").read_text())["lines"]
            s4 = [w for line in strips for w in fix_order(line["4b"])[0].split()]
            s08 = [w for line in strips for w in fix_order(line["0.8b"])[0].split()]
            q = [w for line in words_of(HYP / "nuqta-0.8b-q8_0-metal" / f"{sid}.txt") and
                 (HYP / "nuqta-0.8b-q8_0-metal" / f"{sid}.txt").read_text().splitlines() for w in fix_order(line)[0].split()]
            v = [w for line in ((VISION / f"{sid}.txt").read_text().splitlines() if (VISION / f"{sid}.txt").exists() else [])
                 for w in fix_order(line)[0].split()]
            others = {"s4": s4, "s08": s08, "q": q, "v": v}
            page_lines = [fix_order(line)[0].split(" ") for line in page_text.splitlines()]
            flat = [w for line in page_lines for w in line]
            probs = [p for _, p in conf]
            if len(probs) != len(flat):
                probs = [1.0] * len(flat)
            truths = [(REPO / "ocr-eval/out" / s["truth"]).read_text(encoding="utf-8").split()]
            truths += [(REPO / "ocr-eval/out" / a).read_text(encoding="utf-8").split() for a in s.get("truth_alternatives", [])]
            def right_mask(words):
                best = None
                for truth in truths:
                    m = [False] * len(words)
                    sm = difflib.SequenceMatcher(None, [norm(w) for w in truth], [norm(w) for w in words], autojunk=False)
                    for a, b, n in sm.get_matching_blocks():
                        for k in range(n):
                            m[b + k] = True
                    if best is None or sum(m) > sum(best):
                        best = m
                return best
            right = right_mask(flat)
            new, status = list(flat), ["unchecked"] * len(flat)
            if cfg["targets"]:
                for i, w in enumerate(flat):
                    target = bool(DIGIT.search(w)) or probs[i] < 0.95
                    if not target and "q" in cfg["targets"]:
                        qa = locate(flat, i, q)
                        target = qa is not None and key(qa) != key(w)
                    if not target:
                        continue
                    readers = [r for r in cfg["readers"] if not (r == "v" and cfg.get("v_fields_only") and not DIGIT.search(w))]
                    readings = {r: locate(flat, i, others[r]) for r in readers}
                    rule = decide_strips_first if cfg.get("rule") == "strips" else decide
                    status[i], new[i] = rule(w, readings)
            now_right = right_mask(new)
            for i in range(len(flat)):
                st["words"] += 1
                st["wrong"] += not now_right[i]
                if status[i] == "corrected":
                    if not right[i] and now_right[i]:
                        st["corrected_fix"] += 1
                    elif right[i] and not now_right[i]:
                        st["corrected_break"] += 1
                        print(f"  BROKE {name} {sid[4:]} {flat[i]} -> {new[i]}")
                    else:
                        st["corrected_other"] += 1
                elif status[i] == "uncertain":
                    st["flag_right" if now_right[i] else "flag_wrong"] += 1
                    if not now_right[i] and DIGIT.search(new[i]) and name.startswith(("E", "G")):
                        print(f"  FLAGGED {name} {sid[4:]} {new[i]}")
                if not now_right[i] and status[i] != "uncertain":
                    st["silent_wrong"] += 1
            out, k = [], 0
            for line in page_lines:
                out.append(" ".join(new[k:k + len(line)]))
                k += len(line)
            (OUT / "reconcile" / name / f"{sid}.txt").write_text("\n".join(out) + "\n", encoding="utf-8")
    for name, st in stats.items():
        print(name, st)


if __name__ == "__main__":
    run()
