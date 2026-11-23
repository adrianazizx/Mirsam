//! Finding fields on a flat form (ADR 0047): an assist that suggests where
//! fields go, for the designer to review before any is made.
//!
//! What a printed form leaves blank, it draws:
//!
//! - **Lines** to write on (rules, or runs of underscores in the text): a
//!   text field sits on the line, as long as it is, after its label.
//! - **Boxes**, stroked or made of four lines (a table's cells too): an
//!   empty one is a text field; a small square is a check box. A box with
//!   text in it is a label, not a field.
//! - **Box characters** (☐, □): check boxes.
//!
//! Each suggestion is named after the label beside it: the text before it
//! on its line, in the label's own direction (an Arabic label is to the
//! right of its field), else the text just above it.

use std::collections::HashSet;

use serde::Serialize;

use super::FieldKind;
use crate::text::{Direction, PageText, Rect};
use crate::view::ViewRect;

/// A straight line on the page (page space), from a path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub from: (f32, f32),
    pub to: (f32, f32),
}

/// What the page draws that may mark a field: straight lines, and
/// rectangles (left, bottom, right, top; outlined or filled).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PageShapes {
    pub segments: Vec<Segment>,
    pub rects: Vec<[f32; 4]>,
}

/// A field the page seems to ask for.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedField {
    /// A text field or a check box.
    pub kind: FieldKind,
    /// Its box in page space (left, bottom, right, top).
    pub rect: [f32; 4],
    /// Its box on the displayed page ([`crate::Document::detect_fields`]
    /// fills it in).
    pub view: Option<ViewRect>,
    /// From its label; empty when none was found.
    pub name: String,
    /// A box tall enough for several lines.
    pub multiline: bool,
}

/// Lines thinner than this are lines, not boxes.
const THIN: f32 = 2.5;
/// The shortest line written on.
const MIN_LINE: f32 = 36.0;
/// How far apart two collinear pieces may be and still be one line.
const JOIN: f32 = 1.5;
/// A check box's side.
const CHECK_MIN: f32 = 6.0;
const CHECK_MAX: f32 = 24.0;
/// A text box's height.
const BOX_MIN: f32 = 10.0;
const BOX_MAX: f32 = 120.0;
/// Taller boxes take several lines.
const MULTILINE: f32 = 36.0;
/// The height of a field on a line.
const LINE_FIELD: f32 = 16.0;
/// Pages drawing more lines than this are drawings, not forms.
const MAX_LINES: usize = 2_000;
/// The longest suggested name.
const MAX_NAME: usize = 60;

