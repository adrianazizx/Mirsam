//! A page's text as editable blocks (spec §7, steps 1–2; ADR 0023).
//!
//! The text extractor (ADR 0008) already rebuilds lines and blocks in reading
//! order from PDFium's characters. This adds what editing needs:
//!
//! - **style:** each block as runs of one font, size, and colour, in logical
//!   order (PDFium's per-character font name, size, and fill colour);
//! - **shape:** alignment (left, right, centred, justified) from the lines'
//!   edges, and the distance between baselines;
//! - **sources:** the content-stream operations that drew the block
//!   (`pdf::content`), which in-place editing removes before writing the new
//!   text. PDFium makes one text object per show operation, but numbers its
//!   characters in reading order, which for right-to-left text is not the
//!   content's order (Chrome draws Arabic left to right, a glyph per
//!   operation). So the characters are grouped by their text object, and each
//!   group is traced to an operation, not yet traced, that starts at one of
//!   its characters' origins with the same font, wherever it is in the
//!   content. Operations that drew no character at all (glyphs without a
//!   Unicode mapping, such as dots drawn apart from Arabic letters) go to the
//!   block they are drawn in. Characters that cannot be traced (text inside
//!   form XObjects, content this reader cannot decode) are counted, not
//!   guessed.

use std::collections::{BTreeSet, HashMap};
use std::ops::Range;

use crate::edit::{PageTree, Start};
use crate::error::{Error, Result};
use crate::pdf::Object;
use crate::pdf::content::{ShowOp, show_ops};
use crate::pdf::crypt::Crypt;
use crate::text::{Direction, PageText, Rect};

/// A block's alignment, from the edges of its lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockAlign {
    Left,
    Right,
    Center,
    /// Every line but the last fills the width.
    Justified,
}

