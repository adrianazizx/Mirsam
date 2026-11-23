//! Reading one area of a page again (spec §5.8, ADR 0038).
//!
//! A person draws a rectangle; the quality pipeline reads that area alone,
//! rendered larger than the page pass reads it, with every word re-read. The
//! result takes the place of the area's text: the page's other words stay with
//! their decisions, and another tool's invisible text inside the area is
//! removed by byte range ([`super::layer::Clear::Region`]).
//!
//! **Language and "numbers only"** constrain which reading a word takes, never
//! what the engines are asked: the prompt stays the one the evaluation set
//! measured (ADR 0030). A word that does not fit takes the one reading of
//! another engine that does, and stays flagged as corrected; with none, it
//! keeps its reading and is flagged as uncertain. Nothing is invented.

use serde::{Deserialize, Serialize};

use super::jobs::{OcrWord, PageOcr, QUALITY_SCALE};
use super::layer::{Clear, LayerLine, PageLayer};
use super::reconcile::{ReviewedWord, WordStatus};
use super::review::PageView;

/// The language an area is read as.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Language {
    /// Whatever the page holds.
    #[default]
    Auto,
    /// Arabic script (lines right to left; Latin letters do not fit).
    Arabic,
    /// Latin script (lines left to right; Arabic letters do not fit).
    English,
}

/// What an area's words are expected to be.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Constraint {
    pub language: Language,
    /// Digits and the punctuation of numbers, dates and IDs only.
    pub numbers: bool,
}

fn is_arabic_letter(c: char) -> bool {
    matches!(c, '\u{0620}'..='\u{064A}' | '\u{066E}'..='\u{06D3}' | '\u{06FA}'..='\u{06FF}' | '\u{0750}'..='\u{077F}'
        | '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFC}')
}

fn is_latin_letter(c: char) -> bool {
    c.is_ascii_alphabetic() || matches!(c, '\u{00C0}'..='\u{024F}')
}

/// Characters a number, date or ID is written with besides digits.
fn is_number_mark(c: char) -> bool {
    matches!(
        c,
        '.' | ','
            | '/'
            | '-'
            | '–'
            | ':'
            | '+'
            | '('
            | ')'
            | '%'
            | '#'
            | '\u{066B}'
            | '\u{066C}'
            | '\u{060C}'
            | '\u{066A}'
    )
}

impl Constraint {
    /// Whether `text` is what the area is expected to hold.
    pub fn fits(&self, text: &str) -> bool {
        if self.numbers
            && !(text.chars().any(char::is_numeric)
                && text.chars().all(|c| c.is_numeric() || is_number_mark(c)))
        {
            return false;
        }
        match self.language {
            Language::Auto => true,
            Language::Arabic => !text.chars().any(is_latin_letter),
            Language::English => !text.chars().any(is_arabic_letter),
        }
    }

    fn is_set(&self) -> bool {
        self.numbers || self.language != Language::Auto
    }
}

/// Applies `constraint` to the words of an area's reading: a word that does
/// not fit takes the single fitting reading another engine gave (and is
/// marked corrected), or is flagged uncertain with the readings there were.
pub fn constrain(words: &mut [ReviewedWord], constraint: Constraint) {
    if !constraint.is_set() {
        return;
    }
    for word in words.iter_mut() {
        if constraint.fits(&word.text) {
            continue;
        }
        let readings: Vec<String> = match &word.status {
            WordStatus::Uncertain { alternatives } => alternatives.clone(),
            WordStatus::Corrected { read } | WordStatus::Reordered { read } => vec![read.clone()],
            WordStatus::Unchecked | WordStatus::Confirmed => Vec::new(),
        };
        let mut fitting: Vec<&String> = readings.iter().filter(|r| constraint.fits(r)).collect();
        fitting.dedup();
        match fitting.as_slice() {
            [only] => {
                let read = std::mem::replace(&mut word.text, (*only).clone());
                word.status = WordStatus::Corrected { read };
            }
            _ => word.status = WordStatus::Uncertain { alternatives: readings },
        }
    }
}