#[derive(Debug, Clone, Copy, PartialEq)]
struct HLine {
    y: f32,
    x0: f32,
    x1: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct VLine {
    x: f32,
    y0: f32,
    y1: f32,
}

/// The fields a flat page seems to ask for, avoiding the boxes in `existing`
/// (the page's widgets), top to bottom.
pub fn detect_fields(shapes: &PageShapes, text: &PageText, existing: &[[f32; 4]]) -> Vec<DetectedField> {
    let spans = spans_of(text);
    let (mut horizontal, mut vertical) = lines_of(shapes, &spans);
    if horizontal.len() + vertical.len() > MAX_LINES {
        return Vec::new();
    }
    horizontal = merge_h(horizontal);
    vertical = merge_v(vertical);

    let mut found: Vec<DetectedField> = Vec::new();
    let mut edges: HashSet<usize> = HashSet::new();
    for cell in cells(&horizontal, &vertical, &mut edges) {
        let [l, b, r, t] = cell;
        let (w, h) = (r - l, t - b);
        let square = (w - h).abs() <= (0.2 * w).max(2.0);
        if square && (CHECK_MIN..=CHECK_MAX).contains(&w) {
            found.push(suggestion(FieldKind::Checkbox, cell, false));
        } else if w >= MIN_LINE / 2.0 && (BOX_MIN..=BOX_MAX).contains(&h) && !has_text(&spans, cell) {
            let inner = [l + 1.0, b + 1.0, r - 1.0, t - 1.0];
            found.push(suggestion(FieldKind::Text, inner, h > MULTILINE));
        }
    }
    // Lines not the edge of a box: a field to write on.
    for (i, line) in horizontal.iter().enumerate() {
        if edges.contains(&i) || line.x1 - line.x0 < MIN_LINE {
            continue;
        }
        if let Some(rect) = on_line(line, &spans, &horizontal) {
            found.push(suggestion(FieldKind::Text, rect, false));
        }
    }
    // Box characters.
    for span in spans.iter().filter(|span| matches!(span.text.as_str(), "☐" | "□" | "▢" | "❑" | "❏"))
    {
        let r = span.rect;
        let side = r.width().max(r.height()).clamp(CHECK_MIN, CHECK_MAX);
        let (cx, cy) = ((r.left + r.right) / 2.0, (r.bottom + r.top) / 2.0);
        let rect = [cx - side / 2.0, cy - side / 2.0, cx + side / 2.0, cy + side / 2.0];
        found.push(suggestion(FieldKind::Checkbox, rect, false));
    }

    // Neither over a field the page has, nor twice.
    let mut kept: Vec<DetectedField> = Vec::new();
    for field in found {
        if existing.iter().any(|other| overlaps(field.rect, *other)) {
            continue;
        }
        if kept.iter().any(|other| overlaps(field.rect, other.rect)) {
            continue;
        }
        kept.push(field);
    }
    for field in &mut kept {
        field.name = label_for(field.rect, text);
    }
    // Top to bottom, then in the page's reading direction.
    let rtl = text.direction == Direction::Rtl;
    kept.sort_by(|a, b| {
        let row = |f: &DetectedField| (-f.rect[3] / 4.0).round();
        row(a)
            .total_cmp(&row(b))
            .then_with(|| if rtl { b.rect[2].total_cmp(&a.rect[2]) } else { a.rect[0].total_cmp(&b.rect[0]) })
    });
    kept
}

fn suggestion(kind: FieldKind, rect: [f32; 4], multiline: bool) -> DetectedField {
    DetectedField { kind, rect, view: None, name: String::new(), multiline }
}

/// One drawn character (or ligature) of the page's text, where it is drawn.
#[derive(Debug, Clone)]
struct Span {
    text: String,
    rect: Rect,
}

fn spans_of(text: &PageText) -> Vec<Span> {
    let mut out = Vec::new();
    for line in text.blocks.iter().flat_map(|block| &block.lines) {
        for span in &line.spans {
            let (Some(rect), Some(part)) = (span.rect, line.text.get(span.start..span.end)) else { continue };
            if part.trim().is_empty() {
                continue;
            }
            out.push(Span { text: part.to_string(), rect });
        }
    }
    out
}

/// The page's horizontal and vertical lines: segments, thin rectangles,
/// rectangles' edges, and runs of underscores.
fn lines_of(shapes: &PageShapes, spans: &[Span]) -> (Vec<HLine>, Vec<VLine>) {
    let mut horizontal = Vec::new();
    let mut vertical = Vec::new();
    for segment in &shapes.segments {
        let ((x0, y0), (x1, y1)) = (segment.from, segment.to);
        if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
            continue;
        }
        if (y1 - y0).abs() <= 0.5 && (x1 - x0).abs() >= CHECK_MIN {
            horizontal.push(HLine { y: (y0 + y1) / 2.0, x0: x0.min(x1), x1: x0.max(x1) });
        } else if (x1 - x0).abs() <= 0.5 && (y1 - y0).abs() >= CHECK_MIN {
            vertical.push(VLine { x: (x0 + x1) / 2.0, y0: y0.min(y1), y1: y0.max(y1) });
        }
    }
    for &[l, b, r, t] in &shapes.rects {
        if ![l, b, r, t].iter().all(|v| v.is_finite()) {
            continue;
        }
        let (w, h) = (r - l, t - b);
        if h <= THIN && w >= CHECK_MIN {
            horizontal.push(HLine { y: (b + t) / 2.0, x0: l, x1: r });
        } else if w <= THIN && h >= CHECK_MIN {
            vertical.push(VLine { x: (l + r) / 2.0, y0: b, y1: t });
        } else if w > THIN && h > THIN {
            horizontal.push(HLine { y: b, x0: l, x1: r });
            horizontal.push(HLine { y: t, x0: l, x1: r });
            vertical.push(VLine { x: l, y0: b, y1: t });
            vertical.push(VLine { x: r, y0: b, y1: t });
        }
    }
    // Runs of three underscores or more.
    let mut run: Option<Rect> = None;
    let mut count = 0;
    let flush = |run: &mut Option<Rect>, count: &mut usize, out: &mut Vec<HLine>| {
        if let Some(rect) = run.take()
            && *count >= 3
        {
            out.push(HLine { y: rect.bottom, x0: rect.left, x1: rect.right });
        }
        *count = 0;
    };
    for span in spans {
        if span.text.chars().all(|c| c == '_') {
            let joined = run.filter(|r| {
                (r.bottom - span.rect.bottom).abs() < 2.0
                    && (span.rect.left - r.right).abs() < 3.0 * span.rect.width().max(1.0)
            });
            match joined {
                Some(rect) => run = Some(rect.union(span.rect)),
                None => {
                    flush(&mut run, &mut count, &mut horizontal);
                    run = Some(span.rect);
                }
            }
            count += span.text.chars().count();
        } else {
            flush(&mut run, &mut count, &mut horizontal);
        }
    }
    flush(&mut run, &mut count, &mut horizontal);
    (horizontal, vertical)
}

