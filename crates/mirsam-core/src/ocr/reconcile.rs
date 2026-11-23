//! Re-reading and reconciling: what the pipeline does with words it is not sure of
//! (spec §8 steps 6–7, ADR 0033).
//!
//! The primary engine reads the whole page. Two kinds of words are checked again:
//! every word holding a digit (numbers, dates and IDs, where the 4B is confidently
//! wrong) and every word under the confidence threshold. Other readings of the same
//! word are found by their context:
//!
//! - **strips:** the primary and the partner reading line strips at the page's full
//!   resolution, about twice the glyph size of the 2.4 MP page read;
//! - **page reads:** the partner's whole-page read, and for numbers a third reader of
//!   another family (the platform's OCR) when there is one.
//!
//! Then, comparing what the readings *say* (digits by value, letters without tashkeel):
//!
//! - **confirmed:** every reading agrees with the page read.
//! - **corrected:** both strip reads agree on something else; or one strip read and
//!   another reader agree on something else and no strip read backs the page read.
//!   The text is replaced, in the page read's digit script, and the word stays on
//!   the review list.
//! - **uncertain:** anything else. The page read is kept and the word is flagged.
//!
//! Nothing is invented: a correction is always a reading of this image by one of
//! the engines. Tashkeel the page read kept is never dropped by a correction that
//! differs only in tashkeel; that counts as agreement.

use unicode_normalization::UnicodeNormalization;

use super::WordConfidence;

/// What happened to a word.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WordStatus {
    /// Not checked again: confident, and no digits.
    Unchecked,
    /// Checked again, and every reading agreed.
    Confirmed,
    /// Replaced: other readings agreed on something else. `read` is what the page read said.
    Corrected { read: String },
    /// Readings disagree; the page read was kept. A person should look.
    Uncertain { alternatives: Vec<String> },
    /// A number–unit compound turned into logical order ("bit-64" → "64-bit").
    Reordered { read: String },
}

impl WordStatus {
    /// Whether a word with this status belongs on the review list.
    pub fn needs_review(&self) -> bool {
        matches!(
            self,
            WordStatus::Uncertain { .. } | WordStatus::Corrected { .. } | WordStatus::Reordered { .. }
        )
    }
}

/// A word of the page's text with its confidence and what reconciling did.
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewedWord {
    pub text: String,
    /// The primary engine's probability for the word as it read it (0–1).
    pub probability: f32,
    pub status: WordStatus,
    /// Index of the text line the word is on.
    pub line: usize,
}

impl ReviewedWord {
    /// Whether the word belongs on the review list.
    pub fn needs_review(&self) -> bool {
        self.status.needs_review()
    }
}

/// Other readings of the page, each as text in its own reading order.
#[derive(Clone, Debug, Default)]
pub struct OtherReadings {
    /// The primary engine on each line strip.
    pub primary_strips: Vec<String>,
    /// The partner engine on each line strip.
    pub partner_strips: Vec<String>,
    /// The partner engine on the whole page.
    pub partner_page: Option<String>,
    /// A third reader on the whole page, consulted for words with digits only.
    pub third_page: Option<String>,
}

/// Which words of the page are checked again: digits, and probability under `threshold`.
pub fn is_target(word: &str, probability: f32, threshold: f32) -> bool {
    word.chars().any(|c| c.is_numeric()) || probability < threshold
}

