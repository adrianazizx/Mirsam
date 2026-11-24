//! A model's answer → the page's plain text, and per-word confidence.
//!
//! Markup is removed and nothing else: no character of content is changed, added
//! or reordered (the rules of `spikes/ocr-engines/to_text.py`, which ADR 0030's
//! scores were measured with). Table cells keep the order the model wrote them in.

use std::sync::LazyLock;

use regex::Regex;

use super::WordConfidence;

struct Patterns {
    think: Regex,
    fence: Regex,
    block_end: Regex,
    tag: Regex,
    heading: Regex,
    bullet: Regex,
    emphasis: Regex,
    table_rule: Regex,
}

impl Patterns {
    fn new() -> Result<Self, regex::Error> {
        Ok(Self {
            think: Regex::new(r"(?s)<think>.*?</think>")?,
            fence: Regex::new(r"(?m)^```[a-zA-Z]*\s*$")?,
            block_end: Regex::new(
                r"(?i)</(p|div|h[1-6]|li|tr|td|th|table|thead|tbody|caption|section)\s*>|<br\s*/?>",
            )?,
            tag: Regex::new(r"</?[a-zA-Z][^<>]*>")?,
            heading: Regex::new(r"(?m)^\s{0,3}#{1,6}\s+")?,
            bullet: Regex::new(r"(?m)^\s*[-*+•]\s+")?,
            emphasis: Regex::new(r"(?s)\*\*(.+?)\*\*|__(.+?)__")?,
            table_rule: Regex::new(r"^\s*\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)*\|?\s*$")?,
        })
    }
}

/// Built once. The patterns are constants (a test checks they compile); if they
/// somehow did not, answers pass through unchanged rather than panicking.
static PATTERNS: LazyLock<Option<Patterns>> = LazyLock::new(|| Patterns::new().ok());

/// Removes an empty or filled `<think>…</think>` block (Qwen-family models may
/// begin their answer with one).
pub fn strip_think(raw: &str) -> String {
    match &*PATTERNS {
        Some(p) => p.think.replace_all(raw, "").into_owned(),
        None => raw.to_owned(),
    }
}

/// The answer as plain text in the model's reading order.
pub fn to_text(raw: &str) -> String {
    let Some(p) = &*PATTERNS else { return raw.trim().to_owned() };
    let text = p.think.replace_all(raw, "");
    let text = p.fence.replace_all(&text, "");
    let text = p.block_end.replace_all(&text, "\n");
    let text = p.tag.replace_all(&text, "");
    let text = unescape_html(&text);
    let text = p.heading.replace_all(&text, "");
    let text = p.bullet.replace_all(&text, "");
    let text = p.emphasis.replace_all(&text, |caps: &regex::Captures| {
        caps.get(1).or_else(|| caps.get(2)).map_or(String::new(), |m| m.as_str().to_owned())
    });
    let mut lines = Vec::new();
    for line in text.lines() {
        if p.table_rule.is_match(line) {
            continue;
        }
        let stripped = line.trim();
        if stripped.len() > 1 && stripped.starts_with('|') && stripped.ends_with('|') {
            lines.extend(stripped[1..stripped.len() - 1].split('|').map(|cell| cell.trim().to_owned()));
        } else {
            lines.push(line.trim_end().to_owned());
        }
    }
    lines.retain(|line| !line.trim().is_empty());
    lines.join("\n")
}

