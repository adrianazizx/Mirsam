//! Is a page's existing text layer any good? (spec §5.8 "Detect and replace a
//! poor existing text layer", ADR 0038)
//!
//! Scans often arrive with an invisible text layer from another tool (a
//! scanner's OCR, an old converter). Search and copy then return what that
//! layer says, and Arabic layers are often wrong in ways a reader cannot see:
//!
//! - **mojibake:** Arabic stored in a legacy code page and read as Latin
//!   (Windows-1256 as Windows-1252: "ÇáÚÞÏ"), or UTF-8 read as Latin-1 ("Ø§Ù„");
//! - **no Unicode mapping:** glyph codes with no `/ToUnicode`, which come out
//!   as private-use characters, U+FFFD, control characters, or nothing;
//! - **reversed Arabic:** words stored in visual order, letters last to first;
//! - **misordered:** the right words in the wrong order (lines or words);
//! - **mismatch:** text that is not what the image shows.
//!
//! The text alone shows the first three. The last two, and confirmation of the
//! others, come from comparing the layer with a quick draft of the page's
//! image (Apple Vision or Tesseract, ADR 0035): character trigrams for what is
//! said, the longest common subsequence of shared words for order. A draft's
//! own errors (Vision's Arabic is ~10% CER) are why the thresholds are wide.

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

/// What text a page has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LayerKind {
    /// No text at all (an image-only scan): recognize it.
    None,
    /// A text layer of ours (ADR 0035), or text converted from one.
    Ours,
    /// Text drawn visibly: the page's own content, not a layer over a scan.
    Visible,
    /// Invisible text over the page (render modes 3 and 7): another tool's layer.
    Invisible,
}

/// What is wrong with a layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Problem {
    Mojibake,
    MissingUnicode,
    ReversedArabic,
    Misordered,
    /// It does not say what the image shows.
    Mismatch,
    /// This app's extractor found its glyph mapping suspect (ADR 0008).
    BrokenMapping,
}

/// The verdict on a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    Good,
    Poor,
    /// Not a layer to judge (no text, ours, or visible text).
    NotChecked,
}

/// One page's check, as kept and shown.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageCheck {
    pub page: usize,
    pub kind: LayerKind,
    pub verdict: Verdict,
    pub problems: Vec<Problem>,
    /// How much the layer says what a draft of the image says (0–1), when compared.
    pub similarity: Option<f32>,
    /// The start of what the layer says, as search and copy see it.
    pub sample: String,
}

/// A page's existing text, as the document host reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextInfo {
    /// The page's text in reading order (this app's extractor).
    pub text: String,
    /// Text-showing operators drawn invisibly by another tool, and drawn visibly.
    pub invisible: usize,
    pub visible: usize,
    /// The page has a layer of ours, or text converted from one.
    pub ours: bool,
    /// The extractor found the glyph mapping suspect.
    pub suspect: bool,
}

impl TextInfo {
    pub fn kind(&self) -> LayerKind {
        if self.ours {
            LayerKind::Ours
        } else if self.visible > 0 {
            LayerKind::Visible
        } else if self.invisible > 0 {
            LayerKind::Invisible
        } else {
            LayerKind::None
        }
    }

    /// Whether the check compares the layer with a draft of the image.
    pub fn needs_draft(&self) -> bool {
        self.kind() == LayerKind::Invisible
    }
}

const SAMPLE: usize = 80;

