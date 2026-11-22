//! Geometry → reading order. Pure functions; no PDFium.

use std::collections::HashMap;

use unicode_bidi::{BidiClass, Level, ParagraphBidiInfo, bidi_class};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::canonical_combining_class;

use super::{Direction, PageText, Rect, TextBlock, TextLine, TextQuality, TextSpan};

/// One character as PDFium reports it (page space, points, y up).
///
/// `' '` glyphs stand for space characters the PDF really contains (not ones
/// PDFium generated); they carry no reliable position but their place in the
/// content stream is used.
#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    /// PDFium's character index (content-stream order).
    pub index: usize,
    pub ch: char,
    /// Loose box: the glyph's advance horizontally, the font's ascent/descent vertically.
    pub left: f32,
    pub right: f32,
    pub bottom: f32,
    pub top: f32,
    pub origin_x: f32,
    pub origin_y: f32,
    /// Font size in points after the text matrix.
    pub size: f32,
}

impl Glyph {
    fn center_x(&self) -> f32 {
        (self.left + self.right) / 2.0
    }

    fn rect(&self) -> Rect {
        Rect { left: self.left, bottom: self.bottom, right: self.right, top: self.top }
    }
}

/// A space glyph as drawn: no ink, so only a position (see
/// `glyphs::collect_space_marks`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpaceMark {
    pub x: f32,
    /// Baseline.
    pub y: f32,
    pub size: f32,
}

/// Thresholds, in ems of the font size unless noted. Calibrated on the fixture
/// corpus (`cargo run -p safha-core --example fixture_report`).
#[derive(Debug, Clone, Copy)]
struct Params {
    /// Glyphs whose baselines differ by at most this share a line. Has to absorb
    /// the vertical shifts of cursive attachment in Naskh fonts (up to ~0.45 em).
    line_tolerance: f32,
    /// A gap at least this wide splits a line into segments (columns, table cells).
    segment_gap: f32,
    /// Word-space gap between two Arabic glyphs. Arabic needs a wider threshold
    /// than Latin: non-joining letters (alef, dal, ra, waw…) leave real gaps
    /// inside words, and Naskh spaces are wide.
    arabic_word_gap: f32,
    /// The same when either glyph is shifted by cursive attachment, which opens
    /// false gaps of up to ~0.22 em inside words.
    arabic_displaced_word_gap: f32,
    /// A glyph this far off its line's baseline counts as shifted.
    displaced: f32,
    /// Word-space gap anywhere else (Latin, digits, script boundaries).
    word_gap: f32,
    /// On pages that draw space glyphs, a gap between Arabic glyphs this wide is
    /// a space even without one (justified text).
    marked_arabic_gap: f32,
    /// Largest baseline distance from a block's first line to its second.
    first_pitch: f32,
    /// Later lines may be at most this factor further apart than the first pair.
    pitch_growth: f32,
    /// A line edge within this distance of its column's edge counts as aligned to it.
    align_tolerance: f32,
}

const PARAMS: Params = Params {
    line_tolerance: 0.55,
    segment_gap: 1.0,
    arabic_word_gap: 0.25,
    arabic_displaced_word_gap: 0.28,
    displaced: 0.15,
    word_gap: 0.15,
    marked_arabic_gap: 0.6,
    first_pitch: 2.2,
    pitch_growth: 1.3,
    align_tolerance: 0.6,
};

/// Rebuilds the reading-order text of a page from its glyphs and the space
/// glyphs it draws.
pub fn analyze(glyphs: &[Glyph], spaces: &[SpaceMark]) -> PageText {
    let p = PARAMS;
    let spaces = with_spaces_drawn_in_runs(glyphs, spaces);
    let spaces = spaces.as_slice();
    let units = build_units(glyphs);
    if units.is_empty() {
        return PageText { blocks: Vec::new(), direction: Direction::Ltr, quality: TextQuality::NoText };
    }
    let lines = build_lines(&units, p);
    let quality = assess_quality(&units, &lines);

    let mut segment_members: Vec<(Vec<usize>, f32)> = Vec::new();
    for line in &lines {
        for members in split_segments(&units, line, p) {
            segment_members.push((members, line.baseline));
        }
    }
    // When a page draws space glyphs, they alone decide word breaks next to
    // Arabic: in Naskh text, gaps inside words can be wider than word spaces,
    // and PDFium attaches the page's space characters to the wrong glyphs when
    // the text is marked as stored in visual order. Other pages fall back to
    // stream adjacency and gap width.
    let marked = spaces.len() * 33 >= units.len();
    let forced = if marked {
        spaces_from_marks(spaces, &units, &segment_members, p)
    } else {
        spaces_from_stream(glyphs, &units, &segment_members)
    };

    let segments: Vec<Segment> = segment_members
        .iter()
        .enumerate()
        .map(|(i, (members, baseline))| {
            let forced = forced.get(&i).map(Vec::as_slice).unwrap_or_default();
            make_segment(&units, members, *baseline, forced, marked, p)
        })
        .collect();
    let mut blocks = build_blocks(segments, p);
    assign_directions(&mut blocks, p);
    let direction = page_direction(&blocks);
    let ordered = order_blocks(blocks, direction);

    PageText { blocks: ordered.into_iter().map(finish_block).collect(), direction, quality }
}

/// Adds the space glyphs drawn inside a longer run to the page's space marks.
///
/// Chrome draws each word space as its own text object, which
/// `glyphs::collect_space_marks` finds. Chrome on Linux sometimes draws the
/// space in the same `Tj` as the glyph before it (`<00370001>`), wherever the
/// space's offset equals that glyph's advance. PDFium reports such a space as
/// a full-size character with a real advance, at the right place. The spaces
/// it reports unreliably (ADR 0008) are the zero-width ones it attaches to
/// standalone space objects, which this skips.
fn with_spaces_drawn_in_runs(glyphs: &[Glyph], spaces: &[SpaceMark]) -> Vec<SpaceMark> {
    let in_runs = glyphs.iter().filter(|g| g.ch == ' ' && g.size > 1.5 && g.right - g.left >= 0.1 * g.size);
    spaces
        .iter()
        .copied()
        .chain(in_runs.map(|g| SpaceMark { x: g.left, y: g.origin_y, size: g.size }))
        .collect()
}

// ---------------------------------------------------------------- units

fn is_mark(c: char) -> bool {
    canonical_combining_class(c) != 0
}

fn is_rtl_char(c: char) -> bool {
    matches!(bidi_class(c), BidiClass::R | BidiClass::AL)
}

fn is_arabic_letter(c: char) -> bool {
    matches!(c, '\u{0620}'..='\u{064A}' | '\u{066E}'..='\u{06D3}' | '\u{0750}'..='\u{077F}')
}

