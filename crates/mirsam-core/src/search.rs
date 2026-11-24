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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::document::Document;
use crate::text::{PageText, Rect};
use crate::view::ViewTransform;

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

/// A match in a line of text, with its capture groups (ADR 0052): byte
/// ranges of the original text, each extended over the marks on its last
/// letter, as the match is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub start: usize,
    pub end: usize,
    /// Group 0 is the whole match; a group that took no part is `None`.
    /// Literal queries have group 0 only.
    pub groups: Vec<Option<(usize, usize)>>,
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
            // The pattern runs on folded text. Its Arabic is folded the same
            // way ("أجهزة" finds "اجهزة"); nothing else is (case folding would
            // corrupt its syntax: `\D` is not `\d`), and case is the regex flag's.
            let text = fold_pattern(text, &options);
            let pattern = if options.whole_word { format!(r"\b(?:{text})\b") } else { text };
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
        self.matches_in(line).into_iter().map(|found| (found.start, found.end)).collect()
    }

    /// All matches in one line of text, with their capture groups.
    ///
    /// The pattern runs on folded text, but every range is the original's:
    /// a group that matched Arabic-Indic digits as `\d` gives those digits,
    /// and one that matched a vocalised word gives it with its marks.
    pub fn matches_in(&self, line: &str) -> Vec<Found> {
        let folded = fold(line, &self.options);
        let mut out = Vec::new();
        match &self.matcher {
            Matcher::Regex(regex) => {
                for captures in regex.captures_iter(&folded.text) {
                    let Some(whole) = captures.get(0).filter(|m| !m.is_empty()) else { continue };
                    let Some((start, end)) = folded.original_range(whole.start(), whole.end()) else {
                        continue;
                    };
                    let end = extend_over_marks(line, end);
                    let groups = captures
                        .iter()
                        .map(|group| {
                            let group = group?;
                            let (from, to) = if group.is_empty() {
                                let at = folded.original_position(group.start(), line.len());
                                (at, at)
                            } else {
                                let (from, to) = folded.original_range(group.start(), group.end())?;
                                (from, extend_over_marks(&line[..end], to))
                            };
                            Some((from.clamp(start, end), to.clamp(start, end)))
                        })
                        .collect();
                    out.push(Found { start, end, groups });
                }
            }
            Matcher::Literal(needle) => {
                for (at, matched) in folded.text.match_indices(needle.as_str()) {
                    let end = at + matched.len();
                    if self.options.whole_word && !is_whole_word(&folded.text, at, end) {
                        continue;
                    }
                    let Some((start, end)) = folded.original_range(at, end) else { continue };
                    let end = extend_over_marks(line, end);
                    out.push(Found { start, end, groups: vec![Some((start, end))] });
                }
            }
        }
        out
    }

    /// The text that replaces `found` in `line`, from `template`.
    ///
    /// A literal query's replacement is the template as typed. A regular
    /// expression's may name its groups: `$1` (two digits when that group
    /// exists, as in `$12`), `${1}`, `${name}` or `$<name>`, `$0` or `$&` for
    /// the whole match, and `$$` for a dollar sign. A group that took no part,
    /// or does not exist, gives nothing; a `$` followed by anything else is
    /// kept as it is. Groups give the line's own text, not its folded form.
    pub fn expand(&self, template: &str, found: &Found, line: &str) -> String {
        let Matcher::Regex(regex) = &self.matcher else { return template.to_string() };
        let group = |index: usize| -> &str {
            found.groups.get(index).copied().flatten().and_then(|(a, b)| line.get(a..b)).unwrap_or("")
        };
        let named = |name: &str| -> &str {
            if let Ok(index) = name.parse::<usize>() {
                return group(index);
            }
            regex.capture_names().position(|n| n == Some(name)).map_or("", group)
        };
        let mut out = String::with_capacity(template.len());
        let mut rest = template;
        while let Some(at) = rest.find('$') {
            out.push_str(&rest[..at]);
            let after = &rest[at + 1..];
            let digits = after.bytes().take_while(u8::is_ascii_digit).count().min(2);
            if let Some(tail) = after.strip_prefix('$') {
                out.push('$');
                rest = tail;
            } else if let Some(tail) = after.strip_prefix('&') {
                out.push_str(group(0));
                rest = tail;
            } else if digits > 0 {
                // Two digits only when that group exists ("$10" with one group is "$1" then "0").
                let two = digits == 2 && after[..2].parse::<usize>().is_ok_and(|n| n < found.groups.len());
                let take = if two { 2 } else { 1 };
                out.push_str(group(after[..take].parse().unwrap_or(usize::MAX)));
                rest = &after[take..];
            } else if let Some((name, tail)) = braced(after, '{', '}').or_else(|| braced(after, '<', '>')) {
                out.push_str(named(name));
                rest = tail;
            } else {
                out.push('$');
                rest = after;
            }
        }
        out.push_str(rest);
        out
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
    views: Vec<ViewTransform>,
}