/// Collinear horizontal pieces joined.
fn merge_h(mut lines: Vec<HLine>) -> Vec<HLine> {
    lines.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x0.total_cmp(&b.x0)));
    let mut out: Vec<HLine> = Vec::new();
    for line in lines {
        match out.last_mut() {
            Some(last) if (last.y - line.y).abs() <= 0.75 && line.x0 <= last.x1 + JOIN => {
                last.x1 = last.x1.max(line.x1);
            }
            _ => out.push(line),
        }
    }
    out
}

fn merge_v(mut lines: Vec<VLine>) -> Vec<VLine> {
    lines.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y0.total_cmp(&b.y0)));
    let mut out: Vec<VLine> = Vec::new();
    for line in lines {
        match out.last_mut() {
            Some(last) if (last.x - line.x).abs() <= 0.75 && line.y0 <= last.y1 + JOIN => {
                last.y1 = last.y1.max(line.y1);
            }
            _ => out.push(line),
        }
    }
    out
}

/// The boxes the lines close: between two horizontal lines with no other
/// between them, the spans between neighbouring vertical lines that reach
/// both. The horizontal lines used are noted in `edges`.
fn cells(horizontal: &[HLine], vertical: &[VLine], edges: &mut HashSet<usize>) -> Vec<[f32; 4]> {
    let tol = 1.5;
    let mut out = Vec::new();
    for (i, bottom) in horizontal.iter().enumerate() {
        for (j, top) in horizontal.iter().enumerate() {
            let height = top.y - bottom.y;
            if height < CHECK_MIN - tol || height > BOX_MAX {
                continue;
            }
            let (x0, x1) = (bottom.x0.max(top.x0), bottom.x1.min(top.x1));
            if x1 - x0 < CHECK_MIN - tol {
                continue;
            }
            let mut sides: Vec<f32> = vertical
                .iter()
                .filter(|v| {
                    v.x >= x0 - tol && v.x <= x1 + tol && v.y0 <= bottom.y + tol && v.y1 >= top.y - tol
                })
                .map(|v| v.x)
                .collect();
            sides.sort_by(f32::total_cmp);
            sides.dedup_by(|a, b| (*a - *b).abs() < tol);
            for pair in sides.windows(2) {
                let (l, r) = (pair[0], pair[1]);
                if r - l < CHECK_MIN - tol {
                    continue;
                }
                // Another line across the box splits it: the smaller boxes count.
                let split = horizontal.iter().enumerate().any(|(k, h)| {
                    k != i
                        && k != j
                        && h.y > bottom.y + tol
                        && h.y < top.y - tol
                        && h.x0 <= l + tol
                        && h.x1 >= r - tol
                });
                if split {
                    continue;
                }
                edges.insert(i);
                edges.insert(j);
                out.push([l, bottom.y, r, top.y]);
            }
        }
    }
    out
}