/// One drawn glyph as text: a letter with its marks, or several letters PDFium
/// split out of one ligature glyph.
#[derive(Debug, Clone)]
struct Unit {
    /// Member glyph ids.
    glyphs: Vec<usize>,
    /// The members' PDFium character indices (what in-place editing traces
    /// back to the content stream, ADR 0023).
    chars: Vec<usize>,
    /// The unit's text in logical order.
    text: String,
    /// The character that decides the unit's bidi class (first base letter).
    key: char,
    origin_x: f32,
    origin_y: f32,
    size: f32,
    /// Lowest stream index among the members.
    first_index: usize,
    /// Horizontal extent used for gaps: member boxes and the origin.
    left: f32,
    right: f32,
    rect: Rect,
}

impl Unit {
    fn center_x(&self) -> f32 {
        (self.left + self.right) / 2.0
    }
}

/// Groups glyphs into units.
///
/// Characters drawn from the same origin, close together in the stream, are
/// one glyph: Chrome draws a letter's marks from the letter's own origin, and
/// a ligature with a multi-character mapping yields several characters at one
/// origin. PDFium reports the letters of such an RTL mapping reversed, so they
/// are put back. Marks that share no origin with a letter (CoreText positions
/// them independently) are attached to the letter they sit on.
fn build_units(glyphs: &[Glyph]) -> Vec<Unit> {
    const STREAM_WINDOW: usize = 8;
    let key = |g: &Glyph| ((g.origin_x * 100.0).round() as i64, (g.origin_y * 100.0).round() as i64);

    let mut order: Vec<usize> = (0..glyphs.len()).filter(|&g| glyphs[g].ch != ' ').collect();
    order.sort_by_key(|&g| glyphs[g].index);

    // Same-origin groups, split where members are far apart in the stream.
    let mut open: HashMap<(i64, i64), usize> = HashMap::new();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for &g in &order {
        let k = key(&glyphs[g]);
        let joins = open.get(&k).copied().filter(|&group| {
            groups[group].last().is_some_and(|&last| glyphs[g].index - glyphs[last].index <= STREAM_WINDOW)
        });
        match joins {
            Some(group) => groups[group].push(g),
            None => {
                open.insert(k, groups.len());
                groups.push(vec![g]);
            }
        }
    }

    let reverse_ligatures = page_reverses_ligatures(glyphs, &groups);
    let mut units = Vec::new();
    let mut loose_marks = Vec::new();
    for group in groups {
        let (marks, bases): (Vec<usize>, Vec<usize>) = group.iter().partition(|&&g| is_mark(glyphs[g].ch));
        if bases.is_empty() {
            loose_marks.extend(marks);
        } else {
            units.push(make_unit(glyphs, bases, marks, reverse_ligatures));
        }
    }

    // Attach independently positioned marks to the letter they sit on.
    let mut extra: Vec<Vec<usize>> = vec![Vec::new(); units.len()];
    let mut orphans = Vec::new();
    for m in loose_marks {
        let mark = &glyphs[m];
        let reach_y = 1.2 * mark.size;
        let near: Vec<usize> =
            (0..units.len()).filter(|&u| (units[u].origin_y - mark.origin_y).abs() <= reach_y).collect();
        let containing = near
            .iter()
            .copied()
            .filter(|&u| units[u].left - 0.1 <= mark.center_x() && mark.center_x() <= units[u].right + 0.1)
            .min_by(|&a, &b| {
                let da = (units[a].origin_y - mark.origin_y).abs();
                let db = (units[b].origin_y - mark.origin_y).abs();
                da.total_cmp(&db).then(
                    (units[a].center_x() - mark.center_x())
                        .abs()
                        .total_cmp(&(units[b].center_x() - mark.center_x()).abs()),
                )
            });
        let nearest = || {
            near.iter()
                .copied()
                .filter(|&u| (units[u].center_x() - mark.center_x()).abs() <= 0.6 * mark.size)
                .min_by(|&a, &b| {
                    (units[a].center_x() - mark.center_x())
                        .abs()
                        .total_cmp(&(units[b].center_x() - mark.center_x()).abs())
                })
        };
        match containing.or_else(nearest) {
            Some(u) => extra[u].push(m),
            // A mark with nothing under it is kept as text in its own right.
            None => orphans.push(m),
        }
    }
    let mut units: Vec<Unit> = units
        .into_iter()
        .zip(extra)
        .map(|(unit, extra)| {
            if extra.is_empty() {
                return unit;
            }
            let (marks, bases): (Vec<usize>, Vec<usize>) =
                unit.glyphs.iter().partition(|&&g| is_mark(glyphs[g].ch));
            let mut marks = marks;
            marks.extend(extra);
            marks.sort_by_key(|&m| glyphs[m].index);
            make_unit(glyphs, bases, marks, reverse_ligatures)
        })
        .collect();
    units.extend(orphans.into_iter().map(|m| make_unit(glyphs, vec![m], Vec::new(), reverse_ligatures)));
    drop_duplicates(units)
}

/// Drops units PDFium reports twice.
///
/// With some producers (Chrome with Amiri, or with Naskh in table cells)
/// PDFium reports a cluster both from its `/ActualText` and from one of its
/// glyphs' own mapping (sometimes the dot): the same letter and marks again, a
/// drawn on top of the first. Genuine double letters sit side by side and do
/// not overlap.
///
/// The two are usually a few characters apart in PDFium's stream, but not on a
/// rotated page, where PDFium orders characters by their displayed position
/// and a dot glyph drawn above its letter can land in another run. So
/// duplicates are found by position alone: a sweep over units sorted by their
/// left edge, keeping the one PDFium listed first.
fn drop_duplicates(mut units: Vec<Unit>) -> Vec<Unit> {
    let mut order: Vec<usize> = (0..units.len()).collect();
    order.sort_by(|&a, &b| units[a].left.total_cmp(&units[b].left));

    let mut keep = vec![true; units.len()];
    // (kept, dropped): the kept unit's box grows to cover the duplicate, which
    // is often the letter's dots, so a selection highlight covers the whole
    // letter. Only the display box grows; gaps keep using the unit's own
    // extent, so the extracted text does not change.
    let mut merged = Vec::new();
    for (i, &a) in order.iter().enumerate() {
        for &b in order[i + 1..].iter().take_while(|&&b| units[b].left <= units[a].right) {
            if !keep[a] {
                break;
            }
            let (ua, ub) = (&units[a], &units[b]);
            let narrower = (ua.right - ua.left).min(ub.right - ub.left).max(0.01);
            // A duplicate can come from the letter's dot glyph, drawn ~0.3 em higher.
            let same_line = (ua.origin_y - ub.origin_y).abs() <= 0.55 * ua.size;
            if keep[b]
                && ua.text == ub.text
                && same_line
                && overlap((ua.left, ua.right), (ub.left, ub.right)) >= 0.5 * narrower
            {
                let (kept, dropped) = if ub.first_index < ua.first_index { (b, a) } else { (a, b) };
                keep[dropped] = false;
                merged.push((kept, dropped));
            }
        }
    }
    for (kept, dropped) in merged {
        units[kept].rect = units[kept].rect.union(units[dropped].rect);
    }
    units.into_iter().zip(keep).filter_map(|(unit, keep)| keep.then_some(unit)).collect()
}

