//! Word boxes for text that came without positions (ADR 0031, built in ADR 0035).
//!
//! Nuqta writes the page's text with no positions. The text layer needs a box per
//! word (spec §8). ADR 0031 measured how to get them: put the recogniser's words
//! into detected text lines, guided by a rough reading of each line, then divide
//! each line's ink among its words in visual order, snapping the cuts to real gaps.
//! Here the lines and the guide are the draft reader's ([`super::draft`]): Apple
//! Vision's or Tesseract's line boxes and text, which ADR 0031 named as the next
//! guide to try (the spike used PP-OCRv5's detector and recogniser, which have no
//! Rust runtime yet). Without a draft, the lines are the page's own line strips
//! ([`super::analyze`]) and there is no guide.
//!
//! The method is the spike's M5:
//!
//! 1. **Line order:** rows (either direction) or two columns (either first), the
//!    one whose guide text best matches the recogniser's text.
//! 2. **Words to lines:** a character alignment of the recogniser's text to the
//!    guide votes each word into a line (M1). When the guide matches less than
//!    half of the characters, a dynamic program cuts the words into one run per
//!    line by estimated width instead, keeping words near the lines the guide
//!    puts them on (M4). A line never spans two of the recogniser's lines.
//! 3. **Order in a line:** UAX #9 on the line's words, in the line's direction
//!    (from the column margin it touches, else its first strong letter).
//! 4. **Cutting:** proportional cuts by estimated widths, each moved to an ink gap
//!    by a second dynamic program; each box tightened to its ink.
//!
//! A word whose line came from a guess (no anchor in the guide) is marked
//! `anchored: false`, for the review list (ADR 0031's decision).

use unicode_bidi::{BidiInfo, Level};
use unicode_normalization::UnicodeNormalization as _;

use super::OcrImage;
use super::draft::{DraftPage, PixelBox, first_strong_rtl};
use super::layout::{analyze, ink};

/// A word's box and how it was found.
#[derive(Clone, Debug, PartialEq)]
pub struct BoxedWord {
    /// Index of the word in the recogniser's words.
    pub word: usize,
    pub bounds: PixelBox,
    /// The guide placed the word on its line (otherwise it was placed by width).
    pub anchored: bool,
}

/// A line of the page with its words in logical order.
#[derive(Clone, Debug, PartialEq)]
pub struct BoxedLine {
    pub bounds: PixelBox,
    pub rtl: bool,
    pub words: Vec<BoxedWord>,
}

/// Boxes for `words` (the recogniser's, in reading order; `paragraphs[i]` is the
/// recogniser's line of word `i`) on `image`, using `draft`'s lines as lines and
/// guide when there is one. Lines come in reading order.
pub fn word_boxes(
    image: &OcrImage,
    words: &[String],
    paragraphs: &[usize],
    draft: Option<&DraftPage>,
) -> Vec<BoxedLine> {
    if words.is_empty() {
        return Vec::new();
    }
    let page = Ink::new(image);
    let (segments, texts): (Vec<PixelBox>, Vec<String>) = match draft {
        Some(draft) if !draft.lines.is_empty() => {
            merge_pieces(&draft.lines.iter().map(|l| (l.bounds, l.text.clone())).collect::<Vec<_>>())
        }
        _ => {
            let layout = analyze(image);
            if layout.skew_degrees.abs() >= 0.25 || layout.lines.is_empty() {
                // Strips of a skewed page belong to the deskewed image: one box for the page.
                (vec![[0.0, 0.0, image.width() as f32, image.height() as f32]], vec![String::new()])
            } else {
                layout
                    .lines
                    .iter()
                    .map(|l| ([l.x0 as f32, l.y0 as f32, l.x1 as f32, l.y1 as f32], String::new()))
                    .unzip()
            }
        }
    };
    let page_width = image.width() as f32;

    // 1–2: the line order whose guide matches best, and the words' lines.
    let mut best: Option<Candidate> = None;
    for order in segment_orders(&segments, page_width) {
        let boxes: Vec<PixelBox> = order.iter().map(|&i| segments[i]).collect();
        let guide: Vec<&str> = order.iter().map(|&i| texts[i].as_str()).collect();
        let (lines, ratio, anchors) = assign_by_guide(words, &boxes, &guide);
        if best.as_ref().is_none_or(|(r, ..)| ratio > *r) {
            best = Some((ratio, order, lines, anchors));
        }
    }
    let Some((ratio, order, by_guide, anchors)) = best else { return Vec::new() };
    let boxes: Vec<PixelBox> = order.iter().map(|&i| segments[i]).collect();
    let line_of = if ratio >= 0.5 { by_guide } else { assign_by_width(words, paragraphs, &boxes, &anchors) };

    // 3–4: each line's words, ordered and cut.
    let mut grouped: Vec<Vec<usize>> = vec![Vec::new(); boxes.len()];
    for (i, &line) in line_of.iter().enumerate() {
        grouped[line].push(i);
    }
    let directions = line_directions(&boxes, &grouped, words, page_width);
    grouped
        .iter()
        .enumerate()
        .filter(|(_, members)| !members.is_empty())
        .map(|(s, members)| {
            let texts: Vec<&str> = members.iter().map(|&i| words[i].as_str()).collect();
            let cut = split_line(&page, boxes[s], &texts, directions[s]);
            BoxedLine {
                bounds: boxes[s],
                rtl: directions[s],
                words: members
                    .iter()
                    .zip(cut)
                    .map(|(&word, bounds)| BoxedWord { word, bounds, anchored: anchors[word] == Some(s) })
                    .collect(),
            }
        })
        .collect()
}