/// HTML character references: the named ones a model writes, and numeric ones.
fn unescape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let end = rest[1..].find(';').map(|i| i + 1).filter(|&i| i <= 10);
        let decoded = end.and_then(|end| {
            let name = &rest[1..end];
            let ch = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                _ => name
                    .strip_prefix("#x")
                    .or_else(|| name.strip_prefix("#X"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| name.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                    .and_then(char::from_u32),
            };
            ch.map(|ch| (ch, end))
        });
        match decoded {
            Some((ch, end)) => {
                out.push(ch);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// One generated token: its bytes (a token can end inside a UTF-8 character)
/// and its probability.
#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub bytes: Vec<u8>,
    pub probability: f32,
}

/// For each whitespace-separated word of the decoded answer: the lowest
/// probability of any token that contributed a byte to it. A word is only as
/// certain as its least certain piece.
pub fn word_confidences(tokens: &[Token]) -> Vec<WordConfidence> {
    let mut bytes = Vec::new();
    let mut owner = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        bytes.extend_from_slice(&token.bytes);
        owner.extend(std::iter::repeat_n(index, token.bytes.len()));
    }
    let text = String::from_utf8_lossy(&bytes);
    // Lossy decoding only replaces invalid bytes; map each char back to its byte
    // span when the bytes were valid, otherwise fall back to the token of its start.
    let valid = std::str::from_utf8(&bytes).is_ok();
    let mut words = Vec::new();
    let mut current = String::new();
    let mut low = 1.0f32;
    for (offset, ch) in text.char_indices() {
        if ch.is_whitespace() {
            if !current.is_empty() {
                words.push(WordConfidence { word: std::mem::take(&mut current), probability: low });
            }
            low = 1.0;
            continue;
        }
        current.push(ch);
        let span = if valid {
            offset..offset + ch.len_utf8()
        } else {
            offset.min(owner.len())..offset.min(owner.len())
        };
        for &token in owner.get(span).unwrap_or_default() {
            low = low.min(tokens[token].probability);
        }
    }
    if !current.is_empty() {
        words.push(WordConfidence { word: current, probability: low });
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_patterns_compile() {
        assert!(Patterns::new().is_ok());
    }

    #[test]
    fn markup_rules_match_the_benchmark() {
        // The cases of spikes/ocr-engines/to_text.py.
        let cases = [
            ("# تقرير\n\n**نص** عادي", "تقرير\nنص عادي"),
            (
                "<table><tr><td>البند</td><td>Item</td></tr><tr><td>10</td><td>4</td></tr></table>",
                "البند\nItem\n10\n4",
            ),
            ("| البند | Item |\n|---|---|\n| 10 | 185.00 |", "البند\nItem\n10\n185.00"),
            ("1. Term. The lease runs", "1. Term. The lease runs"),
            ("- bullet\n* other", "bullet\nother"),
            ("<think>x</think>A &amp; B<br>C", "A & B\nC"),
            ("```markdown\nنص\n```", "نص"),
            ("$4,500,000 and 3%", "$4,500,000 and 3%"),
        ];
        for (raw, expected) in cases {
            assert_eq!(to_text(raw), expected, "{raw:?}");
        }
    }

    #[test]
    fn plain_arabic_is_untouched() {
        let raw = "<think>\n\n</think>\n\nرقم الهوية ١٠٢٣٤٥٦٧٨٩، والمبلغ 13,500.75 ريال.";
        assert_eq!(to_text(raw), "رقم الهوية ١٠٢٣٤٥٦٧٨٩، والمبلغ 13,500.75 ريال.");
    }

    #[test]
    fn unescapes_numeric_references_and_leaves_stray_ampersands() {
        assert_eq!(unescape_html("&#1604;&#x627; R&D &unknown;"), "لا R&D &unknown;");
    }

    #[test]
    fn word_confidence_is_the_weakest_token_even_inside_a_character() {
        // "ال" + "سلام" + " " + "عل" + "يكم"; the last Arabic letter of the second
        // word is split across two tokens (bytes 0xD9 | 0x85).
        let tokens = vec![
            Token { bytes: "ال".as_bytes().to_vec(), probability: 0.99 },
            Token { bytes: "سلام".as_bytes().to_vec(), probability: 0.9 },
            Token { bytes: b" ".to_vec(), probability: 0.2 },
            Token { bytes: "عليك".as_bytes().to_vec(), probability: 0.98 },
            Token { bytes: vec![0xD9], probability: 0.6 },
            Token { bytes: vec![0x85], probability: 0.97 },
        ];
        let words = word_confidences(&tokens);
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].word, "السلام");
        assert!((words[0].probability - 0.9).abs() < 1e-6);
        assert_eq!(words[1].word, "عليكم");
        assert!((words[1].probability - 0.6).abs() < 1e-6);
    }
}