/// Arabic letters that join only to the letter before them (or to neither).
/// Such a letter can never be the first letter of a ligature.
fn joins_backward_only(c: char) -> bool {
    matches!(
        c,
        '\u{0621}'..='\u{0625}'
            | '\u{0627}'
            | '\u{0629}'
            | '\u{062F}'..='\u{0632}'
            | '\u{0648}'
            | '\u{0671}'..='\u{0673}'
            | '\u{0675}'..='\u{0677}'
            | '\u{0688}'..='\u{0699}'
            | '\u{06C0}'
            | '\u{06C3}'..='\u{06CB}'
            | '\u{06CD}'
            | '\u{06CF}'
            | '\u{06D2}'
            | '\u{06D3}'
    )
}

/// Which way round a multi-letter Arabic glyph's letters arrive.
///
/// The letters of one glyph come from its Unicode mapping. Some producers'
/// pages (Chrome's, which mark their Arabic `/ReversedChars`) make PDFium
/// report them reversed ("ين" as "ني"); others arrive in order. A letter that
/// only joins backwards cannot start a ligature, so any glyph whose first
/// reported letter is one ("رص", "ال") is reversed, and any whose last is one
/// ("صر") is in order. The page's glyphs vote; a glyph that is itself
/// unambiguous follows its own evidence. Lam-alef does not vote: it arrives in
/// order even on pages that reverse every other ligature.
fn page_reverses_ligatures(glyphs: &[Glyph], groups: &[Vec<usize>]) -> bool {
    let is_lam_alef = |group: &Vec<usize>| {
        let letters: Vec<char> =
            group.iter().map(|&g| glyphs[g].ch).filter(|&c| is_arabic_letter(c)).collect();
        letters.len() == 2
            && letters.contains(&'\u{0644}')
            && letters
                .iter()
                .any(|&c| matches!(c, '\u{0622}' | '\u{0623}' | '\u{0625}' | '\u{0627}' | '\u{0671}'))
    };
    let (mut reversed, mut ordered) = (0, 0);
    for group in groups.iter().filter(|group| !is_lam_alef(group)) {
        match letter_order(glyphs, group) {
            LetterOrder::Reversed => reversed += 1,
            LetterOrder::InOrder => ordered += 1,
            LetterOrder::NotLigature | LetterOrder::Unknown => {}
        }
    }
    // With no evidence either way, reversed: PDFium turns a glyph's mapping,
    // written in logical order, into visual order for right-to-left letters,
    // as it does on Chrome's pages and on the pages this app writes (ADR 0021).
    reversed >= ordered
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LetterOrder {
    /// The first reported letter only joins backwards: reversed.
    Reversed,
    /// The last reported letter only joins backwards: in order.
    InOrder,
    /// Neither letter at either end joins forwards, so this is no ligature but
    /// a cluster the producer labelled as a whole (`/ActualText`), which
    /// arrives in order.
    NotLigature,
    /// The joining rules cannot tell.
    Unknown,
}

fn letter_order(glyphs: &[Glyph], group: &[usize]) -> LetterOrder {
    let letters: Vec<char> = group.iter().map(|&g| glyphs[g].ch).filter(|&c| is_arabic_letter(c)).collect();
    let (Some(&first), Some(&last)) = (letters.first(), letters.last()) else { return LetterOrder::Unknown };
    if letters.len() < 2 {
        return LetterOrder::Unknown;
    }
    match (joins_backward_only(first), joins_backward_only(last)) {
        (true, false) => LetterOrder::Reversed,
        (false, true) => LetterOrder::InOrder,
        (true, true) => LetterOrder::NotLigature,
        (false, false) => LetterOrder::Unknown,
    }
}

/// `bases` and `marks` in stream order.
fn make_unit(glyphs: &[Glyph], mut bases: Vec<usize>, marks: Vec<usize>, reverse_ligatures: bool) -> Unit {
    let first = bases[0];
    if bases.len() > 1 && bases.iter().all(|&b| is_rtl_char(glyphs[b].ch)) {
        let reverse = match letter_order(glyphs, &bases) {
            LetterOrder::Reversed => true,
            LetterOrder::InOrder | LetterOrder::NotLigature => false,
            LetterOrder::Unknown => reverse_ligatures,
        };
        if reverse {
            bases.reverse();
        }
    }
    let mut raw = String::new();
    raw.extend(bases.iter().map(|&b| glyphs[b].ch));
    raw.extend(marks.iter().map(|&m| glyphs[m].ch));
    // Presentation forms (lam-alef ligatures, positional forms) copy and search
    // as their base letters (spec 6.1); NFC puts the marks in canonical order.
    let text: String =
        if raw.chars().any(is_presentation_form) { raw.nfkc().collect() } else { raw.nfc().collect() };
    let key = text.chars().find(|c| !is_mark(*c)).unwrap_or(glyphs[first].ch);

    let members: Vec<usize> = bases.iter().chain(&marks).copied().collect();
    let mut rect = glyphs[first].rect();
    let (mut left, mut right) = (glyphs[first].origin_x, glyphs[first].origin_x);
    let mut size: f32 = 0.0;
    for &g in &members {
        rect = rect.union(glyphs[g].rect());
        left = left.min(glyphs[g].left);
        right = right.max(glyphs[g].right);
        size = size.max(glyphs[g].size);
    }
    let first_index = members.iter().map(|&g| glyphs[g].index).min().unwrap_or(0);
    Unit {
        chars: members.iter().map(|&g| glyphs[g].index).collect(),
        glyphs: members,
        text,
        key,
        first_index,
        origin_x: glyphs[first].origin_x,
        origin_y: glyphs[first].origin_y,
        size,
        left,
        right,
        rect,
    }
}

fn is_presentation_form(c: char) -> bool {
    matches!(c, '\u{FB00}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}')
}

// ---------------------------------------------------------------- lines

struct Line {
    /// Unit ids.
    members: Vec<usize>,
    /// The most common baseline among the members.
    baseline: f32,
    size: f32,
}

fn build_lines(units: &[Unit], p: Params) -> Vec<Line> {
    let mut order: Vec<usize> = (0..units.len()).collect();
    order.sort_by(|&a, &b| {
        units[b].origin_y.total_cmp(&units[a].origin_y).then(units[a].left.total_cmp(&units[b].left))
    });

    let mut lines: Vec<Line> = Vec::new();
    let mut votes: HashMap<i64, usize> = HashMap::new();
    for u in order {
        let unit = &units[u];
        let joins = lines.last().is_some_and(|line| {
            (unit.origin_y - line.baseline).abs() <= p.line_tolerance * unit.size.max(line.size)
        });
        if !joins {
            votes.clear();
            lines.push(Line { members: Vec::new(), baseline: unit.origin_y, size: unit.size });
        }
        let Some(line) = lines.last_mut() else { continue };
        line.members.push(u);
        line.size = line.size.max(unit.size);
        // Baseline = mode of members' baselines (to 0.1 pt), so shifted glyphs do not drag it.
        let key = (unit.origin_y * 10.0).round() as i64;
        *votes.entry(key).or_default() += 1;
        if let Some((&best, _)) = votes.iter().max_by_key(|(key, count)| (**count, **key)) {
            line.baseline = best as f32 / 10.0;
        }
    }
    lines
}

/// Visual (left-to-right) order: by origin, which producers place at the
/// glyph's left edge.
///
/// Origins closer than a twentieth of an em belong to glyphs drawn with no
/// advance between them: a cluster the shaper positioned by offsets (Amiri
/// draws `وَإِ` from two origins 0.05 pt apart, the waw to the right). Their
/// ink says which is where. Units drawn from the very same origin with the
/// same ink centre (one cluster split by a vertical offset) keep stream order:
/// logical, so for right-to-left letters the later one is further left.
fn visual_order(units: &[Unit], members: &mut [usize]) {
    members.sort_by(|&a, &b| {
        let (ua, ub) = (&units[a], &units[b]);
        let near = 0.05 * ua.size.max(ub.size);
        if (ua.origin_x - ub.origin_x).abs() >= near.max(0.01) {
            ua.origin_x.total_cmp(&ub.origin_x)
        } else if (ua.center_x() - ub.center_x()).abs() >= 0.01 {
            ua.center_x().total_cmp(&ub.center_x())
        } else {
            let (ia, ib) = (ua.first_index, ub.first_index);
            if is_rtl_char(ua.key) && is_rtl_char(ub.key) { ib.cmp(&ia) } else { ia.cmp(&ib) }
        }
    });
}

/// Splits a line at gaps wide enough to separate columns or table cells.
fn split_segments(units: &[Unit], line: &Line, p: Params) -> Vec<Vec<usize>> {
    let mut members = line.members.clone();
    visual_order(units, &mut members);
    let mut segments: Vec<Vec<usize>> = Vec::new();
    let mut right_edge = f32::NEG_INFINITY;
    for u in members {
        if segments.is_empty() || units[u].left - right_edge > p.segment_gap * units[u].size {
            segments.push(Vec::new());
            right_edge = f32::NEG_INFINITY;
        }
        if let Some(segment) = segments.last_mut() {
            segment.push(u);
        }
        right_edge = right_edge.max(units[u].right);
    }
    segments
}

/// Gaps that a real space character vouches for.
///
/// A space separates the glyphs just before and after it in the content
/// stream. When those two are visual neighbours, the gap between them is a
/// word space however narrow it is. (When they are not — Chrome writes some
/// Arabic words in pieces — the space says nothing reliable about where it
/// goes, and geometry decides.) Returns, per segment, gap positions `k`
/// (between members `k` and `k + 1`).
fn spaces_from_stream(
    glyphs: &[Glyph],
    units: &[Unit],
    segments: &[(Vec<usize>, f32)],
) -> HashMap<usize, Vec<usize>> {
    let mut unit_of: HashMap<usize, usize> = HashMap::new();
    for (u, unit) in units.iter().enumerate() {
        for &g in &unit.glyphs {
            unit_of.insert(g, u);
        }
    }
    let mut position: HashMap<usize, (usize, usize)> = HashMap::new();
    for (s, (members, _)) in segments.iter().enumerate() {
        for (k, &u) in members.iter().enumerate() {
            position.insert(u, (s, k));
        }
    }

    let mut stream: Vec<usize> = (0..glyphs.len()).collect();
    stream.sort_by_key(|&g| glyphs[g].index);

    let mut forced: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, &g) in stream.iter().enumerate() {
        if glyphs[g].ch != ' ' {
            continue;
        }
        let before = stream[..i].iter().rev().find(|&&x| glyphs[x].ch != ' ');
        let after = stream[i + 1..].iter().find(|&&x| glyphs[x].ch != ' ');
        let (Some(before), Some(after)) = (before, after) else { continue };
        let locate = |g: &usize| unit_of.get(g).and_then(|u| position.get(u)).copied();
        let (Some((sa, ka)), Some((sb, kb))) = (locate(before), locate(after)) else { continue };
        if sa == sb && ka.abs_diff(kb) == 1 {
            forced.entry(sa).or_default().push(ka.min(kb));
        }
    }
    forced
}