/// A line order and the words' lines under it: (guide match, order, line per
/// word, anchor per word).
type Candidate = (f32, Vec<usize>, Vec<usize>, Vec<Option<usize>>);

/// Text for comparing readings: no tashkeel or tatweel, Western digits, lower case.
fn norm(text: &str) -> String {
    text.nfkd()
        .filter(|&c| !is_mark(c) && c != '\u{640}')
        .map(|c| match c {
            '\u{660}'..='\u{669}' => char::from(b'0' + (c as u32 - 0x660) as u8),
            '\u{6F0}'..='\u{6F9}' => char::from(b'0' + (c as u32 - 0x6F0) as u8),
            _ => c,
        })
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_mark(c: char) -> bool {
    matches!(c, '\u{300}'..='\u{36F}' | '\u{610}'..='\u{61A}' | '\u{64B}'..='\u{65F}' | '\u{670}'
        | '\u{6D6}'..='\u{6DC}' | '\u{6DF}'..='\u{6E4}' | '\u{6E7}'..='\u{6E8}' | '\u{6EA}'..='\u{6ED}')
}

fn is_arabic(c: char) -> bool {
    matches!(c, '\u{600}'..='\u{6FF}' | '\u{750}'..='\u{77F}' | '\u{8A0}'..='\u{8FF}')
}

/// A word's estimated width, in units of an average letter.
fn weight(word: &str) -> f32 {
    let w: f32 = word
        .chars()
        .filter(|&c| !is_mark(c))
        .map(|c| {
            if is_arabic(c) {
                1.0
            } else if c.is_numeric() {
                0.95
            } else if c.is_alphabetic() {
                if c.is_uppercase() { 1.0 } else { 0.85 }
            } else {
                0.45
            }
        })
        .sum();
    w.max(0.5)
}

/// The ink mask, for extents and gaps.
struct Ink {
    mask: Vec<bool>,
    width: usize,
    height: usize,
}

impl Ink {
    fn new(image: &OcrImage) -> Self {
        Self { mask: ink(image), width: image.width() as usize, height: image.height() as usize }
    }

    fn clamp(&self, b: PixelBox) -> (usize, usize, usize, usize) {
        let x = |v: f32| (v.round().max(0.0) as usize).min(self.width);
        let y = |v: f32| (v.round().max(0.0) as usize).min(self.height);
        (x(b[0]), y(b[1]), x(b[2]), y(b[3]))
    }

    /// The tight box of the ink inside `b` (grown by `pad`), if there is any.
    fn extent(&self, b: PixelBox, pad: f32) -> Option<PixelBox> {
        let (x0, y0, x1, y1) = self.clamp([b[0] - pad, b[1] - pad, b[2] + pad, b[3] + pad]);
        let (mut l, mut t, mut r, mut bo) = (usize::MAX, usize::MAX, 0, 0);
        for y in y0..y1 {
            let row = &self.mask[y * self.width..(y + 1) * self.width];
            for (x, &on) in row.iter().enumerate().take(x1).skip(x0) {
                if on {
                    l = l.min(x);
                    r = r.max(x + 1);
                    t = t.min(y);
                    bo = bo.max(y + 1);
                }
            }
        }
        (l < r).then_some([l as f32, t as f32, r as f32, bo as f32])
    }

    /// Ink pixels per column of `b`.
    fn columns(&self, b: PixelBox) -> Vec<u32> {
        let (x0, y0, x1, y1) = self.clamp(b);
        (x0..x1).map(|x| (y0..y1).filter(|&y| self.mask[y * self.width + x]).count() as u32).collect()
    }
}

// ---------- line order ----------

/// Boxes grouped into visual rows by vertical overlap, top to bottom.
fn rows(boxes: &[PixelBox]) -> Vec<Vec<usize>> {
    let mut order: Vec<usize> = (0..boxes.len()).collect();
    order.sort_by(|&a, &b| (boxes[a][1] + boxes[a][3]).total_cmp(&(boxes[b][1] + boxes[b][3])));
    let mut out: Vec<Vec<usize>> = Vec::new();
    for i in order {
        let b = boxes[i];
        if let Some(row) = out.last_mut() {
            let top = row.iter().map(|&j| boxes[j][1]).fold(f32::INFINITY, f32::min);
            let bottom = row.iter().map(|&j| boxes[j][3]).fold(f32::NEG_INFINITY, f32::max);
            let overlap = b[3].min(bottom) - b[1].max(top);
            if overlap > 0.5 * (b[3] - b[1]).min(bottom - top) {
                row.push(i);
                continue;
            }
        }
        out.push(vec![i]);
    }
    out
}

/// The draft reader's pieces of one line joined: Apple Vision often cuts a line
/// where its direction changes, and a piece alone can look like a line of the
/// other direction. Pieces on one row less than one line height apart are one
/// line; their texts are joined in the row's reading order (right to left when
/// most of its letters are Arabic).
fn merge_pieces(pieces: &[(PixelBox, String)]) -> (Vec<PixelBox>, Vec<String>) {
    let boxes: Vec<PixelBox> = pieces.iter().map(|(b, _)| *b).collect();
    let (mut out_boxes, mut out_texts) = (Vec::new(), Vec::new());
    for row in rows(&boxes) {
        let mut row = row;
        row.sort_by(|&a, &b| boxes[a][0].total_cmp(&boxes[b][0]));
        let mut groups: Vec<Vec<usize>> = Vec::new();
        for i in row {
            let height = boxes[i][3] - boxes[i][1];
            match groups.last_mut() {
                Some(group)
                    if boxes[i][0] - group.iter().map(|&j| boxes[j][2]).fold(f32::NEG_INFINITY, f32::max)
                        < 2.0 * height =>
                {
                    group.push(i);
                }
                _ => groups.push(vec![i]),
            }
        }
        for group in groups {
            let all: String = group.iter().map(|&i| pieces[i].1.as_str()).collect::<Vec<_>>().join(" ");
            let arabic = all.chars().filter(|&c| is_arabic(c)).count();
            let latin = all.chars().filter(|c| c.is_ascii_alphabetic()).count();
            let mut ordered = group.clone();
            if arabic > latin {
                ordered.reverse();
            }
            out_boxes.push([
                group.iter().map(|&i| boxes[i][0]).fold(f32::INFINITY, f32::min),
                group.iter().map(|&i| boxes[i][1]).fold(f32::INFINITY, f32::min),
                group.iter().map(|&i| boxes[i][2]).fold(f32::NEG_INFINITY, f32::max),
                group.iter().map(|&i| boxes[i][3]).fold(f32::NEG_INFINITY, f32::max),
            ]);
            out_texts.push(ordered.iter().map(|&i| pieces[i].1.as_str()).collect::<Vec<_>>().join(" "));
        }
    }
    (out_boxes, out_texts)
}

/// Candidate reading orders of the lines: rows right to left or left to right,
/// and, when the page has two columns, either column first.
fn segment_orders(boxes: &[PixelBox], page_width: f32) -> Vec<Vec<usize>> {
    let rows = rows(boxes);
    let mut orders = Vec::new();
    for rtl in [true, false] {
        orders.push(
            rows.iter()
                .flat_map(|row| {
                    let mut row = row.clone();
                    row.sort_by(|&a, &b| boxes[a][0].total_cmp(&boxes[b][0]));
                    if rtl {
                        row.reverse();
                    }
                    row
                })
                .collect(),
        );
    }
    let mut centres: Vec<(f32, usize)> =
        boxes.iter().enumerate().map(|(i, b)| ((b[0] + b[2]) / 2.0, i)).collect();
    centres.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut columns: Vec<Vec<usize>> = vec![vec![centres[0].1]];
    for pair in centres.windows(2) {
        if pair[1].0 - pair[0].0 > 0.2 * page_width {
            columns.push(vec![pair[1].1]);
        } else if let Some(column) = columns.last_mut() {
            column.push(pair[1].1);
        }
    }
    if columns.len() == 2 {
        for first in [0, 1] {
            let ordered = if first == 0 { [&columns[0], &columns[1]] } else { [&columns[1], &columns[0]] };
            orders.push(
                ordered
                    .iter()
                    .flat_map(|column| {
                        let mut column = (*column).clone();
                        column.sort_by(|&a, &b| boxes[a][1].total_cmp(&boxes[b][1]));
                        column
                    })
                    .collect(),
            );
        }
    }
    orders
}

// ---------- words to lines ----------

/// Pairs of positions of a longest common subsequence of `a` and `b`. Pages too
/// large for the table get none (the width fit takes over).
fn common_pairs(a: &[char], b: &[char]) -> Vec<(usize, usize)> {
    let (n, m) = (a.len(), b.len());
    if n == 0 || m == 0 || n.saturating_mul(m) > 40_000_000 {
        return Vec::new();
    }
    // Lengths of the LCS of suffixes, one row at a time; directions kept for the walk.
    let mut next = vec![0u32; m + 1];
    let mut take = vec![0u8; n * m]; // 1: match, 2: skip a, 3: skip b
    for i in (0..n).rev() {
        let mut row = vec![0u32; m + 1];
        for j in (0..m).rev() {
            let (value, how) = if a[i] == b[j] {
                (next[j + 1] + 1, 1)
            } else if next[j] >= row[j + 1] {
                (next[j], 2)
            } else {
                (row[j + 1], 3)
            };
            row[j] = value;
            take[i * m + j] = how;
        }
        next = row;
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < n && j < m {
        match take[i * m + j] {
            1 => {
                out.push((i, j));
                i += 1;
                j += 1;
            }
            2 => i += 1,
            _ => j += 1,
        }
    }
    out
}

/// M1: each word's line by a character alignment of the words to the guide's
/// lines (a vote per matched character); words with no vote fill the lines
/// between their anchored neighbours by capacity. Also the share of the words'
/// characters the guide matched, and each word's anchor line.
fn assign_by_guide(
    words: &[String],
    boxes: &[PixelBox],
    guide: &[&str],
) -> (Vec<usize>, f32, Vec<Option<usize>>) {
    let (mut v_chars, mut v_word) = (Vec::new(), Vec::new());
    for (i, word) in words.iter().enumerate() {
        for c in norm(word).chars().filter(|c| !c.is_whitespace()) {
            v_chars.push(c);
            v_word.push(i);
        }
    }
    let (mut g_chars, mut g_line) = (Vec::new(), Vec::new());
    for (s, text) in guide.iter().enumerate() {
        for c in norm(text).chars().filter(|c| !c.is_whitespace()) {
            g_chars.push(c);
            g_line.push(s);
        }
    }
    let pairs = common_pairs(&v_chars, &g_chars);
    let mut votes: Vec<Vec<(usize, u32)>> = vec![Vec::new(); words.len()];
    for &(a, b) in &pairs {
        let tally = &mut votes[v_word[a]];
        match tally.iter_mut().find(|(s, _)| *s == g_line[b]) {
            Some((_, n)) => *n += 1,
            None => tally.push((g_line[b], 1)),
        }
    }
    // A word anchors to a line where at least half its characters matched;
    // fewer is chance (a guide that wrote "bit-64" for "64-bit" matches a
    // bracket and a letter on the next line).
    let mut length = vec![0u32; words.len()];
    for &w in &v_word {
        length[w] += 1;
    }
    let anchors: Vec<Option<usize>> = votes
        .iter()
        .enumerate()
        .map(|(i, tally)| {
            tally
                .iter()
                .max_by_key(|(s, n)| (*n, std::cmp::Reverse(*s)))
                .filter(|(_, n)| 2 * n >= length[i])
                .map(|(s, _)| *s)
        })
        .collect();
    // Anchors must not go back up the page.
    let mut line_of: Vec<Option<usize>> = anchors.clone();
    let mut last = 0;
    for slot in &mut line_of {
        match *slot {
            Some(s) if s < last => *slot = None,
            Some(s) => last = s,
            None => {}
        }
    }
    let widths: Vec<f32> = boxes.iter().map(|b| (b[2] - b[0]).max(1.0)).collect();
    let units: f32 = words.iter().map(|w| weight(w) + 0.5).sum();
    let per_unit = widths.iter().sum::<f32>() / units.max(1.0);
    let capacity: Vec<f32> = widths.iter().map(|w| w / per_unit).collect();
    let mut used = vec![0.0f32; boxes.len()];
    for (i, slot) in line_of.iter().enumerate() {
        if let Some(s) = slot {
            used[*s] += weight(&words[i]) + 0.5;
        }
    }
    let (mut i, mut previous) = (0, 0);
    while i < words.len() {
        if let Some(s) = line_of[i] {
            previous = s;
            i += 1;
            continue;
        }
        let mut j = i;
        while j < words.len() && line_of[j].is_none() {
            j += 1;
        }
        let next = if j < words.len() { line_of[j].unwrap_or(previous) } else { boxes.len() - 1 };
        let mut current = previous;
        for k in i..j {
            if current < next && used[current] >= capacity[current] * 0.92 {
                current += 1;
            }
            line_of[k] = Some(current);
            used[current] += weight(&words[k]) + 0.5;
        }
        i = j;
    }
    let ratio = pairs.len() as f32 / v_chars.len().max(1) as f32;
    (line_of.into_iter().map(|s| s.unwrap_or(0)).collect(), ratio, anchors)
}

/// M4: cuts the words into one run per line so each run's estimated width fits
/// its line (under-filled lines cost less than overfull ones; a line never spans
/// two of the recogniser's lines), with a penalty for moving a word off the line
/// its anchor names.
fn assign_by_width(
    words: &[String],
    paragraphs: &[usize],
    boxes: &[PixelBox],
    anchors: &[Option<usize>],
) -> Vec<usize> {
    const ANCHOR_WEIGHT: f32 = 0.6;
    let (n, m) = (words.len(), boxes.len());
    let weights: Vec<f32> = words.iter().map(|w| weight(w)).collect();
    let mut prefix = vec![0.0f32; n + 1];
    for (i, w) in weights.iter().enumerate() {
        prefix[i + 1] = prefix[i] + w;
    }
    let units = |c: usize, d: usize| prefix[d] - prefix[c] + 0.5 * d.saturating_sub(c + 1) as f32;
    let lengths: Vec<f32> = boxes.iter().map(|b| (b[2] - b[0]).max(1.0)).collect();
    // Anchor penalties: off[i][c..d] counted by prefix sums per line.
    let mut off = vec![vec![0u32; n + 1]; m];
    for (s, row) in off.iter_mut().enumerate() {
        for i in 0..n {
            row[i + 1] = row[i] + u32::from(anchors[i].is_some_and(|a| a != s));
        }
    }
    let mut scale = lengths.iter().sum::<f32>() / units(0, n).max(1e-6);
    let mut result = vec![0; n];
    for _ in 0..3 {
        let mut cost = vec![vec![f32::INFINITY; n + 1]; m + 1];
        let mut back = vec![vec![0usize; n + 1]; m + 1];
        cost[0][0] = 0.0;
        for i in 1..=m {
            let length = lengths[i - 1];
            for d in 0..=n {
                for c in 0..=d {
                    let before = cost[i - 1][c];
                    if !before.is_finite() {
                        continue;
                    }
                    if c < d && paragraphs[c] != paragraphs[d - 1] {
                        continue;
                    }
                    let step = if c == d {
                        2.0
                    } else {
                        let r = (scale * units(c, d) - length) / length;
                        let fit = if r > 0.0 { 4.0 * r * r } else { 0.5 * r * r };
                        fit + ANCHOR_WEIGHT * (off[i - 1][d] - off[i - 1][c]) as f32
                    };
                    if before + step < cost[i][d] {
                        cost[i][d] = before + step;
                        back[i][d] = c;
                    }
                }
            }
        }
        let mut cuts = vec![n];
        let mut d = n;
        for i in (1..=m).rev() {
            d = back[i][d];
            cuts.push(d);
        }
        cuts.reverse();
        for s in 0..m {
            for slot in &mut result[cuts[s]..cuts[s + 1]] {
                *slot = s;
            }
        }
        let mut ratios: Vec<f32> = (0..m)
            .filter(|&s| {
                let (c, d) = (cuts[s], cuts[s + 1]);
                d >= c + 2 && (d == n || paragraphs[d - 1] == paragraphs[d])
            })
            .map(|s| lengths[s] / units(cuts[s], cuts[s + 1]))
            .collect();
        if ratios.is_empty() {
            break;
        }
        ratios.sort_by(f32::total_cmp);
        scale = ratios[ratios.len() / 2];
    }
    result
}

/// Each line's direction: from the margin of its column it touches, else its
/// words' first strong letter (Arabic when there is none).
fn line_directions(
    boxes: &[PixelBox],
    grouped: &[Vec<usize>],
    words: &[String],
    page_width: f32,
) -> Vec<bool> {
    boxes
        .iter()
        .enumerate()
        .map(|(s, b)| {
            let mates: Vec<&PixelBox> = boxes
                .iter()
                .filter(|o| o[2].min(b[2]) - o[0].max(b[0]) > 0.3 * (o[2] - o[0]).min(b[2] - b[0]))
                .collect();
            let right = mates.iter().map(|o| o[2]).fold(f32::NEG_INFINITY, f32::max);
            let left = mates.iter().map(|o| o[0]).fold(f32::INFINITY, f32::min);
            let tolerance = 0.02 * page_width;
            let (touches_right, touches_left) = (b[2] >= right - tolerance, b[0] <= left + tolerance);
            if touches_right && !touches_left {
                return true;
            }
            if touches_left && !touches_right {
                return false;
            }
            let text: Vec<&str> = grouped[s].iter().map(|&i| words[i].as_str()).collect();
            first_strong_rtl(&text.join(" ")).unwrap_or(true)
        })
        .collect()
}

// ---------- cutting a line ----------

/// The words (logical order) left to right: their indices, by UAX #9 on the line.
pub(super) fn visual_order(words: &[&str], rtl: bool) -> Vec<usize> {
    let mut text = String::new();
    let mut starts = Vec::new();
    for (i, word) in words.iter().enumerate() {
        if i > 0 {
            text.push(' ');
        }
        starts.push(text.len()..text.len() + word.len());
        text.push_str(word);
    }
    let bidi = BidiInfo::new(&text, Some(if rtl { Level::rtl() } else { Level::ltr() }));
    let Some(paragraph) = bidi.paragraphs.first() else { return (0..words.len()).collect() };
    let (levels, runs) = bidi.visual_runs(paragraph, paragraph.range.clone());
    let mut visual = Vec::with_capacity(text.len());
    for run in runs {
        let mut offsets: Vec<usize> = run.clone().filter(|&at| text.is_char_boundary(at)).collect();
        if levels[run.start].is_rtl() {
            offsets.reverse();
        }
        visual.extend(offsets);
    }
    // A word sits where its letters are: punctuation at a direction boundary
    // ("Acrobat؟" in an Arabic line) is drawn away from its word.
    let strong = |at: usize| text[at..].chars().next().is_some_and(|c| c.is_alphanumeric());
    let place = |i: usize| {
        visual
            .iter()
            .position(|&at| starts[i].contains(&at) && strong(at))
            .or_else(|| visual.iter().position(|at| starts[i].contains(at)))
            .unwrap_or(usize::MAX)
    };
    let mut order: Vec<usize> = (0..words.len()).collect();
    order.sort_by_key(|&i| place(i));
    order
}

/// Gaps between ink runs of a column profile, inside the line: (centre, width).
fn gaps(profile: &[u32], x0: f32, height: f32) -> Vec<(f32, f32)> {
    let threshold = ((0.02 * height) as u32).max(1);
    let (mut out, mut i, n) = (Vec::new(), 0, profile.len());
    while i < n {
        if profile[i] < threshold {
            let start = i;
            while i < n && profile[i] < threshold {
                i += 1;
            }
            if start > 0 && i < n {
                out.push((x0 + (start + i) as f32 / 2.0, (i - start) as f32));
            }
        } else {
            i += 1;
        }
    }
    out
}

/// One cut per expected boundary, in order: a gap near it (wider is better), or
/// the expected position itself.
fn snap(expected: &[f32], gaps: &[(f32, f32)], length: f32, height: f32) -> Vec<f32> {
    let (k, g) = (expected.len(), gaps.len());
    if k == 0 {
        return Vec::new();
    }
    let reference = 0.3 * height;
    // cost[j][last + 1]: the first j cuts placed, the last gap used `last` (-1: none).
    let mut cost = vec![vec![f32::INFINITY; g + 1]; k + 1];
    let mut back: Vec<Vec<(usize, Option<usize>)>> = vec![vec![(0, None); g + 1]; k + 1];
    cost[0][0] = 0.0;
    for j in 0..k {
        for last in 0..=g {
            let here = cost[j][last];
            if !here.is_finite() {
                continue;
            }
            if here + 1.0 < cost[j + 1][last] {
                cost[j + 1][last] = here + 1.0;
                back[j + 1][last] = (last, None);
            }
            for q in last..g {
                let (centre, width) = gaps[q];
                let step = 60.0 * ((centre - expected[j]) / length).powi(2)
                    + 0.4 * (1.0 - (width / reference).min(1.0));
                if here + step < cost[j + 1][q + 1] {
                    cost[j + 1][q + 1] = here + step;
                    back[j + 1][q + 1] = (last, Some(q));
                }
            }
        }
    }
    let mut state = (0..=g).min_by(|&a, &b| cost[k][a].total_cmp(&cost[k][b])).unwrap_or(0);
    let mut out = vec![0.0; k];
    for j in (1..=k).rev() {
        let (last, q) = back[j][state];
        out[j - 1] = q.map_or(expected[j - 1], |q| gaps[q].0);
        state = last;
    }
    out.sort_by(f32::total_cmp);
    out
}

/// Boxes for a line's `words` (logical order), returned in the same order.
fn split_line(page: &Ink, line: PixelBox, words: &[&str], rtl: bool) -> Vec<PixelBox> {
    let extent = page.extent(line, 2.0).unwrap_or(line);
    let [x0, y0, x1, y1] = extent;
    let order = visual_order(words, rtl);
    let widths: Vec<f32> = order.iter().map(|&i| weight(words[i])).collect();
    let gap = 0.5;
    let total = widths.iter().sum::<f32>() + gap * (widths.len().saturating_sub(1)) as f32;
    let length = x1 - x0;
    let mut expected = Vec::new();
    let mut sum = 0.0;
    for width in &widths[..widths.len().saturating_sub(1)] {
        sum += width;
        expected.push(x0 + length * (sum + gap / 2.0) / total);
        sum += gap;
    }
    let cuts = snap(&expected, &gaps(&page.columns(extent), x0, y1 - y0), length, y1 - y0);
    let mut edges = vec![x0];
    edges.extend(cuts);
    edges.push(x1);
    let mut out = vec![[0.0; 4]; words.len()];
    for (j, &i) in order.iter().enumerate() {
        let cell = [edges[j], y0, edges[j + 1].max(edges[j]), y1];
        out[i] = page.extent(cell, 0.0).unwrap_or(cell);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::draft::DraftLine;
    use super::*;

    /// A white page with dark blocks for words: each `(x0, x1)` on the row at `y`.
    fn page(rows: &[(usize, &[(usize, usize)])]) -> OcrImage {
        let (w, h) = (600usize, 300usize);
        let mut rgb = vec![255u8; w * h * 3];
        for (y, words) in rows {
            for &(a, b) in *words {
                for yy in *y..y + 20 {
                    for x in a..b {
                        rgb[(yy * w + x) * 3..(yy * w + x) * 3 + 3].copy_from_slice(&[0, 0, 0]);
                    }
                }
            }
        }
        OcrImage::from_rgb(w as u32, h as u32, rgb).expect("valid")
    }

    #[test]
    fn words_go_to_their_lines_and_gaps_in_visual_order() {
        // Two Arabic lines, three and two words, against the right margin.
        let image = page(&[(50, &[(300, 360), (380, 470), (490, 560)]), (120, &[(400, 450), (470, 560)])]);
        let words: Vec<String> = ["واحد", "اثنان", "ثلاثة", "أربعة", "خمسة"].map(String::from).to_vec();
        let draft = DraftPage {
            lines: vec![
                DraftLine {
                    text: "واحد اثنان ثلاثه".into(),
                    confidence: 0.5,
                    bounds: [298.0, 48.0, 562.0, 72.0],
                    words: vec![],
                },
                DraftLine {
                    text: "اربعه خمسة".into(),
                    confidence: 0.5,
                    bounds: [398.0, 118.0, 562.0, 142.0],
                    words: vec![],
                },
            ],
        };
        let lines = word_boxes(&image, &words, &[0, 0, 0, 1, 1], Some(&draft));
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|l| l.rtl));
        let boxes: Vec<(usize, [f32; 2])> =
            lines.iter().flat_map(|l| l.words.iter().map(|w| (w.word, [w.bounds[0], w.bounds[2]]))).collect();
        // The first word is the rightmost on its line.
        assert_eq!(
            boxes,
            [
                (0, [490.0, 560.0]),
                (1, [380.0, 470.0]),
                (2, [300.0, 360.0]),
                (3, [470.0, 560.0]),
                (4, [400.0, 450.0])
            ]
        );
        assert!(lines.iter().flat_map(|l| &l.words).all(|w| w.anchored));
    }

    #[test]
    fn without_a_guide_the_widths_decide() {
        let image = page(&[(50, &[(40, 140), (160, 300)]), (120, &[(40, 200)])]);
        let words: Vec<String> = ["Hello", "wonderful", "people"].map(String::from).to_vec();
        let lines = word_boxes(&image, &words, &[0, 0, 0], None);
        let placed: Vec<(usize, usize)> = lines
            .iter()
            .enumerate()
            .flat_map(|(l, line)| line.words.iter().map(move |w| (l, w.word)))
            .collect();
        assert_eq!(placed, [(0, 0), (0, 1), (1, 2)]);
        assert!(!lines[0].rtl);
        assert_eq!(lines[0].words[1].bounds[0], 160.0);
    }

    #[test]
    fn mixed_lines_put_english_left_to_right_inside_arabic() {
        assert_eq!(visual_order(&["اشتريت", "iPhone", "15", "Pro", "بسعر"], true), [4, 1, 2, 3, 0]);
        assert_eq!(visual_order(&["The", "end"], false), [0, 1]);
        assert_eq!(visual_order(&["برنامج", "Adobe", "Acrobat؟", "نعم"], true), [3, 1, 2, 0]);
    }

    #[test]
    fn the_common_subsequence_is_found() {
        let a: Vec<char> = "abcde".chars().collect();
        let b: Vec<char> = "xbydz".chars().collect();
        assert_eq!(common_pairs(&a, &b), [(1, 1), (3, 3)]);
    }
}