/// How a run of text is set.
#[derive(Debug, Clone, PartialEq)]
pub struct RunStyle {
    /// The font's name without a subset tag (`ABCDEF+Amiri-Regular` → `Amiri-Regular`).
    pub font: String,
    /// Points, as drawn.
    pub size: f32,
    /// RGB, each 0 to 1.
    pub color: [f32; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct StyledRun {
    /// Byte range in the block's text.
    pub range: Range<usize>,
    pub style: RunStyle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EditLine {
    /// Byte range in the block's text (without the line break after it).
    pub range: Range<usize>,
    /// Page space.
    pub baseline: f32,
    pub left: f32,
    pub right: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EditBlock {
    /// In logical order, lines joined by `\n`.
    pub text: String,
    pub direction: Direction,
    pub align: BlockAlign,
    /// Whether `align` was seen (two lines or more) or assumed from the direction.
    pub align_seen: bool,
    pub bounds: Rect,
    pub lines: Vec<EditLine>,
    /// Baseline to baseline (the median); `None` for a single line.
    pub line_pitch: Option<f32>,
    pub runs: Vec<StyledRun>,
    /// Indices into [`PageAnalysis::ops`] of the operations that draw it.
    pub sources: Vec<usize>,
    /// Characters of the block not traced to an operation.
    pub untraced: usize,
    /// One of its operations also draws another block's text.
    pub shared: bool,
    /// It was written by this app (`/MirsamText`, ADR 0021).
    pub written_here: bool,
}

/// A text-showing operation of the page's content, as the edit model reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceOp {
    /// Which of the page's content streams, and its bytes there (decoded).
    pub stream: usize,
    pub range: Range<usize>,
    /// The font's `/BaseFont`, without a subset tag (empty if unknown).
    pub font: String,
    pub size: f32,
    /// Where its first glyph starts (page space); `None` when it continues
    /// the operation before.
    pub origin: Option<(f32, f32)>,
    pub mirsam: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PageAnalysis {
    pub blocks: Vec<EditBlock>,
    pub ops: Vec<SourceOp>,
    /// The page's whole content could be read.
    pub complete: bool,
}

/// What PDFium reports of one character.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CharInfo {
    pub(crate) index: usize,
    pub(crate) generated: bool,
    pub(crate) font: String,
    pub(crate) size: f32,
    pub(crate) color: [f32; 3],
    pub(crate) origin: (f32, f32),
    /// PDFium's text object holding it, by its bounds and matrix (in
    /// hundredths): the same for every character one show operation drew.
    pub(crate) object: Option<[i32; 8]>,
}

/// `ABCDEF+Name` → `Name`.
pub(crate) fn without_subset_tag(name: &str) -> &str {
    match name.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.bytes().all(|b| b.is_ascii_uppercase()) => rest,
        _ => name,
    }
}

/// The page's text-showing operations, with each one's font `/BaseFont`, and
/// whether its whole content could be read.
pub(crate) fn page_ops(
    bytes: &[u8],
    password: Option<&str>,
    page: usize,
) -> Result<(Vec<ShowOp>, Vec<String>, bool)> {
    let Start { file, .. } = Start::read(bytes)?;
    let tree = PageTree::read(&file)?;
    let leaf =
        tree.leaves.get(page).ok_or(Error::PageOutOfRange { index: page, count: tree.leaves.len() })?;
    let crypt = Crypt::of(&file, password)?;
    let contents: Vec<Object> = match leaf.dict.get(b"Contents") {
        None | Some(Object::Null) => Vec::new(),
        Some(Object::Array(items)) => items.clone(),
        Some(reference @ Object::Ref(..)) => match file.resolve(reference) {
            Some(Object::Array(items)) => items,
            _ => vec![reference.clone()],
        },
        Some(_) => Vec::new(),
    };
    let (streams, complete) = decoded_contents(&file, crypt.as_ref(), &contents);
    let (ops, read) = show_ops(&streams);
    // Each operation's font, by its resource name.
    let fonts = tree
        .attribute(leaf, b"Resources")
        .and_then(|value| file.resolve(&value))
        .and_then(|value| value.as_dict().cloned())
        .and_then(|resources| resources.get(b"Font").and_then(|value| file.resolve(value)))
        .and_then(|value| value.as_dict().cloned())
        .unwrap_or_default();
    let base_font = |resource: &[u8]| -> String {
        fonts
            .get(resource)
            .and_then(|font| file.resolve(font))
            .and_then(|font| {
                font.as_dict().and_then(|d| d.get(b"BaseFont")).and_then(Object::as_name).map(<[u8]>::to_vec)
            })
            .map(|name| without_subset_tag(&String::from_utf8_lossy(&name)).to_string())
            .unwrap_or_default()
    };
    let names = ops.iter().map(|op| base_font(&op.font)).collect();
    Ok((ops, names, complete && read))
}

/// The page's content streams (`/Contents` items), decoded, and whether all
/// of them could be (an unreadable one is empty).
pub(crate) fn decoded_contents(
    file: &crate::pdf::File<'_>,
    crypt: Option<&Crypt>,
    contents: &[Object],
) -> (Vec<Vec<u8>>, bool) {
    let mut complete = true;
    let mut streams = Vec::new();
    for item in contents {
        let decoded = (|| {
            let &Object::Ref(number, generation) = item else { return None };
            let crate::pdf::Entry::Offset(offset) = *file.entries.get(&number)? else { return None };
            let (dict, data) = file.indirect_at(offset)?;
            let data = data?;
            let data = match crypt {
                Some(crypt) => crypt.decrypt_stream(number, generation, data)?,
                None => data.to_vec(),
            };
            crate::pdf::decode_stream(&dict, &data, file)
        })();
        match decoded {
            Some(data) => streams.push(data),
            None => {
                complete = false;
                streams.push(Vec::new());
            }
        }
    }
    (streams, complete)
}

/// Which operation drew each character (by position in `chars`), or `None`.
pub(crate) fn trace(chars: &[CharInfo], ops: &[ShowOp], fonts: &[String]) -> Vec<Option<usize>> {
    let same_font = |op: usize, ch: &CharInfo| {
        let font = fonts.get(op).map(String::as_str).unwrap_or("");
        font.is_empty() || ch.font.is_empty() || without_subset_tag(&ch.font) == font
    };
    // Consecutive characters of one PDFium text object. A character in no
    // text object (a word space PDFium reports at the next letter's origin)
    // was drawn by no operation of its own.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (i, ch) in chars.iter().enumerate() {
        if ch.generated || ch.object.is_none() {
            continue;
        }
        let joins = groups.last().and_then(|group| group.last()).is_some_and(|&last| {
            let before = &chars[last];
            ch.object.is_some()
                && before.object == ch.object
                && without_subset_tag(&before.font) == without_subset_tag(&ch.font)
        });
        match groups.last_mut() {
            Some(group) if joins => group.push(i),
            _ => groups.push(vec![i]),
        }
    }

    // Operations by where they start, to the tenth of a point.
    let key = |(x, y): (f32, f32)| ((x * 10.0).round() as i64, (y * 10.0).round() as i64);
    let mut starting: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (j, op) in ops.iter().enumerate() {
        if let Some(origin) = op.origin {
            starting.entry(key(origin)).or_default().push(j);
        }
    }
    // Each group's candidates: the operations starting at one of its
    // characters' origins, with its font.
    let candidates: Vec<Vec<usize>> = groups
        .iter()
        .map(|group| {
            let first = &chars[group[0]];
            let mut found: Vec<usize> = group
                .iter()
                .flat_map(|&i| {
                    let (x, y) = key(chars[i].origin);
                    [-1, 0, 1]
                        .into_iter()
                        .flat_map(move |dx| [-1, 0, 1].into_iter().map(move |dy| (x + dx, y + dy)))
                })
                .filter_map(|k| starting.get(&k))
                .flatten()
                .copied()
                .filter(|&j| same_font(j, first))
                .collect();
            found.sort_unstable();
            found.dedup();
            found
        })
        .collect();
    // The groups with the fewest choices choose first (a letter and the mark
    // drawn from its origin compete for the same starting points); each takes
    // the earliest operation left.
    let mut order: Vec<usize> = (0..groups.len()).collect();
    order.sort_by_key(|&g| (candidates[g].len(), g));
    let mut taken = vec![false; ops.len()];
    let mut traced = vec![None; chars.len()];
    let mut unmatched = Vec::new();
    for g in order {
        match candidates[g].iter().copied().find(|&j| !taken[j]) {
            Some(j) => {
                taken[j] = true;
                for &i in &groups[g] {
                    traced[i] = Some(j);
                }
            }
            None => unmatched.push(&groups[g]),
        }
    }
    // A mark drawn with an offset from its letter starts near, not at, the
    // origin PDFium reports for it: the nearest operation left within half
    // an em, with its font.
    let mut still = Vec::new();
    for group in unmatched {
        let first = &chars[group[0]];
        let reach = 0.5 * first.size.max(1.0);
        let distance = |j: usize| {
            let (x, y) = ops[j].origin?;
            group
                .iter()
                .map(|&i| ((chars[i].origin.0 - x).powi(2) + (chars[i].origin.1 - y).powi(2)).sqrt())
                .reduce(f32::min)
                .filter(|d| *d <= reach)
        };
        let nearest = (0..ops.len())
            .filter(|&j| !taken[j] && same_font(j, first))
            .filter_map(|j| distance(j).map(|d| (d, j)))
            .min_by(|a, b| a.0.total_cmp(&b.0));
        match nearest {
            Some((_, j)) => {
                taken[j] = true;
                for &i in group {
                    traced[i] = Some(j);
                }
            }
            None => still.push(group),
        }
    }
    // A show operation that continues the pen of the one before has no
    // starting point: it goes to a group on the same baseline as that one.
    for group in still {
        let first = &chars[group[0]];
        let found = (0..ops.len()).find(|&j| {
            if taken[j] || ops[j].origin.is_some() || !same_font(j, first) {
                return false;
            }
            let before = (0..j).rev().find_map(|k| ops[k].origin);
            before.is_some_and(|(_, y)| (y - first.origin.1).abs() <= 0.5 * first.size.max(1.0))
        });
        if let Some(j) = found {
            taken[j] = true;
            for &i in group {
                traced[i] = Some(j);
            }
        }
    }
    traced
}

/// The page's blocks, with style, shape, and sources.
pub(crate) fn build(
    page_text: &PageText,
    chars: &[CharInfo],
    ops: &[ShowOp],
    fonts: &[String],
    complete: bool,
) -> PageAnalysis {
    let traced = trace(chars, ops, fonts);
    let position: HashMap<usize, usize> = chars.iter().enumerate().map(|(i, ch)| (ch.index, i)).collect();
    let op_of = |char_index: usize| position.get(&char_index).and_then(|&i| traced[i]);
    // A space no text object drew (see `trace`) has nothing to trace.
    let objectless_space =
        |char_index: usize| position.get(&char_index).is_some_and(|&i| chars[i].object.is_none());
    let style_of = |char_index: usize| {
        position.get(&char_index).map(|&i| {
            let ch = &chars[i];
            RunStyle { font: without_subset_tag(&ch.font).to_string(), size: ch.size, color: ch.color }
        })
    };

    let mut blocks = Vec::new();
    let mut owners: HashMap<usize, BTreeSet<usize>> = HashMap::new();
    for (block_index, block) in page_text.blocks.iter().enumerate() {
        let mut text = String::new();
        let mut lines = Vec::new();
        let mut runs: Vec<StyledRun> = Vec::new();
        let mut sources = BTreeSet::new();
        let mut untraced = 0;
        for (n, line) in block.lines.iter().enumerate() {
            if n > 0 {
                text.push('\n');
                if let Some(last) = runs.last_mut() {
                    last.range.end = text.len();
                }
            }
            let offset = text.len();
            text.push_str(&line.text);
            lines.push(EditLine {
                range: offset..text.len(),
                baseline: line.baseline,
                left: line.bounds.left,
                right: line.bounds.right,
            });
            for span in &line.spans {
                let range = offset + span.start..offset + span.end;
                let whitespace = line.text.get(span.start..span.end).is_some_and(|t| t.trim().is_empty());
                for &c in &span.chars {
                    match op_of(c) {
                        Some(op) => {
                            sources.insert(op);
                        }
                        None if whitespace && objectless_space(c) => {}
                        None => untraced += 1,
                    }
                }
                // An inferred space takes the style before it.
                let style = span.chars.first().and_then(|&c| style_of(c));
                match (runs.last_mut(), style) {
                    (Some(last), None) => last.range.end = range.end,
                    (Some(last), Some(style)) if same_style(&last.style, &style) => {
                        last.range.end = range.end
                    }
                    (_, Some(style)) => runs.push(StyledRun { range, style }),
                    (None, None) => {}
                }
            }
        }
        // Operations with no origin of their own continue the one before.
        let mut with_continuations = sources.clone();
        for &op in &sources {
            let mut next = op + 1;
            while next < ops.len() && ops[next].origin.is_none() {
                with_continuations.insert(next);
                next += 1;
            }
        }
        for &op in &with_continuations {
            owners.entry(op).or_default().insert(block_index);
        }
        let (align, align_seen) = alignment(block.direction, &lines, &runs);
        let mut baselines: Vec<f32> =
            lines.windows(2).map(|pair| (pair[0].baseline - pair[1].baseline).abs()).collect();
        baselines.sort_by(f32::total_cmp);
        let written_here =
            !with_continuations.is_empty() && with_continuations.iter().all(|&op| ops[op].mirsam);
        blocks.push(EditBlock {
            text,
            direction: block.direction,
            align,
            align_seen,
            bounds: block.bounds,
            lines,
            line_pitch: baselines.get(baselines.len() / 2).copied(),
            runs,
            sources: with_continuations.into_iter().collect(),
            untraced,
            shared: false,
            written_here,
        });
    }
    // Operations that drew no character (glyphs without a Unicode mapping:
    // the dots Chrome's Type 3 fonts and CoreText draw apart from Arabic
    // letters) belong to the block they are drawn in: the smallest one whose
    // box, with a little room, holds their starting point.
    let mut orphans: Vec<(usize, usize)> = Vec::new();
    for (j, op) in ops.iter().enumerate() {
        if owners.contains_key(&j) {
            continue;
        }
        let Some((x, y)) = op.origin else { continue };
        let room = 0.3 * op.drawn_size.max(1.0);
        let holder = blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                let r = b.bounds;
                x >= r.left - room && x <= r.right + room && y >= r.bottom - room && y <= r.top + room
            })
            .min_by(|(_, a), (_, b)| {
                let area = |r: Rect| (r.right - r.left) * (r.top - r.bottom);
                area(a.bounds).total_cmp(&area(b.bounds))
            })
            .map(|(i, _)| i);
        if let Some(block) = holder {
            orphans.push((block, j));
            // Continuations of an orphan go with it.
            let mut next = j + 1;
            while next < ops.len() && ops[next].origin.is_none() && !owners.contains_key(&next) {
                orphans.push((block, next));
                next += 1;
            }
        }
    }
    for (block, op) in orphans {
        let sources = &mut blocks[block].sources;
        if let Err(at) = sources.binary_search(&op) {
            sources.insert(at, op);
        }
        owners.entry(op).or_default().insert(block);
    }
    for block in &mut blocks {
        block.shared = block.sources.iter().any(|op| owners.get(op).is_some_and(|set| set.len() > 1));
    }
    let ops = ops
        .iter()
        .zip(fonts)
        .map(|(op, font)| SourceOp {
            stream: op.stream,
            range: op.range.clone(),
            font: font.clone(),
            size: op.drawn_size,
            origin: op.origin,
            mirsam: op.mirsam,
        })
        .collect();
    PageAnalysis { blocks, ops, complete }
}

