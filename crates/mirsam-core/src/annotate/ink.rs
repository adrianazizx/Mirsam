//! Freehand ink (ADR 0018): `/InkList` for every viewer, and an appearance
//! whose width follows the pen's pressure.

use super::{Appearance, Content, bounds, check_color, in_range, num};
use crate::error::{Error, Result};
use crate::pdf::{Dict, Object};
use crate::view::ViewTransform;

/// A point of a stroke in page space, with the pen's pressure (0 to 1; a
/// mouse reports 0.5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InkPoint {
    pub x: f32,
    pub y: f32,
    pub pressure: f32,
}

impl InkPoint {
    /// The page-space point under a point of the displayed page.
    pub fn from_view(x: f32, y: f32, pressure: f32, view: &ViewTransform) -> Self {
        let (x, y) = view.page_point(x, y);
        Self { x, y, pressure }
    }
}

/// An ink annotation to add.
#[derive(Debug, Clone, PartialEq)]
pub struct Ink {
    /// Each stroke from pen-down to pen-up, in order.
    pub strokes: Vec<Vec<InkPoint>>,
    /// RGB, each 0 to 1.
    pub color: [f32; 3],
    /// The line width at the middle pressure (0.5), in points.
    pub width: f32,
}

const MAX_STROKES: usize = 1_000;
const MAX_POINTS: usize = 200_000;

impl Ink {
    /// A dark blue, like a ballpoint pen.
    pub const DEFAULT_COLOR: [f32; 3] = [0.05, 0.22, 0.6];
    pub const DEFAULT_WIDTH: f32 = 2.0;

    /// `/InkList` (where other viewers draw from) and `/BS` (its width).
    pub(crate) fn entries(&self) -> Vec<(Vec<u8>, Object)> {
        let real = |value: f32| Object::Real(f64::from(value));
        let list = self
            .strokes
            .iter()
            .map(|stroke| Object::Array(stroke.iter().flat_map(|p| [real(p.x), real(p.y)]).collect()))
            .collect();
        let border = Dict(vec![(b"W".to_vec(), real(self.width))]);
        vec![(b"InkList".to_vec(), Object::Array(list)), (b"BS".to_vec(), Object::Dict(border))]
    }

    /// Drawn in page space: each run of segments of one width is one path,
    /// with round caps and joins, so the runs meet without seams.
    pub(crate) fn appearance(&self) -> Result<Appearance> {
        let count: usize = self.strokes.iter().map(Vec::len).sum();
        if self.strokes.is_empty()
            || self.strokes.len() > MAX_STROKES
            || self.strokes.iter().any(Vec::is_empty)
            || count > MAX_POINTS
        {
            return Err(Error::InvalidEdit("ink needs 1 to 1,000 strokes of at most 200,000 points"));
        }
        if self.strokes.iter().flatten().any(|p| !in_range((p.x, p.y)) || !p.pressure.is_finite()) {
            return Err(Error::InvalidEdit("ink lies outside the page"));
        }
        if !(0.1..=100.0).contains(&self.width) {
            return Err(Error::InvalidEdit("ink is 0.1 to 100 points wide"));
        }
        check_color(self.color)?;

        let widest = self.width * 1.5;
        let [left, bottom, right, top] = bounds(self.strokes.iter().flatten().map(|p| (p.x, p.y)));
        let margin = widest / 2.0 + 0.5;
        let rect = [left - margin, bottom - margin, right + margin, top + margin];

        let mut out = Content::default();
        let [r, g, b] = self.color;
        out.op(&format!("{} {} {} RG 1 J 1 j", num(r), num(g), num(b)));
        for stroke in &self.strokes {
            let point = |p: &InkPoint| (p.x, p.y);
            if let [only] = stroke.as_slice() {
                // A tap is a dot: a zero-length line with round caps.
                out.op(&format!("{} w", num(self.width_at(only.pressure))));
                out.move_to(point(only));
                out.line_to(point(only));
                out.op("S");
                continue;
            }
            let mut current: Option<String> = None;
            for pair in stroke.windows(2) {
                let width = num(self.width_at((pair[0].pressure + pair[1].pressure) / 2.0));
                if current.as_deref() != Some(width.as_str()) {
                    if current.is_some() {
                        out.op("S");
                    }
                    out.op(&format!("{width} w"));
                    out.move_to(point(&pair[0]));
                    current = Some(width);
                }
                out.line_to(point(&pair[1]));
            }
            out.op("S");
        }
        Ok(Appearance::in_page_space(rect, Dict::default(), out.0))
    }

    /// Half as wide at no pressure, half again as wide at full pressure, in
    /// steps of a tenth of a point (fewer width changes to write).
    fn width_at(&self, pressure: f32) -> f32 {
        let width = self.width * (0.5 + pressure.clamp(0.0, 1.0));
        (width * 10.0).round() / 10.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(points: &[(f32, f32, f32)]) -> Vec<InkPoint> {
        points.iter().map(|&(x, y, pressure)| InkPoint { x, y, pressure }).collect()
    }

    fn content(ink: &Ink) -> String {
        String::from_utf8(ink.appearance().expect("valid").content).expect("ascii")
    }

    #[test]
    fn harder_pressure_draws_wider() {
        let ink = Ink {
            strokes: vec![stroke(&[(0.0, 0.0, 0.5), (10.0, 0.0, 0.5), (20.0, 0.0, 1.0), (30.0, 0.0, 1.0)])],
            color: Ink::DEFAULT_COLOR,
            width: 2.0,
        };
        let content = content(&ink);
        // Mouse pressure: the nominal width; then wider, a new path.
        assert!(
            content.contains("2 w\n0 0 m\n10 0 l\nS\n2.5 w\n10 0 m\n20 0 l\nS\n3 w\n20 0 m\n30 0 l\nS"),
            "{content}"
        );
        let appearance = ink.appearance().expect("valid");
        // Room for the widest line around the points.
        assert_eq!(appearance.rect, [-2.0, -2.0, 32.0, 2.0]);
    }

    #[test]
    fn a_tap_is_a_dot_and_strokes_stay_apart() {
        let ink = Ink {
            strokes: vec![stroke(&[(5.0, 5.0, 0.5)]), stroke(&[(0.0, 10.0, 0.5), (8.0, 12.0, 0.5)])],
            color: [0.0; 3],
            width: 1.0,
        };
        let content = content(&ink);
        assert!(content.contains("1 w\n5 5 m\n5 5 l\nS"));
        assert_eq!(content.matches(" m\n").count(), 2);
        let list = &ink.entries()[0].1;
        assert!(matches!(list, Object::Array(strokes) if strokes.len() == 2));
    }

    #[test]
    fn bad_ink_is_refused() {
        let empty = Ink { strokes: vec![], color: [0.0; 3], width: 1.0 };
        assert!(matches!(empty.appearance(), Err(Error::InvalidEdit(_))));
        let wild = Ink { strokes: vec![stroke(&[(f32::NAN, 0.0, 0.5)])], color: [0.0; 3], width: 1.0 };
        assert!(matches!(wild.appearance(), Err(Error::InvalidEdit(_))));
        let thin = Ink { strokes: vec![stroke(&[(0.0, 0.0, 0.5)])], color: [0.0; 3], width: 0.0 };
        assert!(matches!(thin.appearance(), Err(Error::InvalidEdit(_))));
    }
}
