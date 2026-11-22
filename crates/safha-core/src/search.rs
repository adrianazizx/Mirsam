//! In-document search with Arabic-aware matching (docs/SPEC.md section 6.2).
//!
//! Text is compared in a *folded* form. With Arabic normalization on (the
//! default), folding ignores tashkeel and tatweel, unifies alef variants
//! (أ إ آ ٱ → ا) and ى with ي, optionally ة with ه, treats Arabic-Indic and
//! Western digits as equal, and maps presentation forms to base letters. Case
//! folding applies when the search is case-insensitive. Every folded character
//! remembers which original characters it came from, so a match maps back to
//! the exact range of the page text, and through its spans to glyph boxes for
//! highlighting.

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::document::Document;
use crate::text::{PageText, Rect};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    /// Treat the query as a regular expression (matched against folded text).
    pub regex: bool,
    /// Ignore tashkeel and tatweel; unify alef variants, ى/ي, and digit styles.
    pub arabic_normalize: bool,
    /// Also treat ة and ه as equal.
    pub ta_marbuta_as_ha: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            case_sensitive: false,
            whole_word: false,
            regex: false,
            arabic_normalize: true,
            ta_marbuta_as_ha: false,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SearchError {
    #[error("the search text is empty")]
    Empty,
    #[error("invalid regular expression: {0}")]
    InvalidRegex(String),
}

/// One match on a page.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchHit {
    pub block: usize,
    pub line: usize,
    /// Byte range in the line's text.
    pub start: usize,
    pub end: usize,
    /// Boxes of the matched glyphs, for highlighting.
    pub rects: Vec<Rect>,
}

/// A query compiled for repeated use across pages.
#[derive(Debug, Clone)]
pub struct Query {
    options: SearchOptions,
    matcher: Matcher,
}

#[derive(Debug, Clone)]
enum Matcher {
    Literal(String),
    Regex(Regex),
}

impl Query {
    pub fn new(text: &str, options: SearchOptions) -> std::result::Result<Self, SearchError> {
        let matcher = if options.regex {
            if text.is_empty() {
                return Err(SearchError::Empty);
            }
            // The pattern runs on folded text, so it is not folded itself
            // (that would corrupt its syntax); case is handled by the regex flag.
            let pattern = if options.whole_word { format!(r"\b(?:{text})\b") } else { text.to_string() };
            let regex = RegexBuilder::new(&pattern)
                .case_insensitive(!options.case_sensitive)
                .size_limit(1 << 20)
                .build()
                .map_err(|err| SearchError::InvalidRegex(err.to_string()))?;
            Matcher::Regex(regex)
        } else {
            let folded = fold(text, &options).text;
            if folded.trim().is_empty() {
                return Err(SearchError::Empty);
            }
            Matcher::Literal(folded)
        };
        Ok(Self { options, matcher })
    }

    /// All matches in one line of text, as byte ranges of the original.
    pub fn find_in(&self, line: &str) -> Vec<(usize, usize)> {
        let folded = fold(line, &self.options);
        let ranges: Vec<(usize, usize)> = match &self.matcher {
            Matcher::Regex(regex) => regex
                .find_iter(&folded.text)
                .filter(|m| !m.is_empty())
                .map(|m| (m.start(), m.end()))
                .collect(),
            Matcher::Literal(needle) => folded
                .text
                .match_indices(needle.as_str())
                .map(|(start, matched)| (start, start + matched.len()))
                .filter(|&(start, end)| !self.options.whole_word || is_whole_word(&folded.text, start, end))
                .collect(),
        };
        ranges
            .into_iter()
            .filter_map(|(start, end)| folded.original_range(start, end))
            .map(|(start, end)| (start, extend_over_marks(line, end)))
            .collect()
    }

    /// All matches on a page, with highlight boxes.
    pub fn find_on_page(&self, page: &PageText) -> Vec<SearchHit> {
        let mut hits = Vec::new();
        for (b, block) in page.blocks.iter().enumerate() {
            for (l, line) in block.lines.iter().enumerate() {
                for (start, end) in self.find_in(&line.text) {
                    let rects = line
                        .spans
                        .iter()
                        .filter(|span| span.start < end && start < span.end)
                        .filter_map(|span| span.rect)
                        .collect();
                    hits.push(SearchHit { block: b, line: l, start, end, rects });
                }
            }
        }
        hits
    }
}