/// Gaps that hold a drawn space glyph: the mark lies on the segment's line,
/// between the centres of two neighbouring units.
fn spaces_from_marks(
    spaces: &[SpaceMark],
    units: &[Unit],
    segments: &[(Vec<usize>, f32)],
    p: Params,
) -> HashMap<usize, Vec<usize>> {
    let mut forced: HashMap<usize, Vec<usize>> = HashMap::new();
    for (s, (members, baseline)) in segments.iter().enumerate() {
        let Some(size) = members.iter().map(|&u| units[u].size).reduce(f32::max) else { continue };
        for mark in spaces.iter().filter(|m| (m.y - baseline).abs() <= p.line_tolerance * size.max(m.size)) {
            let gap = members
                .windows(2)
                .position(|pair| units[pair[0]].center_x() <= mark.x && mark.x <= units[pair[1]].center_x());
            if let Some(k) = gap {
                forced.entry(s).or_default().push(k);
            }
        }
    }
    forced
}

// ---------------------------------------------------------------- segments

/// A unit or an inferred space, in visual order.
#[derive(Debug, Clone)]
struct Cluster {
    text: String,
    rect: Option<Rect>,
    key: char,
    chars: Vec<usize>,
}

struct Segment {
    clusters: Vec<Cluster>,
    bounds: Rect,
    baseline: f32,
    size: f32,
    rtl_strong: usize,
    ltr_strong: usize,
}

