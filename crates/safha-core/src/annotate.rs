//! Markup annotations — highlight, underline, strikethrough, squiggly —
//! added and removed as incremental updates (ADR 0017), so undo covers them
//! like page edits (ADR 0014).
//!
//! Each is a standard text-markup annotation (ISO 32000-2, 12.5.6.10) with
//! its own appearance stream, so every viewer draws it the same way instead
//! of guessing from `/QuadPoints`. Adding one appends three objects' worth:
//! the appearance stream, the annotation, and the page's `/Annots` (the page
//! dictionary, or the array when it is an object of its own). In an
//! encrypted file the new objects are encrypted (`pdf::crypt`).

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::edit::{Leaf, PageTree, Start, set};
use crate::error::{Error, Result};
use crate::pdf::crypt::Crypt;
use crate::pdf::write::Update;
use crate::pdf::{Dict, File, Object};
use crate::view::{ViewRect, ViewTransform};

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
    fn subtype(self) -> &'static [u8] {
        match self {
            MarkupKind::Highlight => b"Highlight",
            MarkupKind::Underline => b"Underline",
            MarkupKind::StrikeOut => b"StrikeOut",
            MarkupKind::Squiggly => b"Squiggly",
        }
    }

    fn from_subtype(name: &[u8]) -> Option<Self> {
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

/// One change to a page's annotations.
#[derive(Debug, Clone, PartialEq)]
pub enum AnnotationEdit {
    AddMarkup {
        page: usize,
        markup: Markup,
    },
    /// Removes the annotation at `index` of the page's `/Annots` (as
    /// [`crate::Document::page_markup`] numbers them), with its pop-up and
    /// replies. Only markup annotations, so far.
    Remove {
        page: usize,
        index: usize,
    },
}

/// A markup annotation on a page, as PDFium reads it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MarkupInfo {
    /// Its position in the page's `/Annots`.
    pub index: usize,
    pub kind: MarkupKind,
    /// The page-space bounding box of each quad (or of the annotation, when
    /// it has no quads).
    pub rects: Vec<crate::text::Rect>,
}

const MAX_QUADS: usize = 10_000;
/// Far outside any page (PDF's limit is 14,400 points), but finite.
const MAX_COORDINATE: f32 = 1.0e6;

/// The bytes to append to `bytes` to apply `edit`. `password` is the one the
/// document was opened with, needed to encrypt new objects.
pub fn annotation_update(bytes: &[u8], password: Option<&str>, edit: &AnnotationEdit) -> Result<Vec<u8>> {
    let Start { file, section, form, mut next_number } = Start::read(bytes)?;
    let tree = PageTree::read(&file)?;
    let leaf = |page: usize| {
        tree.leaves.get(page).ok_or(Error::PageOutOfRange { index: page, count: tree.leaves.len() })
    };
    let mut update = Update::new(bytes);
    match edit {
        AnnotationEdit::AddMarkup { page, markup } => {
            let leaf = leaf(*page)?;
            let appearance = Appearance::of(markup)?;
            let crypt = Crypt::of(&file, password)?;
            let annotation = next_number;
            let stream =
                annotation.checked_add(1).ok_or(Error::EditUnsupported("it has too many objects"))?;
            next_number = stream.checked_add(1).ok_or(Error::EditUnsupported("it has too many objects"))?;

            let data = match &crypt {
                Some(crypt) => crypt.encrypt_stream(stream, 0, &appearance.content),
                None => appearance.content.clone(),
            };
            update.stream(stream, 0, appearance.form(markup.kind), &data);

            let mut dict = Object::Dict(annotation_dict(markup, &appearance, leaf, stream));
            if let Some(crypt) = &crypt {
                crypt.encrypt_strings(annotation, 0, &mut dict);
            }
            update.object(annotation, 0, &dict);

            let mut annots = Annots::read(&file, leaf)?;
            annots.items.push(Object::Ref(annotation, 0));
            annots.write(&mut update, leaf);
        }
        AnnotationEdit::Remove { page, index } => {
            let leaf = leaf(*page)?;
            let mut annots = Annots::read(&file, leaf)?;
            let target = annots.items.get(*index).cloned().ok_or(Error::InvalidEdit("no such annotation"))?;
            let dict = resolve_dict(&file, &target)
                .ok_or(Error::EditUnsupported("this annotation cannot be read"))?;
            dict.get(b"Subtype")
                .and_then(Object::as_name)
                .and_then(MarkupKind::from_subtype)
                .ok_or(Error::InvalidEdit("only markup annotations can be removed so far"))?;
            // Its pop-up, and replies to it (with theirs), go with it.
            let mut doomed = vec![target.clone()];
            doomed.extend(popup(&dict));
            if matches!(target, Object::Ref(..)) {
                for item in &annots.items {
                    if let Some(reply) = resolve_dict(&file, item)
                        && reply.get(b"IRT") == Some(&target)
                    {
                        doomed.push(item.clone());
                        doomed.extend(popup(&reply));
                    }
                }
            }
            let mut position = 0;
            annots.items.retain(|item| {
                let keep = position != *index && !doomed.contains(item);
                position += 1;
                keep
            });
            annots.write(&mut update, leaf);
        }
    }
    Ok(update.finish(form, file.trailer.clone(), section, next_number))
}

