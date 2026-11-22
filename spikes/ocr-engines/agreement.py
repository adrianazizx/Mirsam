"""Would a second engine help the primary one? (ADR 0030, ensemble partner)

    python agreement.py <primary> <partner> [<partner> ...]

Spec §8 step 6 re-reads low-confidence lines with a second engine and reconciles.
That only pays off if the partner disagrees where the primary is wrong and agrees
where it is right. Per partner, over every page:

- caught: of the primary's wrong words, the share where the partner wrote something else
  (disagreement would flag them for re-reading or review);
- false alarms: of the primary's right words, the share where the partner disagrees;
- partner right: of the reference words the primary missed, the share the partner
  has right (roughly what a perfect reconciler could recover);
- oracle CER: page CER if, per page, the better of the two outputs were kept.

Words are compared after the scorer's strict normalization (NFC, bidi controls
removed, whitespace collapsed). Word alignment is difflib's, a spike-grade proxy for
the scorer's Levenshtein alignment.
"""

from __future__ import annotations

import difflib
import json
import re
import sys
import unicodedata

from common import MANIFEST, OUT

BIDI = re.compile("[؜‎‏‪-‮⁦-⁩]")


def words(text: str) -> list[str]:
    return BIDI.sub("", unicodedata.normalize("NFC", text)).split()


def correct_mask(reference: list[str], hypothesis: list[str]) -> list[bool]:
    """For each hypothesis word: does it sit in a block that matches the reference?"""
    mask = [False] * len(hypothesis)
    for block in difflib.SequenceMatcher(None, reference, hypothesis, autojunk=False).get_matching_blocks():
        for j in range(block.b, block.b + block.size):
            mask[j] = True
    return mask


def read(engine: str, sample_id: str) -> list[str]:
    path = OUT / "hyp" / engine / f"{sample_id}.txt"
    return words(path.read_text(encoding="utf-8")) if path.exists() else []


def main() -> None:
    primary, partners = sys.argv[1], sys.argv[2:]
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    reports = {e: json.loads((OUT / "reports" / f"{e}.json").read_text()) for e in [primary, *partners]}

    print("| Partner | primary's wrong words | caught | partner right | false alarms | oracle CER (all) |")
    print("|---|---|---|---|---|---|")
    for partner in partners:
        wrong = caught = recoverable = missed_total = right = false_alarm = 0
        oracle_errors = reference = 0
        for sample in manifest["samples"]:
            truths = [sample["truth"], *sample.get("truth_alternatives", [])]
            truth_words = [words((MANIFEST.parent / t).read_text(encoding="utf-8")) for t in truths]
            p, q = read(primary, sample["id"]), read(partner, sample["id"])
            # The truth the primary's output is closest to (the scorer keeps the best one too).
            t = max(truth_words, key=lambda tw: difflib.SequenceMatcher(None, tw, p, autojunk=False).ratio())
            p_right = correct_mask(t, p)
            agrees = correct_mask(q, p)
            q_right_words = {w for w, ok in zip(q, correct_mask(t, q)) if ok}
            for word, ok, agree in zip(p, p_right, agrees):
                if ok:
                    right += 1
                    false_alarm += not agree
                else:
                    wrong += 1
                    caught += not agree
            # Words of the truth the primary missed but the partner has right, in order.
            missed = [w for w, ok in zip(t, correct_mask(p, t)) if not ok]
            recoverable += sum(1 for w in missed if w in q_right_words)
            missed_total += len(missed)
            sp, sq = reports[primary]["samples"][sample["id"]], reports[partner]["samples"][sample["id"]]
            oracle_errors += min(sp["cer"]["errors"], sq["cer"]["errors"])
            reference += sp["cer"]["reference"]
        pct = lambda a, b: f"{100 * a / b:.1f}%" if b else "–"  # noqa: E731
        print(
            f"| {partner} | {wrong} | {pct(caught, wrong)} | {pct(recoverable, missed_total)} | "
            f"{pct(false_alarm, right)} | {100 * oracle_errors / reference:.2f}% |"
        )


if __name__ == "__main__":
    main()