fn make_segment(
    units: &[Unit],
    members: &[usize],
    baseline: f32,
    forced: &[usize],
    marked: bool,
    p: Params,
) -> Segment {
    let mut clusters = Vec::with_capacity(members.len() + members.len() / 4);
    let mut bounds: Option<Rect> = None;
    let mut size: f32 = 0.0;
    let (mut rtl_strong, mut ltr_strong) = (0, 0);

    for (k, &u) in members.iter().enumerate() {
        let unit = &units[u];
        size = size.max(unit.size);

        if k > 0 {
            let before = &units[members[k - 1]];
            let gap = unit.left - before.right;
            let em = unit.size.max(before.size);
            let arabic = is_arabic_letter(unit.key) && is_arabic_letter(before.key);
            // On marked pages, punctuation and script boundaries next to Arabic
            // also wait for a drawn space ("…القلق." has no space before the stop).
            let near_arabic = is_arabic_letter(unit.key) || is_arabic_letter(before.key);
            let punctuation = !unit.key.is_alphanumeric() || !before.key.is_alphanumeric();
            let displaced = |x: &Unit| (x.origin_y - baseline).abs() > p.displaced * x.size;
            let threshold = if marked && (near_arabic || (punctuation && arabic)) {
                p.marked_arabic_gap
            } else if arabic && (displaced(unit) || displaced(before)) {
                p.arabic_displaced_word_gap
            } else if arabic {
                p.arabic_word_gap
            } else {
                p.word_gap
            };
            if forced.contains(&(k - 1)) || gap > threshold * em {
                clusters.push(Cluster { text: " ".into(), rect: None, key: ' ', chars: Vec::new() });
            }
        }

        for c in unit.text.chars() {
            match bidi_class(c) {
                BidiClass::R | BidiClass::AL => rtl_strong += 1,
                BidiClass::L => ltr_strong += 1,
                _ => {}
            }
        }
        bounds = Some(bounds.map_or(unit.rect, |b| b.union(unit.rect)));
        clusters.push(Cluster {
            text: unit.text.clone(),
            rect: Some(unit.rect),
            key: unit.key,
            chars: unit.chars.clone(),
        });
    }

    Segment {
        clusters,
        bounds: bounds.unwrap_or(Rect { left: 0.0, bottom: 0.0, right: 0.0, top: 0.0 }),
        baseline,
        size,
        rtl_strong,
        ltr_strong,
    }
}

// ---------------------------------------------------------------- blocks

struct Block {
    segments: Vec<Segment>,
    direction: Direction,
}

impl Block {
    fn left(&self) -> f32 {
        self.segments.iter().map(|s| s.bounds.left).fold(f32::INFINITY, f32::min)
    }
    fn right(&self) -> f32 {
        self.segments.iter().map(|s| s.bounds.right).fold(f32::NEG_INFINITY, f32::max)
    }
    fn size(&self) -> f32 {
        self.segments.first().map_or(0.0, |s| s.size)
    }
    /// Vertical extent from baselines, independent of fonts' ascent and descent.
    fn top(&self) -> f32 {
        self.segments.first().map_or(0.0, |s| s.baseline + 0.8 * s.size)
    }
    fn bottom(&self) -> f32 {
        self.segments.last().map_or(0.0, |s| s.baseline - 0.3 * s.size)
    }
    fn strong(&self) -> (usize, usize) {
        self.segments.iter().fold((0, 0), |(r, l), s| (r + s.rtl_strong, l + s.ltr_strong))
    }
}

fn overlap(a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.1.min(b.1) - a.0.max(b.0)).max(0.0)
}

/// Groups segments into blocks (paragraphs, cells): same size, horizontally
/// overlapping, and at a steady line pitch.
fn build_blocks(mut segments: Vec<Segment>, p: Params) -> Vec<Block> {
    segments.sort_by(|a, b| b.baseline.total_cmp(&a.baseline).then(a.bounds.left.total_cmp(&b.bounds.left)));
    let mut blocks: Vec<Block> = Vec::new();

    for segment in segments {
        let x = (segment.bounds.left, segment.bounds.right);
        let candidate = blocks
            .iter()
            .enumerate()
            .filter_map(|(i, block)| {
                let last = block.segments.last()?;
                let pitch = last.baseline - segment.baseline;
                if pitch <= 0.0 || (segment.size - block.size()).abs() > 0.15 * block.size() {
                    return None;
                }
                let allowed = match block.segments.as_slice() {
                    [first, second, ..] => p.pitch_growth * (first.baseline - second.baseline),
                    _ => p.first_pitch * segment.size,
                };
                let width = (x.1 - x.0).min(block.right() - block.left());
                let shared = overlap(x, (block.left(), block.right()));
                (pitch <= allowed && shared >= 0.5 * width).then_some((i, pitch))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));

        match candidate {
            Some((i, _)) => blocks[i].segments.push(segment),
            None => blocks.push(Block { segments: vec![segment], direction: Direction::Ltr }),
        }
    }
    blocks
}

/// A block's direction, from the first signal that decides it:
///
/// 1. alignment within its column (an RTL paragraph's lines hug the right
///    edge of the column);
/// 2. a clear majority of strongly directional characters (twice as many of
///    one direction as of the other);
/// 3. alignment within the page's text area (a heading set alone above a
///    table forms its own column, but still starts at the page's right edge);
/// 4. the majority on the whole page.
fn assign_directions(blocks: &mut [Block], p: Params) {
    let extents: Vec<(f32, f32)> = blocks.iter().map(|b| (b.left(), b.right())).collect();
    let page =
        extents.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |acc, e| (acc.0.min(e.0), acc.1.max(e.1)));
    let (page_rtl, page_ltr) = blocks.iter().fold((0, 0), |(r, l), b| {
        let (br, bl) = b.strong();
        (r + br, l + bl)
    });
    let majority = |rtl: usize, ltr: usize| match rtl.cmp(&ltr) {
        std::cmp::Ordering::Greater => Some(Direction::Rtl),
        std::cmp::Ordering::Less => Some(Direction::Ltr),
        std::cmp::Ordering::Equal => None,
    };

    for (i, block) in blocks.iter_mut().enumerate() {
        let own = extents[i];
        // The column: every block that shares this block's horizontal range.
        let column = extents
            .iter()
            .filter(|other| overlap(own, **other) > 0.0)
            .fold(own, |acc, other| (acc.0.min(other.0), acc.1.max(other.1)));
        let tolerance = p.align_tolerance * block.size();
        let aligned = |(left, right): (f32, f32)| {
            let (mut rtl, mut ltr) = (0, 0);
            for segment in &block.segments {
                let left_gap = segment.bounds.left - left;
                let right_gap = right - segment.bounds.right;
                if right_gap <= tolerance && left_gap > 2.0 * tolerance {
                    rtl += 1;
                } else if left_gap <= tolerance && right_gap > 2.0 * tolerance {
                    ltr += 1;
                }
            }
            majority(rtl, ltr)
        };
        let (rtl_strong, ltr_strong) = block.strong();
        let clear = (rtl_strong.max(ltr_strong) >= 2 * rtl_strong.min(ltr_strong))
            .then(|| majority(rtl_strong, ltr_strong))
            .flatten();
        block.direction = aligned(column)
            .or(clear)
            .or_else(|| aligned(page))
            .or_else(|| majority(page_rtl, page_ltr))
            .unwrap_or(Direction::Ltr);
    }
}

fn page_direction(blocks: &[Block]) -> Direction {
    let (rtl, ltr) = blocks.iter().fold((0, 0), |(r, l), block| {
        let (rs, ls) = block.strong();
        match block.direction {
            Direction::Rtl => (r + rs + ls, l),
            Direction::Ltr => (r, l + rs + ls),
        }
    });
    if rtl > ltr { Direction::Rtl } else { Direction::Ltr }
}

type Extent = ((f32, f32), (f32, f32));

