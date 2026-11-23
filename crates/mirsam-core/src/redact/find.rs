//! Finding what to redact by pattern (ADR 0042): a regular expression or a
//! preset (e-mail addresses, phone numbers, IBANs, ID numbers, card
//! numbers), run over the same folded text search uses. Folding makes the
//! presets Arabic-aware without extra rules: Arabic-Indic and Persian digits
//! arrive as 0–9, tashkeel and tatweel are gone, so `٠٥٥ ١٢٣ ٤٥٦٧` is a phone
//! number like `055 123 4567`. Each hit carries the boxes of the glyphs it
//! covers, merged into one box per run of text on a line.

use serde::Serialize;

use crate::search::{DocumentIndex, HitContext, Query, SearchError, SearchOptions};
use crate::text::Rect;

/// A ready-made pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Preset {
    Email,
    Phone,
    Iban,
    /// A national or civil ID: 9 to 14 digits.
    IdNumber,
    /// A payment card number that passes the Luhn check.
    CardNumber,
}

impl Preset {
    pub const ALL: [Preset; 5] =
        [Preset::Email, Preset::Phone, Preset::Iban, Preset::IdNumber, Preset::CardNumber];

    /// The pattern, which runs on folded text (digits are 0–9, letters lower case).
    pub fn pattern(self) -> &'static str {
        match self {
            Preset::Email => r"[\p{L}\p{N}._%+\-]+@[\p{L}\p{N}\-]+(?:\.[\p{L}\p{N}\-]+)*\.\p{L}{2,}",
            Preset::Phone => r"(?:\+|\b00)?[0-9][0-9 ().\-]{6,18}[0-9]",
            Preset::Iban => r"\b[a-z]{2}[0-9]{2}(?: ?[a-z0-9]{4}){2,7}(?: ?[a-z0-9]{1,3})?\b",
            Preset::IdNumber => r"\b[0-9]{9,14}\b",
            Preset::CardNumber => r"\b(?:[0-9][ \-]?){12,18}[0-9]\b",
        }
    }

    /// How many bytes of a match of the pattern are really one of these
    /// (the whole match, or less where the pattern ran on into the next
    /// word), or `None` when it is not one.
    fn accept(self, matched: &str) -> Option<usize> {
        let digits: Vec<u32> = matched.chars().filter_map(digit_value).collect();
        let ok = match self {
            Preset::Phone => (8..=15).contains(&digits.len()),
            Preset::CardNumber => (13..=19).contains(&digits.len()) && luhn(&digits),
            Preset::Iban => {
                // The last group may be short, so the pattern can take the
                // next word along: drop groups until the check digits agree.
                let mut candidate = matched;
                loop {
                    if iban_valid(candidate) {
                        return Some(candidate.len());
                    }
                    candidate = &candidate[..candidate.rfind(' ')?];
                }
            }
            _ => true,
        };
        ok.then_some(matched.len())
    }
}

/// ISO 13616: move the first four characters to the end, read letters as
/// 10–35, and the number must leave 1 modulo 97.
fn iban_valid(text: &str) -> bool {
    let compact: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    if !(15..=34).contains(&compact.len()) {
        return false;
    }
    let mut remainder = 0u32;
    for c in compact[4..].iter().chain(&compact[..4]) {
        let value = match digit_value(*c) {
            Some(d) => d,
            None if c.is_ascii_alphabetic() => c.to_ascii_uppercase() as u32 - 'A' as u32 + 10,
            None => return false,
        };
        remainder = if value >= 10 { (remainder * 100 + value) % 97 } else { (remainder * 10 + value) % 97 };
    }
    remainder == 1
}

/// A digit's value in any of the styles search folds together.
fn digit_value(c: char) -> Option<u32> {
    match c {
        '\u{0660}'..='\u{0669}' => Some(c as u32 - 0x0660),
        '\u{06F0}'..='\u{06F9}' => Some(c as u32 - 0x06F0),
        _ => c.to_digit(10),
    }
}

fn luhn(digits: &[u32]) -> bool {
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, d)| {
            if i % 2 == 1 {
                let doubled = d * 2;
                doubled - if doubled > 9 { 9 } else { 0 }
            } else {
                *d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

/// What to look for.
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// Text, or a regular expression when `options.regex` is set.
    Text {
        text: String,
        options: SearchOptions,
    },
    Preset(Preset),
}

/// One match, with where it is.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hit {
    pub page: usize,
    /// The matched text as it is on the page.
    pub text: String,
    pub before: String,
    pub after: String,
    /// One box per run of the match on a line, in page space.
    pub rects: Vec<Rect>,
}