/// Checks page `page`'s text (`info`) against `draft`, a quick reading of its
/// image when there is one.
pub fn check_page(page: usize, info: &TextInfo, draft: Option<&str>) -> PageCheck {
    let kind = info.kind();
    let sample: String =
        info.text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(SAMPLE).collect();
    if kind != LayerKind::Invisible {
        return PageCheck {
            page,
            kind,
            verdict: Verdict::NotChecked,
            problems: Vec::new(),
            similarity: None,
            sample,
        };
    }
    let mut problems = Vec::new();
    let text = &info.text;
    let letters = text.chars().filter(|c| !c.is_whitespace()).count();
    if letters < 3 || unmapped_share(text) > 0.1 {
        problems.push(Problem::MissingUnicode);
    }
    let redecoded = redecode(text);
    if redecoded.is_some() {
        problems.push(Problem::Mojibake);
    }
    if reversed_signals(text) {
        problems.push(Problem::ReversedArabic);
    }
    if info.suspect {
        problems.push(Problem::BrokenMapping);
    }
    let mut similarity = None;
    if let Some(draft) = draft.filter(|d| d.chars().filter(|c| c.is_alphanumeric()).count() >= 12) {
        let (seen, layer) = (words(draft), words(text));
        let forward = dice(&seen, &layer);
        similarity = Some(forward);
        if let Some(fixed) = &redecoded
            && dice(&seen, &words(fixed)) > forward + 0.2
            && !problems.contains(&Problem::Mojibake)
        {
            problems.push(Problem::Mojibake);
        }
        let flipped: Vec<String> = layer.iter().map(|w| flip_arabic(w)).collect();
        if dice(&seen, &flipped) > forward + 0.2 && !problems.contains(&Problem::ReversedArabic) {
            problems.push(Problem::ReversedArabic);
        }
        if forward >= 0.5 && order(&seen, &layer) < 0.6 {
            problems.push(Problem::Misordered);
        }
        if forward < 0.3 && problems.is_empty() {
            problems.push(Problem::Mismatch);
        }
    }
    let verdict = if problems.is_empty() { Verdict::Good } else { Verdict::Poor };
    PageCheck { page, kind, verdict, problems, similarity, sample }
}

/// The share of characters that carry no text: U+FFFD, private use, controls.
fn unmapped_share(text: &str) -> f32 {
    let mut total = 0usize;
    let mut bad = 0usize;
    for c in text.chars().filter(|c| !c.is_whitespace()) {
        total += 1;
        if matches!(c, '\u{FFFD}' | '\u{E000}'..='\u{F8FF}' | '\u{F0000}'..='\u{10FFFF}')
            || (c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            bad += 1;
        }
    }
    if total == 0 { 0.0 } else { bad as f32 / total as f32 }
}

fn is_arabic_letter(c: char) -> bool {
    matches!(c, '\u{0620}'..='\u{064A}' | '\u{0671}'..='\u{06D3}' | '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFC}')
}

/// Windows-1252's bytes 0x80–0x9F (where it differs from Latin-1).
const CP1252_HIGH: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}', '\u{90}',
    '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
];

/// Windows-1256 (Arabic) for bytes 0x80–0xFF.
const CP1256_HIGH: [char; 128] = [
    '€', 'پ', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'ٹ', '‹', 'Œ', 'چ', 'ژ', 'ڈ', 'گ', '‘', '’', '“', '”',
    '•', '–', '—', 'ک', '™', 'ڑ', '›', 'œ', '\u{200C}', '\u{200D}', 'ں', '\u{A0}', '،', '¢', '£', '¤', '¥',
    '¦', '§', '¨', '©', 'ھ', '«', '¬', '\u{AD}', '®', '¯', '°', '±', '²', '³', '´', 'µ', '¶', '·', '¸', '¹',
    '؛', '»', '¼', '½', '¾', '؟', 'ہ', 'ء', 'آ', 'أ', 'ؤ', 'إ', 'ئ', 'ا', 'ب', 'ة', 'ت', 'ث', 'ج', 'ح', 'خ',
    'د', 'ذ', 'ر', 'ز', 'س', 'ش', 'ص', 'ض', '×', 'ط', 'ظ', 'ع', 'غ', 'ـ', 'ف', 'ق', 'ك', 'à', 'ل', 'â', 'م',
    'ن', 'ه', 'و', 'ç', 'è', 'é', 'ê', 'ë', 'ى', 'ي', 'î', 'ï', '\u{64B}', '\u{64C}', '\u{64D}', '\u{64E}',
    'ô', '\u{64F}', '\u{650}', '÷', '\u{651}', 'ù', '\u{652}', 'û', 'ü', '\u{200E}', '\u{200F}', 'ے',
];