/// Reading order: bands of vertically overlapping blocks top to bottom; inside a
/// band, columns in the page's direction; inside a column, top to bottom.
fn order_blocks(blocks: Vec<Block>, direction: Direction) -> Vec<Block> {
    let extents: Vec<Extent> =
        blocks.iter().map(|b| ((b.bottom(), b.top()), (b.left(), b.right()))).collect();
    let mut bands = groups(blocks.len(), |a, b| overlap(extents[a].0, extents[b].0) > 0.0);
    bands.sort_by(|a, b| top_of(b, &extents).total_cmp(&top_of(a, &extents)));

    let mut slots: Vec<Option<Block>> = blocks.into_iter().map(Some).collect();
    let mut ordered = Vec::with_capacity(slots.len());
    for band in bands {
        let mut columns: Vec<Vec<usize>> =
            groups(band.len(), |a, b| overlap(extents[band[a]].1, extents[band[b]].1) > 0.0)
                .into_iter()
                .map(|column| column.into_iter().map(|i| band[i]).collect())
                .collect();
        columns.sort_by(|a, b| {
            let (la, lb) = (left_of(a, &extents), left_of(b, &extents));
            match direction {
                Direction::Ltr => la.total_cmp(&lb),
                Direction::Rtl => lb.total_cmp(&la),
            }
        });
        for mut column in columns {
            column.sort_by(|&a, &b| extents[b].0.1.total_cmp(&extents[a].0.1));
            ordered.extend(column.into_iter().filter_map(|i| slots[i].take()));
        }
    }
    ordered
}

fn top_of(group: &[usize], extents: &[Extent]) -> f32 {
    group.iter().map(|&i| extents[i].0.1).fold(f32::NEG_INFINITY, f32::max)
}

fn left_of(group: &[usize], extents: &[Extent]) -> f32 {
    group.iter().map(|&i| extents[i].1.0).fold(f32::INFINITY, f32::min)
}

/// Connected components of `0..n` under `related` (transitive closure).
fn groups(n: usize, related: impl Fn(usize, usize) -> bool) -> Vec<Vec<usize>> {
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut parent: Vec<usize> = (0..n).collect();
    for a in 0..n {
        for b in a + 1..n {
            if related(a, b) {
                let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                if ra != rb {
                    parent[rb] = ra;
                }
            }
        }
    }
    let mut out: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        let root = find(&mut parent, i);
        out.entry(root).or_default().push(i);
    }
    let mut out: Vec<Vec<usize>> = out.into_values().collect();
    out.sort_by_key(|group| group[0]);
    out
}

// ---------------------------------------------------------------- bidi

fn finish_block(block: Block) -> TextBlock {
    let direction = block.direction;
    let lines: Vec<TextLine> =
        block.segments.into_iter().map(|segment| logical_line(segment, direction)).collect();
    let bounds = lines.iter().map(|line| line.bounds).reduce(Rect::union).unwrap_or(Rect {
        left: 0.0,
        bottom: 0.0,
        right: 0.0,
        top: 0.0,
    });
    TextBlock { direction, lines, bounds }
}

fn mirror(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        '\u{FD3E}' => '\u{FD3F}',
        '\u{FD3F}' => '\u{FD3E}',
        other => other,
    }
}

fn is_opening(c: char) -> bool {
    matches!(c, '(' | '[' | '{' | '«' | '\u{FD3E}')
}

fn is_closing(c: char) -> bool {
    matches!(c, ')' | ']' | '}' | '»' | '\u{FD3F}')
}

fn single(cluster: &Cluster) -> Option<char> {
    let mut chars = cluster.text.chars();
    let c = chars.next()?;
    chars.next().is_none().then_some(c)
}

/// Gives bracket pairs their drawn shapes, in visual order, and returns the
/// positions of the brackets that form pairs.
///
/// Whatever the direction, a pair is drawn `(…)` from left to right: next to
/// right-to-left text each bracket is drawn mirrored. Producers disagree about
/// whether such a glyph maps to the character typed or the shape drawn, and
/// some do both on one line, giving `)…)`. Unmatched closers come in pairs
/// where the first is drawn opening; unmatched openers in pairs where the
/// second is drawn closing; a closer before an opener (`)…(`, both mapped as
/// typed) swaps both. A lone bracket (`1)` in a list) keeps its mapping.
fn bracket_shapes(clusters: &mut [Cluster]) -> Vec<usize> {
    let scan = |clusters: &[Cluster]| {
        let (mut open, mut stray_closers, mut pairs) = (Vec::new(), Vec::new(), Vec::new());
        for (i, cluster) in clusters.iter().enumerate() {
            match single(cluster) {
                Some(c) if is_opening(c) => open.push(i),
                Some(c) if is_closing(c) => match open.pop() {
                    Some(opener) => pairs.extend([opener, i]),
                    None => stray_closers.push(i),
                },
                _ => {}
            }
        }
        (open, stray_closers, pairs)
    };

    let (mut open, mut stray_closers, _) = scan(clusters);
    let mut flips = Vec::new();
    if let (Some(&closer), Some(&opener)) = (stray_closers.last(), open.first())
        && stray_closers.len() % 2 == 1
        && open.len() % 2 == 1
        && closer < opener
    {
        flips.extend([closer, opener]);
        stray_closers.pop();
        open.remove(0);
    }
    flips.extend(stray_closers.as_chunks::<2>().0.iter().map(|[first, _]| *first));
    flips.extend(open.as_chunks::<2>().0.iter().map(|[_, second]| *second));
    for i in flips {
        let cluster = &mut clusters[i];
        if let Some(c) = single(cluster) {
            cluster.text = mirror(c).to_string();
            cluster.key = mirror(c);
        }
    }
    scan(clusters).2
}

/// Curly quotation marks mapped the wrong way (`“…“`), in logical order: two
/// of a kind on a line are one pair, opening then closing.
fn repair_quotes(clusters: &mut [Cluster]) {
    for (opening, closing) in [('\u{201C}', '\u{201D}'), ('\u{2018}', '\u{2019}')] {
        let positions: Vec<usize> = clusters
            .iter()
            .enumerate()
            .filter(|(_, cluster)| matches!(single(cluster), Some(c) if c == opening || c == closing))
            .map(|(i, _)| i)
            .collect();
        let all_same = positions.iter().all(|&i| single(&clusters[i]) == single(&clusters[positions[0]]));
        if positions.len() == 2 && all_same {
            clusters[positions[0]].text = opening.to_string();
            clusters[positions[1]].text = closing.to_string();
        }
    }
}

