//! Laid-out text drawn as filled glyph outlines instead of text (ADR 0050):
//! for watermarks, which are decoration. As text, a watermark across a page
//! would be read into copy, search, and text export, and at an angle it reads
//! back as scattered letters; as shapes it looks the same and reads as nothing.
//!
//! The outlines come from the same font subsets the text writer embeds
//! (`typeset::subset_fonts`), which are static TrueType instances (a variable
//! face is instanced at its variation), so the shapes match the text exactly.
//! Simple and composite `glyf` glyphs are supported; quadratic curves become
//! cubic ones, filled with the non-zero winding rule as TrueType expects.

use std::fmt::Write as _;

use read_fonts::tables::glyf::{Anchor, Glyf, Glyph};
use read_fonts::tables::loca::Loca;
use read_fonts::types::GlyphId;
use read_fonts::{FontRef, TableProvider};

use crate::error::{Error, Result};
use crate::typeset::{FontFace, TextLayout, subset_fonts};

/// Composite glyphs nest at most this deep.
const MAX_COMPONENT_DEPTH: usize = 8;

/// A 2×3 affine map from font units to the drawing's space.
type Affine = [f32; 6];

fn apply(m: &Affine, x: f32, y: f32) -> (f32, f32) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

/// `inner` then `outer`.
fn then(inner: &Affine, outer: &Affine) -> Affine {
    [
        inner[0] * outer[0] + inner[1] * outer[2],
        inner[0] * outer[1] + inner[1] * outer[3],
        inner[2] * outer[0] + inner[3] * outer[2],
        inner[2] * outer[1] + inner[3] * outer[3],
        inner[4] * outer[0] + inner[5] * outer[2] + outer[4],
        inner[4] * outer[1] + inner[5] * outer[3] + outer[5],
    ]
}

/// Draws `layout` (of `text`, set with `faces`) as filled paths with its box's
/// top-left corner at (`left`, `top`) in the content stream's space (y up).
pub(crate) fn draw_outlines(
    text: &str,
    layout: &TextLayout,
    faces: &[FontFace],
    (left, top): (f32, f32),
    color: [f32; 3],
) -> Result<Vec<u8>> {
    let subsets = subset_fonts(text, layout, faces).map_err(|err| Error::Typeset(err.code()))?;
    let unreadable = || Error::Typeset("font_unreadable");
    let mut tables = Vec::new();
    for subset in &subsets {
        let font = FontRef::new(&subset.data).map_err(|_| unreadable())?;
        let loca = font.loca(None).map_err(|_| unreadable())?;
        let glyf = font.glyf().map_err(|_| unreadable())?;
        let upem = f32::from(font.head().map_err(|_| unreadable())?.units_per_em().max(1));
        tables.push((subset, loca, glyf, upem));
    }

    let mut out = String::new();
    let [r, g, b] = color;
    let _ = writeln!(out, "{} {} {} rg", num(r), num(g), num(b));
    for line in &layout.lines {
        let baseline = top - line.baseline;
        for run in &line.runs {
            let Some((subset, loca, glyf, upem)) = tables.iter().find(|(s, ..)| s.font == run.font) else {
                continue;
            };
            let scale = run.size / upem;
            for glyph in &run.glyphs {
                let Some(cid) = subset.cid(glyph.id) else { continue };
                let at = [scale, 0.0, 0.0, scale, left + glyph.x, baseline + glyph.y_offset];
                let mut path = String::new();
                glyph_path(loca, glyf, GlyphId::new(u32::from(cid)), &at, 0, &mut path);
                if !path.is_empty() {
                    out.push_str(&path);
                    out.push_str("f\n");
                }
            }
        }
    }
    Ok(out.into_bytes())
}