impl DocumentIndex {
    /// Extracts every page. `cancelled` is polled between pages; returning
    /// `true` stops early and yields `None`.
    pub fn build(document: &Document, cancelled: impl Fn() -> bool) -> crate::Result<Option<Self>> {
        let mut pages = Vec::with_capacity(document.page_count());
        let mut views = Vec::with_capacity(document.page_count());
        for index in 0..document.page_count() {
            if cancelled() {
                return Ok(None);
            }
            let (text, view) = document.page_text_view(index)?;
            pages.push(text);
            views.push(view);
        }
        Ok(Some(Self { pages, views }))
    }

    pub fn page(&self, index: usize) -> Option<&PageText> {
        self.pages.get(index)
    }

    /// The page's mapping to displayed coordinates (see [`ViewTransform`]).
    pub fn view(&self, index: usize) -> Option<&ViewTransform> {
        self.views.get(index)
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// All matches in the document, in page order.
    pub fn search(&self, query: &Query) -> Vec<(usize, SearchHit)> {
        self.search_limited(query, usize::MAX).0
    }

    /// The first `limit` matches in page order, and whether there were more.
    pub fn search_limited(&self, query: &Query, limit: usize) -> (Vec<(usize, SearchHit)>, bool) {
        let mut hits = Vec::new();
        for (page, text) in self.pages.iter().enumerate() {
            for hit in query.find_on_page(text) {
                if hits.len() == limit {
                    return (hits, true);
                }
                hits.push((page, hit));
            }
        }
        (hits, false)
    }
}

/// A document's search index, built at most once and shared by every search.
///
/// The first search builds it (one text extraction per page); concurrent
/// searches wait for that build instead of starting their own. Closing the
/// document calls [`cancel`](Self::cancel), which stops a build in progress.
#[derive(Debug, Default)]
pub struct IndexCache {
    index: OnceLock<Arc<DocumentIndex>>,
    building: Mutex<()>,
    cancelled: AtomicBool,
}

impl IndexCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The index, building it first if needed. `None` when cancelled.
    pub fn get_or_build(&self, document: &Document) -> crate::Result<Option<Arc<DocumentIndex>>> {
        if let Some(index) = self.index.get() {
            return Ok(Some(Arc::clone(index)));
        }
        let _building = self.building.lock().unwrap_or_else(PoisonError::into_inner);
        // Another search may have finished the build while this one waited.
        if let Some(index) = self.index.get() {
            return Ok(Some(Arc::clone(index)));
        }
        let Some(built) = DocumentIndex::build(document, || self.cancelled.load(Ordering::Relaxed))? else {
            return Ok(None);
        };
        Ok(Some(Arc::clone(self.index.get_or_init(|| Arc::new(built)))))
    }

