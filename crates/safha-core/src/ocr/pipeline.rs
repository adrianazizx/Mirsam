//! The quality pipeline for one page (spec §8, ADR 0033):
//!
//! 1. **Assess and lay out** ([`analyze`]): skew, noise, line strips.
//! 2. **Read** the page with the primary engine (Nuqta 4B). Read the original image,
//!    as measured in ADR 0030; the strips come from the deskewed one.
//! 3. **Order:** number–unit compounds in Arabic lines in logical order
//!    ([`resolve_unit_order`]).
//! 4. **Re-read** when any word is a target (a digit, or under the confidence
//!    threshold):
//!    - the partner (Nuqta 0.8B) reads the page and every strip;
//!    - the primary reads the strips holding a target (all strips when the partner
//!      could not place one);
//!    - a third reader (the platform's OCR) reads the page for numbers.
//! 5. **Reconcile** ([`reconcile`]): confirm, correct where the re-reads agree,
//!    flag the rest.

use std::time::{Duration, Instant};

use super::layout::{LineBox, PageLayout, analyze, deskew};
use super::order::resolve_unit_order;
use super::reconcile::{
    OtherReadings, ReviewedWord, WordStatus, is_target, key, reconcile, words_with_confidence,
};
use super::{OcrEngine, OcrError, OcrImage};

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Words the primary is less sure of than this are re-read (ADR 0030, finding 3).
    pub low_confidence: f32,
    /// Re-read every strip with the primary, not only those holding a target.
    pub reread_all_strips: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { low_confidence: 0.95, reread_all_strips: false }
    }
}

/// The engines of the quality pipeline.
pub struct QualityReader<'a> {
    pub primary: &'a dyn OcrEngine,
    /// Re-reads targets (Nuqta 0.8B). Without it, only the primary's strips re-read.
    pub partner: Option<&'a dyn OcrEngine>,
    /// A reader of another family for numbers (the platform's OCR), when there is one.
    pub third: Option<&'a dyn OcrEngine>,
    pub settings: Settings,
}

/// One page, read and reconciled.
#[derive(Clone, Debug, PartialEq)]
pub struct PageResult {
    /// The page's words in reading order, each with its line, confidence and status.
    pub words: Vec<ReviewedWord>,
    pub layout: PageLayout,
    /// The primary's answer was cut off at its token limit.
    pub truncated: bool,
    /// Engine requests made for this page.
    pub reads: u32,
    pub elapsed: Duration,
}

impl PageResult {
    /// The page's text: one line per text line of the reading.
    pub fn text(&self) -> String {
        let mut lines: Vec<Vec<&str>> = Vec::new();
        for word in &self.words {
            while lines.len() <= word.line {
                lines.push(Vec::new());
            }
            lines[word.line].push(&word.text);
        }
        lines.iter().filter(|l| !l.is_empty()).map(|l| l.join(" ")).collect::<Vec<_>>().join("\n")
    }

    /// The words a person should look at, in reading order.
    pub fn review(&self) -> impl Iterator<Item = &ReviewedWord> {
        self.words.iter().filter(|w| w.needs_review())
    }

    /// Whether the page goes on the pages-needing-review list: an uncertain word, or
    /// the answer cut off. (Corrected and reordered words are listed for a look,
    /// but do not hold the page back.)
    pub fn needs_review(&self) -> bool {
        self.truncated || self.words.iter().any(|w| matches!(w.status, WordStatus::Uncertain { .. }))
    }

    /// The lowest word probability on the page (1 for an empty page).
    pub fn lowest_probability(&self) -> f32 {
        self.words.iter().map(|w| w.probability).fold(1.0, f32::min)
    }
}

/// A page on the review list, with what it holds.
#[derive(Clone, Debug, PartialEq)]
pub struct PageReview {
    pub page: usize,
    /// Words the readers disagree on (the page read was kept).
    pub uncertain: usize,
    /// Words replaced because re-reads agreed on something else.
    pub corrected: usize,
    /// Number–unit compounds turned into logical order.
    pub reordered: usize,
    /// The primary's answer was cut off.
    pub truncated: bool,
    pub lowest_probability: f32,
}

/// The pages-needing-review list (spec §5.8): every page with something to look at,
/// pages that need a decision (uncertain words, cut-off answers) first, then by page.
pub fn review_list<'a>(pages: impl IntoIterator<Item = (usize, &'a PageResult)>) -> Vec<PageReview> {
    let mut list: Vec<PageReview> = pages
        .into_iter()
        .map(|(page, result)| {
            let count = |f: fn(&WordStatus) -> bool| result.words.iter().filter(|w| f(&w.status)).count();
            PageReview {
                page,
                uncertain: count(|s| matches!(s, WordStatus::Uncertain { .. })),
                corrected: count(|s| matches!(s, WordStatus::Corrected { .. })),
                reordered: count(|s| matches!(s, WordStatus::Reordered { .. })),
                truncated: result.truncated,
                lowest_probability: result.lowest_probability(),
            }
        })
        .filter(|r| r.uncertain + r.corrected + r.reordered > 0 || r.truncated)
        .collect();
    list.sort_by_key(|r| (!(r.truncated || r.uncertain > 0), std::cmp::Reverse(r.uncertain), r.page));
    list
}

