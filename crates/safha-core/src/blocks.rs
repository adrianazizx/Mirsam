//! Page analysis for in-place editing (spec §7, step 1; ADR 0023): glyphs →
//! lines → blocks, in the page as displayed.
//!
//! PDF has no paragraphs, only positioned glyphs. This module rebuilds them
//! from geometry alone:
//!
//! 1. **Lines:** glyphs on one baseline, split where the gap is wider than two
//!    ems (columns, table cells). A space is inserted where the gap is wide
//!    enough to be one.
//! 2. **Logical order:** the line's glyphs sit in visual order; they are put
//!    back in logical order from their directions (Arabic and Hebrew runs
//!    reversed, Latin and digit runs kept), with marks kept on their letters.
//! 3. **Blocks:** lines of one font and size, stacked at one line pitch and
//!    aligned (flush start, flush end, centred, or justified), form one
//!    paragraph.
//!
//! Direction, alignment, line pitch, size, and colour are read off the lines.
//! Each character keeps the style it was drawn in (font, size, colour), so a
//! paragraph is also a list of styled runs in logical order (ADR 0024).
//! The module knows nothing of PDF syntax: it works on [`Atom`]s.

use serde::Serialize;

/// One glyph, placed in the page as displayed (y down).
#[derive(Debug, Clone)]
pub(crate) struct Atom {
    pub(crate) x0: f32,
    pub(crate) x1: f32,
    pub(crate) baseline: f32,
    pub(crate) size: f32,
    pub(crate) text: String,
    /// The text-showing operator it came from.
    pub(crate) shown: usize,
    /// Fonts that are the same font have the same key.
    pub(crate) font: usize,
    pub(crate) color: [f32; 3],
}

/// How a stretch of text was drawn: its font (a key into the page's fonts),
/// size as displayed, and fill colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RunStyle {
    pub(crate) font: usize,
    pub(crate) size: f32,
    pub(crate) color: [f32; 3],
}

impl RunStyle {
    fn of(atom: &Atom) -> Self {
        // Rounded, so glyphs of one run compare equal.
        let round = |v: f32, step: f32| (v / step).round() * step;
        RunStyle {
            font: atom.font,
            size: round(atom.size, 0.01),
            color: atom.color.map(|c| round(c, 1.0 / 255.0)),
        }
    }
}

/// A stretch of a line's or a block's text in one style: a byte range of
/// its text, in logical order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Run {
    pub(crate) range: std::ops::Range<usize>,
    pub(crate) style: RunStyle,
}