/// Whether text is drawn inside `rect` (a label cell, not a blank).
fn has_text(spans: &[Span], [l, b, r, t]: [f32; 4]) -> bool {
    spans.iter().any(|span| {
        let (cx, cy) = ((span.rect.left + span.rect.right) / 2.0, (span.rect.bottom + span.rect.top) / 2.0);
        cx > l && cx < r && cy > b && cy < t && !span.text.chars().all(|c| c == '_' || c == '.')
    })
}

/// A field written on `line`: as long as the line where no label overlaps
/// it, as tall as fits under whatever is above.
fn on_line(line: &HLine, spans: &[Span], horizontal: &[HLine]) -> Option<[f32; 4]> {
    let bottom = line.y + 1.0;
    // Text sitting on the line (a label printed over its start) shortens it.
    let mut free = vec![(line.x0, line.x1)];
    for span in spans {
        let r = span.rect;
        if span.text.chars().all(|c| c == '_' || c == '.' || c == '…') {
            continue;
        }
        if r.bottom > bottom + LINE_FIELD || r.top < line.y {
            continue;
        }
        free = free
            .into_iter()
            .flat_map(|(a, b)| {
                let (cut0, cut1) = (r.left - 2.0, r.right + 2.0);
                if cut1 <= a || cut0 >= b {
                    vec![(a, b)]
                } else {
                    [(a, cut0), (cut1, b)].into_iter().filter(|(x, y)| y - x > 0.0).collect()
                }
            })
            .collect();
    }
    let (x0, x1) = free.into_iter().max_by(|a, b| (a.1 - a.0).total_cmp(&(b.1 - b.0)))?;
    if x1 - x0 < MIN_LINE {
        return None;
    }
    // Up to what is above: the next line, or text, over the same stretch.
    let mut top = bottom + LINE_FIELD;
    for other in horizontal {
        if other.y > bottom + 2.0 && other.x1 > x0 && other.x0 < x1 {
            top = top.min(other.y - 1.0);
        }
    }
    for span in spans {
        let r = span.rect;
        if r.bottom >= bottom && r.right > x0 && r.left < x1 {
            top = top.min(r.bottom - 1.0);
        }
    }
    (top - bottom >= BOX_MIN * 0.8).then_some([x0, bottom, x1, top])
}

/// Whether two boxes share more than a third of the smaller one.
fn overlaps(a: [f32; 4], b: [f32; 4]) -> bool {
    let w = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let h = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let area = |r: [f32; 4]| (r[2] - r[0]) * (r[3] - r[1]);
    w * h > area(a).min(area(b)) / 3.0
}

