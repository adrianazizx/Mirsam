//! What an editor surface needs from a layout (spec §7, ADR 0022): where the
//! caret goes for each place in the text, and the glyphs' outlines, so it
//! can show exactly what will be written.

use std::ops::Range;

use icu_segmenter::GraphemeClusterSegmenter;
use read_fonts::tables::glyf::{Anchor, Glyph};
use read_fonts::types::GlyphId;
use read_fonts::{FontRef, TableProvider};

use super::TextLayout;

/// A place the caret can stand, on one line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaretStop {
    /// Byte offset into the text: the caret stands before this character.
    pub offset: usize,
    pub line: usize,
    /// Points from the left edge of the layout box.
    pub x: f32,
    /// The stop sits where two runs of different direction meet: the same
    /// offset has another stop elsewhere on the line (bidi caret ambiguity).
    pub split: bool,
}

/// Every caret stop of `layout` (of `text`): at each grapheme boundary of
/// each line, on the side of the cluster the text's direction puts it (never
/// between a letter and its marks). Inside a ligature (several graphemes, one
/// glyph) the stops divide its width evenly. Stops come line by line, in
/// visual order.
pub fn caret_stops(text: &str, layout: &TextLayout) -> Vec<CaretStop> {
    let graphemes: Vec<usize> = GraphemeClusterSegmenter::new().segment_str(text).collect();
    let boundaries_in = |range: &Range<usize>| -> Vec<usize> {
        graphemes.iter().copied().filter(|&g| g > range.start && g < range.end).collect()
    };
    let mut stops = Vec::new();
    for (line_index, line) in layout.lines.iter().enumerate() {
        let mut line_stops: Vec<CaretStop> = Vec::new();
        for run in &line.runs {
            for cluster in &run.clusters {
                let glyphs = &run.glyphs[cluster.glyphs.clone()];
                let Some(first) = glyphs.first() else { continue };
                let left = glyphs.iter().map(|g| g.x).fold(first.x, f32::min);
                let width: f32 = glyphs.iter().map(|g| g.advance).sum();
                let right = left + width;
                let inner = boundaries_in(&cluster.text);
                let pieces = inner.len() + 1;
                let at = |k: usize| {
                    let share = width * k as f32 / pieces as f32;
                    if run.rtl { right - share } else { left + share }
                };
                // A mark the shaper keeps apart from its letter is no place for the caret.
                let mut add = |offset: usize, x: f32| {
                    if graphemes.binary_search(&offset).is_ok() {
                        line_stops.push(CaretStop { offset, line: line_index, x, split: false });
                    }
                };
                add(cluster.text.start, at(0));
                for (k, &offset) in inner.iter().enumerate() {
                    add(offset, at(k + 1));
                }
                add(cluster.text.end, at(pieces));
            }
        }
        // One stop per offset where the two sides agree; both, marked, where they do not.
        line_stops.sort_by(|a, b| a.offset.cmp(&b.offset).then(a.x.total_cmp(&b.x)));
        let mut merged: Vec<CaretStop> = Vec::new();
        for stop in line_stops {
            match merged.last_mut() {
                Some(last) if last.offset == stop.offset && (last.x - stop.x).abs() < 0.01 => {}
                Some(last) if last.offset == stop.offset => {
                    last.split = true;
                    merged.push(CaretStop { split: true, ..stop });
                }
                _ => merged.push(stop),
            }
        }
        merged.sort_by(|a, b| a.x.total_cmp(&b.x));
        stops.extend(merged);
    }
    stops
}

/// The outline of glyph `id` of a static TrueType font (such as a subset from
/// [`super::subset_fonts`]) as SVG path data in font units, y up. `None` when
/// the font has no `glyf` outline for it (an empty glyph gives `""`).
pub fn glyph_outline(font: &[u8], id: u32) -> Option<String> {
    let font = FontRef::new(font).ok()?;
    let loca = font.loca(None).ok()?;
    let glyf = font.glyf().ok()?;
    let mut path = String::new();
    outline_into(&loca, &glyf, GlyphId::new(id), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], 0, &mut path)?;
    Some(path)
}