/// Runs from characters and their styles: consecutive equal styles merged.
fn runs_of(chars: &[(char, RunStyle)]) -> (String, Vec<Run>) {
    let mut text = String::new();
    let mut runs: Vec<Run> = Vec::new();
    for &(c, style) in chars {
        let start = text.len();
        text.push(c);
        match runs.last_mut() {
            Some(run) if run.style == style => run.range.end = text.len(),
            _ => runs.push(Run { range: start..text.len(), style }),
        }
    }
    (text, runs)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BlockAlign {
    Left,
    Right,
    Center,
    Justify,
}

/// A paragraph rebuilt from a page's glyphs.
#[derive(Debug, Clone)]
pub(crate) struct Block {
    pub(crate) lines: Vec<Line>,
    pub(crate) text: String,
    pub(crate) rtl: bool,
    pub(crate) align: BlockAlign,
    /// Left and right edges, top of the first line, bottom of the last.
    pub(crate) x0: f32,
    pub(crate) x1: f32,
    pub(crate) top: f32,
    pub(crate) bottom: f32,
    pub(crate) size: f32,
    pub(crate) color: [f32; 3],
    pub(crate) font: usize,
    /// Baseline to baseline; `None` for a single line.
    pub(crate) pitch: Option<f32>,
    /// Its text in styled runs, in logical order (cover `text`).
    pub(crate) runs: Vec<Run>,
    /// The operators that drew it, in drawing order.
    pub(crate) shown: Vec<usize>,
    /// Some operator that drew it also drew other text.
    pub(crate) shared: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct Line {
    pub(crate) x0: f32,
    pub(crate) x1: f32,
    pub(crate) baseline: f32,
    pub(crate) size: f32,
    pub(crate) font: usize,
    pub(crate) color: [f32; 3],
    pub(crate) text: String,
    pub(crate) rtl: bool,
    pub(crate) shown: Vec<usize>,
    /// Its text in styled runs, in logical order.
    pub(crate) runs: Vec<Run>,
}

const COLUMN_GAP_EMS: f32 = 2.0;
const SPACE_GAP_EMS: f32 = 0.2;
const MAX_ATOMS: usize = 400_000;

/// Blocks of the page's text, in reading order top to bottom.
pub(crate) fn analyze(mut atoms: Vec<Atom>) -> Vec<Block> {
    atoms.truncate(MAX_ATOMS);
    let mut lines = build_lines(atoms);
    lines.sort_by(|a, b| a.baseline.total_cmp(&b.baseline).then(a.x0.total_cmp(&b.x0)));
    let mut blocks = group(lines);
    // An operator that drew text of two blocks cannot be edited for either.
    let mut owner: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let mut shared = vec![false; blocks.len()];
    for (b, block) in blocks.iter().enumerate() {
        for &shown in &block.shown {
            match owner.insert(shown, b) {
                Some(other) if other != b => {
                    shared[b] = true;
                    shared[other] = true;
                }
                _ => {}
            }
        }
    }
    for (block, shared) in blocks.iter_mut().zip(shared) {
        block.shared = shared;
    }
    blocks
}

/// A glyph that stands on another: combining marks, and the dots a letter is
/// drawn with separately (text the page attaches to the letter's own glyph).
fn is_mark(text: &str) -> bool {
    text.is_empty()
        || text.chars().all(|c| {
            matches!(c, '\u{0300}'..='\u{036F}' | '\u{064B}'..='\u{065F}' | '\u{0670}' | '\u{06D6}'..='\u{06ED}' | '\u{200C}' | '\u{200D}')
        })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    R,
    L,
    N,
}

fn class_of(text: &str) -> Class {
    for c in text.chars() {
        if matches!(c, '\u{0590}'..='\u{08FF}' | '\u{FB1D}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}')
            && !c.is_ascii_digit()
            && !matches!(c, '\u{0660}'..='\u{0669}' | '\u{06F0}'..='\u{06F9}')
        {
            if c.is_alphabetic() {
                return Class::R;
            }
        } else if c.is_alphanumeric() {
            return Class::L;
        }
    }
    Class::N
}

/// A visual unit: a glyph and the marks drawn on it.
struct Unit {
    x0: f32,
    x1: f32,
    text: String,
    shown: usize,
    atom: usize,
}

fn build_lines(atoms: Vec<Atom>) -> Vec<Line> {
    if atoms.is_empty() {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..atoms.len()).collect();
    order.sort_by(|&a, &b| atoms[a].baseline.total_cmp(&atoms[b].baseline));
    // Baseline groups.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut reference = f32::NEG_INFINITY;
    let mut tolerance = 0.0f32;
    for &i in &order {
        let atom = &atoms[i];
        if groups.is_empty() || atom.baseline - reference > tolerance {
            groups.push(Vec::new());
            reference = atom.baseline;
            tolerance = 0.3 * atom.size;
        }
        if let Some(g) = groups.last_mut() {
            g.push(i);
        }
    }

    let mut lines = Vec::new();
    for group in groups {
        let mut group = group;
        group.sort_by(|&a, &b| atoms[a].x0.total_cmp(&atoms[b].x0));
        // Marks sit on a base glyph: attach each to the base it overlaps most.
        let bases: Vec<usize> = group.iter().copied().filter(|&i| !is_mark(&atoms[i].text)).collect();
        let mut units: Vec<Unit> = bases
            .iter()
            .map(|&i| Unit {
                x0: atoms[i].x0,
                x1: atoms[i].x1,
                text: atoms[i].text.clone(),
                shown: atoms[i].shown,
                atom: i,
            })
            .collect();
        let mut marks: Vec<usize> = group.iter().copied().filter(|&i| is_mark(&atoms[i].text)).collect();
        marks.sort_unstable(); // glyph (drawing) order decides the order on a letter
        for mark in marks {
            let centre = (atoms[mark].x0 + atoms[mark].x1) / 2.0;
            let target = units.iter_mut().min_by(|a, b| distance(a, centre).total_cmp(&distance(b, centre)));
            if let Some(unit) = target {
                unit.text.push_str(&atoms[mark].text);
            }
        }
        // Split into segments at wide gaps.
        let mut start = 0;
        for i in 1..=units.len() {
            let split = i == units.len() || {
                let size = atoms[units[i].atom].size;
                units[i].x0 - units[i - 1].x1 > COLUMN_GAP_EMS * size
            };
            if split {
                lines.push(segment(&atoms, &units[start..i]));
                start = i;
            }
        }
    }
    lines
}

fn distance(unit: &Unit, x: f32) -> f32 {
    if x >= unit.x0 && x <= unit.x1 { 0.0 } else { (x - unit.x0).abs().min((x - unit.x1).abs()) }
}

/// One line from its units in visual order.
fn segment(atoms: &[Atom], units: &[Unit]) -> Line {
    let first = &atoms[units[0].atom];
    let size = most_common(units.iter().map(|u| atoms[u.atom].size.round() as i64)) as f32;
    let font = most_common(units.iter().map(|u| atoms[u.atom].font as i64)) as usize;
    let mut shown: Vec<usize> = units.iter().map(|u| u.shown).collect();
    shown.sort_unstable();
    shown.dedup();

    // Spaces where the gap is wide enough to be one.
    let mut visual: Vec<(String, Class, RunStyle)> = Vec::new();
    for (i, unit) in units.iter().enumerate() {
        let style = RunStyle::of(&atoms[unit.atom]);
        if i > 0 {
            let gap = unit.x0 - units[i - 1].x1;
            let spaced = unit.text.trim().is_empty() || units[i - 1].text.trim().is_empty();
            if gap > SPACE_GAP_EMS * size && !spaced {
                visual.push((" ".into(), Class::N, style));
            }
        }
        visual.push((unit.text.clone(), class_of(&unit.text), style));
    }
    let letters = |wanted: Class| visual.iter().filter(|(_, c, _)| *c == wanted).count();
    let rtl = letters(Class::R) > letters(Class::L);

    // Neutrals take the class of the strong text on both sides, else the line's.
    let paragraph = if rtl { Class::R } else { Class::L };
    let mut classes: Vec<Class> = visual.iter().map(|(_, c, _)| *c).collect();
    let mut i = 0;
    while i < classes.len() {
        if classes[i] != Class::N {
            i += 1;
            continue;
        }
        let end = classes[i..].iter().position(|c| *c != Class::N).map_or(classes.len(), |n| i + n);
        let before = i.checked_sub(1).map(|b| classes[b]);
        let after = classes.get(end).copied();
        let resolved = match (before, after) {
            (Some(a), Some(b)) if a == b => a,
            _ => paragraph,
        };
        classes[i..end].iter_mut().for_each(|c| *c = resolved);
        i = end;
    }
    // Runs of one class, then logical order.
    let mut runs: Vec<(Class, Vec<(&str, RunStyle)>)> = Vec::new();
    for ((text, _, style), class) in visual.iter().zip(&classes) {
        match runs.last_mut() {
            Some((c, items)) if c == class => items.push((text, *style)),
            _ => runs.push((*class, vec![(text, *style)])),
        }
    }
    if rtl {
        runs.reverse();
    }
    let mut chars: Vec<(char, RunStyle)> = Vec::new();
    for (class, items) in runs {
        let items: Vec<(&str, RunStyle)> =
            if class == Class::R { items.into_iter().rev().collect() } else { items };
        for (item, style) in items {
            chars.extend(item.chars().map(|c| (c, style)));
        }
    }
    let _ = first;
    let color_key = |c: [f32; 3]| {
        (c[0] * 255.0).round() as i64 * 65_536
            + (c[1] * 255.0).round() as i64 * 256
            + (c[2] * 255.0).round() as i64
    };
    let key = most_common(units.iter().map(|u| color_key(atoms[u.atom].color)));
    let color = units.iter().map(|u| atoms[u.atom].color).find(|c| color_key(*c) == key).unwrap_or([0.0; 3]);
    let (text, runs) = runs_of(&collapse_spaces(&chars));
    Line {
        color,
        x0: units.iter().map(|u| u.x0).fold(f32::MAX, f32::min),
        x1: units.iter().map(|u| u.x1).fold(f32::MIN, f32::max),
        baseline: atoms[units[0].atom].baseline,
        size,
        font,
        text,
        rtl,
        shown,
        runs,
    }
}

/// Runs of spaces become one, the ends are trimmed, and every space takes
/// the style of the character before it in logical order (as typing would).
fn collapse_spaces(chars: &[(char, RunStyle)]) -> Vec<(char, RunStyle)> {
    let space = |c: char| c == ' ' || c == '\u{a0}';
    let mut out: Vec<(char, RunStyle)> = Vec::with_capacity(chars.len());
    for &(c, style) in chars {
        if space(c) && out.last().is_none_or(|&(p, _)| space(p)) {
            continue;
        }
        let style = if space(c) { out.last().map_or(style, |&(_, s)| s) } else { style };
        out.push((c, style));
    }
    while out.last().is_some_and(|&(c, _)| space(c)) {
        out.pop();
    }
    out
}

fn most_common(values: impl Iterator<Item = i64>) -> i64 {
    let mut counts: Vec<(i64, usize)> = Vec::new();
    for v in values {
        match counts.iter_mut().find(|(k, _)| *k == v) {
            Some((_, n)) => *n += 1,
            None => counts.push((v, 1)),
        }
    }
    counts.into_iter().max_by_key(|(_, n)| *n).map_or(0, |(v, _)| v)
}

fn group(lines: Vec<Line>) -> Vec<Block> {
    let mut groups: Vec<Vec<Line>> = Vec::new();
    for line in lines {
        let mut best: Option<(usize, f32)> = None;
        for (g, group) in groups.iter().enumerate() {
            let Some(previous) = group.last() else { continue };
            if fits(group, previous, &line) {
                let dy = line.baseline - previous.baseline;
                if best.is_none_or(|(_, d)| dy < d) {
                    best = Some((g, dy));
                }
            }
        }
        match best {
            Some((g, _)) => groups[g].push(line),
            None => groups.push(vec![line]),
        }
    }
    let mut blocks: Vec<Block> = groups.into_iter().map(block_of).collect();
    blocks.sort_by(|a, b| a.top.total_cmp(&b.top).then(a.x0.total_cmp(&b.x0)));
    blocks
}

/// Whether `line` continues the paragraph `group` whose last line is `previous`.
fn fits(group: &[Line], previous: &Line, line: &Line) -> bool {
    let dy = line.baseline - previous.baseline;
    let size = previous.size.max(1.0);
    // One font in common: a line mostly in a paragraph's italic is still of
    // it; a bold heading over body text shares none.
    let shares_a_font = line.font == previous.font
        || line.runs.iter().any(|r| previous.runs.iter().any(|p| p.style.font == r.style.font));
    if dy <= 0.5 * size || (line.size - previous.size).abs() > 0.12 * size || !shares_a_font {
        return false;
    }
    // Distance: one pitch, as the paragraph has set it.
    let pitch_ok = match group.len() {
        1 => dy <= 1.9 * size,
        n => {
            let pitch = (group[n - 1].baseline - group[0].baseline) / (n - 1) as f32;
            (dy - pitch).abs() <= 0.25 * size + 0.12 * pitch
        }
    };
    if !pitch_ok {
        return false;
    }
    // Horizontal: the lines overlap, and share an edge or a centre (or the
    // paragraph's first line is indented).
    let overlap = line.x1.min(previous.x1) - line.x0.max(previous.x0);
    if overlap < 0.3 * (line.x1 - line.x0).min(previous.x1 - previous.x0) {
        return false;
    }
    let tolerance = 0.6 * size;
    let (left, right) = (
        group.iter().map(|l| l.x0).fold(f32::MAX, f32::min),
        group.iter().map(|l| l.x1).fold(f32::MIN, f32::max),
    );
    let centre = |l: &Line| (l.x0 + l.x1) / 2.0;
    let edge = (line.x0 - left).abs() <= tolerance
        || (line.x1 - right).abs() <= tolerance
        || (centre(line) - centre(previous)).abs() <= tolerance
        || (line.x1 - previous.x1).abs() <= tolerance
        || (line.x0 - previous.x0).abs() <= tolerance;
    let indented = group.len() == 1 && (line.x0 - previous.x0).abs() <= 3.5 * size
        || group.len() == 1 && (line.x1 - previous.x1).abs() <= 3.5 * size;
    edge || indented
}

fn block_of(lines: Vec<Line>) -> Block {
    let n = lines.len();
    let x0 = lines.iter().map(|l| l.x0).fold(f32::MAX, f32::min);
    let x1 = lines.iter().map(|l| l.x1).fold(f32::MIN, f32::max);
    let size = most_common(lines.iter().map(|l| l.size.round() as i64)) as f32;
    let font = most_common(lines.iter().map(|l| l.font as i64)) as usize;
    let rtl = {
        let rtl_lines = lines.iter().filter(|l| l.rtl).count();
        rtl_lines * 2 > n || (rtl_lines * 2 == n && lines[0].rtl)
    };
    // Lines joined by a space in the style of the line it ends.
    let mut chars: Vec<(char, RunStyle)> = Vec::new();
    for line in &lines {
        if let Some(&(_, style)) = chars.last() {
            chars.push((' ', style));
        }
        for run in &line.runs {
            chars.extend(line.text[run.range.clone()].chars().map(|c| (c, run.style)));
        }
    }
    let (text, runs) = runs_of(&chars);
    let pitch = (n > 1).then(|| (lines[n - 1].baseline - lines[0].baseline) / (n - 1) as f32);
    let mut shown: Vec<usize> = lines.iter().flat_map(|l| l.shown.iter().copied()).collect();
    shown.sort_unstable();
    shown.dedup();

    // Alignment from the lines' edges. The first line may be indented.
    let tolerance = 0.5 * size + 0.5;
    let spread = |value: &dyn Fn(&Line) -> f32, skip_first: bool| -> f32 {
        let values: Vec<f32> = lines.iter().skip(usize::from(skip_first && n > 2)).map(value).collect();
        values.iter().copied().fold(f32::MIN, f32::max) - values.iter().copied().fold(f32::MAX, f32::min)
    };
    let (left, right, centre) = (
        spread(&|l: &Line| l.x0, true),
        spread(&|l: &Line| l.x1, true),
        spread(&|l: &Line| (l.x0 + l.x1) / 2.0, false),
    );
    let align = if n == 1 {
        if rtl { BlockAlign::Right } else { BlockAlign::Left }
    } else if n >= 3 && left <= tolerance && {
        // Justified: every line but the last reaches the right edge too.
        let inner = &lines[..n - 1];
        let reach = inner.iter().map(|l| l.x1).fold(f32::MIN, f32::max);
        inner.iter().all(|l| (l.x1 - reach).abs() <= tolerance) && lines[n - 1].x1 <= reach + tolerance
    } {
        BlockAlign::Justify
    } else {
        let candidates = [(BlockAlign::Left, left), (BlockAlign::Right, right), (BlockAlign::Center, centre)];
        let best = candidates.iter().filter(|(_, s)| *s <= tolerance).min_by(|a, b| a.1.total_cmp(&b.1));
        match best {
            // Ties between the flush sides go to the paragraph's start side.
            Some(&(_, s)) if left <= tolerance && right <= tolerance && (left - s).abs() < f32::EPSILON => {
                if rtl {
                    BlockAlign::Right
                } else {
                    BlockAlign::Left
                }
            }
            Some(&(align, _)) => align,
            None => {
                if rtl {
                    BlockAlign::Right
                } else {
                    BlockAlign::Left
                }
            }
        }
    };
    let lines_color = lines[0].color;
    let top = lines[0].baseline - lines[0].size * 0.95;
    let bottom = lines[n - 1].baseline + lines[n - 1].size * 0.3;
    Block {
        lines,
        text,
        rtl,
        align,
        x0,
        x1,
        top,
        bottom,
        size,
        color: lines_color,
        font,
        pitch,
        runs,
        shown,
        shared: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atom(text: &str, x0: f32, x1: f32, baseline: f32, shown: usize) -> Atom {
        Atom { x0, x1, baseline, size: 10.0, text: text.into(), shown, font: 0, color: [0.0; 3] }
    }

    /// Glyphs of `text` at 5 pt each from `x`, one operator per line.
    fn row(text: &str, x: f32, baseline: f32, shown: usize) -> Vec<Atom> {
        text.chars()
            .enumerate()
            .map(|(i, c)| atom(&c.to_string(), x + 5.0 * i as f32, x + 5.0 * (i + 1) as f32, baseline, shown))
            .collect()
    }

    /// `row`, in font `font` and colour `color`.
    fn styled(text: &str, x: f32, baseline: f32, font: usize, color: [f32; 3]) -> Vec<Atom> {
        row(text, x, baseline, font).into_iter().map(|a| Atom { font, color, ..a }).collect()
    }

    /// A block's runs as (text, font, red?) triples.
    fn runs(block: &Block) -> Vec<(&str, usize, bool)> {
        block
            .runs
            .iter()
            .map(|r| (&block.text[r.range.clone()], r.style.font, r.style.color[0] > 0.5))
            .collect()
    }

    #[test]
    fn styles_become_runs_in_logical_order_and_spaces_take_the_style_before_them() {
        // "The rent is due" with "rent" in font 1, at 5 pt a glyph and a space's gap between words.
        let black = [0.0; 3];
        let mut atoms = styled("The", 10.0, 100.0, 0, black);
        atoms.extend(styled("rent", 30.0, 100.0, 1, black));
        atoms.extend(styled("is", 55.0, 100.0, 0, black));
        atoms.extend(styled("due", 70.0, 100.0, 0, black));
        let blocks = analyze(atoms);
        assert_eq!(blocks[0].text, "The rent is due");
        assert_eq!(runs(&blocks[0]), vec![("The ", 0, false), ("rent ", 1, false), ("is due", 0, false)]);

        // Arabic drawn left to right, its middle word red: "يدفع الإيجار شهريا".
        let red = [1.0, 0.0, 0.0];
        let mut atoms = styled("ايرهش", 10.0, 100.0, 0, black);
        let visual: String = "الإيجار".chars().rev().collect();
        atoms.extend(styled(&visual, 45.0, 100.0, 0, red));
        atoms.extend(styled("عفدي", 90.0, 100.0, 0, black));
        let blocks = analyze(atoms);
        assert_eq!(blocks[0].text, "يدفع الإيجار شهريا");
        assert_eq!(runs(&blocks[0]), vec![("يدفع ", 0, false), ("الإيجار ", 0, true), ("شهريا", 0, false)]);
    }

    #[test]
    fn a_paragraph_joins_its_lines_runs() {
        let black = [0.0; 3];
        // A bold word ending the first line (the line's font is its most common).
        let mut atoms = styled("first", 10.0, 100.0, 0, black);
        atoms.extend(styled("of", 45.0, 100.0, 1, black));
        atoms.extend(styled("three", 10.0, 112.0, 0, black));
        atoms.extend(styled("four", 40.0, 112.0, 0, black));
        atoms.extend(styled("five", 10.0, 124.0, 0, black));
        let blocks = analyze(atoms);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, "first of three four five");
        // The space ending a line takes that line's last style.
        assert_eq!(
            runs(&blocks[0]),
            vec![("first ", 0, false), ("of ", 1, false), ("three four five", 0, false)]
        );
        let covered: usize = blocks[0].runs.iter().map(|r| r.range.len()).sum();
        assert_eq!(covered, blocks[0].text.len());
    }

    #[test]
    fn glyphs_become_lines_with_spaces_at_gaps() {
        let mut atoms = row("Hello", 10.0, 100.0, 0);
        atoms.extend(row("world", 40.0, 100.0, 0)); // a 10 pt gap: a space
        let blocks = analyze(atoms);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, "Hello world");
        assert!(!blocks[0].rtl);
    }

    #[test]
    fn stacked_lines_make_one_paragraph_and_a_gap_makes_two() {
        let mut atoms = Vec::new();
        for (i, (text, y)) in
            [("one two three", 100.0), ("four five six seven", 112.0), ("eight nine", 124.0)]
                .into_iter()
                .enumerate()
        {
            atoms.extend(row(text, 10.0, y, i));
        }
        atoms.extend(row("next paragraph", 10.0, 170.0, 3));
        let blocks = analyze(atoms);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].lines.len(), 3);
        assert_eq!(blocks[0].pitch, Some(12.0));
        assert_eq!(blocks[0].align, BlockAlign::Left);
        assert_eq!(blocks[1].lines.len(), 1);
    }

    #[test]
    fn columns_on_one_baseline_are_separate_blocks() {
        let mut atoms = row("left column", 10.0, 100.0, 0);
        atoms.extend(row("right column", 200.0, 100.0, 1));
        assert_eq!(analyze(atoms).len(), 2);
    }

    #[test]
    fn arabic_in_visual_order_reads_in_logical_order() {
        // "سلام" drawn left to right as م ا ل س.
        let blocks = analyze(row("مالس", 10.0, 100.0, 0));
        assert_eq!(blocks[0].text, "سلام");
        assert!(blocks[0].rtl);
        assert_eq!(blocks[0].align, BlockAlign::Right);
    }

    #[test]
    fn a_latin_word_inside_arabic_keeps_its_order() {
        // Logical: "ثم iPhone ثم" laid out right to left: visual left to right is م ث, iPhone, م ث.
        let visual = "مث iPhone مث";
        let blocks = analyze(row(visual, 10.0, 100.0, 0));
        assert_eq!(blocks[0].text, "ثم iPhone ثم");
    }

    #[test]
    fn marks_stay_on_their_letter() {
        let mut atoms = vec![atom("ب", 10.0, 15.0, 100.0, 0), atom("\u{064E}", 11.0, 11.0, 100.0, 0)];
        atoms.push(atom("ا", 5.0, 10.0, 100.0, 0));
        let blocks = analyze(atoms);
        assert_eq!(blocks[0].text, "ب\u{064E}ا");
    }

    #[test]
    fn an_operator_shared_by_two_blocks_is_flagged() {
        let mut atoms = row("left", 10.0, 100.0, 0);
        atoms.extend(row("right", 200.0, 100.0, 0));
        let blocks = analyze(atoms);
        assert!(blocks.iter().all(|b| b.shared));
    }

    #[test]
    fn centred_and_justified_paragraphs_are_recognised() {
        let mut centred = Vec::new();
        for (i, (text, y)) in
            [("short", 100.0), ("a longer line", 112.0), ("mid size", 124.0)].into_iter().enumerate()
        {
            let width = 5.0 * text.chars().count() as f32;
            centred.extend(row(text, 100.0 - width / 2.0, y, i));
        }
        assert_eq!(analyze(centred)[0].align, BlockAlign::Center);

        let mut justified = Vec::new();
        for (i, y) in [100.0, 112.0, 124.0].into_iter().enumerate() {
            justified.extend(row("aaaa bbbb cccc dddd", 10.0, y, i));
        }
        justified.extend(row("end", 10.0, 136.0, 3));
        assert_eq!(analyze(justified)[0].align, BlockAlign::Justify);
    }
}