fn same_style(a: &RunStyle, b: &RunStyle) -> bool {
    a.font == b.font
        && (a.size - b.size).abs() < 0.1
        && a.color.iter().zip(&b.color).all(|(x, y)| (x - y).abs() < 0.01)
}

/// The block's alignment from its lines' edges; assumed from the direction
/// for a single line.
fn alignment(direction: Direction, lines: &[EditLine], runs: &[StyledRun]) -> (BlockAlign, bool) {
    let start = if direction == Direction::Rtl { BlockAlign::Right } else { BlockAlign::Left };
    if lines.len() < 2 {
        return (start, false);
    }
    let size = runs.iter().map(|run| run.style.size).fold(0.0_f32, f32::max).max(1.0);
    let tolerance = (0.1 * size).max(0.75);
    let lefts: Vec<f32> = lines.iter().map(|l| l.left).collect();
    let rights: Vec<f32> = lines.iter().map(|l| l.right).collect();
    let aligned = |values: &[f32], target: f32| values.iter().all(|v| (v - target).abs() <= tolerance);
    let (min_left, max_right) =
        (lefts.iter().copied().fold(f32::MAX, f32::min), rights.iter().copied().fold(f32::MIN, f32::max));
    let body = lines.len() - 1;
    // Two lines cannot tell justified from ragged text: a ragged paragraph's
    // first line fills its width too.
    let can_justify = body >= 2;
    let (left_all, right_all) = (aligned(&lefts, min_left), aligned(&rights, max_right));
    let (left_body, right_body) = (aligned(&lefts[..body], min_left), aligned(&rights[..body], max_right));
    let centers: Vec<f32> = lines.iter().map(|l| (l.left + l.right) / 2.0).collect();
    let center = centers.iter().sum::<f32>() / centers.len() as f32;
    let align = match direction {
        Direction::Ltr if can_justify && left_all && right_body && !right_all => BlockAlign::Justified,
        Direction::Rtl if can_justify && right_all && left_body && !left_all => BlockAlign::Justified,
        _ if left_all && right_all => start,
        Direction::Ltr if left_all => BlockAlign::Left,
        Direction::Rtl if right_all => BlockAlign::Right,
        _ if left_all => BlockAlign::Left,
        _ if right_all => BlockAlign::Right,
        _ if aligned(&centers, center) => BlockAlign::Center,
        _ => start,
    };
    (align, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::content::show_ops;

    fn ch(index: usize, font: &str, origin: (f32, f32), object: i32) -> CharInfo {
        CharInfo {
            index,
            generated: false,
            font: font.into(),
            size: 10.0,
            color: [0.0; 3],
            origin,
            object: Some([object, 0, 0, 0, 0, 0, 0, 0]),
        }
    }

    #[test]
    fn text_objects_are_traced_to_the_operation_that_starts_at_one_of_their_characters() {
        let (ops, _) = show_ops(&[b"BT /F1 10 Tf 10 100 Td (ab) Tj (c) Tj 0 -20 Td (de) Tj ET".to_vec()]);
        let fonts = vec!["Helv".to_string(); ops.len()];
        let chars = [
            ch(0, "Helv", (10.0, 100.0), 1),
            ch(1, "Helv", (16.0, 100.0), 1),
            // `(c) Tj` continues the pen: no starting point, simply the next operation.
            ch(2, "Helv", (22.0, 100.0), 2),
            // A right-to-left object: PDFium lists its letters in reading order,
            // so the one at the operation's start comes last.
            ch(3, "ABCDEF+Helv", (16.0, 80.0), 3),
            ch(4, "Helv", (10.0, 80.0), 3),
            ch(5, "Other", (40.0, 300.0), 4), // drawn by a form: not traced
        ];
        assert_eq!(trace(&chars, &ops, &fonts), [Some(0), Some(0), Some(1), Some(2), Some(2), None]);
    }

    #[test]
    fn subset_tags_are_dropped_only_when_they_are_tags() {
        assert_eq!(without_subset_tag("ABCDEF+Amiri-Regular"), "Amiri-Regular");
        assert_eq!(without_subset_tag("Helvetica"), "Helvetica");
        assert_eq!(without_subset_tag("Abc+Def"), "Abc+Def");
    }

    #[test]
    fn alignment_comes_from_the_lines_edges() {
        let line = |left: f32, right: f32, baseline: f32| EditLine { range: 0..0, baseline, left, right };
        let runs =
            [StyledRun { range: 0..1, style: RunStyle { font: "F".into(), size: 10.0, color: [0.0; 3] } }];
        let justified = [line(10.0, 200.0, 700.0), line(10.0, 200.0, 686.0), line(10.0, 120.0, 672.0)];
        assert_eq!(alignment(Direction::Ltr, &justified, &runs), (BlockAlign::Justified, true));
        let left = [line(10.0, 190.0, 700.0), line(10.0, 160.0, 686.0)];
        assert_eq!(alignment(Direction::Ltr, &left, &runs).0, BlockAlign::Left);
        // Two lines, the first full: ragged or justified cannot be told; the start.
        let two = [line(10.0, 200.0, 700.0), line(10.0, 120.0, 686.0)];
        assert_eq!(alignment(Direction::Ltr, &two, &runs).0, BlockAlign::Left);
        let rtl_justified = [line(10.0, 200.0, 700.0), line(80.0, 200.0, 686.0)];
        assert_eq!(
            alignment(Direction::Rtl, &rtl_justified, &runs).0,
            BlockAlign::Right,
            "two lines, last short: right"
        );
        let rtl_full = [line(10.0, 200.0, 700.0), line(10.0, 200.0, 686.0), line(90.0, 200.0, 672.0)];
        assert_eq!(alignment(Direction::Rtl, &rtl_full, &runs).0, BlockAlign::Justified);
        let centred = [line(40.0, 160.0, 700.0), line(60.0, 140.0, 686.0)];
        assert_eq!(alignment(Direction::Ltr, &centred, &runs).0, BlockAlign::Center);
        assert_eq!(alignment(Direction::Rtl, &[line(0.0, 50.0, 700.0)], &runs), (BlockAlign::Right, false));
    }
}