    /// The index if it is already built; never builds.
    pub fn ready(&self) -> Option<Arc<DocumentIndex>> {
        self.index.get().cloned()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

/// Text around a match for a results list: up to `chars` characters on each
/// side, cut at a word boundary where one is near, with `…` where text was cut.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HitContext {
    pub before: String,
    pub matched: String,
    pub after: String,
}

impl HitContext {
    pub fn of(line: &str, start: usize, end: usize, chars: usize) -> Self {
        let before = &line[..start];
        let after = &line[end..];
        let mut head: String = {
            let skip = before.chars().count().saturating_sub(chars);
            let kept: String = before.chars().skip(skip).collect();
            if skip == 0 {
                kept
            } else {
                // Start at a word if one begins in the first third.
                let cut =
                    kept.char_indices().find(|&(_, c)| c.is_whitespace()).map(|(i, c)| i + c.len_utf8());
                let kept = match cut {
                    Some(cut) if kept[..cut].chars().count() * 3 <= chars => kept[cut..].to_string(),
                    _ => kept,
                };
                format!("…{kept}")
            }
        };
        let tail: String = {
            let total = after.chars().count();
            let kept: String = after.chars().take(chars).collect();
            if total <= chars {
                kept
            } else {
                let cut = kept.char_indices().rev().find(|&(_, c)| c.is_whitespace()).map(|(i, _)| i);
                let kept = match cut {
                    Some(cut) if kept[cut..].chars().count() * 3 <= chars => kept[..cut].to_string(),
                    _ => kept,
                };
                format!("{kept}…")
            }
        };
        if head.trim().is_empty() {
            head.clear();
        }
        Self { before: head, matched: line[start..end].to_string(), after: tail }
    }
}

/// `{name}` (or `<name>`) at the start of `text`: the name and what follows.
fn braced(text: &str, open: char, close: char) -> Option<(&str, &str)> {
    let inner = text.strip_prefix(open)?;
    let end = inner.find(close)?;
    let name = &inner[..end];
    let valid = !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    valid.then(|| (name, &inner[end + close.len_utf8()..]))
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

    /// Where folded byte `at` falls in the original (`len`, the original's
    /// length, past the end): for an empty group.
    fn original_position(&self, at: usize, len: usize) -> usize {
        self.origin.get(at).map_or(len, |range| range.0)
    }
}

fn is_tashkeel(c: char) -> bool {
    matches!(c, '\u{0610}'..='\u{061A}' | '\u{064B}'..='\u{065F}' | '\u{0670}' | '\u{06D6}'..='\u{06ED}')
}

/// One character as Arabic normalisation folds it: `None` drops it
/// (tashkeel, tatweel).
fn fold_arabic(c: char, options: &SearchOptions) -> Option<char> {
    if is_tashkeel(c) || c == '\u{0640}' {
        return None;
    }
    Some(match c {
        'أ' | 'إ' | 'آ' | 'ٱ' => 'ا',
        'ى' => 'ي',
        'ة' if options.ta_marbuta_as_ha => 'ه',
        '\u{0660}'..='\u{0669}' => char::from(b'0' + (c as u32 - 0x0660) as u8),
        '\u{06F0}'..='\u{06F9}' => char::from(b'0' + (c as u32 - 0x06F0) as u8),
        other => other,
    })
}

/// A regular expression's Arabic, folded as the text it runs on is.
fn fold_pattern(pattern: &str, options: &SearchOptions) -> String {
    if !options.arabic_normalize {
        return pattern.to_string();
    }
    let mut out = String::with_capacity(pattern.len());
    for c in pattern.chars() {
        if matches!(c, '\u{FB00}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}') {
            out.extend(c.to_string().nfkc().filter_map(|c| fold_arabic(c, options)));
        } else {
            out.extend(fold_arabic(c, options));
        }
    }
    out
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
                let Some(c) = fold_arabic(c, options) else { continue };
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
        // The pattern's Arabic folds as the text does: any alef form, marks or not.
        assert_eq!(find(r"(\d+) أجهزة", "اشتريت 3 اجهزة و٤ أَجهزة", regex.clone()), ["3 اجهزة", "٤ أَجهزة"]);
        assert_eq!(find(r"إلى|٢٠٢٥", "الى عام 2025", regex.clone()), ["الى", "2025"]);
        // Escapes keep their case: \D is not \d.
        assert_eq!(find(r"\D+", "ab12", regex.clone()), ["ab"]);
        assert!(matches!(Query::new("(", regex), Err(SearchError::InvalidRegex(_))));
    }

