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
    /// Several fonts, sizes, or colours were seen: editing sets it in one.
    pub(crate) mixed: bool,
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
    let mut visual: Vec<(String, Class)> = Vec::new();
    for (i, unit) in units.iter().enumerate() {
        if i > 0 {
            let gap = unit.x0 - units[i - 1].x1;
            let spaced = unit.text.trim().is_empty() || units[i - 1].text.trim().is_empty();
            if gap > SPACE_GAP_EMS * size && !spaced {
                visual.push((" ".into(), Class::N));
            }
        }
        visual.push((unit.text.clone(), class_of(&unit.text)));
    }
    let letters = |wanted: Class| visual.iter().filter(|(_, c)| *c == wanted).count();
    let rtl = letters(Class::R) > letters(Class::L);

    // Neutrals take the class of the strong text on both sides, else the line's.
    let paragraph = if rtl { Class::R } else { Class::L };
    let mut classes: Vec<Class> = visual.iter().map(|(_, c)| *c).collect();
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
    let mut runs: Vec<(Class, Vec<&str>)> = Vec::new();
    for ((text, _), class) in visual.iter().zip(&classes) {
        match runs.last_mut() {
            Some((c, items)) if c == class => items.push(text),
            _ => runs.push((*class, vec![text])),
        }
    }
    if rtl {
        runs.reverse();
    }
    let mut text = String::new();
    for (class, items) in runs {
        let items: Vec<&str> = if class == Class::R { items.into_iter().rev().collect() } else { items };
        for item in items {
            text.push_str(item);
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
    Line {
        color,
        x0: units.iter().map(|u| u.x0).fold(f32::MAX, f32::min),
        x1: units.iter().map(|u| u.x1).fold(f32::MIN, f32::max),
        baseline: atoms[units[0].atom].baseline,
        size,
        font,
        text: collapse_spaces(&text),
        rtl,
        shown,
    }
}

fn collapse_spaces(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut previous_space = false;
    for c in text.chars() {
        let space = c == ' ' || c == '\u{a0}';
        if !(space && previous_space) {
            out.push(c);
        }
        previous_space = space;
    }
    out.trim().to_string()
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
    if dy <= 0.5 * size || (line.size - previous.size).abs() > 0.12 * size || line.font != previous.font {
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
    let text = lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join(" ");
    let pitch = (n > 1).then(|| (lines[n - 1].baseline - lines[0].baseline) / (n - 1) as f32);
    let mut shown: Vec<usize> = lines.iter().flat_map(|l| l.shown.iter().copied()).collect();
    shown.sort_unstable();
    shown.dedup();
    let mixed =
        lines.iter().any(|l| (l.size - size).abs() > 0.6 || l.font != font || l.color != lines[0].color);

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
        mixed,
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
