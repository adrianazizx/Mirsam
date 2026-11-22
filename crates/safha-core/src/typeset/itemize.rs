//! Itemization: split one bidi paragraph into runs that can each be shaped in a
//! single call — one embedding level (UAX #9), one script, one font, one span.

use std::ops::Range;

use icu_properties::props::{BidiMirroringGlyph, BidiPairedBracketType, GeneralCategory, Script};
use icu_properties::script::ScriptWithExtensions;
use icu_properties::{CodePointMapData, PropertyNamesShort};
use icu_segmenter::GraphemeClusterSegmenter;
use unicode_bidi::Level;

use super::lines::is_hard_break;
use super::{FontFace, Span};

/// A shapeable run, in logical order.
#[derive(Debug, Clone)]
pub(super) struct Item {
    /// Byte range in the full text.
    pub range: Range<usize>,
    pub level: Level,
    pub script: harfrust::Script,
    pub font: usize,
    pub span: usize,
}

/// Itemizes `range` (one bidi paragraph) of `text`. `levels` holds a level per
/// byte of the full text, as unicode-bidi resolves them.
pub(super) fn itemize(
    text: &str,
    range: Range<usize>,
    levels: &[Level],
    spans: &[Span],
    fonts: &[FontFace],
) -> Vec<Item> {
    let para = &text[range.clone()];
    let scripts = resolve_scripts(para);

    // Font per grapheme cluster, so a mark never leaves its base letter's font.
    let mut items: Vec<Item> = Vec::new();
    let mut previous_font: Option<usize> = None;
    let mut boundaries = GraphemeClusterSegmenter::new().segment_str(para).peekable();
    let mut start = boundaries.next().unwrap_or(0);
    let mut span_index = 0;
    for end in boundaries {
        let cluster = &para[start..end];
        let abs = range.start + start;
        while span_index + 1 < spans.len() && spans[span_index].range.end <= abs {
            span_index += 1;
        }
        let span = &spans[span_index];
        let neutral = cluster.chars().all(is_script_neutral);
        let font = choose_font(cluster, span.font, neutral.then_some(previous_font).flatten(), fonts);
        previous_font = Some(font);

        // A cluster can hold several levels only in degenerate input (marks
        // after an isolate control); split at every change of level or script.
        for (offset, ch) in cluster.char_indices() {
            // Line break characters end lines; they are never shaped or drawn.
            if is_hard_break(ch) {
                continue;
            }
            let at = abs + offset;
            let level = levels[at];
            let script = scripts[start + offset];
            let char_end = at + ch.len_utf8();
            match items.last_mut() {
                Some(item)
                    if item.range.end == at
                        && item.level == level
                        && item.script == script
                        && item.font == font
                        && item.span == span_index =>
                {
                    item.range.end = char_end;
                }
                _ => items.push(Item { range: at..char_end, level, script, font, span: span_index }),
            }
        }
        start = end;
    }
    items
}

/// The span's own font if it has every character of the cluster; else the
/// previous cluster's font for spaces and punctuation it can draw (so they do
/// not hop between fonts); else the first font in the set that has them all;
/// else the span's font, which will draw .notdef.
fn choose_font(cluster: &str, preferred: usize, previous: Option<usize>, fonts: &[FontFace]) -> usize {
    let covers =
        |font: &FontFace| cluster.chars().filter(|c| !is_default_ignorable(*c)).all(|c| font.covers(c));
    if let Some(previous) = previous
        && fonts.get(previous).is_some_and(covers)
    {
        return previous;
    }
    if fonts.get(preferred).is_some_and(covers) {
        return preferred;
    }
    fonts.iter().position(covers).unwrap_or(preferred)
}

/// Characters that belong to whatever script surrounds them: Common and
/// Inherited (spaces, digits, punctuation, marks, joiners).
fn is_script_neutral(ch: char) -> bool {
    matches!(CodePointMapData::<Script>::new().get(ch), Script::Common | Script::Inherited)
}

/// Joiners, variation selectors, bidi controls: no glyph of their own, so they
/// must not decide which font a cluster gets.
fn is_default_ignorable(ch: char) -> bool {
    matches!(ch, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}' | '\u{034F}' | '\u{061C}')
        || CodePointMapData::<GeneralCategory>::new().get(ch) == GeneralCategory::Format
}