/// Units per em of a font, for scaling [`glyph_outline`]'s paths.
pub fn units_per_em(font: &[u8]) -> Option<u16> {
    Some(FontRef::new(font).ok()?.head().ok()?.units_per_em())
}

fn outline_into(
    loca: &read_fonts::tables::loca::Loca<'_>,
    glyf: &read_fonts::tables::glyf::Glyf<'_>,
    id: GlyphId,
    [a, b, c, d, e, f]: [f32; 6],
    depth: usize,
    path: &mut String,
) -> Option<()> {
    use std::fmt::Write as _;
    if depth > 8 {
        return None;
    }
    let Some(glyph) = loca.get_glyf(id, glyf).ok()? else { return Some(()) };
    let map = |x: f32, y: f32| (a * x + c * y + e, b * x + d * y + f);
    match glyph {
        Glyph::Simple(simple) => {
            let points: Vec<_> = simple.points().collect();
            let mut start = 0;
            for end in simple.end_pts_of_contours() {
                let end = usize::from(end.get());
                let contour = points.get(start..=end)?;
                start = end + 1;
                if contour.is_empty() {
                    continue;
                }
                let point = |i: usize| {
                    let p = contour[i % contour.len()];
                    (f32::from(p.x), f32::from(p.y), p.on_curve)
                };
                // Start on an on-curve point, or between two off-curve ones.
                let first_on = (0..contour.len()).find(|&i| point(i).2);
                let (begin, origin) = match first_on {
                    Some(i) => (i, (point(i).0, point(i).1)),
                    None => {
                        let (p, q) = (point(0), point(1));
                        (0, ((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0))
                    }
                };
                let (x, y) = map(origin.0, origin.1);
                let _ = write!(path, "M{x:.1} {y:.1}");
                let mut control: Option<(f32, f32)> =
                    if first_on.is_none() { Some((point(0).0, point(0).1)) } else { None };
                for step in 1..=contour.len() {
                    let (px, py, on) = point(begin + step);
                    match (on, control) {
                        (true, None) => {
                            let (x, y) = map(px, py);
                            let _ = write!(path, "L{x:.1} {y:.1}");
                        }
                        (true, Some((cx, cy))) => {
                            let ((cx, cy), (x, y)) = (map(cx, cy), map(px, py));
                            let _ = write!(path, "Q{cx:.1} {cy:.1} {x:.1} {y:.1}");
                            control = None;
                        }
                        (false, None) => control = Some((px, py)),
                        (false, Some((cx, cy))) => {
                            let (mx, my) = ((cx + px) / 2.0, (cy + py) / 2.0);
                            let ((cx, cy), (x, y)) = (map(cx, cy), map(mx, my));
                            let _ = write!(path, "Q{cx:.1} {cy:.1} {x:.1} {y:.1}");
                            control = Some((px, py));
                        }
                    }
                }
                if let Some((cx, cy)) = control {
                    let ((cx, cy), (x, y)) = (map(cx, cy), map(origin.0, origin.1));
                    let _ = write!(path, "Q{cx:.1} {cy:.1} {x:.1} {y:.1}");
                }
                path.push('Z');
            }
        }
        Glyph::Composite(composite) => {
            for component in composite.components() {
                let (dx, dy) = match component.anchor {
                    Anchor::Offset { x, y } => (f32::from(x), f32::from(y)),
                    Anchor::Point { .. } => (0.0, 0.0),
                };
                let t = component.transform;
                let (xx, yx, xy, yy) = (t.xx.to_f32(), t.yx.to_f32(), t.xy.to_f32(), t.yy.to_f32());
                // The component's own matrix, then this glyph's.
                let inner = [xx, yx, xy, yy, dx, dy];
                let outer = [
                    a * inner[0] + c * inner[1],
                    b * inner[0] + d * inner[1],
                    a * inner[2] + c * inner[3],
                    b * inner[2] + d * inner[3],
                    a * inner[4] + c * inner[5] + e,
                    b * inner[4] + d * inner[5] + f,
                ];
                outline_into(loca, glyf, component.glyph, outer, depth + 1, path)?;
            }
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typeset::{FontFace, ParagraphStyle, Span, layout, subset_fonts};

    fn arabic() -> FontFace {
        FontFace::from_bytes(
            include_bytes!("../../../../assets/fonts/IBMPlexSansArabic-Regular.ttf").to_vec(),
            0,
        )
        .expect("font")
    }

    fn latin() -> FontFace {
        FontFace::from_bytes(include_bytes!("../../../../assets/fonts/IBMPlexSans-VF.ttf").to_vec(), 0)
            .and_then(|face| face.with_variations(&[(*b"wght", 400.0)]))
            .expect("font")
    }

    #[test]
    fn caret_stops_follow_each_runs_direction() {
        // Mixed: an English word, then Arabic (a right-to-left run), one line.
        let text = "Sign وقّع";
        let faces = [latin(), arabic()];
        let laid = layout(text, &[Span::new(0..text.len(), 0, 12.0)], &faces, &ParagraphStyle::default())
            .expect("lays out");
        let stops = caret_stops(text, &laid);
        let x = |offset: usize| -> Vec<f32> {
            stops.iter().filter(|s| s.offset == offset).map(|s| s.x).collect()
        };
        // Left to right: S before i before g.
        assert!(x(0)[0] < x(1)[0] && x(1)[0] < x(2)[0]);
        // Right to left: و (byte 5) stands right of ق (byte 7).
        let (waw, qaf) = (x(5), x(7));
        assert!(waw.iter().cloned().fold(f32::MIN, f32::max) > qaf[0], "{waw:?} {qaf:?}");
        // The shadda (a mark) is no stop of its own: "قّ" is one grapheme.
        assert!(x(9).is_empty(), "no caret between a letter and its mark");
        // Where the two directions meet, the offset has two places.
        assert!(stops.iter().any(|s| s.split));
    }

    #[test]
    fn a_ligatures_width_is_shared_by_its_letters() {
        // Lam-alef: one glyph, two graphemes; the caret can stand between them.
        let text = "لا";
        let faces = [arabic()];
        let laid = layout(text, &[Span::new(0..text.len(), 0, 20.0)], &faces, &ParagraphStyle::default())
            .expect("lays out");
        let stops = caret_stops(text, &laid);
        let x = |offset: usize| stops.iter().find(|s| s.offset == offset).map(|s| s.x).expect("stop");
        let (start, middle, end) = (x(0), x(2), x(4));
        assert!(start > middle && middle > end, "right to left: {start} {middle} {end}");
        assert!(((start - middle) - (middle - end)).abs() < 0.01, "halves");
    }

    #[test]
    fn outlines_come_from_the_subset_written_to_the_pdf() {
        let text = "بيت House";
        let faces = [arabic(), latin()];
        let laid = layout(text, &[Span::new(0..text.len(), 0, 12.0)], &faces, &ParagraphStyle::default())
            .expect("lays out");
        let subsets = subset_fonts(text, &laid, &faces).expect("subsets");
        let mut drawn = 0;
        for subset in &subsets {
            assert!(units_per_em(&subset.data).is_some_and(|upem| upem >= 1000));
            for glyph in subset.glyphs.iter().skip(1) {
                let path = glyph_outline(&subset.data, u32::from(glyph.cid)).expect("an outline");
                // A space is empty; anything else is closed contours.
                if !path.is_empty() {
                    assert!(path.starts_with('M') && path.ends_with('Z'), "cid {}: {path}", glyph.cid);
                    drawn += 1;
                }
            }
        }
        // Three Arabic letter forms and five Latin letters.
        assert!(drawn >= 8, "{drawn} glyphs drawn");
    }
}