/// Converts one segment from visual to logical order.
///
/// The clusters arrive in visual (left-to-right) order. Running the bidi
/// algorithm's reordering over a visual string gives back the logical one,
/// because for the character classes it depends on the reordering is its own
/// inverse. Each cluster moves as a unit, so a ligature's letters and a
/// letter's marks keep their (already logical) internal order.
///
/// Bracket pairs go in with their drawn shapes and come out mirrored where
/// they sit at a right-to-left level (UAX #9, rule L4), which turns a drawn
/// `)` at the start of an Arabic parenthesis back into the `(` typed.
fn logical_line(segment: Segment, direction: Direction) -> TextLine {
    let bounds = segment.bounds;
    let segment_baseline = segment.baseline;
    let mut visual_clusters = segment.clusters;
    let paired = bracket_shapes(&mut visual_clusters);
    let visual: String = visual_clusters.iter().map(|c| c.key).collect();
    let base = match direction {
        Direction::Ltr => Level::ltr(),
        Direction::Rtl => Level::rtl(),
    };
    let info = ParagraphBidiInfo::new(&visual, Some(base));
    // One level per character, i.e. per cluster (each contributes one key character).
    let levels: Vec<Level> = info.reordered_levels_per_char(0..visual.len());
    let order = ParagraphBidiInfo::reorder_visual(&levels);

    let mut clusters: Vec<Cluster> = order
        .into_iter()
        .map(|i| {
            let mut cluster = visual_clusters[i].clone();
            if levels[i].is_rtl()
                && paired.contains(&i)
                && let Some(c) = single(&cluster)
            {
                cluster.text = mirror(c).to_string();
            }
            cluster
        })
        .collect();
    repair_quotes(&mut clusters);

    let mut text = String::new();
    let mut spans = Vec::with_capacity(clusters.len());
    for cluster in clusters {
        let start = text.len();
        text.push_str(&cluster.text);
        spans.push(TextSpan { start, end: text.len(), rect: cluster.rect, chars: cluster.chars });
    }
    TextLine { text, spans, bounds, baseline: segment_baseline }
}

// ---------------------------------------------------------------- quality

