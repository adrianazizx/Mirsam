//! Sticky notes (`/Text` annotations, ADR 0018): an icon on the page and
//! the note's text in `/Contents`, which viewers show in their own pop-ups
//! (so the text needs no shaping here; the app shows it in the UI's own,
//! bidi-aware text field).

use super::{Appearance, Content, bounds, check_color, check_contents, in_range, num, text_string};
use crate::error::{Error, Result};
use crate::pdf::{Dict, Object};
use crate::view::ViewTransform;

/// The icon's side, in points.
pub const NOTE_SIZE: f32 = 20.0;

/// A note to add.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    /// The icon's square in page space (left, bottom, right, top).
    pub rect: [f32; 4],
    pub contents: String,
    /// RGB, each 0 to 1.
    pub color: [f32; 3],
}

impl Note {
    /// Yellow, as sticky notes are.
    pub const DEFAULT_COLOR: [f32; 3] = [1.0, 0.86, 0.0];

    /// The page-space square of an icon whose top-left corner is displayed
    /// at (`x`, `y`).
    pub fn rect_at(x: f32, y: f32, view: &ViewTransform) -> [f32; 4] {
        let corners = [(x, y), (x + NOTE_SIZE, y), (x, y + NOTE_SIZE), (x + NOTE_SIZE, y + NOTE_SIZE)];
        bounds(corners.into_iter().map(|(x, y)| view.page_point(x, y)))
    }

    /// `/Contents`, the icon's name, and closed.
    pub(crate) fn entries(&self) -> Vec<(Vec<u8>, Object)> {
        vec![
            (b"Contents".to_vec(), Object::Str(text_string(&self.contents))),
            (b"Name".to_vec(), Object::Name(b"Comment".to_vec())),
            (b"Open".to_vec(), Object::Bool(false)),
        ]
    }

    /// The icon, drawn upright as the page is displayed: on a page turned by
    /// `rotate` degrees its form is turned back by as much (`/Matrix`).
    pub(crate) fn appearance(&self, rotate: i64) -> Result<Appearance> {
        check_contents(&self.contents)?;
        check_color(self.color)?;
        let [left, bottom, right, top] = self.rect;
        if !in_range((left, bottom)) || !in_range((right, top)) || right <= left || top <= bottom {
            return Err(Error::InvalidEdit("a note needs a place on the page"));
        }
        let mut out = Content::default();
        let [r, g, b] = self.color;
        // A speech bubble with three lines of "text".
        out.op(&format!("0.6 w 1 j 0.25 G {} {} {} rg", num(r), num(g), num(b)));
        for (i, point) in
            [(1.5, 5.5), (1.5, 18.5), (18.5, 18.5), (18.5, 5.5), (9.0, 5.5), (4.5, 1.5), (5.5, 5.5)]
                .into_iter()
                .enumerate()
        {
            if i == 0 { out.move_to(point) } else { out.line_to(point) }
        }
        out.op("h B");
        out.op("1 w 0 J");
        for (y, end) in [(15.0, 15.5), (12.0, 15.5), (9.0, 12.0)] {
            out.move_to((4.5, y));
            out.line_to((end, y));
        }
        out.op("S");

        let turn = (rotate.rem_euclid(360) as f32).to_radians();
        // Exact quarter turns; `+ 0.0` turns a negative zero into a plain one.
        let (sin, cos) = (turn.sin().round() + 0.0, turn.cos().round() + 0.0);
        Ok(Appearance {
            rect: self.rect,
            bbox: [0.0, 0.0, NOTE_SIZE, NOTE_SIZE],
            // Anticlockwise by the page's clockwise turn.
            matrix: (rotate.rem_euclid(360) != 0).then_some([cos, sin, -sin + 0.0, cos, 0.0, 0.0]),
            resources: Dict::default(),
            content: out.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(contents: &str) -> Note {
        Note { rect: [100.0, 680.0, 120.0, 700.0], contents: contents.into(), color: Note::DEFAULT_COLOR }
    }

    #[test]
    fn the_icon_turns_back_against_the_page() {
        assert_eq!(note("").appearance(0).expect("valid").matrix, None);
        assert_eq!(note("").appearance(90).expect("valid").matrix, Some([0.0, 1.0, -1.0, 0.0, 0.0, 0.0]));
        assert_eq!(note("").appearance(-90).expect("valid").matrix, Some([0.0, -1.0, 1.0, 0.0, 0.0, 0.0]));
        let appearance = note("").appearance(180).expect("valid");
        assert_eq!(appearance.matrix, Some([-1.0, 0.0, 0.0, -1.0, 0.0, 0.0]));
        assert_eq!(appearance.bbox, [0.0, 0.0, NOTE_SIZE, NOTE_SIZE]);
        assert_eq!(appearance.rect, [100.0, 680.0, 120.0, 700.0]);
    }

    #[test]
    fn the_text_goes_in_contents() {
        let entries = note("ملاحظة: راجع البند 4").entries();
        let Object::Str(contents) = &entries[0].1 else { panic!("a string") };
        assert_eq!(contents[..2], [0xFE, 0xFF]);
    }

    #[test]
    fn a_note_on_a_turned_page_is_a_square_there() {
        let turned = ViewTransform::of_rotation_for_tests();
        let rect = Note::rect_at(30.0, 40.0, &turned);
        assert_eq!((rect[2] - rect[0], rect[3] - rect[1]), (NOTE_SIZE, NOTE_SIZE));
    }

    #[test]
    fn too_long_a_note_is_refused() {
        assert!(matches!(note(&"x".repeat(20_001)).appearance(0), Err(Error::InvalidEdit(_))));
        assert!(note(&"x".repeat(20_000)).appearance(0).is_ok());
    }
}
