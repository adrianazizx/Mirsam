"""Markup → plain text, the same way for every vision-language model (ADR 0030).

Document VLMs answer in Markdown or HTML. The scorer wants the page's text in
reading order, so markup is removed and nothing else: no character of content is
changed, added or reordered. Table cells keep the order the model wrote them in
(one per line), so a table written in the wrong direction is still scored as a
reading-order error, which it is.

Run `python to_text.py` to check the rules.
"""

from __future__ import annotations

import html
import re

_THINK = re.compile(r"<think>.*?</think>", re.S)
_FENCE = re.compile(r"^```[a-zA-Z]*\s*$", re.M)
_BLOCK_END = re.compile(r"</(p|div|h[1-6]|li|tr|td|th|table|thead|tbody|caption|section)\s*>|<br\s*/?>", re.I)
_TAG = re.compile(r"</?[a-zA-Z][^<>]*>")
_HEADING = re.compile(r"^\s{0,3}#{1,6}\s+", re.M)
_BULLET = re.compile(r"^\s*[-*+•]\s+", re.M)
_EMPHASIS = re.compile(r"(\*\*|__)(.+?)\1", re.S)
_TABLE_RULE = re.compile(r"^\s*\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)*\|?\s*$")


def to_text(raw: str) -> str:
    text = _THINK.sub("", raw)
    text = _FENCE.sub("", text)
    text = _BLOCK_END.sub("\n", text)
    text = _TAG.sub("", text)
    text = html.unescape(text)
    text = _HEADING.sub("", text)
    text = _BULLET.sub("", text)
    text = _EMPHASIS.sub(r"\2", text)
    lines = []
    for line in text.splitlines():
        if _TABLE_RULE.match(line):
            continue
        stripped = line.strip()
        if stripped.startswith("|") and stripped.endswith("|") and len(stripped) > 1:
            lines.extend(cell.strip() for cell in stripped[1:-1].split("|"))
        else:
            lines.append(line.rstrip())
    return "\n".join(line for line in lines if line.strip())


if __name__ == "__main__":
    cases = {
        "# تقرير\n\n**نص** عادي": "تقرير\nنص عادي",
        "<table><tr><td>البند</td><td>Item</td></tr><tr><td>10</td><td>4</td></tr></table>": "البند\nItem\n10\n4",
        "| البند | Item |\n|---|---|\n| 10 | 185.00 |": "البند\nItem\n10\n185.00",
        "1. Term. The lease runs": "1. Term. The lease runs",
        "- bullet\n* other": "bullet\nother",
        "<think>x</think>A &amp; B<br>C": "A & B\nC",
        "```markdown\nنص\n```": "نص",
        "$4,500,000 and 3%": "$4,500,000 and 3%",
    }
    for raw, expected in cases.items():
        got = to_text(raw)
        assert got == expected, (raw, got, expected)
    print(f"{len(cases)} cases pass")