/// Pixels per point an area is read at: 300 dpi at least, more for a small
/// area (up to 600 dpi), so its glyphs are larger than the page pass sees them,
/// within 12 megapixels.
pub fn region_scale(rect: [f32; 4]) -> f32 {
    let (w, h) = ((rect[2] - rect[0]).max(1.0), (rect[3] - rect[1]).max(1.0));
    let wanted = (1600.0 / w.max(h)).clamp(QUALITY_SCALE, 600.0 / 72.0);
    wanted.min((12.0e6 / (w * h)).sqrt()).max(1.0)
}

/// An area's result, read from an image of the area alone, moved to where the
/// area is on the page; lines take the forced language's direction.
pub fn shift_page(result: &mut PageOcr, dx: f32, dy: f32, language: Language) {
    let shift = |b: &mut [f32; 4]| {
        b[0] += dx;
        b[1] += dy;
        b[2] += dx;
        b[3] += dy;
    };
    for word in &mut result.words {
        if let Some(b) = word.bounds.as_mut() {
            shift(b);
        }
    }
    for line in &mut result.layer.lines {
        match language {
            Language::Arabic => line.rtl = true,
            Language::English => line.rtl = false,
            Language::Auto => {}
        }
        for w in &mut line.words {
            w.left += dx;
            w.right += dx;
            w.top += dy;
            w.bottom += dy;
        }
    }
}

fn centre_in(b: [f32; 4], rect: [f32; 4]) -> bool {
    let (x, y) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
    x >= rect[0] && x <= rect[2] && y >= rect[1] && y <= rect[3]
}