/// The byte Windows-1252 writes `c` as.
fn cp1252_byte(c: char) -> Option<u8> {
    let code = c as u32;
    if code < 0x80 || (0xA0..=0xFF).contains(&code) {
        return Some(code as u8);
    }
    CP1252_HIGH.iter().position(|&h| h == c).map(|i| 0x80 + i as u8)
}

/// The Arabic text `text` was before it was read in the wrong code page —
/// Windows-1256 or UTF-8 bytes read as Windows-1252 — when it is mojibake.
pub fn redecode(text: &str) -> Option<String> {
    let letters = text.chars().filter(|c| c.is_alphabetic()).count();
    let latin_high = text.chars().filter(|&c| matches!(c, '\u{00C0}'..='\u{00FF}')).count();
    if letters < 4
        || text.chars().filter(|&c| is_arabic_letter(c)).count() * 10 > letters
        || latin_high * 3 < letters
    {
        return None;
    }
    let bytes: Option<Vec<u8>> = text.chars().map(cp1252_byte).collect();
    let bytes = bytes?;
    let arabic_share = |s: &str| {
        let letters = s.chars().filter(|c| c.is_alphabetic()).count().max(1);
        s.chars().filter(|&c| is_arabic_letter(c)).count() as f32 / letters as f32
    };
    let utf8 = String::from_utf8(bytes.clone()).ok();
    let cp1256: String = bytes
        .iter()
        .map(|&b| if b < 0x80 { char::from(b) } else { CP1256_HIGH[usize::from(b - 0x80)] })
        .collect();
    [utf8, Some(cp1256)]
        .into_iter()
        .flatten()
        .filter(|s| arabic_share(s) >= 0.6)
        .max_by(|a, b| arabic_share(a).total_cmp(&arabic_share(b)))
}

/// Arabic words stored last letter first: the definite article at their ends
/// and word-final letters (ة, ى) at their starts.
fn reversed_signals(text: &str) -> bool {
    let (mut forward, mut backward) = (0usize, 0usize);
    for word in text.nfkc().collect::<String>().split_whitespace() {
        let letters: Vec<char> = word.chars().filter(|&c| is_arabic_letter(c)).collect();
        if letters.len() < 3 {
            continue;
        }
        let (first, last) = (letters[0], letters[letters.len() - 1]);
        if letters.len() >= 4 && letters[0] == 'ا' && letters[1] == 'ل' {
            forward += 1;
        }
        if letters.len() >= 4 && letters[letters.len() - 1] == 'ا' && letters[letters.len() - 2] == 'ل' {
            backward += 1;
        }
        if matches!(last, 'ة' | 'ى') {
            forward += 1;
        }
        if matches!(first, 'ة' | 'ى') {
            backward += 1;
        }
    }
    backward >= 3 && backward > 2 * forward
}

/// An Arabic word's letters last to first (other words as they are).
fn flip_arabic(word: &str) -> String {
    if word.chars().any(is_arabic_letter) { word.chars().rev().collect() } else { word.to_owned() }
}

/// Words as compared: NFKC, no tashkeel or tatweel, alef and yeh forms folded,
/// Arabic-Indic digits as ASCII, lower case, punctuation gone.
fn words(text: &str) -> Vec<String> {
    let folded: String = text
        .nfkc()
        .filter_map(|c| match c {
            '\u{064B}'..='\u{065F}' | '\u{0670}' | '\u{0640}' => None,
            'أ' | 'إ' | 'آ' | 'ٱ' => Some('ا'),
            'ى' => Some('ي'),
            '٠'..='٩' => char::from_digit(c as u32 - '٠' as u32, 10),
            '۰'..='۹' => char::from_digit(c as u32 - '۰' as u32, 10),
            c if c.is_alphanumeric() => Some(c),
            _ => Some(' '),
        })
        .flat_map(char::to_lowercase)
        .collect();
    folded.split_whitespace().map(str::to_owned).collect()
}