/// Script per byte of `para`, with Common and Inherited characters resolved to
/// their context, as HarfBuzz and ICU's script runs do:
/// - an Inherited character (a combining mark) takes its base's script;
/// - a Common character takes the current script when its Script_Extensions
///   allow it (the Arabic comma and Arabic-Indic digits stay Arabic) or it has
///   no script of its own;
/// - a closing bracket takes its opening bracket's script;
/// - Common characters before the first real script take that script.
fn resolve_scripts(para: &str) -> Vec<harfrust::Script> {
    let map = CodePointMapData::<Script>::new();
    let extensions = ScriptWithExtensions::new();
    let brackets = CodePointMapData::<BidiMirroringGlyph>::new();

    let mut per_char: Vec<(usize, usize, Option<Script>)> = Vec::new();
    let mut current: Option<Script> = None;
    let mut open_brackets: Vec<(char, Option<Script>)> = Vec::new();
    for (at, ch) in para.char_indices() {
        let own = map.get(ch);
        let bracket = brackets.get(ch);
        let resolved = match own {
            Script::Common | Script::Inherited | Script::Unknown => match bracket.paired_bracket_type {
                BidiPairedBracketType::Open => {
                    open_brackets.push((bracket.mirroring_glyph.unwrap_or(ch), current));
                    current
                }
                BidiPairedBracketType::Close => {
                    match open_brackets.iter().rposition(|(closer, _)| *closer == ch) {
                        Some(i) => {
                            let script = open_brackets[i].1.or(current);
                            open_brackets.truncate(i);
                            script
                        }
                        None => current,
                    }
                }
                _ => current,
            },
            real => match current {
                // e.g. a character shared by several scripts that include the current one
                Some(cur) if cur != real && extensions.has_script(ch, cur) => Some(cur),
                _ => {
                    current = Some(real);
                    Some(real)
                }
            },
        };
        per_char.push((at, ch.len_utf8(), resolved));
    }

    // Leading neutral characters take the first real script that follows.
    let first = per_char.iter().find_map(|(_, _, s)| *s);
    let names = PropertyNamesShort::<Script>::new();
    let to_harfrust = |script: Option<Script>| -> harfrust::Script {
        script
            .or(first)
            .and_then(|s| names.get_locale_script(s))
            .and_then(|tag| harfrust::Script::from_iso15924_tag(harfrust::Tag::from_be_bytes(tag.into_raw())))
            .unwrap_or(harfrust::Script::COMMON)
    };

    let mut out = vec![harfrust::Script::COMMON; para.len()];
    for (at, len, script) in per_char {
        let script = to_harfrust(script);
        out[at..at + len].fill(script);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(text: &str) -> Vec<String> {
        let scripts = resolve_scripts(text);
        text.char_indices()
            .map(|(i, _)| String::from_utf8_lossy(&scripts[i].tag().to_be_bytes()).into_owned())
            .collect()
    }

    #[test]
    fn neutral_characters_follow_their_context() {
        // "2026 مرحبا": leading digits and space take the first real script.
        assert!(tags("2026 مرحبا").iter().all(|t| t == "Arab"));
        // Space and digits after Latin stay Latin; the Arabic word is Arabic.
        let t = tags("iPhone 15 هاتف");
        assert_eq!(&t[..10], &["Latn"; 10]);
        assert_eq!(&t[10..], &["Arab"; 4]);
    }

    #[test]
    fn marks_and_arabic_punctuation_stay_arabic() {
        // Fatha, shadda, Arabic comma, Arabic-Indic digits.
        assert!(tags("قَدَّ، ١٢").iter().all(|t| t == "Arab"));
    }

    #[test]
    fn closing_bracket_takes_its_opening_brackets_script() {
        // "(" opens inside Arabic; ")" follows Latin but closes the Arabic pair.
        let t = tags("نص (Pro) نص");
        let chars: Vec<char> = "نص (Pro) نص".chars().collect();
        let close = chars.iter().position(|c| *c == ')').unwrap_or(0);
        assert_eq!(t[close], "Arab");
        assert_eq!(t[close - 1], "Latn");
    }
}