fn resolve_dict(file: &File<'_>, object: &Object) -> Option<Dict> {
    file.resolve(object).and_then(|object| object.as_dict().cloned())
}

fn popup(dict: &Dict) -> Option<Object> {
    dict.get(b"Popup").filter(|popup| matches!(popup, Object::Ref(..))).cloned()
}

/// A page's `/Annots`, and where it lives.
struct Annots {
    /// The array's own object, when it is not in the page dictionary.
    own: Option<(u32, u16)>,
    items: Vec<Object>,
}

impl Annots {
    fn read(file: &File<'_>, leaf: &Leaf) -> Result<Self> {
        let unreadable = || Error::EditUnsupported("the page's annotations cannot be read");
        match leaf.dict.get(b"Annots") {
            None | Some(Object::Null) => Ok(Self { own: None, items: Vec::new() }),
            Some(Object::Array(items)) => Ok(Self { own: None, items: items.clone() }),
            Some(reference @ Object::Ref(number, generation)) => {
                let items = match file.resolve(reference) {
                    Some(Object::Array(items)) => items,
                    Some(Object::Null) => Vec::new(),
                    _ => return Err(unreadable()),
                };
                Ok(Self { own: Some((*number, *generation)), items })
            }
            Some(_) => Err(unreadable()),
        }
    }

    /// Rewrites the array where it was; a page that had none gets one in its
    /// dictionary. Either keeps its object number, so strings already in it
    /// (encrypted under that number) stay valid.
    fn write(self, update: &mut Update, leaf: &Leaf) {
        match self.own {
            Some((number, generation)) => update.object(number, generation, &Object::Array(self.items)),
            None => {
                let mut dict = leaf.dict.clone();
                set(&mut dict, b"Annots", Object::Array(self.items));
                update.object(leaf.number, leaf.generation, &Object::Dict(dict));
            }
        }
    }
}

fn annotation_dict(markup: &Markup, appearance: &Appearance, leaf: &Leaf, stream: u32) -> Dict {
    let real = |value: f32| Object::Real(f64::from(value));
    let now = pdf_date(SystemTime::now());
    let quad_points =
        markup.quads.iter().flat_map(|quad| quad.points()).flat_map(|(x, y)| [real(x), real(y)]).collect();
    Dict(vec![
        (b"Type".to_vec(), Object::Name(b"Annot".to_vec())),
        (b"Subtype".to_vec(), Object::Name(markup.kind.subtype().to_vec())),
        (b"Rect".to_vec(), Object::Array(appearance.rect.iter().map(|v| real(*v)).collect())),
        (b"QuadPoints".to_vec(), Object::Array(quad_points)),
        (b"C".to_vec(), Object::Array(markup.color.iter().map(|v| real(*v)).collect())),
        // Print: markup prints with the page, as in other viewers.
        (b"F".to_vec(), Object::Int(4)),
        (b"P".to_vec(), Object::Ref(leaf.number, leaf.generation)),
        (b"NM".to_vec(), Object::Str(unique_name())),
        (b"M".to_vec(), Object::Str(now.clone())),
        (b"CreationDate".to_vec(), Object::Str(now)),
        (b"AP".to_vec(), Object::Dict(Dict(vec![(b"N".to_vec(), Object::Ref(stream, 0))]))),
    ])
}

/// A markup's appearance: a form XObject drawn in page space (its `/BBox` is
/// the annotation's `/Rect` and its matrix the identity).
struct Appearance {
    rect: [f32; 4],
    content: Vec<u8>,
}