/// Folding drops a letter's marks, so a match would stop short of them; a
/// highlight must cover the whole letter it ends on.
fn extend_over_marks(text: &str, mut end: usize) -> usize {
    for c in text[end..].chars() {
        if is_tashkeel(c) || c == '\u{0640}' || unicode_normalization::char::canonical_combining_class(c) != 0
        {
            end += c.len_utf8();
        } else {
            break;
        }
    }
    end
}

/// Every page's text, extracted once, for searching the whole document.
///
/// Building is the expensive part (one extraction per page); searching a
/// built index only folds and scans text. The spec's budget (500 pages in at
/// most 1 s) applies after the first build.
#[derive(Debug, Clone, Default)]
pub struct DocumentIndex {
    pages: Vec<PageText>,
}

impl DocumentIndex {
    /// Extracts every page. `cancelled` is polled between pages; returning
    /// `true` stops early and yields `None`.
    pub fn build(document: &Document, cancelled: impl Fn() -> bool) -> crate::Result<Option<Self>> {
        let mut pages = Vec::with_capacity(document.page_count());
        for index in 0..document.page_count() {
            if cancelled() {
                return Ok(None);
            }
            pages.push(document.page_text(index)?);
        }
        Ok(Some(Self { pages }))
    }

    pub fn page(&self, index: usize) -> Option<&PageText> {
        self.pages.get(index)
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// All matches in the document, in page order.
    pub fn search(&self, query: &Query) -> Vec<(usize, SearchHit)> {
        self.pages
            .iter()
            .enumerate()
            .flat_map(|(page, text)| query.find_on_page(text).into_iter().map(move |hit| (page, hit)))
            .collect()
    }
}

fn is_whole_word(text: &str, start: usize, end: usize) -> bool {
    let word = |c: char| c.is_alphanumeric();
    let before = text[..start].chars().next_back().is_none_or(|c| !word(c));
    let after = text[end..].chars().next().is_none_or(|c| !word(c));
    before && after
}

/// Folded text plus, per folded byte, the original byte range it came from.
struct Folded {
    text: String,
    /// For each byte of `text`, the (start, end) byte range in the original.
    origin: Vec<(usize, usize)>,
}

impl Folded {
    fn original_range(&self, start: usize, end: usize) -> Option<(usize, usize)> {
        let first = self.origin.get(start)?;
        let last = self.origin.get(end.checked_sub(1)?)?;
        Some((first.0, last.1))
    }
}

fn is_tashkeel(c: char) -> bool {
    matches!(c, '\u{0610}'..='\u{061A}' | '\u{064B}'..='\u{065F}' | '\u{0670}' | '\u{06D6}'..='\u{06ED}')
}

/// Folds `text` for comparison under `options`.
fn fold(text: &str, options: &SearchOptions) -> Folded {
    let mut folded = String::with_capacity(text.len());
    let mut origin = Vec::with_capacity(text.len());
    let mut push = |s: &str, range: (usize, usize)| {
        folded.push_str(s);
        origin.extend(std::iter::repeat_n(range, s.len()));
    };

    for (start, c) in text.char_indices() {
        let range = (start, start + c.len_utf8());
        // Presentation forms (lam-alef ligatures, positional forms) as base letters.
        let expanded: String = if matches!(c, '\u{FB00}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}') {
            c.to_string().nfkc().collect()
        } else {
            c.to_string()
        };
        for c in expanded.chars() {
            if options.arabic_normalize {
                if is_tashkeel(c) || c == '\u{0640}' {
                    continue;
                }
                let c = match c {
                    'أ' | 'إ' | 'آ' | 'ٱ' => 'ا',
                    'ى' => 'ي',
                    'ة' if options.ta_marbuta_as_ha => 'ه',
                    '\u{0660}'..='\u{0669}' => char::from(b'0' + (c as u32 - 0x0660) as u8),
                    '\u{06F0}'..='\u{06F9}' => char::from(b'0' + (c as u32 - 0x06F0) as u8),
                    other => other,
                };
                push_cased(&mut push, c, options.case_sensitive, range);
            } else {
                push_cased(&mut push, c, options.case_sensitive, range);
            }
        }
    }
    Folded { text: folded, origin }
}

fn push_cased(
    push: &mut impl FnMut(&str, (usize, usize)),
    c: char,
    case_sensitive: bool,
    range: (usize, usize),
) {
    if case_sensitive {
        push(c.encode_utf8(&mut [0; 4]), range);
    } else {
        for lower in c.to_lowercase() {
            push(lower.encode_utf8(&mut [0; 4]), range);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(query: &str, text: &str, options: SearchOptions) -> Vec<String> {
        Query::new(query, options)
            .expect("valid query")
            .find_in(text)
            .into_iter()
            .map(|(s, e)| text[s..e].to_string())
            .collect()
    }

    fn default_find(query: &str, text: &str) -> Vec<String> {
        find(query, text, SearchOptions::default())
    }

    #[test]
    fn tashkeel_and_tatweel_are_ignored_both_ways() {
        let text = "قَالَ الْمُعَلِّمُ: إِنَّ الْعِلْمَ نُورٌ";
        assert_eq!(default_find("المعلم", text), ["الْمُعَلِّمُ"]);
        assert_eq!(default_find("الْعِلْمَ", "طلب العلم"), ["العلم"]);
        assert_eq!(default_find("مرحبا", "مـــرحـبا"), ["مـــرحـبا"]);
    }

    #[test]
    fn alef_variants_and_alef_maksura_are_unified() {
        assert_eq!(default_find("الاسبوع", "قبل نهاية الأسبوع."), ["الأسبوع"]);
        assert_eq!(default_find("الى", "إلى الإدارة"), ["إلى"]);
        assert_eq!(default_find("علي", "على الطاولة"), ["على"]);
    }

    #[test]
    fn ta_marbuta_is_unified_with_ha_only_when_asked() {
        let text = "المدرسة";
        assert!(default_find("المدرسه", text).is_empty());
        let options = SearchOptions { ta_marbuta_as_ha: true, ..SearchOptions::default() };
        assert_eq!(find("المدرسه", text, options), ["المدرسة"]);
    }

    #[test]
    fn arabic_indic_and_western_digits_match_each_other() {
        assert_eq!(default_find("1234", "صدر القرار رقم ١٢٣٤ لسنة ٢٠٢٥"), ["١٢٣٤"]);
        assert_eq!(default_find("٢٠٢٦", "بتاريخ 12/03/2026"), ["2026"]);
    }

    #[test]
    fn presentation_forms_match_base_letters() {
        // U+FEFB: lam-alef ligature.
        assert_eq!(default_find("لا", "\u{FEFB} بأس"), ["\u{FEFB}"]);
    }

    #[test]
    fn normalization_can_be_turned_off() {
        let strict = SearchOptions { arabic_normalize: false, ..SearchOptions::default() };
        assert!(find("المعلم", "الْمُعَلِّمُ", strict.clone()).is_empty());
        assert_eq!(find("الْمُعَلِّمُ", "قال الْمُعَلِّمُ", strict), ["الْمُعَلِّمُ"]);
    }

    #[test]
    fn case_and_whole_word() {
        assert_eq!(default_find("pdf", "PDF and pdf"), ["PDF", "pdf"]);
        let sensitive = SearchOptions { case_sensitive: true, ..SearchOptions::default() };
        assert_eq!(find("pdf", "PDF and pdf", sensitive), ["pdf"]);
        let whole = SearchOptions { whole_word: true, ..SearchOptions::default() };
        assert_eq!(find("art", "art, part, artist", whole.clone()), ["art"]);
        // Arabic words are words too: "علم" inside "المعلم" is not a whole word.
        assert!(find("علم", "المعلم", whole.clone()).is_empty());
        assert_eq!(find("علم", "علم نافع", whole), ["علم"]);
    }

    #[test]
    fn regex_runs_on_folded_text() {
        let regex = SearchOptions { regex: true, ..SearchOptions::default() };
        assert_eq!(
            find(r"INV-\d{4}-\d+", "رقم الفاتورة INV-2026-00417 بتاريخ", regex.clone()),
            ["INV-2026-00417"]
        );
        // Arabic-Indic digits fold to ASCII, so \d finds them.
        assert_eq!(find(r"\d+", "رقم ١٢٣٤", regex.clone()), ["١٢٣٤"]);
        assert!(matches!(Query::new("(", regex), Err(SearchError::InvalidRegex(_))));
    }

    #[test]
    fn empty_queries_are_rejected() {
        assert_eq!(Query::new("", SearchOptions::default()).err(), Some(SearchError::Empty));
        // Only tashkeel folds to nothing.
        assert_eq!(Query::new("\u{064E}\u{0650}", SearchOptions::default()).err(), Some(SearchError::Empty));
    }

    #[test]
    fn matches_map_back_to_whole_original_characters() {
        // "ﻻ" is one character standing for two; matching just "ا" still returns all of it.
        assert_eq!(default_find("ا", "\u{FEFB}"), ["\u{FEFB}"]);
        // Overlapping occurrences are not double-counted.
        assert_eq!(default_find("aa", "aaaa"), ["aa", "aa"]);
    }
}