/// The Dice coefficient of two texts' character trigrams (each word padded).
fn dice(a: &[String], b: &[String]) -> f32 {
    use std::collections::HashMap;
    let grams = |words: &[String]| {
        let mut map: HashMap<[char; 3], usize> = HashMap::new();
        for word in words {
            let chars: Vec<char> =
                std::iter::once('^').chain(word.chars()).chain(std::iter::once('$')).collect();
            for w in chars.windows(3) {
                *map.entry([w[0], w[1], w[2]]).or_default() += 1;
            }
        }
        map
    };
    let (ga, gb) = (grams(a), grams(b));
    let (na, nb): (usize, usize) = (ga.values().sum(), gb.values().sum());
    if na + nb == 0 {
        return 0.0;
    }
    let common: usize = ga.iter().map(|(g, n)| (*n).min(gb.get(g).copied().unwrap_or(0))).sum();
    2.0 * common as f32 / (na + nb) as f32
}

/// How much of the words both texts share come in the same order: the longest
/// common subsequence of the shared words over their count.
fn order(seen: &[String], layer: &[String]) -> f32 {
    use std::collections::HashSet;
    let (in_seen, in_layer): (HashSet<&String>, HashSet<&String>) =
        (seen.iter().collect(), layer.iter().collect());
    let a: Vec<&String> = seen.iter().filter(|w| in_layer.contains(w)).take(2000).collect();
    let b: Vec<&String> = layer.iter().filter(|w| in_seen.contains(w)).take(2000).collect();
    if a.len() < 4 || b.len() < 4 {
        return 1.0;
    }
    let mut row = vec![0u32; b.len() + 1];
    for x in &a {
        let mut diagonal = 0;
        for (j, y) in b.iter().enumerate() {
            let up = row[j + 1];
            row[j + 1] = if x == y { diagonal + 1 } else { up.max(row[j]) };
            diagonal = up;
        }
    }
    row[b.len()] as f32 / a.len().min(b.len()) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARABIC: &str = "نص العقد بين الطرفين في مدينة الرياض بتاريخ اليوم على أن يلتزم المستأجر بدفع الأجرة الشهرية كاملة في موعدها المحدد";
    const ENGLISH: &str = "This lease agreement is made between the parties in the city of Riyadh on this day and the tenant shall pay the monthly rent";

    fn layer(text: &str) -> TextInfo {
        TextInfo { text: text.into(), invisible: 40, visible: 0, ours: false, suspect: false }
    }

    /// A draft reader's errors: a letter dropped here and there.
    fn draft(text: &str) -> String {
        text.split(' ')
            .enumerate()
            .map(|(i, w)| if i % 5 == 2 { w.chars().skip(1).collect() } else { w.to_owned() })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn cp1256_mojibake(text: &str) -> String {
        text.chars()
            .map(|c| match CP1256_HIGH.iter().position(|&h| h == c) {
                Some(i) => {
                    let byte = 0x80 + i as u8;
                    if (0xA0..=0xFF).contains(&byte) {
                        char::from(byte)
                    } else {
                        CP1252_HIGH[usize::from(byte - 0x80)]
                    }
                }
                None => c,
            })
            .collect()
    }

    fn utf8_mojibake(text: &str) -> String {
        text.bytes()
            .map(
                |b| {
                    if (0x80..0xA0).contains(&b) { CP1252_HIGH[usize::from(b - 0x80)] } else { char::from(b) }
                },
            )
            .collect()
    }

    #[test]
    fn good_layers_pass_in_arabic_english_and_mixed() {
        for text in [ARABIC, ENGLISH, &format!("{ARABIC} iPhone 15 Pro {ENGLISH}")] {
            let check = check_page(0, &layer(text), Some(&draft(text)));
            assert_eq!((check.verdict, &check.problems[..]), (Verdict::Good, &[][..]), "{text}");
            assert!(check.similarity.is_some_and(|s| s > 0.7));
        }
    }

    #[test]
    fn mojibake_is_found_from_the_text_alone() {
        let latin = cp1256_mojibake(ARABIC);
        assert!(latin.contains('Ç') && !latin.chars().any(is_arabic_letter), "{latin}");
        assert_eq!(redecode(&latin).as_deref(), Some(ARABIC));
        assert_eq!(check_page(0, &layer(&latin), None).problems, [Problem::Mojibake]);
        let utf8 = utf8_mojibake(ARABIC);
        assert!(utf8.contains('Ø'), "{utf8}");
        assert_eq!(redecode(&utf8).as_deref(), Some(ARABIC));
        let check = check_page(0, &layer(&utf8), Some(&draft(ARABIC)));
        assert_eq!(check.verdict, Verdict::Poor);
        assert!(check.problems.contains(&Problem::Mojibake));
        // French is not Arabic mojibake.
        assert_eq!(redecode("Le café était très réputé à Montréal et à Québec"), None);
    }

    #[test]
    fn reversed_arabic_is_found_and_confirmed_by_the_image() {
        let reversed: String =
            ARABIC.split(' ').map(|w| w.chars().rev().collect::<String>()).collect::<Vec<_>>().join(" ");
        let check = check_page(0, &layer(&reversed), Some(&draft(ARABIC)));
        assert!(check.problems.contains(&Problem::ReversedArabic), "{:?}", check.problems);
        assert_eq!(check.verdict, Verdict::Poor);
        // English is never "reversed Arabic".
        let english: String =
            ENGLISH.split(' ').map(|w| w.chars().rev().collect::<String>()).collect::<Vec<_>>().join(" ");
        assert!(
            !check_page(0, &layer(&english), Some(&draft(ENGLISH)))
                .problems
                .contains(&Problem::ReversedArabic)
        );
    }

    #[test]
    fn missing_mapping_misorder_and_mismatch() {
        let pua: String = ARABIC.chars().map(|c| if c == ' ' { c } else { '\u{E012}' }).collect();
        assert_eq!(check_page(0, &layer(&pua), None).problems, [Problem::MissingUnicode]);
        assert_eq!(check_page(0, &layer(""), None).problems, [Problem::MissingUnicode]);
        // The same words, lines in reverse order and words reversed in each line.
        let shuffled: String = ARABIC.split(' ').rev().collect::<Vec<_>>().join(" ");
        let check = check_page(0, &layer(&shuffled), Some(&draft(ARABIC)));
        assert_eq!(check.problems, [Problem::Misordered]);
        let other = check_page(0, &layer(ENGLISH), Some(&draft(ARABIC)));
        assert_eq!(other.problems, [Problem::Mismatch]);
        assert!(other.similarity.is_some_and(|s| s < 0.1));
    }

    #[test]
    fn only_invisible_layers_of_other_tools_are_judged() {
        let ours = TextInfo { ours: true, ..layer(ARABIC) };
        let visible = TextInfo { visible: 10, ..layer(ARABIC) };
        let none = TextInfo { invisible: 0, ..layer("") };
        for (info, kind) in [(ours, LayerKind::Ours), (visible, LayerKind::Visible), (none, LayerKind::None)]
        {
            assert!(!info.needs_draft());
            let check = check_page(3, &info, None);
            assert_eq!((check.kind, check.verdict), (kind, Verdict::NotChecked));
        }
        let check = check_page(3, &layer(ARABIC), None);
        assert_eq!(
            (check.kind, check.verdict, check.similarity),
            (LayerKind::Invisible, Verdict::Good, None)
        );
        assert!(check.sample.starts_with("نص العقد"));
    }
}