/// Appends a glyph's contours as path operators.
fn glyph_path(loca: &Loca<'_>, glyf: &Glyf<'_>, id: GlyphId, at: &Affine, depth: usize, out: &mut String) {
    if depth > MAX_COMPONENT_DEPTH {
        return;
    }
    let Ok(Some(glyph)) = loca.get_glyf(id, glyf) else { return };
    match glyph {
        Glyph::Simple(simple) => {
            let points: Vec<_> = simple.points().collect();
            let mut start = 0usize;
            for end in simple.end_pts_of_contours() {
                let end = usize::from(end.get());
                if end < start || end >= points.len() {
                    break;
                }
                let contour: Vec<(f32, f32, bool)> = points[start..=end]
                    .iter()
                    .map(|p| {
                        let (x, y) = apply(at, f32::from(p.x), f32::from(p.y));
                        (x, y, p.on_curve)
                    })
                    .collect();
                contour_path(&contour, out);
                start = end + 1;
            }
        }
        Glyph::Composite(composite) => {
            for component in composite.components() {
                let (dx, dy) = match component.anchor {
                    Anchor::Offset { x, y } => (f32::from(x), f32::from(y)),
                    Anchor::Point { .. } => (0.0, 0.0),
                };
                let t = &component.transform;
                let own = [t.xx.to_f32(), t.yx.to_f32(), t.xy.to_f32(), t.yy.to_f32(), dx, dy];
                glyph_path(loca, glyf, component.glyph, &then(&own, at), depth + 1, out);
            }
        }
    }
}

/// One closed TrueType contour: on-curve points joined by lines, off-curve
/// points as quadratic control points (two in a row imply an on-curve point
/// between them), written as cubic curves.
fn contour_path(points: &[(f32, f32, bool)], out: &mut String) {
    let n = points.len();
    if n < 2 {
        return;
    }
    let mid = |a: (f32, f32, bool), b: (f32, f32, bool)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    // Start at an on-curve point, or between two off-curve ones.
    let (first, start) = match points.iter().position(|p| p.2) {
        Some(i) => ((points[i].0, points[i].1), i),
        None => (mid(points[0], points[1]), 0),
    };
    let _ = writeln!(out, "{} {} m", num(first.0), num(first.1));
    let mut current = first;
    let mut control: Option<(f32, f32)> = None;
    for k in 1..=n {
        let p = points[(start + k) % n];
        let point = (p.0, p.1);
        if p.2 {
            match control.take() {
                Some(c) => quad(out, current, c, point),
                None => {
                    let _ = writeln!(out, "{} {} l", num(point.0), num(point.1));
                }
            }
            current = point;
        } else if let Some(c) = control {
            let implied = ((c.0 + point.0) / 2.0, (c.1 + point.1) / 2.0);
            quad(out, current, c, implied);
            current = implied;
            control = Some(point);
        } else {
            control = Some(point);
        }
    }
    if let Some(c) = control {
        quad(out, current, c, first);
    }
    out.push_str("h\n");
}

/// A quadratic curve from `p0` through control `c` to `p1`, as a cubic.
fn quad(out: &mut String, p0: (f32, f32), c: (f32, f32), p1: (f32, f32)) {
    let c1 = (p0.0 + 2.0 / 3.0 * (c.0 - p0.0), p0.1 + 2.0 / 3.0 * (c.1 - p0.1));
    let c2 = (p1.0 + 2.0 / 3.0 * (c.0 - p1.0), p1.1 + 2.0 / 3.0 * (c.1 - p1.1));
    let _ = writeln!(
        out,
        "{} {} {} {} {} {} c",
        num(c1.0),
        num(c1.1),
        num(c2.0),
        num(c2.1),
        num(p1.0),
        num(p1.1)
    );
}

fn num(value: f32) -> String {
    crate::annotate::num(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_square_of_on_curve_points_is_four_lines() {
        let mut out = String::new();
        let square = [(0.0, 0.0, true), (10.0, 0.0, true), (10.0, 10.0, true), (0.0, 10.0, true)];
        contour_path(&square, &mut out);
        assert_eq!(out, "0 0 m\n10 0 l\n10 10 l\n0 10 l\n0 0 l\nh\n");
    }

    #[test]
    fn off_curve_points_become_curves_through_implied_points() {
        let mut out = String::new();
        // A diamond of control points only: four curves through the midpoints.
        let circle = [(0.0, 10.0, false), (10.0, 0.0, false), (0.0, -10.0, false), (-10.0, 0.0, false)];
        contour_path(&circle, &mut out);
        assert!(out.starts_with("5 5 m\n"), "{out}");
        assert_eq!(out.matches(" c\n").count(), 4, "{out}");
    }
}