impl QualityReader<'_> {
    pub fn read_page(&self, image: &OcrImage) -> Result<PageResult, OcrError> {
        let started = Instant::now();
        let layout = analyze(image);
        let page = self.primary.recognize(image)?;
        let mut reads = 1;

        let lines: Vec<String> = page.text.lines().map(str::to_owned).collect();
        let mut words = words_with_confidence(&lines, &page.words);
        apply_order(&mut words, &lines);

        let threshold = self.settings.low_confidence;
        let targets: Vec<usize> = (0..words.len())
            .filter(|&i| is_target(&words[i].text, words[i].probability, threshold))
            .collect();
        if !targets.is_empty() {
            let level = deskew(image, layout.skew_degrees);
            let crops: Vec<OcrImage> = layout.lines.iter().filter_map(|line| crop(&level, line)).collect();
            let mut others = OtherReadings::default();
            let mut place_with: Option<Vec<String>> = None;
            if let Some(partner) = self.partner {
                others.partner_page = Some(ordered(&partner.recognize(image)?.text));
                reads += 1;
                let strips: Vec<String> = crops
                    .iter()
                    .map(|crop| partner.recognize(crop).map(|r| ordered(&r.text)))
                    .collect::<Result<_, _>>()?;
                reads += crops.len() as u32;
                place_with = Some(strips.clone());
                others.partner_strips = strips;
            }
            let wanted = match (&place_with, self.settings.reread_all_strips) {
                (Some(strips), false) => strips_holding(&words, &targets, strips),
                _ => None,
            };
            others.primary_strips = crops
                .iter()
                .enumerate()
                .map(|(i, crop)| {
                    if wanted.as_ref().is_none_or(|w| w.contains(&i)) {
                        reads += 1;
                        self.primary.recognize(crop).map(|r| ordered(&r.text))
                    } else {
                        Ok(String::new())
                    }
                })
                .collect::<Result<_, _>>()?;
            if let Some(third) = self.third
                && targets.iter().any(|&i| words[i].text.chars().any(|c| c.is_numeric()))
            {
                others.third_page = Some(ordered(&third.recognize(image)?.text));
                reads += 1;
            }
            reconcile(&mut words, &others, threshold);
        }
        Ok(PageResult { words, layout, truncated: page.truncated, reads, elapsed: started.elapsed() })
    }
}

/// Applies [`resolve_unit_order`] line by line, marking the words it turned round.
fn apply_order(words: &mut [ReviewedWord], lines: &[String]) {
    let mut first = 0;
    for (line_index, line) in lines.iter().enumerate() {
        let count = words[first..].iter().take_while(|w| w.line == line_index).count();
        let normalized: String = line.split_whitespace().collect::<Vec<_>>().join(" ");
        for fix in resolve_unit_order(&normalized).1 {
            if let Some(word) = words.get_mut(first + fix.word)
                && word.line == line_index
            {
                word.text = fix.logical;
                word.status = WordStatus::Reordered { read: fix.read };
            }
        }
        first += count;
    }
}

/// A re-read's text with the same order rule as the page read, so a strip that
/// says "bit-64" does not undo the page's "64-bit".
fn ordered(text: &str) -> String {
    text.lines().map(|line| resolve_unit_order(line).0).collect::<Vec<_>>().join("\n")
}

fn crop(image: &OcrImage, line: &LineBox) -> Option<OcrImage> {
    image.crop(line.x0, line.y0, line.x1, line.y1).ok()
}