/// The label of a field at `rect`: the text just before it on its line, in
/// the label's direction, else the text just above it.
fn label_for(rect: [f32; 4], text: &PageText) -> String {
    let [l, b, r, t] = rect;
    let middle = (b + t) / 2.0;
    let height = (t - b).max(8.0);
    let mut best: Option<(f32, String)> = None;
    for block in &text.blocks {
        for line in &block.lines {
            let bounds = line.bounds;
            let level = middle >= bounds.bottom - height / 2.0 && middle <= bounds.top + height / 2.0;
            let rtl = block.direction == Direction::Rtl;
            // The spans before the field: left of it for a left-to-right
            // label, right of it for a right-to-left one.
            let before: Vec<Rect> = line
                .spans
                .iter()
                .filter_map(|span| span.rect)
                .filter(|s| if rtl { s.left >= r - 2.0 } else { s.right <= l + 2.0 })
                .collect();
            if level && !before.is_empty() {
                let gap = if rtl {
                    before.iter().map(|s| s.left - r).fold(f32::INFINITY, f32::min)
                } else {
                    before.iter().map(|s| l - s.right).fold(f32::INFINITY, f32::min)
                };
                // The nearest stretch: from the field back to a wide gap.
                let mut sorted = before.clone();
                sorted.sort_by(
                    |a, c| if rtl { a.left.total_cmp(&c.left) } else { c.right.total_cmp(&a.right) },
                );
                let mut kept = Vec::new();
                let mut edge = if rtl { r } else { l };
                for span in sorted {
                    let distance = if rtl { span.left - edge } else { edge - span.right };
                    if distance > 3.0 * span.height().max(6.0) && !kept.is_empty() {
                        break;
                    }
                    edge = if rtl { span.right } else { span.left };
                    kept.push(span);
                }
                let label = clean(&text_of(line, &kept));
                if !label.is_empty() && best.as_ref().is_none_or(|(d, _)| gap < *d) {
                    best = Some((gap, label));
                }
            }
            // Above, overlapping it, close.
            let above =
                bounds.bottom >= t - 2.0 && bounds.bottom - t < 24.0 && bounds.right > l && bounds.left < r;
            if above {
                let gap = bounds.bottom - t + 1_000.0;
                let label = clean(&line.text);
                if !label.is_empty() && best.as_ref().is_none_or(|(d, _)| gap < *d) {
                    best = Some((gap, label));
                }
            }
        }
    }
    best.map(|(_, label)| label).unwrap_or_default()
}

/// The text of `line`'s spans drawn at `rects`, in reading order.
fn text_of(line: &crate::text::TextLine, rects: &[Rect]) -> String {
    let mut out = String::new();
    let mut gap = false;
    for span in &line.spans {
        match span.rect {
            Some(rect) if rects.contains(&rect) => {
                if gap && !out.is_empty() {
                    out.push(' ');
                }
                gap = false;
                out.push_str(line.text.get(span.start..span.end).unwrap_or(""));
            }
            None => gap = true,
            Some(_) => gap = !out.is_empty() || gap,
        }
    }
    out
}

/// A label as a field name: no leaders, colons, or full stops (which
/// separate a name's parts), spaces collapsed, at most 60 characters.
fn clean(label: &str) -> String {
    let words: Vec<&str> = label
        .split(|c: char| c.is_whitespace() || matches!(c, '_' | '…' | '.'))
        .map(|word| word.trim_matches(|c: char| matches!(c, ':' | '：' | '؛' | ';' | '*' | '(' | ')')))
        .filter(|word| !word.is_empty())
        .collect();
    words.join(" ").chars().take(MAX_NAME).collect::<String>().trim().to_string()
}