/// The first `limit` matches of `pattern` in the document, and whether there
/// were more.
pub fn find_matches(
    index: &DocumentIndex,
    pattern: &Pattern,
    limit: usize,
) -> std::result::Result<(Vec<Hit>, bool), SearchError> {
    let (query, preset) = match pattern {
        Pattern::Text { text, options } => (Query::new(text, options.clone())?, None),
        Pattern::Preset(preset) => {
            let options = SearchOptions { regex: true, ..SearchOptions::default() };
            (Query::new(preset.pattern(), options)?, Some(*preset))
        }
    };
    let mut hits = Vec::new();
    for page in 0..index.page_count() {
        let Some(text) = index.page(page) else { continue };
        for hit in query.find_on_page(text) {
            let Some(line) = text.blocks.get(hit.block).and_then(|b| b.lines.get(hit.line)) else { continue };
            let mut end = hit.end;
            let mut rects = hit.rects.clone();
            if let Some(preset) = preset {
                let Some(kept) = preset.accept(&line.text[hit.start..hit.end]) else { continue };
                if hit.start + kept < end {
                    end = hit.start + kept;
                    rects = line
                        .spans
                        .iter()
                        .filter(|span| span.start < end && hit.start < span.end)
                        .filter_map(|span| span.rect)
                        .collect();
                }
            }
            let matched = &line.text[hit.start..end];
            if hits.len() == limit {
                return Ok((hits, true));
            }
            let context = HitContext::of(&line.text, hit.start, end, 24);
            hits.push(Hit {
                page,
                text: matched.to_string(),
                before: context.before,
                after: context.after,
                rects: merge(&rects),
            });
        }
    }
    Ok((hits, false))
}

/// Glyph boxes joined into one box per run on a line: boxes that share a
/// line and sit within a space's width of each other.
pub(crate) fn merge(rects: &[Rect]) -> Vec<Rect> {
    let mut merged: Vec<Rect> = Vec::new();
    for rect in rects {
        merged.push(*rect);
    }
    loop {
        let mut joined = false;
        'scan: for i in 0..merged.len() {
            for j in i + 1..merged.len() {
                if same_run(&merged[i], &merged[j]) {
                    merged[i] = merged[i].union(merged[j]);
                    merged.remove(j);
                    joined = true;
                    break 'scan;
                }
            }
        }
        if !joined {
            return merged;
        }
    }
}

fn same_run(a: &Rect, b: &Rect) -> bool {
    let overlap = a.top.min(b.top) - a.bottom.max(b.bottom);
    let shorter = a.height().min(b.height());
    if shorter <= 0.0 || overlap < 0.5 * shorter {
        return false;
    }
    let gap = (a.left.max(b.left) - a.right.min(b.right)).max(0.0);
    gap <= 0.8 * a.height().max(b.height())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(preset: Preset, text: &str) -> Vec<String> {
        let options = SearchOptions { regex: true, ..SearchOptions::default() };
        let query = Query::new(preset.pattern(), options).expect("compiles");
        query
            .find_in(text)
            .into_iter()
            .map(|(s, e)| text[s..e].to_string())
            .filter_map(|m| preset.accept(&m).map(|kept| m[..kept].to_string()))
            .collect()
    }

    #[test]
    fn e_mail_addresses_are_found_in_arabic_text() {
        let found = matches(Preset::Email, "للتواصل: Jane.Roe+x@example.co.uk أو غيره");
        assert_eq!(found, ["Jane.Roe+x@example.co.uk"]);
        assert!(matches(Preset::Email, "no address here @ all").is_empty());
    }

    #[test]
    fn phone_numbers_match_in_both_digit_styles() {
        assert_eq!(matches(Preset::Phone, "call +966 55 123 4567 now"), ["+966 55 123 4567"]);
        assert_eq!(matches(Preset::Phone, "هاتف ٠٥٥ ١٢٣ ٤٥٦٧ الآن"), ["٠٥٥ ١٢٣ ٤٥٦٧"]);
        assert!(matches(Preset::Phone, "page 12 of 40").is_empty());
    }

    #[test]
    fn iban_and_id_numbers_are_found() {
        assert_eq!(
            matches(Preset::Iban, "IBAN SA03 8000 0000 6080 1016 7519 for"),
            ["SA03 8000 0000 6080 1016 7519"]
        );
        assert!(matches(Preset::Iban, "IBAN SA03 8000 0000 6080 1016 7518 for").is_empty());
        assert_eq!(matches(Preset::IdNumber, "رقم الهوية ١٠٢٣٤٥٦٧٨٩ صادر"), ["١٠٢٣٤٥٦٧٨٩"]);
        assert!(matches(Preset::IdNumber, "year 2026").is_empty());
    }

    #[test]
    fn card_numbers_must_pass_the_luhn_check() {
        assert_eq!(matches(Preset::CardNumber, "card 4111 1111 1111 1111 end"), ["4111 1111 1111 1111"]);
        assert!(matches(Preset::CardNumber, "card 4111 1111 1111 1112 end").is_empty());
    }

    #[test]
    fn glyph_boxes_join_per_run() {
        let glyph = |left: f32| Rect { left, bottom: 0.0, right: left + 5.0, top: 10.0 };
        let merged = merge(&[glyph(0.0), glyph(5.0), glyph(10.0), glyph(100.0), glyph(105.0)]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].right, 15.0);
        // A second line stays separate.
        let below = Rect { left: 0.0, bottom: -20.0, right: 5.0, top: -10.0 };
        assert_eq!(merge(&[glyph(0.0), below]).len(), 2);
    }
}