/// The page with the area `rect` read again: `base` (the page as the document
/// shows it, `None` when it has no result of ours) without its words in the
/// area, and `area`'s words in their place, its lines where the first line
/// taken out was (or by height). The words keep their decisions; the layer
/// clears another tool's invisible text in the area.
pub fn merge_region(base: Option<&PageView>, area: PageOcr, rect: [f32; 4]) -> PageOcr {
    let Some(base) = base else {
        let mut area = area;
        area.layer.clear = Clear::Region { rect };
        area.digest = Some(area.layer.digest());
        return area;
    };
    // The base's lines without the area's words, and where each kept word went.
    let mut kept: Vec<LayerLine> = Vec::new();
    let mut moved: std::collections::HashMap<[u32; 2], [u32; 2]> = std::collections::HashMap::new();
    let mut first_cut: Option<usize> = None;
    for (l, line) in base.layer.lines.iter().enumerate() {
        let mut out = LayerLine { rtl: line.rtl, words: Vec::new() };
        let mut cut = false;
        for (p, w) in line.words.iter().enumerate() {
            if centre_in([w.left, w.top, w.right, w.bottom], rect) {
                cut = true;
                continue;
            }
            moved.insert([l as u32, p as u32], [kept.len() as u32, out.words.len() as u32]);
            out.words.push(w.clone());
        }
        if cut && first_cut.is_none() {
            // A line partly in the area: the area's lines go before what is left
            // of it when the area is on the side it starts from, else after.
            let after = !out.words.is_empty() && {
                let centre = |l: f32, r: f32| (l + r) / 2.0;
                let left =
                    out.words.iter().map(|w| centre(w.left, w.right)).sum::<f32>() / out.words.len() as f32;
                let area_x = centre(rect[0], rect[2]);
                if line.rtl { area_x < left } else { area_x > left }
            };
            first_cut = Some(kept.len() + usize::from(after));
        }
        if !out.words.is_empty() {
            kept.push(out);
        }
    }
    let area_top = area.layer.lines.first().and_then(|l| l.words.first()).map_or(rect[1], |w| w.top);
    let at = first_cut.unwrap_or_else(|| {
        kept.iter()
            .position(|line| line.words.iter().map(|w| w.top).fold(f32::INFINITY, f32::min) > area_top)
            .unwrap_or(kept.len())
    });
    let added = area.layer.lines.len() as u32;
    let mut lines = kept;
    let tail = lines.split_off(at);
    lines.extend(area.layer.lines.iter().cloned());
    lines.extend(tail);

    let place = |[l, p]: [u32; 2]| if l as usize >= at { [l + added, p] } else { [l, p] };
    let mut words: Vec<(Option<[u32; 2]>, OcrWord)> = Vec::new();
    for word in &base.words {
        let inside = word.bounds.is_some_and(|b| centre_in(b, rect));
        if inside {
            continue;
        }
        let mut word = word.clone();
        word.at = word.at.and_then(|a| moved.get(&a).copied()).map(place);
        words.push((word.at, word));
    }
    for word in &area.words {
        let mut word = word.clone();
        word.at = word.at.map(|[l, p]| [l + at as u32, p]);
        words.push((word.at, word));
    }
    // Reading order: by place in the layer; words without one keep theirs at the end.
    words.sort_by_key(|(at, _)| at.map_or((u32::MAX, u32::MAX), |[l, p]| (l, p)));
    let layer = PageLayer {
        page: base.page,
        quality: area.layer.quality,
        lines,
        placement: area.layer.placement,
        clear: Clear::Region { rect },
    };
    PageOcr {
        page: base.page,
        digest: Some(layer.digest()),
        layer,
        words: words.into_iter().map(|(_, w)| w).collect(),
        truncated: base.truncated || area.truncated,
        engine: area.engine,
        seconds: area.seconds,
        checked: base.checked,
        decisions: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::layer::{LayerQuality, LayerWord};
    use super::super::review::WordReview;
    use super::*;

    fn reviewed(text: &str, status: WordStatus) -> ReviewedWord {
        ReviewedWord { text: text.into(), probability: 0.9, status, line: 0 }
    }

    #[test]
    fn numbers_only_chooses_among_readings_and_never_invents() {
        let numbers = Constraint { language: Language::Auto, numbers: true };
        assert!(
            numbers.fits("1234")
                && numbers.fits("١٢٣٤")
                && numbers.fits("12/05/2025")
                && numbers.fits("+966")
        );
        assert!(!numbers.fits("I234") && !numbers.fits("-") && !numbers.fits("رقم"));
        let mut words = vec![
            reviewed("1234", WordStatus::Confirmed),
            reviewed("I234", WordStatus::Uncertain { alternatives: vec!["1234".into(), "l234".into()] }),
            reviewed("O5", WordStatus::Confirmed),
            reviewed("S0", WordStatus::Uncertain { alternatives: vec!["50".into(), "60".into()] }),
        ];
        constrain(&mut words, numbers);
        assert_eq!(words[0].status, WordStatus::Confirmed);
        assert_eq!(
            (words[1].text.as_str(), &words[1].status),
            ("1234", &WordStatus::Corrected { read: "I234".into() })
        );
        // No reading fits: kept as read, flagged.
        assert_eq!(
            (words[2].text.as_str(), &words[2].status),
            ("O5", &WordStatus::Uncertain { alternatives: vec![] })
        );
        // Two readings fit: the readers still disagree.
        assert_eq!(words[3].text, "S0");
        assert!(matches!(words[3].status, WordStatus::Uncertain { .. }));
    }

    #[test]
    fn a_forced_language_flags_words_of_the_other_script() {
        let arabic = Constraint { language: Language::Arabic, numbers: false };
        let english = Constraint { language: Language::English, numbers: false };
        assert!(arabic.fits("العقد") && arabic.fits("٢٠٢٥") && arabic.fits("(١)"));
        assert!(!arabic.fits("Lease") && english.fits("Lease") && !english.fits("العقد"));
        let mut words = vec![
            reviewed("العقد", WordStatus::Unchecked),
            reviewed("iPhone", WordStatus::Corrected { read: "ايفون".into() }),
        ];
        constrain(&mut words, arabic);
        assert_eq!(words[0].status, WordStatus::Unchecked);
        assert_eq!(
            (words[1].text.as_str(), &words[1].status),
            ("ايفون", &WordStatus::Corrected { read: "iPhone".into() })
        );
    }

    #[test]
    fn small_areas_are_read_larger() {
        assert!((region_scale([0.0, 0.0, 600.0, 800.0]) - QUALITY_SCALE).abs() < 1e-3);
        assert!((region_scale([10.0, 10.0, 110.0, 30.0]) - 600.0 / 72.0).abs() < 1e-3);
        let mid = region_scale([0.0, 0.0, 300.0, 40.0]);
        assert!(mid > QUALITY_SCALE && mid < 600.0 / 72.0);
    }

    fn lw(text: &str, b: [f32; 4]) -> LayerWord {
        LayerWord { text: text.into(), left: b[0], top: b[1], right: b[2], bottom: b[3] }
    }

    fn ow(text: &str, b: [f32; 4], at: [u32; 2]) -> OcrWord {
        OcrWord {
            text: text.into(),
            probability: 0.99,
            status: WordStatus::Unchecked,
            bounds: Some(b),
            placed_by_width: false,
            at: Some(at),
            review: WordReview::Open,
        }
    }

    #[test]
    fn an_area_takes_its_words_place_and_the_rest_keeps_its_decisions() {
        // Three lines; the middle line's number is read again.
        let (a, b, c, d) = (
            [300.0, 100.0, 400.0, 112.0],
            [100.0, 130.0, 160.0, 142.0],
            [200.0, 130.0, 260.0, 142.0],
            [300.0, 160.0, 400.0, 172.0],
        );
        let mut first = ow("العقد", a, [0, 0]);
        first.review = WordReview::Accepted;
        let base = PageView {
            page: 1,
            words: vec![first, ow("رقم", c, [1, 0]), ow("I234", b, [1, 1]), ow("Lease", d, [2, 0])],
            layer: PageLayer {
                page: 1,
                quality: LayerQuality::Final,
                lines: vec![
                    LayerLine { rtl: true, words: vec![lw("العقد", a)] },
                    LayerLine { rtl: true, words: vec![lw("رقم", c), lw("I234", b)] },
                    LayerLine { rtl: false, words: vec![lw("Lease", d)] },
                ],
                placement: [1.0, 0.0, 0.0, 1.0, 0.0, 842.0],
                clear: Clear::None,
            },
            checked: false,
            truncated: false,
        };
        let area_box = [98.0, 128.0, 165.0, 145.0];
        let area = PageOcr {
            page: 1,
            layer: PageLayer {
                page: 1,
                quality: LayerQuality::Final,
                lines: vec![LayerLine { rtl: true, words: vec![lw("1234", [101.0, 130.0, 159.0, 142.0])] }],
                placement: [1.0, 0.0, 0.0, 1.0, 0.0, 842.0],
                clear: Clear::None,
            },
            words: vec![ow("1234", [101.0, 130.0, 159.0, 142.0], [0, 0])],
            truncated: false,
            engine: "e".into(),
            seconds: 1.0,
            checked: false,
            decisions: vec![],
            digest: None,
        };
        let merged = merge_region(Some(&base), area, area_box);
        let texts: Vec<Vec<&str>> =
            merged.layer.lines.iter().map(|l| l.words.iter().map(|w| w.text.as_str()).collect()).collect();
        assert_eq!(texts, [vec!["العقد"], vec!["رقم"], vec!["1234"], vec!["Lease"]]);
        assert_eq!(merged.layer.clear, Clear::Region { rect: area_box });
        let words: Vec<(&str, Option<[u32; 2]>)> =
            merged.words.iter().map(|w| (w.text.as_str(), w.at)).collect();
        assert_eq!(
            words,
            [("العقد", Some([0, 0])), ("رقم", Some([1, 0])), ("1234", Some([2, 0])), ("Lease", Some([3, 0]))]
        );
        assert_eq!(merged.words[0].review, WordReview::Accepted);
        assert_eq!(merged.digest, Some(merged.layer.digest()));
        // Every word's place points at its own text.
        for w in &merged.words {
            let [l, p] = w.at.expect("placed");
            assert_eq!(merged.layer.lines[l as usize].words[p as usize].text, w.text);
        }
    }
}