/// The strips (by index) whose partner reading holds a target's context, or `None`
/// when some target could not be placed (then every strip is re-read).
fn strips_holding(words: &[ReviewedWord], targets: &[usize], strips: &[String]) -> Option<Vec<usize>> {
    let page: Vec<String> = words.iter().map(|w| key(&w.text)).collect();
    let mut wanted = Vec::new();
    for &i in targets {
        let mut best: Option<(usize, usize)> = None;
        for (s, strip) in strips.iter().enumerate() {
            let strip: Vec<String> = strip.split_whitespace().map(key).collect();
            let context: usize = [-2i64, -1, 1, 2]
                .iter()
                .filter(|&&d| {
                    let a = i as i64 + d;
                    a >= 0 && (a as usize) < page.len() && strip.contains(&page[a as usize])
                })
                .count();
            if context > 0 && best.is_none_or(|(c, _)| context > c) {
                best = Some((context, s));
            }
        }
        let (_, s) = best?;
        if !wanted.contains(&s) {
            wanted.push(s);
        }
    }
    Some(wanted)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::super::{Recognition, Timings, WordConfidence};
    use super::*;

    /// An engine that answers by image size: the page, or the strip of a given height.
    struct Scripted {
        name: &'static str,
        answers: HashMap<(u32, u32), String>,
        fallback: String,
        calls: RefCell<u32>,
    }

    impl OcrEngine for Scripted {
        fn name(&self) -> &str {
            self.name
        }

        fn recognize(&self, image: &OcrImage) -> Result<Recognition, OcrError> {
            *self.calls.borrow_mut() += 1;
            let text = self
                .answers
                .get(&(image.width(), image.height()))
                .cloned()
                .unwrap_or_else(|| self.fallback.clone());
            let words = text
                .split_whitespace()
                .map(|w| WordConfidence { word: w.into(), probability: 1.0 })
                .collect();
            Ok(Recognition { raw: text.clone(), text, words, truncated: false, timings: Timings::default() })
        }
    }

    fn page_image() -> OcrImage {
        // Two dark bars: two line strips.
        let (w, h) = (300usize, 200usize);
        let mut rgb = vec![255u8; w * h * 3];
        for (top, right) in [(50usize, 270usize), (120, 200)] {
            for y in top..top + 14 {
                for x in 30..right {
                    if x % 12 >= 6 {
                        rgb[(y * w + x) * 3..(y * w + x) * 3 + 3].copy_from_slice(&[10, 10, 10]);
                    }
                }
            }
        }
        OcrImage::from_rgb(w as u32, h as u32, rgb).expect("valid")
    }

    #[test]
    fn a_page_without_targets_is_read_once() {
        let primary =
            Scripted {
                name: "p", answers: HashMap::new(), fallback: "نص عربي واضح".into(), calls: 0.into()
            };
        let partner =
            Scripted { name: "q", answers: HashMap::new(), fallback: "نص".into(), calls: 0.into() };
        let reader = QualityReader {
            primary: &primary,
            partner: Some(&partner),
            third: None,
            settings: Settings::default(),
        };
        let result = reader.read_page(&page_image()).expect("read");
        assert_eq!(result.text(), "نص عربي واضح");
        assert_eq!((result.reads, *partner.calls.borrow()), (1, 0));
        assert!(!result.needs_review());
    }

    #[test]
    fn numbers_are_reread_and_corrected_and_order_is_fixed() {
        let image = page_image();
        let layout = analyze(&image);
        assert_eq!(layout.lines.len(), 2);
        let size = |l: &LineBox| (l.x1 - l.x0, l.y1 - l.y0);
        let (first, second) = (size(&layout.lines[0]), size(&layout.lines[1]));
        let page = "صدر القرار رقم ١٣٣٤ لسنة ٢٠٢٥\nالإصدار 2024 (bit-64)!";
        let primary = Scripted {
            name: "p",
            answers: HashMap::from([
                (first, "صدر القرار رقم ١٢٣٤ لسنة ٢٠٢٥".to_owned()),
                (second, "الإصدار 2024 (bit-64)!".to_owned()),
            ]),
            fallback: page.into(),
            calls: 0.into(),
        };
        let partner = Scripted {
            name: "q",
            answers: HashMap::from([
                (first, "صدر القرار رقم 1234 لسنة 2025".to_owned()),
                (second, "الإصدار 2024 (bit-64)!".to_owned()),
            ]),
            fallback: page.into(),
            calls: 0.into(),
        };
        let reader = QualityReader {
            primary: &primary,
            partner: Some(&partner),
            third: None,
            settings: Settings::default(),
        };
        let result = reader.read_page(&image).expect("read");
        assert_eq!(result.text(), "صدر القرار رقم ١٢٣٤ لسنة ٢٠٢٥\nالإصدار 2024 (64-bit)!");
        let statuses: Vec<&WordStatus> = result.review().map(|w| &w.status).collect();
        assert_eq!(
            statuses,
            [
                &WordStatus::Corrected { read: "١٣٣٤".into() },
                &WordStatus::Reordered { read: "(bit-64)!".into() }
            ]
        );
        assert!(!result.needs_review(), "corrected words are listed, not blocking");
        let list = review_list([(4, &result)]);
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].page, list[0].corrected, list[0].reordered, list[0].uncertain), (4, 1, 1, 0));
        // page + 2 strips by the partner, page + 2 strips by the primary
        assert_eq!(result.reads, 1 + 1 + 2 + 2);
    }
}