    #[test]
    fn empty_queries_are_rejected() {
        assert_eq!(Query::new("", SearchOptions::default()).err(), Some(SearchError::Empty));
        // Only tashkeel folds to nothing.
        assert_eq!(Query::new("\u{064E}\u{0650}", SearchOptions::default()).err(), Some(SearchError::Empty));
    }

    #[test]
    fn context_is_cut_at_words_and_marked() {
        let line = "The tenant shall pay SAR 4,500 per month for a term of twelve months in total";
        let start = line.find("4,500").expect("present");
        let context = HitContext::of(line, start, start + 5, 16);
        assert_eq!(context.matched, "4,500");
        assert_eq!(context.before, "…shall pay SAR ");
        assert_eq!(context.after, " per month for…");
        // Short lines are not marked.
        let short = HitContext::of("عقد إيجار", 0, "عقد".len(), 16);
        assert_eq!((short.before.as_str(), short.after.as_str()), ("", " إيجار"));
    }

    fn replaced(query: &str, template: &str, text: &str, options: SearchOptions) -> String {
        let query = Query::new(query, options).expect("valid query");
        let mut out = text.to_string();
        for found in query.matches_in(text).into_iter().rev() {
            let with = query.expand(template, &found, text);
            out.replace_range(found.start..found.end, &with);
        }
        out
    }

    #[test]
    fn regex_replacements_take_their_groups_from_the_original_text() {
        let regex = SearchOptions { regex: true, ..SearchOptions::default() };
        assert_eq!(
            replaced(r"(\w+)@(\w+)", "$2 at $1", "mail bob@example now", regex.clone()),
            "mail example at bob now"
        );
        // Folded, \d matches Arabic-Indic digits; the group gives them as written.
        assert_eq!(
            replaced(r"رقم (\d+)", "العدد $1", "صدر القرار رقم ١٢٣٤ أمس", regex.clone()),
            "صدر القرار العدد ١٢٣٤ أمس"
        );
        // A vocalised word is matched unvocalised and its group keeps its marks.
        assert_eq!(replaced(r"(العلم)", "[$1]", "طلب الْعِلْمِ فريضة", regex.clone()), "طلب [الْعِلْمِ] فريضة");
        // Named groups, the whole match, dollars, and groups that are not there.
        assert_eq!(replaced(r"(?<n>\d+) SAR", "${n} ريال", "pay 4500 SAR", regex.clone()), "pay 4500 ريال");
        assert_eq!(replaced(r"(?<n>\d+) SAR", "$<n>$$", "pay 4500 SAR", regex.clone()), "pay 4500$");
        assert_eq!(replaced(r"\d+", "<$0|$&>", "a 12 b", regex.clone()), "a <12|12> b");
        assert_eq!(replaced(r"(a)|(b)", "[$2]", "ab", regex.clone()), "[][b]");
        assert_eq!(replaced(r"(x)", "$9$x", "x", regex.clone()), "$x");
        // "$10" with one group is group 1, then "0".
        assert_eq!(replaced(r"(x)", "$10", "x", regex), "x0");
    }

    #[test]
    fn literal_replacements_are_taken_as_typed() {
        assert_eq!(replaced("price", "$1 cost", "the price", SearchOptions::default()), "the $1 cost");
        // Normalised matching finds every spelling; each is replaced whole, marks and all.
        assert_eq!(
            replaced("الى", "إلى", "ذهب الى البيت ثم إلَى السوق", SearchOptions::default()),
            "ذهب إلى البيت ثم إلى السوق"
        );
        let whole = SearchOptions { whole_word: true, ..SearchOptions::default() };
        assert_eq!(replaced("art", "craft", "art, part, artist", whole), "craft, part, artist");
    }

    #[test]
    fn matches_map_back_to_whole_original_characters() {
        // "ﻻ" is one character standing for two; matching just "ا" still returns all of it.
        assert_eq!(default_find("ا", "\u{FEFB}"), ["\u{FEFB}"]);
        // Overlapping occurrences are not double-counted.
        assert_eq!(default_find("aa", "aaaa"), ["aa", "aa"]);
    }
}