/// Flags pages whose ToUnicode mapping is evidently broken.
///
/// Three signatures, all seen in real producer output (CoreText with Noto
/// Naskh, whose letters are drawn as a dotless body plus separate dot glyphs,
/// and with Amiri): dotless letter forms, which modern text almost never uses;
/// ASCII symbols drawn on top of an Arabic letter, which real punctuation never
/// is; and ASCII characters carrying Arabic vowel marks.
fn assess_quality(units: &[Unit], lines: &[Line]) -> TextQuality {
    let arabic = units.iter().filter(|u| is_arabic_letter(u.key)).count();
    if arabic == 0 {
        return TextQuality::Good;
    }
    let dotless = |c: char| matches!(c, '\u{066E}' | '\u{066F}' | '\u{06BA}' | '\u{06A1}');
    let arabic_mark = |c: char| matches!(c, '\u{064B}'..='\u{065F}' | '\u{0670}');
    let mut suspicious = units
        .iter()
        .filter(|u| {
            u.text.chars().any(dotless)
                // An ASCII character carrying an Arabic vowel mark never occurs in real text.
                || (u.key.is_ascii() && u.text.chars().any(arabic_mark))
        })
        .count();
    for line in lines {
        for &s in &line.members {
            let symbol = &units[s];
            if !symbol.key.is_ascii_punctuation() {
                continue;
            }
            let width = (symbol.right - symbol.left).max(0.01);
            let on_letter = line.members.iter().any(|&l| {
                is_arabic_letter(units[l].key)
                    && overlap((symbol.left, symbol.right), (units[l].left, units[l].right)) >= 0.5 * width
            });
            if on_letter {
                suspicious += 1;
            }
        }
    }
    if suspicious >= 3 && suspicious * 50 >= arabic {
        TextQuality::Suspect { suspicious_glyphs: suspicious, glyphs: units.len() }
    } else {
        TextQuality::Good
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A glyph of width `w` at `x` on baseline `y`, `size` points.
    fn g(index: usize, ch: char, x: f32, w: f32, y: f32, size: f32) -> Glyph {
        Glyph {
            index,
            ch,
            left: x,
            right: x + w,
            bottom: y - 0.25 * size,
            top: y + 0.75 * size,
            origin_x: x,
            origin_y: y,
            size,
        }
    }

    /// Lays out `words` (each in *visual* left-to-right glyph order) on one line,
    /// with `space` points between words, in the given stream order.
    fn line(words: &[&str], y: f32, x0: f32, space: f32, size: f32, first_index: usize) -> Vec<Glyph> {
        let mut glyphs = Vec::new();
        let mut x = x0;
        let mut index = first_index;
        for word in words {
            for c in word.chars() {
                glyphs.push(g(index, c, x, 0.5 * size, y, size));
                x += 0.5 * size;
                index += 1;
            }
            x += space;
        }
        glyphs
    }

    fn text(glyphs: &[Glyph]) -> String {
        analyze(glyphs, &[]).text()
    }

    #[test]
    fn empty_page_has_no_text() {
        assert_eq!(analyze(&[], &[]).quality, TextQuality::NoText);
    }

    #[test]
    fn latin_line_reads_left_to_right_with_spaces() {
        let glyphs = line(&["Hello", "world"], 700.0, 72.0, 3.0, 12.0, 0);
        assert_eq!(text(&glyphs), "Hello world");
    }

    #[test]
    fn arabic_stored_in_visual_order_comes_out_logical() {
        // "مرحبا بكم" drawn right to left; glyphs listed left to right.
        let visual_words: Vec<String> =
            ["مرحبا", "بكم"].iter().rev().map(|w| w.chars().rev().collect()).collect();
        let words: Vec<&str> = visual_words.iter().map(String::as_str).collect();
        let glyphs = line(&words, 700.0, 300.0, 4.0, 12.0, 0);
        let page = analyze(&glyphs, &[]);
        assert_eq!(page.text(), "مرحبا بكم");
        assert_eq!(page.direction, Direction::Rtl);
    }

    #[test]
    fn stream_order_does_not_matter() {
        let mut glyphs = line(&["Hello", "world"], 700.0, 72.0, 3.0, 12.0, 0);
        // Same geometry, reversed content stream.
        let n = glyphs.len();
        for (i, glyph) in glyphs.iter_mut().enumerate() {
            glyph.index = n - i;
        }
        assert_eq!(text(&glyphs), "Hello world");
    }

    #[test]
    fn numbers_and_latin_inside_arabic_keep_their_order() {
        // Logical: "سعر 4,500 ريال" — visual left to right: "لاير" "4,500" "رعس".
        let glyphs = line(&["لاير", "4,500", "رعس"], 700.0, 300.0, 4.0, 12.0, 0);
        assert_eq!(text(&glyphs), "سعر 4,500 ريال");
    }

    #[test]
    fn marks_follow_their_letter_whatever_the_stream_order() {
        // "بَ" drawn as Chrome does: the mark first in the stream, same origin as the letter.
        let mut fatha = g(0, '\u{064E}', 100.0, 3.0, 700.0, 12.0);
        fatha.origin_x = 100.0;
        let beh = g(1, 'ب', 100.0, 6.0, 700.0, 12.0);
        assert_eq!(text(&[fatha, beh]), "بَ");
    }

    #[test]
    fn rtl_letters_sharing_one_glyph_are_put_back_in_logical_order() {
        // One ligature glyph mapped to "صر"; PDFium reports its letters reversed.
        let ra = g(0, 'ر', 100.0, 6.0, 700.0, 12.0);
        let sad = g(1, 'ص', 100.0, 6.0, 700.0, 12.0);
        assert_eq!(text(&[ra, sad]), "صر");
    }

    #[test]
    fn a_ligature_whose_letters_cannot_tell_their_order_is_put_back_too() {
        // Noon-yeh drawn as one glyph mapped to "ني" (both letters join both
        // ways, so they carry no evidence); PDFium reports "ين". With nothing
        // else on the page to vote, its letters are reversed (ADR 0021).
        let yeh = g(0, 'ي', 100.0, 6.0, 700.0, 12.0);
        let noon = g(1, 'ن', 100.0, 6.0, 700.0, 12.0);
        assert_eq!(text(&[yeh, noon]), "ني");
    }

    #[test]
    fn real_space_between_visual_neighbours_counts_even_if_narrow() {
        let mut glyphs = line(&["ab", "cd"], 700.0, 72.0, 0.5, 12.0, 0);
        // Re-number so a real space sits between "b" (1) and "c" (3) in the stream.
        glyphs[2].index = 3;
        glyphs[3].index = 4;
        glyphs.push(Glyph { ch: ' ', index: 2, ..g(2, ' ', 84.0, 0.0, 700.0, 12.0) });
        assert_eq!(text(&glyphs), "ab cd");
    }

    #[test]
    fn two_columns_are_not_merged_into_one_line() {
        let mut glyphs = line(&["left", "column"], 700.0, 72.0, 3.0, 12.0, 0);
        glyphs.extend(line(&["right", "column"], 700.0, 320.0, 3.0, 12.0, 100));
        let page = analyze(&glyphs, &[]);
        assert_eq!(page.blocks.len(), 2);
        assert_eq!(page.text(), "left column\n\nright column");
    }

    #[test]
    fn right_aligned_mixed_heading_is_read_right_to_left() {
        // A right-aligned heading "عنوان — Title" above a full-width Arabic paragraph.
        let mut glyphs = line(&["Title", "—", "ناونع"], 760.0, 446.0, 4.0, 12.0, 0);
        let width = 520.0 - 72.0;
        let letters = (width / 6.0) as usize;
        let body: String = std::iter::repeat_n('ب', letters).collect();
        glyphs.extend(line(&[&body], 730.0, 72.0, 0.0, 12.0, 100));
        let page = analyze(&glyphs, &[]);
        assert_eq!(page.blocks[0].direction, Direction::Rtl);
        assert_eq!(page.blocks[0].lines[0].text, "عنوان — Title");
    }

    #[test]
    fn presentation_forms_become_base_letters() {
        // U+FEFB is the isolated lam-alef ligature.
        assert_eq!(text(&[g(0, '\u{FEFB}', 100.0, 8.0, 700.0, 12.0)]), "لا");
    }

    #[test]
    fn mirrored_brackets_drawn_as_seen_are_restored() {
        // Logical "(نص)" ; a producer that maps glyphs to what is drawn stores
        // visual left to right: "(" "صن" ")" as the *drawn* shapes.
        let glyphs = line(&["(صن)"], 700.0, 300.0, 0.0, 12.0, 0);
        assert_eq!(text(&glyphs), "(نص)");
    }

    #[test]
    fn mirrored_brackets_mapped_as_typed_are_restored() {
        // The same, from a producer that maps each glyph to the character typed:
        // the left glyph (drawn "(") maps to ")".
        let glyphs = line(&[")صن("], 700.0, 300.0, 0.0, 12.0, 0);
        assert_eq!(text(&glyphs), "(نص)");
    }

    #[test]
    fn brackets_mapped_both_ways_around_latin_in_arabic_are_restored() {
        // Logical "نصوص (abc)!" in an Arabic line; drawn "!(abc) صوصن". Chrome maps
        // the left bracket as typed and the right one as drawn: both come out ")".
        let glyphs = line(&["!)abc)", "صوصن"], 700.0, 300.0, 4.0, 12.0, 0);
        assert_eq!(text(&glyphs), "نصوص (abc)!");
    }

    #[test]
    fn a_space_drawn_in_the_same_run_as_a_letter_is_a_word_break() {
        // A page that draws its spaces (Chrome): there, only drawn spaces break
        // Arabic words. "بكم مرحبا" (visual: "ابحرم" then "مكب"), 0.3 em apart.
        let mut glyphs = line(&["ابحرم", "مكب"], 700.0, 300.0, 3.5, 12.0, 0);
        let marks = [SpaceMark { x: 100.0, y: 650.0, size: 12.0 }];
        assert_eq!(analyze(&glyphs, &marks).blocks[0].lines[0].text, "بكممرحبا");
        // Chrome on Linux draws that space in one `Tj` with the letter before it;
        // PDFium reports it as a full-size space with a real advance.
        glyphs.push(g(99, ' ', 330.0, 3.5, 700.0, 12.0));
        assert_eq!(analyze(&glyphs, &marks).blocks[0].lines[0].text, "بكم مرحبا");
    }

    #[test]
    fn a_cluster_drawn_from_one_origin_is_ordered_by_its_ink() {
        // "وإ" (waw then alef with hamza below): the shaper drew both from origins
        // 0.05 pt apart, the waw's ink to the right. Stream order says alef first.
        let mut alef = g(0, 'إ', 100.0, 1.5, 700.0, 12.0);
        alef.left = 100.1;
        let mut waw = g(1, 'و', 99.95, 1.5, 700.0, 12.0);
        waw.left = 101.5;
        waw.right = 103.0;
        let before = line(&["ن"], 700.0, 94.0, 0.0, 12.0, 2);
        let mut glyphs = vec![alef, waw];
        glyphs.extend(before);
        assert_eq!(text(&glyphs), "وإن");
    }

    #[test]
    fn a_heading_alone_in_its_column_takes_the_page_edge_it_hugs() {
        // "عنوان — Title", as many Arabic as Latin letters, ends at the right edge
        // of the page's text area; the table cells below do not overlap it.
        let mut glyphs = line(&["Title", "—", "ناونع"], 760.0, 450.0, 4.0, 12.0, 0);
        let heading_right = glyphs.iter().map(|g| g.right).fold(0.0, f32::max);
        glyphs.extend(line(&["ب"], 730.0, heading_right - 6.0, 0.0, 12.0, 100));
        glyphs.extend(line(&["12"], 730.0, 72.0, 0.0, 12.0, 200));
        let page = analyze(&glyphs, &[]);
        assert_eq!(page.blocks[0].direction, Direction::Rtl);
        assert_eq!(page.blocks[0].lines[0].text, "عنوان — Title");
    }
}