/// `base`, or `base 2`, `base 3`… whichever no name in `taken` has.
pub fn unique_name(base: &str, taken: &HashSet<String>) -> String {
    if !taken.contains(base) {
        return base.to_string();
    }
    (2..).map(|n| format!("{base} {n}")).find(|name| !taken.contains(name)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{TextBlock, TextLine, TextQuality, TextSpan};

    /// A line of text, one span per character, each `w` wide, starting at `x`.
    fn line(text: &str, x: f32, y: f32, w: f32, direction: Direction) -> TextBlock {
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        let n = chars.len();
        let spans: Vec<TextSpan> = chars
            .iter()
            .enumerate()
            .map(|(i, (at, c))| {
                // Right to left: the first character is drawn rightmost.
                let slot = if direction == Direction::Rtl { n - 1 - i } else { i };
                let left = x + slot as f32 * w;
                TextSpan {
                    start: *at,
                    end: at + c.len_utf8(),
                    rect: (!c.is_whitespace()).then_some(Rect {
                        left,
                        bottom: y,
                        right: left + w,
                        top: y + 10.0,
                    }),
                }
            })
            .collect();
        let bounds = Rect { left: x, bottom: y, right: x + n as f32 * w, top: y + 10.0 };
        TextBlock { direction, lines: vec![TextLine { text: text.to_string(), spans, bounds }], bounds }
    }

    fn page(blocks: Vec<TextBlock>, direction: Direction) -> PageText {
        PageText { blocks, direction, quality: TextQuality::Good }
    }

    fn rule(x0: f32, x1: f32, y: f32) -> Segment {
        Segment { from: (x0, y), to: (x1, y) }
    }

    #[test]
    fn a_line_after_a_label_is_a_text_field_named_after_it() {
        let text = page(vec![line("Name:", 72.0, 700.0, 6.0, Direction::Ltr)], Direction::Ltr);
        let shapes = PageShapes { segments: vec![rule(110.0, 300.0, 699.0)], rects: vec![] };
        let found = detect_fields(&shapes, &text, &[]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].kind, FieldKind::Text);
        assert_eq!(found[0].name, "Name");
        let [l, b, r, t] = found[0].rect;
        assert!(
            (l - 110.0).abs() < 1.0 && (r - 300.0).abs() < 1.0 && b >= 699.0 && t - b >= 10.0,
            "{:?}",
            found[0].rect
        );
    }

    #[test]
    fn an_arabic_label_names_the_line_to_its_left() {
        // "الاسم" drawn from x = 400 to 430, the line from 200 to 390.
        let text = page(vec![line("الاسم", 400.0, 700.0, 6.0, Direction::Rtl)], Direction::Rtl);
        let shapes = PageShapes { segments: vec![rule(200.0, 390.0, 699.0)], rects: vec![] };
        let found = detect_fields(&shapes, &text, &[]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].name, "الاسم");
    }

    #[test]
    fn underscores_are_lines_and_labels_are_cut_from_them() {
        let text = page(vec![line("Date: ______________", 72.0, 600.0, 6.0, Direction::Ltr)], Direction::Ltr);
        let found = detect_fields(&PageShapes::default(), &text, &[]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].name, "Date");
        assert!(found[0].rect[0] >= 72.0 + 5.0 * 6.0, "after the label: {:?}", found[0].rect);
    }

    #[test]
    fn boxes_become_fields_or_check_boxes_and_labelled_cells_do_not() {
        let text = page(
            vec![
                line("Agree", 100.0, 500.0, 6.0, Direction::Ltr),
                line("Address", 72.0, 430.0, 6.0, Direction::Ltr),
            ],
            Direction::Ltr,
        );
        let shapes = PageShapes {
            segments: vec![],
            rects: vec![
                [80.0, 499.0, 92.0, 511.0],  // a check box before "Agree"
                [72.0, 360.0, 400.0, 420.0], // an empty box under "Address": multi-line
                [60.0, 425.0, 140.0, 445.0], // the box around "Address": a label
            ],
        };
        let found = detect_fields(&shapes, &text, &[]);
        let kinds: Vec<(FieldKind, bool)> = found.iter().map(|f| (f.kind, f.multiline)).collect();
        assert_eq!(kinds, [(FieldKind::Checkbox, false), (FieldKind::Text, true)], "{found:?}");
        assert_eq!(found[1].name, "Address");
    }

    #[test]
    fn a_grid_of_lines_makes_cells_and_existing_fields_are_left_alone() {
        // Two rows of two cells: 100..300..500 across, 600..620..640 up.
        let mut segments =
            vec![rule(100.0, 500.0, 600.0), rule(100.0, 500.0, 620.0), rule(100.0, 500.0, 640.0)];
        for x in [100.0, 300.0, 500.0] {
            segments.push(Segment { from: (x, 600.0), to: (x, 640.0) });
        }
        let text = page(vec![], Direction::Ltr);
        let shapes = PageShapes { segments, rects: vec![] };
        assert_eq!(detect_fields(&shapes, &text, &[]).len(), 4);
        let existing = [[100.0, 620.0, 300.0, 640.0]];
        assert_eq!(detect_fields(&shapes, &text, &existing).len(), 3);
    }

    #[test]
    fn names_are_cleaned_and_made_unique() {
        assert_eq!(clean("  Full name (as in passport):  "), "Full name as in passport");
        assert_eq!(clean("Ref. no.:"), "Ref no");
        let taken: HashSet<String> = ["Name".to_string(), "Name 2".to_string()].into();
        assert_eq!(unique_name("Name", &taken), "Name 3");
        assert_eq!(unique_name("الاسم", &taken), "الاسم");
    }
}