impl Appearance {
    fn of(markup: &Markup) -> Result<Self> {
        if markup.quads.is_empty() || markup.quads.len() > MAX_QUADS {
            return Err(Error::InvalidEdit("markup needs between one and 10,000 quads"));
        }
        let points = markup.quads.iter().flat_map(Quad::points);
        if points.clone().any(|(x, y)| !(x.abs() <= MAX_COORDINATE && y.abs() <= MAX_COORDINATE)) {
            return Err(Error::InvalidEdit("markup lies outside the page"));
        }
        if markup.color.iter().any(|c| !(0.0..=1.0).contains(c)) {
            return Err(Error::InvalidEdit("colour components run from 0 to 1"));
        }
        let mut rect = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
        for (x, y) in points {
            rect = [rect[0].min(x), rect[1].min(y), rect[2].max(x), rect[3].max(y)];
        }
        if rect[2] - rect[0] <= 0.0 || rect[3] - rect[1] <= 0.0 {
            return Err(Error::InvalidEdit("markup covers nothing"));
        }

        let mut out = Content::default();
        let [r, g, b] = markup.color;
        match markup.kind {
            MarkupKind::Highlight => {
                out.op(&format!("/G0 gs {} {} {} rg", num(r), num(g), num(b)));
                for quad in &markup.quads {
                    out.move_to(quad.upper_left);
                    out.line_to(quad.upper_right);
                    out.line_to(quad.lower_right);
                    out.line_to(quad.lower_left);
                    out.op("h f");
                }
            }
            MarkupKind::Underline | MarkupKind::StrikeOut => {
                out.op(&format!("{} {} {} RG 0 J", num(r), num(g), num(b)));
                for quad in &markup.quads {
                    let (height, up) = quad.up();
                    if height <= 0.0 {
                        continue;
                    }
                    let width = (height * 0.07).max(0.5);
                    // An underline just inside the bottom of the line box (below the
                    // descenders); a strikethrough through its middle.
                    let lift = if markup.kind == MarkupKind::Underline { width } else { height / 2.0 };
                    out.op(&format!("{} w", num(width)));
                    out.move_to(offset(quad.lower_left, up, lift));
                    out.line_to(offset(quad.lower_right, up, lift));
                    out.op("S");
                }
            }
            MarkupKind::Squiggly => {
                out.op(&format!("{} {} {} RG 1 j 0 J", num(r), num(g), num(b)));
                for quad in &markup.quads {
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
        Ok(Self { rect, content: out.0 })
    }

    fn form(&self, kind: MarkupKind) -> Dict {
        let real = |value: f32| Object::Real(f64::from(value));
        let mut form = Dict(vec![
            (b"Type".to_vec(), Object::Name(b"XObject".to_vec())),
            (b"Subtype".to_vec(), Object::Name(b"Form".to_vec())),
            (b"BBox".to_vec(), Object::Array(self.rect.iter().map(|v| real(*v)).collect())),
        ]);
        let resources = if kind == MarkupKind::Highlight {
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
        form.0.push((b"Resources".to_vec(), Object::Dict(resources)));
        form
    }
}

/// A content stream being written.
#[derive(Default)]
struct Content(Vec<u8>);

impl Content {
    fn op(&mut self, text: &str) {
        self.0.extend_from_slice(text.as_bytes());
        self.0.push(b'\n');
    }

    fn move_to(&mut self, (x, y): (f32, f32)) {
        self.op(&format!("{} {} m", num(x), num(y)));
    }

    fn line_to(&mut self, (x, y): (f32, f32)) {
        self.op(&format!("{} {} l", num(x), num(y)));
    }
}

fn offset((x, y): (f32, f32), (dx, dy): (f32, f32), distance: f32) -> (f32, f32) {
    (x + dx * distance, y + dy * distance)
}

/// A number for a content stream: three decimals at most, no exponent.
fn num(value: f32) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-0" || text == "-" { "0".into() } else { text.into() }
}

/// A PDF date (`D:YYYYMMDDHHmmSSZ`), in UTC.
fn pdf_date(time: SystemTime) -> Vec<u8> {
    let seconds = time.duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());
    let (year, month, day) = civil_from_days(i64::try_from(seconds / 86_400).unwrap_or(0));
    let rest = seconds % 86_400;
    format!("D:{year:04}{month:02}{day:02}{:02}{:02}{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
        .into_bytes()
}

/// Year, month, day of a day count from 1970-01-01 (proleptic Gregorian;
/// Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

/// An annotation's unique name (`/NM`): 128 random bits, in hex.
fn unique_name() -> Vec<u8> {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_nanos());
        bytes = nanos.to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect::<String>().into_bytes()
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
    fn dates_are_utc_calendar_dates() {
        let time = UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        assert_eq!(pdf_date(time), b"D:20010909014640Z");
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }

    #[test]
    fn numbers_have_three_decimals_and_no_negative_zero() {
        assert_eq!(num(612.0), "612");
        assert_eq!(num(0.12345), "0.123");
        assert_eq!(num(-0.0001), "0");
        assert_eq!(num(-2.5), "-2.5");
    }

    #[test]
    fn a_highlight_fills_each_quad_multiplied() {
        let markup = Markup {
            kind: MarkupKind::Highlight,
            quads: vec![quad(10.0, 100.0, 60.0, 112.0), quad(10.0, 86.0, 30.0, 98.0)],
            color: [1.0, 0.86, 0.0],
        };
        let appearance = Appearance::of(&markup).expect("valid");
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
        let content = String::from_utf8(Appearance::of(&markup).expect("valid").content).expect("ascii");
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
        let appearance = Appearance::of(&markup).expect("valid");
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
        assert!(matches!(Appearance::of(&empty), Err(Error::InvalidEdit(_))));
        let flat =
            Markup { kind: MarkupKind::Underline, quads: vec![quad(0.0, 5.0, 10.0, 5.0)], color: [0.0; 3] };
        assert!(matches!(Appearance::of(&flat), Err(Error::InvalidEdit(_))));
        let wild = Markup {
            kind: MarkupKind::Highlight,
            quads: vec![quad(0.0, 0.0, f32::NAN, 5.0)],
            color: [0.0; 3],
        };
        assert!(matches!(Appearance::of(&wild), Err(Error::InvalidEdit(_))));
        let colour = Markup {
            kind: MarkupKind::Highlight,
            quads: vec![quad(0.0, 0.0, 5.0, 5.0)],
            color: [2.0, 0.0, 0.0],
        };
        assert!(matches!(Appearance::of(&colour), Err(Error::InvalidEdit(_))));
    }
}
