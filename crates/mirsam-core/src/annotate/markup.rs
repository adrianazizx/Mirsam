//! Text markup: highlight, underline, strikethrough, squiggly (ADR 0017).

use super::{Appearance, Content, MAX_COORDINATE, bounds, check_color, num, offset};
use crate::error::{Error, Result};
use crate::pdf::{Dict, Object};
use crate::view::{ViewRect, ViewTransform};
use serde::{Deserialize, Serialize};

/// The four text-markup annotations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MarkupKind {
    Highlight,
    Underline,
    StrikeOut,
    Squiggly,
}

impl MarkupKind {
    pub(crate) fn subtype(self) -> &'static [u8] {
        match self {
            MarkupKind::Highlight => b"Highlight",
            MarkupKind::Underline => b"Underline",
            MarkupKind::StrikeOut => b"StrikeOut",
            MarkupKind::Squiggly => b"Squiggly",
        }
    }

    pub(crate) fn from_subtype(name: &[u8]) -> Option<Self> {
        [MarkupKind::Highlight, MarkupKind::Underline, MarkupKind::StrikeOut, MarkupKind::Squiggly]
            .into_iter()
            .find(|kind| kind.subtype() == name)
    }

    /// The colour (RGB, 0 to 1) a new markup of this kind is drawn in.
    pub fn default_color(self) -> [f32; 3] {
        match self {
            // Yellow under the text, multiplied, so the text stays black.
            MarkupKind::Highlight => [1.0, 0.86, 0.0],
            MarkupKind::StrikeOut => [0.86, 0.13, 0.13],
            MarkupKind::Underline | MarkupKind::Squiggly => [0.0, 0.42, 0.9],
        }
    }
}

/// A quadrilateral of page space (points, y up), in `/QuadPoints` order: the
/// upper-left, upper-right, lower-left, and lower-right corners of the text
/// it covers, "upper" meaning toward the top of that text, however the page
/// is rotated.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quad {
    pub upper_left: (f32, f32),
    pub upper_right: (f32, f32),
    pub lower_left: (f32, f32),
    pub lower_right: (f32, f32),
}

impl Quad {
    /// The quad under a rectangle of the displayed page, whose top edge is the
    /// top of the text (as the viewer shows a selection).
    pub fn from_view(rect: ViewRect, view: &ViewTransform) -> Self {
        let (right, bottom) = (rect.x + rect.width, rect.y + rect.height);
        Self {
            upper_left: view.page_point(rect.x, rect.y),
            upper_right: view.page_point(right, rect.y),
            lower_left: view.page_point(rect.x, bottom),
            lower_right: view.page_point(right, bottom),
        }
    }

    fn points(&self) -> [(f32, f32); 4] {
        [self.upper_left, self.upper_right, self.lower_left, self.lower_right]
    }

    /// The text's height, and its unit "up" vector.
    fn up(&self) -> (f32, (f32, f32)) {
        let (dx, dy) = (self.upper_left.0 - self.lower_left.0, self.upper_left.1 - self.lower_left.1);
        let height = dx.hypot(dy);
        if height > 0.0 { (height, (dx / height, dy / height)) } else { (0.0, (0.0, 1.0)) }
    }
}

/// A markup annotation to add.
#[derive(Debug, Clone, PartialEq)]
pub struct Markup {
    pub kind: MarkupKind,
    /// One quad per run of marked text, in reading order.
    pub quads: Vec<Quad>,
    /// RGB, each 0 to 1.
    pub color: [f32; 3],
}

const MAX_QUADS: usize = 10_000;

impl Markup {
    /// `/QuadPoints`.
    pub(crate) fn entries(&self) -> Vec<(Vec<u8>, Object)> {
        let real = |value: f32| Object::Real(f64::from(value));
        let quad_points =
            self.quads.iter().flat_map(|quad| quad.points()).flat_map(|(x, y)| [real(x), real(y)]).collect();
        vec![(b"QuadPoints".to_vec(), Object::Array(quad_points))]
    }

