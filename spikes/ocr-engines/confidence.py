"""Does the model's own confidence flag its errors? (ADR 0030)

    python confidence.py <model>+conf [partner ...]

Reads the per-word confidences written by `run_vlm.py --confidence` (the lowest
token probability in each word). For several thresholds: the share of the model's
wrong words below it (caught) and of its right words below it (false alarms, i.e.
review work). With partners, a word is also flagged when a partner disagrees, which
is spec §8 step 6 (re-read low-confidence lines with a second engine).
"""

from __future__ import annotations

import json
import sys

from agreement import correct_mask, read, words
from common import MANIFEST, OUT

THRESHOLDS = [0.5, 0.8, 0.9, 0.95, 0.99]


def main() -> None:
    engine, partners = sys.argv[1], sys.argv[2:]
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    rows = {(t, p): [0, 0, 0, 0] for t in THRESHOLDS for p in [None, *partners]}  # caught, wrong, alarms, right
    for sample in manifest["samples"]:
        conf = json.loads((OUT / "hyp" / engine / f"{sample['id']}.conf.json").read_text(encoding="utf-8"))
        hyp = [words(word)[0] if words(word) else word for word, _ in conf]
        probs = [p for _, p in conf]
        truths = [words((MANIFEST.parent / t).read_text(encoding="utf-8")) for t in [sample["truth"], *sample.get("truth_alternatives", [])]]
        right = max((correct_mask(t, hyp) for t in truths), key=sum)
        disagree = {p: [not a for a in correct_mask(read(p, sample["id"]), hyp)] for p in partners}
        for t in THRESHOLDS:
            for p in [None, *partners]:
                row = rows[(t, p)]
                for i, ok in enumerate(right):
                    flagged = probs[i] < t or (p is not None and disagree[p][i])
                    if ok:
                        row[3] += 1
                        row[2] += flagged
                    else:
                        row[1] += 1
                        row[0] += flagged
    print("| Flag a word when | caught (of wrong words) | false alarms (of right words) |")
    print("|---|---|---|")
    for p in [None, *partners]:
        for t in THRESHOLDS:
            caught, wrong, alarms, right = rows[(t, p)]
            rule = f"p < {t}" + (f" or {p} disagrees" if p else "")
            print(f"| {rule} | {caught}/{wrong} = {100 * caught / max(1, wrong):.0f}% | {100 * alarms / max(1, right):.1f}% |")


if __name__ == "__main__":
    main()