/// Reconciles each target word of `words` (the page read, in order) against `others`.
/// `words` comes in with `Unchecked` or `Reordered` status; targets get one of the
/// others.
pub fn reconcile(words: &mut [ReviewedWord], others: &OtherReadings, threshold: f32) {
    let page: Vec<String> = words.iter().map(|w| w.text.clone()).collect();
    let split = |texts: &[String]| -> Vec<String> {
        texts.iter().flat_map(|t| t.split_whitespace().map(str::to_owned).collect::<Vec<_>>()).collect()
    };
    let strips_primary = split(&others.primary_strips);
    let strips_partner = split(&others.partner_strips);
    let partner_page = split(others.partner_page.as_slice());
    let third_page = split(others.third_page.as_slice());
    for i in 0..words.len() {
        let word = &words[i];
        if !is_target(&word.text, word.probability, threshold) {
            continue;
        }
        let has_digit = word.text.chars().any(|c| c.is_numeric());
        let mut readings: Vec<(Reader, String)> = Vec::new();
        for (reader, other) in [
            (Reader::PrimaryStrip, &strips_primary),
            (Reader::PartnerStrip, &strips_partner),
            (Reader::PartnerPage, &partner_page),
            (Reader::ThirdPage, &third_page),
        ] {
            if reader == Reader::ThirdPage && !has_digit {
                continue;
            }
            if let Some(found) = locate(&page, i, other) {
                readings.push((reader, found));
            }
        }
        let decision = decide(&page[i], &readings);
        let word = &mut words[i];
        match decision {
            Decision::Unchecked => {}
            Decision::Confirmed => {
                if !matches!(word.status, WordStatus::Reordered { .. }) {
                    word.status = WordStatus::Confirmed;
                }
            }
            Decision::Corrected(text) => {
                let read = std::mem::replace(&mut word.text, text);
                word.status = WordStatus::Corrected { read };
            }
            Decision::Uncertain(alternatives) => word.status = WordStatus::Uncertain { alternatives },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reader {
    PrimaryStrip,
    PartnerStrip,
    PartnerPage,
    ThirdPage,
}

impl Reader {
    fn is_strip(self) -> bool {
        matches!(self, Reader::PrimaryStrip | Reader::PartnerStrip)
    }
}

#[derive(Debug, PartialEq)]
enum Decision {
    Unchecked,
    Confirmed,
    Corrected(String),
    Uncertain(Vec<String>),
}

fn decide(page_word: &str, readings: &[(Reader, String)]) -> Decision {
    if readings.is_empty() {
        return Decision::Unchecked;
    }
    let page_key = key(page_word);
    if readings.iter().all(|(_, r)| key(r) == page_key) {
        return Decision::Confirmed;
    }
    let find = |reader: Reader| readings.iter().find(|(r, _)| *r == reader).map(|(_, text)| text);
    // Both full-resolution strip reads agree on another value.
    if let (Some(a), Some(b)) = (find(Reader::PrimaryStrip), find(Reader::PartnerStrip))
        && key(a) == key(b)
        && key(a) != page_key
    {
        return Decision::Corrected(in_style_of(a, page_word));
    }
    // One strip read and another reader agree on another value, and no strip read
    // backs the page read.
    let strip_backs_page = readings.iter().any(|(r, text)| r.is_strip() && key(text) == page_key);
    if !strip_backs_page {
        for (reader, text) in readings.iter().filter(|(r, _)| r.is_strip()) {
            let k = key(text);
            if k != page_key && readings.iter().any(|(other, t)| other != reader && key(t) == k) {
                return Decision::Corrected(in_style_of(text, page_word));
            }
        }
    }
    let mut alternatives: Vec<String> = Vec::new();
    for (_, text) in readings {
        if key(text) != page_key && !alternatives.iter().any(|a| key(a) == key(text)) {
            alternatives.push(text.clone());
        }
    }
    Decision::Uncertain(alternatives)
}

/// What a reading says, ignoring how it was written: NFC, no bidi controls, no
/// tashkeel or tatweel, digits as ASCII values, no surrounding punctuation.
pub fn key(word: &str) -> String {
    let text: String = word
        .nfc()
        .filter(|c| !is_bidi_control(*c) && !is_tashkeel(*c))
        .map(|c| match c {
            '\u{0660}'..='\u{0669}' => char::from(b'0' + (c as u32 - 0x0660) as u8),
            '\u{06F0}'..='\u{06F9}' => char::from(b'0' + (c as u32 - 0x06F0) as u8),
            other => other,
        })
        .collect();
    text.trim_matches(|c: char| ".,،؛:;!?؟()[]\"'“”«»-–—".contains(c)).to_owned()
}

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

fn is_tashkeel(c: char) -> bool {
    matches!(c, '\u{064B}'..='\u{065F}' | '\u{0670}' | '\u{06D6}'..='\u{06ED}' | '\u{0640}')
}

/// `word` with its digits in the script `model` uses (the page read's: digits are
/// kept as printed).
fn in_style_of(word: &str, model: &str) -> String {
    let arabic_indic = model.chars().any(|c| ('\u{0660}'..='\u{0669}').contains(&c));
    let western = model.chars().any(|c| c.is_ascii_digit());
    word.chars()
        .map(|c| match c {
            '0'..='9' if arabic_indic => char::from_u32(0x0660 + (c as u32 - '0' as u32)).unwrap_or(c),
            '\u{0660}'..='\u{0669}' if western && !arabic_indic => {
                char::from(b'0' + (c as u32 - 0x0660) as u8)
            }
            other => other,
        })
        .collect()
}

/// Similar enough to be another reading of the same word: their keys share at least
/// half their characters (2·LCS / total ≥ 0.5).
fn similar(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (key(a).chars().collect(), key(b).chars().collect());
    if a.is_empty() || b.is_empty() {
        return a == b;
    }
    let mut row = vec![0usize; b.len() + 1];
    for &ca in &a {
        let mut diagonal = 0;
        for (j, &cb) in b.iter().enumerate() {
            let up = row[j + 1];
            row[j + 1] = if ca == cb { diagonal + 1 } else { row[j + 1].max(row[j]) };
            diagonal = up;
        }
    }
    2 * row[b.len()] * 2 >= a.len() + b.len()
}

/// The word of `other` standing where `page[i]` stands: the best match of its
/// context (three words either side; nearer ones weigh more) among words similar to
/// `page[i]`. `None` when the context does not match at least one neighbour.
fn locate(page: &[String], i: usize, other: &[String]) -> Option<String> {
    let page_keys: Vec<String> = page.iter().map(|w| key(w)).collect();
    let other_keys: Vec<String> = other.iter().map(|w| key(w)).collect();
    let mut best: Option<(f32, usize)> = None;
    for j in 0..other.len() {
        let mut score = 0.0;
        for d in [-3i64, -2, -1, 1, 2, 3] {
            let (a, b) = (i as i64 + d, j as i64 + d);
            if a >= 0
                && b >= 0
                && (a as usize) < page.len()
                && (b as usize) < other.len()
                && page_keys[a as usize] == other_keys[b as usize]
            {
                score += 1.0 / d.unsigned_abs() as f32;
            }
        }
        if best.is_none_or(|(s, _)| score > s) && similar(&page[i], &other[j]) {
            best = Some((score, j));
        }
    }
    best.filter(|(score, _)| *score >= 1.0).map(|(_, j)| other[j].clone())
}

/// The page read's words with the primary engine's confidences, aligned by text.
/// `confidences` come from the model's tokens and can differ from the text where
/// markup was removed; unmatched words get probability 1 (they are not raw output).
pub fn words_with_confidence(lines: &[String], confidences: &[WordConfidence]) -> Vec<ReviewedWord> {
    let words: Vec<(usize, &str)> = lines
        .iter()
        .enumerate()
        .flat_map(|(line, text)| text.split_whitespace().map(move |w| (line, w)))
        .collect();
    // Longest common subsequence of the two word lists.
    let (n, m) = (words.len(), confidences.len());
    let mut table = vec![0u32; (n + 1) * (m + 1)];
    for a in (0..n).rev() {
        for b in (0..m).rev() {
            table[a * (m + 1) + b] = if words[a].1 == confidences[b].word {
                table[(a + 1) * (m + 1) + b + 1] + 1
            } else {
                table[(a + 1) * (m + 1) + b].max(table[a * (m + 1) + b + 1])
            };
        }
    }
    let mut out = Vec::with_capacity(n);
    let (mut a, mut b) = (0, 0);
    while a < n {
        if b < m && words[a].1 == confidences[b].word {
            out.push((a, confidences[b].probability));
            a += 1;
            b += 1;
        } else if b < m && table[a * (m + 1) + b + 1] >= table[(a + 1) * (m + 1) + b] {
            b += 1;
        } else {
            out.push((a, 1.0));
            a += 1;
        }
    }
    let out: Vec<ReviewedWord> = out
        .into_iter()
        .map(|(a, probability)| ReviewedWord {
            text: words[a].1.to_owned(),
            probability,
            status: WordStatus::Unchecked,
            line: words[a].0,
        })
        .collect();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(text: &str) -> Vec<ReviewedWord> {
        text.split_whitespace()
            .map(|w| ReviewedWord {
                text: w.into(),
                probability: 1.0,
                status: WordStatus::Unchecked,
                line: 0,
            })
            .collect()
    }

    fn texts(words: &[ReviewedWord]) -> String {
        words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
    }

    const PAGE: &str = "صدر القرار رقم ١٣٣٤ لسنة ٢٠٢٥ في ١٥ مادة.";

    #[test]
    fn two_strip_reads_correct_a_number_in_the_printed_digits() {
        let mut words = page(PAGE);
        let others = OtherReadings {
            primary_strips: vec!["صدر القرار رقم ١٢٣٤ لسنة ٢٠٢٥ في ١٥ مادة.".into()],
            // The partner writes Western digits; the value is what counts.
            partner_strips: vec!["صدر القرار رقم 1234 لسنة 2025 في 15 مادة".into()],
            partner_page: Some(PAGE.into()),
            third_page: None,
        };
        reconcile(&mut words, &others, 0.95);
        assert_eq!(texts(&words), "صدر القرار رقم ١٢٣٤ لسنة ٢٠٢٥ في ١٥ مادة.");
        assert_eq!(words[3].status, WordStatus::Corrected { read: "١٣٣٤".into() });
        assert_eq!(words[5].status, WordStatus::Confirmed);
        assert_eq!(words[0].status, WordStatus::Unchecked);
    }

    #[test]
    fn a_strip_read_backing_the_page_keeps_it_and_flags_it() {
        let mut words = page(PAGE);
        let others = OtherReadings {
            primary_strips: vec![PAGE.into()],
            partner_strips: vec!["صدر القرار رقم 1334 لسنة 2025 في 10 مادة".into()],
            partner_page: Some("صدر القرار رقم ١٢٢٤ لسنة ٢٠٢٥ في ١٥ مادة.".into()),
            third_page: Some("صدر القرار رقم ١٢٢٤ لسنة ٢٠٢٥ في ١٥ مادة.".into()),
        };
        reconcile(&mut words, &others, 0.95);
        assert_eq!(words[3].text, "١٣٣٤");
        assert_eq!(words[3].status, WordStatus::Uncertain { alternatives: vec!["١٢٢٤".into()] });
        // ١٥ against the partner strip's "10": kept and flagged, not changed.
        assert!(matches!(words[7].status, WordStatus::Uncertain { .. }));
    }

    #[test]
    fn one_strip_read_and_another_reader_correct_when_no_strip_backs_the_page() {
        let mut words = page("رقم الهوية ١٠٢٣٤٥٦٧٨٩٩ الإدارة");
        let others = OtherReadings {
            primary_strips: vec!["رقم الهوية ١٠٢٣٤٥٦٧٨٩".into()],
            partner_strips: vec!["رقم الهوية ١٠٣٣٤٥٦٧٨٩".into()],
            partner_page: Some("رقم الهوية ١٠٢٣٤٥٦٧٨٩ الإدارة".into()),
            third_page: None,
        };
        reconcile(&mut words, &others, 0.95);
        assert_eq!(words[2].text, "١٠٢٣٤٥٦٧٨٩");
        assert!(matches!(words[2].status, WordStatus::Corrected { .. }));
    }

    #[test]
    fn tashkeel_alone_is_agreement_and_is_kept() {
        let mut words = page("ما أُنجز خلال الربع");
        words[1].probability = 0.6;
        let others = OtherReadings {
            primary_strips: vec!["ما أنجز خلال الربع".into()],
            partner_strips: vec!["ما أنجز خلال الربع".into()],
            ..Default::default()
        };
        reconcile(&mut words, &others, 0.95);
        assert_eq!(words[1].text, "أُنجز");
        assert_eq!(words[1].status, WordStatus::Confirmed);
    }

    #[test]
    fn a_word_is_never_replaced_by_an_unrelated_one() {
        // Two-column strips interleave the columns; "1." must not become "March".
        let mut words = page("1. Term. The lease runs for 12 months");
        let others = OtherReadings {
            primary_strips: vec!["March 1. البند الأول Term. The lease".into()],
            partner_strips: vec!["March 1. Term. The lease".into()],
            ..Default::default()
        };
        reconcile(&mut words, &others, 0.95);
        assert_eq!(words[0].text, "1.");
        assert_eq!(words[0].status, WordStatus::Confirmed);
    }

    #[test]
    fn keys_compare_values_not_spelling() {
        assert_eq!(key("(١٢٣٤)"), "1234");
        assert_eq!(key("١٢٣٤"), key("1234"));
        assert_eq!(key("أُنجز"), key("أنجز"));
        assert_ne!(key("١٢٣٤"), key("١٣٣٤"));
        assert_eq!(in_style_of("1234", "١٣٣٤"), "١٢٣٤");
        assert_eq!(in_style_of("١٢٣٤", "1334"), "1234");
    }

    #[test]
    fn confidences_follow_the_words_through_markup_changes() {
        let conf = |word: &str, probability: f32| WordConfidence { word: word.into(), probability };
        let lines = vec!["تقرير".to_owned(), "نص عادي".to_owned()];
        let confidences = vec![conf("#", 0.99), conf("تقرير", 0.9), conf("**نص**", 0.5), conf("عادي", 0.7)];
        let words = words_with_confidence(&lines, &confidences);
        assert_eq!(words.len(), 3);
        assert_eq!((words[0].probability, words[0].line), (0.9, 0));
        assert_eq!((words[1].probability, words[1].line), (1.0, 1)); // markup changed it
        assert_eq!((words[2].probability, words[2].line), (0.7, 1));
    }
}