    /// Drawn in page space (`/BBox` = `/Rect`).
    pub(crate) fn appearance(&self) -> Result<Appearance> {
        if self.quads.is_empty() || self.quads.len() > MAX_QUADS {
            return Err(Error::InvalidEdit("markup needs between one and 10,000 quads"));
        }
        let points = self.quads.iter().flat_map(Quad::points);
        if points.clone().any(|(x, y)| !(x.abs() <= MAX_COORDINATE && y.abs() <= MAX_COORDINATE)) {
            return Err(Error::InvalidEdit("markup lies outside the page"));
        }
        check_color(self.color)?;
        let rect = bounds(points);
        if rect[2] - rect[0] <= 0.0 || rect[3] - rect[1] <= 0.0 {
            return Err(Error::InvalidEdit("markup covers nothing"));
        }
        let mut out = Content::default();
        let [r, g, b] = self.color;
        match self.kind {
            MarkupKind::Highlight => {
                out.op(&format!("/G0 gs {} {} {} rg", num(r), num(g), num(b)));
                for quad in &self.quads {
                    out.move_to(quad.upper_left);
                    out.line_to(quad.upper_right);
                    out.line_to(quad.lower_right);
                    out.line_to(quad.lower_left);
                    out.op("h f");
                }
            }
            MarkupKind::Underline | MarkupKind::StrikeOut => {
                out.op(&format!("{} {} {} RG 0 J", num(r), num(g), num(b)));
                for quad in &self.quads {
                    let (height, up) = quad.up();
                    if height <= 0.0 {
                        continue;
                    }
                    let width = (height * 0.07).max(0.5);
                    // An underline just inside the bottom of the line box (below the
                    // descenders); a strikethrough through its middle.
                    let lift = if self.kind == MarkupKind::Underline { width } else { height / 2.0 };
                    out.op(&format!("{} w", num(width)));
                    out.move_to(offset(quad.lower_left, up, lift));
                    out.line_to(offset(quad.lower_right, up, lift));
                    out.op("S");
                }
            }
            MarkupKind::Squiggly => {
                out.op(&format!("{} {} {} RG 1 j 0 J", num(r), num(g), num(b)));
                for quad in &self.quads {
                    let (height, up) = quad.up();
                    let (dx, dy) =
                        (quad.lower_right.0 - quad.lower_left.0, quad.lower_right.1 - quad.lower_left.1);
                    let length = dx.hypot(dy);
                    if height <= 0.0 || length <= 0.0 {
                        continue;
                    }
                    let along = (dx / length, dy / length);
                    let width = (height * 0.05).max(0.4);
                    // A zigzag whose half-wave is an eighth of the line's height.
                    let step = (height / 8.0).max(1.0);
                    let amplitude = step / 2.0;
                    let base = offset(quad.lower_left, up, amplitude + width);
                    let steps = (length / step).ceil().min(10_000.0) as usize;
                    out.op(&format!("{} w", num(width)));
                    for i in 0..=steps {
                        let at = (i as f32 * step).min(length);
                        let side = if i % 2 == 0 { -amplitude } else { amplitude };
                        let point = offset(offset(base, along, at), up, side);
                        if i == 0 { out.move_to(point) } else { out.line_to(point) }
                    }
                    out.op("S");
                }
            }
        }
        let resources = if self.kind == MarkupKind::Highlight {
            // Multiplied with the page, so the text under it stays as dark as it was.
            let state = Dict(vec![
                (b"Type".to_vec(), Object::Name(b"ExtGState".to_vec())),
                (b"BM".to_vec(), Object::Name(b"Multiply".to_vec())),
            ]);
            Dict(vec![(
                b"ExtGState".to_vec(),
                Object::Dict(Dict(vec![(b"G0".to_vec(), Object::Dict(state))])),
            )])
        } else {
            Dict::default()
        };
        Ok(Appearance::in_page_space(rect, resources, out.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(left: f32, bottom: f32, right: f32, top: f32) -> Quad {
        Quad {
            upper_left: (left, top),
            upper_right: (right, top),
            lower_left: (left, bottom),
            lower_right: (right, bottom),
        }
    }

    #[test]
    fn a_highlight_fills_each_quad_multiplied() {
        let markup = Markup {
            kind: MarkupKind::Highlight,
            quads: vec![quad(10.0, 100.0, 60.0, 112.0), quad(10.0, 86.0, 30.0, 98.0)],
            color: [1.0, 0.86, 0.0],
        };
        let appearance = markup.appearance().expect("valid");
        assert_eq!(appearance.rect, [10.0, 86.0, 60.0, 112.0]);
        let content = String::from_utf8(appearance.content).expect("ascii");
        assert_eq!(content.lines().next(), Some("/G0 gs 1 0.86 0 rg"));
        assert_eq!(content.matches(" f").count(), 2);
        assert!(content.contains("10 112 m\n60 112 l\n60 100 l\n10 100 l\nh f"));
    }

    #[test]
    fn lines_follow_the_text_on_a_turned_page() {
        // Text running up the page: its "up" points to page-space -x.
        let turned = Quad {
            upper_left: (100.0, 10.0),
            upper_right: (100.0, 60.0),
            lower_left: (112.0, 10.0),
            lower_right: (112.0, 60.0),
        };
        let markup = Markup { kind: MarkupKind::StrikeOut, quads: vec![turned], color: [0.0, 0.0, 0.0] };
        let content = String::from_utf8(markup.appearance().expect("valid").content).expect("ascii");
        // Through the middle, from the text's start to its end.
        assert!(content.contains("106 10 m\n106 60 l\nS"), "{content}");
    }

    #[test]
    fn a_squiggle_zigzags_inside_the_quad() {
        let markup = Markup {
            kind: MarkupKind::Squiggly,
            quads: vec![quad(0.0, 0.0, 40.0, 16.0)],
            color: [0.0, 0.0, 1.0],
        };
        let appearance = markup.appearance().expect("valid");
        let content = String::from_utf8(appearance.content).expect("ascii");
        let ys: Vec<f32> = content
            .lines()
            .filter(|line| line.ends_with(" l") || line.ends_with(" m"))
            .filter_map(|line| line.split(' ').nth(1)?.parse().ok())
            .collect();
        assert_eq!(ys.len(), 21, "a point every 2 points along 40");
        assert!(ys.iter().all(|y| (0.0..=16.0).contains(y)));
        assert!(ys.windows(2).all(|pair| pair[0] != pair[1]), "alternates");
    }

    #[test]
    fn bad_markup_is_refused() {
        let empty = Markup { kind: MarkupKind::Underline, quads: vec![], color: [0.0; 3] };
        assert!(matches!(empty.appearance(), Err(Error::InvalidEdit(_))));
        let flat =
            Markup { kind: MarkupKind::Underline, quads: vec![quad(0.0, 5.0, 10.0, 5.0)], color: [0.0; 3] };
        assert!(matches!(flat.appearance(), Err(Error::InvalidEdit(_))));
        let wild = Markup {
            kind: MarkupKind::Highlight,
            quads: vec![quad(0.0, 0.0, f32::NAN, 5.0)],
            color: [0.0; 3],
        };
        assert!(matches!(wild.appearance(), Err(Error::InvalidEdit(_))));
        let colour = Markup {
            kind: MarkupKind::Highlight,
            quads: vec![quad(0.0, 0.0, 5.0, 5.0)],
            color: [2.0, 0.0, 0.0],
        };
        assert!(matches!(colour.appearance(), Err(Error::InvalidEdit(_))));
    }
}
