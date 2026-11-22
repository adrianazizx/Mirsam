//! Annotations — markup (highlight, underline, strikethrough, squiggly),
//! ink, and sticky notes — added, changed, and removed as incremental
//! updates (ADRs 0017 and 0018), so undo covers them like page edits
//! (ADR 0014).
//!
//! Each is a standard annotation (ISO 32000-2, 12.5) with its own appearance
//! stream, so every viewer draws it the same way instead of improvising one.
//! Adding one appends three objects' worth: the appearance stream, the
//! annotation, and the page's `/Annots` (the page dictionary, or the array
//! when it is an object of its own). In an encrypted file the new objects,
//! and new strings in changed ones, are encrypted (`pdf::crypt`).

mod ink;
mod markup;
mod note;

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub use ink::{Ink, InkPoint};
pub use markup::{Markup, MarkupKind, Quad};
pub use note::{NOTE_SIZE, Note};

use crate::edit::{Leaf, PageTree, Start, set};
use crate::error::{Error, Result};
use crate::pdf::crypt::Crypt;
use crate::pdf::write::Update;
use crate::pdf::{Dict, File, Object};

/// The annotations this module writes, and can remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AnnotationKind {
    Highlight,
    Underline,
    StrikeOut,
    Squiggly,
    Ink,
    /// A sticky note (`/Text`).
    Note,
}

impl AnnotationKind {
    fn from_subtype(name: &[u8]) -> Option<Self> {
        match name {
            b"Ink" => Some(AnnotationKind::Ink),
            b"Text" => Some(AnnotationKind::Note),
            other => MarkupKind::from_subtype(other).map(Self::from),
        }
    }
}

impl From<MarkupKind> for AnnotationKind {
    fn from(kind: MarkupKind) -> Self {
        match kind {
            MarkupKind::Highlight => AnnotationKind::Highlight,
            MarkupKind::Underline => AnnotationKind::Underline,
            MarkupKind::StrikeOut => AnnotationKind::StrikeOut,
            MarkupKind::Squiggly => AnnotationKind::Squiggly,
        }
    }
}

/// An annotation on a page, as PDFium reads it (ink strokes from `/InkList`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnnotationInfo {
    /// Its position in the page's `/Annots`.
    pub index: usize,
    pub kind: AnnotationKind,
    /// Page-space rectangles it covers: one per quad for markup, else its
    /// `/Rect`.
    pub rects: Vec<crate::text::Rect>,
    /// A note's text.
    pub contents: Option<String>,
    /// An ink annotation's strokes, in page space (empty when unreadable).
    pub strokes: Vec<Vec<(f32, f32)>>,
}

/// One change to a page's annotations.
#[derive(Debug, Clone, PartialEq)]
pub enum AnnotationEdit {
    AddMarkup {
        page: usize,
        markup: Markup,
    },
    AddInk {
        page: usize,
        ink: Ink,
    },
    AddNote {
        page: usize,
        note: Note,
    },
    /// Replaces the text of the note at `index` of the page's `/Annots`.
    SetContents {
        page: usize,
        index: usize,
        contents: String,
    },
    /// Removes the annotation at `index` of the page's `/Annots` (as
    /// [`crate::Document::page_annotations`] numbers them), with its pop-up
    /// and replies. Only the kinds in [`AnnotationKind`].
    Remove {
        page: usize,
        index: usize,
    },
}

/// Far outside any page (PDF's limit is 14,400 points), but finite.
pub(crate) const MAX_COORDINATE: f32 = 1.0e6;
/// The longest note, in UTF-16 code units.
pub const MAX_CONTENTS: usize = 20_000;

/// The bytes to append to `bytes` to apply `edit`. `password` is the one the
/// document was opened with, needed to encrypt new objects and strings.
pub fn annotation_update(bytes: &[u8], password: Option<&str>, edit: &AnnotationEdit) -> Result<Vec<u8>> {
    let Start { file, section, form, mut next_number } = Start::read(bytes)?;
    let tree = PageTree::read(&file)?;
    let leaf = |page: usize| {
        tree.leaves.get(page).ok_or(Error::PageOutOfRange { index: page, count: tree.leaves.len() })
    };
    let mut writer =
        Writer { file: &file, password, update: Update::new(bytes), next_number: &mut next_number };
    match edit {
        AnnotationEdit::AddMarkup { page, markup } => {
            let leaf = leaf(*page)?;
            let appearance = markup.appearance()?;
            writer.add(leaf, markup.kind.subtype(), markup.color, &appearance, markup.entries())?;
        }
        AnnotationEdit::AddInk { page, ink } => {
            let leaf = leaf(*page)?;
            let appearance = ink.appearance()?;
            writer.add(leaf, b"Ink", ink.color, &appearance, ink.entries())?;
        }
        AnnotationEdit::AddNote { page, note } => {
            let leaf = leaf(*page)?;
            let appearance = note.appearance(leaf.rotate)?;
            writer.add(leaf, b"Text", note.color, &appearance, note.entries())?;
        }
        AnnotationEdit::SetContents { page, index, contents } => {
            let leaf = leaf(*page)?;
            check_contents(contents)?;
            let annots = Annots::read(&file, leaf)?;
            let target = annots.items.get(*index).ok_or(Error::InvalidEdit("no such annotation"))?;
            let &Object::Ref(number, generation) = target else {
                return Err(Error::EditUnsupported("this annotation is not an object of its own"));
            };
            let mut dict = resolve_dict(&file, target)
                .ok_or(Error::EditUnsupported("this annotation cannot be read"))?;
            if kind_of(&dict) != Some(AnnotationKind::Note) {
                return Err(Error::InvalidEdit("only a note's text can be changed so far"));
            }
            // The object keeps its number: strings already in it stay as
            // read; only the new ones are encrypted, under that number.
            let mut text = Object::Str(text_string(contents));
            let mut date = Object::Str(pdf_date(SystemTime::now()));
            if let Some(crypt) = Crypt::of(&file, password)? {
                crypt.encrypt_strings(number, generation, &mut text);
                crypt.encrypt_strings(number, generation, &mut date);
            }
            set(&mut dict, b"Contents", text);
            set(&mut dict, b"M", date);
            writer.update.object(number, generation, &Object::Dict(dict));
        }
        AnnotationEdit::Remove { page, index } => {
            let leaf = leaf(*page)?;
            let mut annots = Annots::read(&file, leaf)?;
            let target = annots.items.get(*index).cloned().ok_or(Error::InvalidEdit("no such annotation"))?;
            let dict = resolve_dict(&file, &target)
                .ok_or(Error::EditUnsupported("this annotation cannot be read"))?;
            kind_of(&dict).ok_or(Error::InvalidEdit("only markup, ink, and notes can be removed so far"))?;
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
            annots.write(&mut writer.update, leaf);
        }
    }
    let update = writer.update;
    Ok(update.finish(form, file.trailer.clone(), section, next_number))
}

/// The strokes (`/InkList`) of each ink annotation on `page`, by position in
/// its `/Annots`, in page space. PDFium's safe API does not expose them.
/// Empty where the page cannot be read (an encrypted object stream).
pub fn ink_strokes(bytes: &[u8], page: usize) -> HashMap<usize, Vec<Vec<(f32, f32)>>> {
    let mut out = HashMap::new();
    let Ok(Start { file, .. }) = Start::read(bytes) else { return out };
    let Ok(tree) = PageTree::read(&file) else { return out };
    let Some(leaf) = tree.leaves.get(page) else { return out };
    let Ok(annots) = Annots::read(&file, leaf) else { return out };
    for (index, item) in annots.items.iter().enumerate() {
        let Some(dict) = resolve_dict(&file, item) else { continue };
        if kind_of(&dict) != Some(AnnotationKind::Ink) {
            continue;
        }
        let Some(Object::Array(list)) = dict.get(b"InkList").and_then(|list| file.resolve(list)) else {
            continue;
        };
        let strokes = list
            .iter()
            .filter_map(|stroke| match file.resolve(stroke) {
                Some(Object::Array(numbers)) => Some(
                    numbers
                        .iter()
                        .filter_map(|n| number(&file.resolve(n)?))
                        .collect::<Vec<_>>()
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|&[x, y]| (x, y))
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .filter(|stroke: &Vec<(f32, f32)>| !stroke.is_empty())
            .collect();
        out.insert(index, strokes);
    }
    out
}

fn number(object: &Object) -> Option<f32> {
    match object {
        Object::Int(value) => Some(*value as f32),
        Object::Real(value) => Some(*value as f32),
        _ => None,
    }
}

fn kind_of(dict: &Dict) -> Option<AnnotationKind> {
    dict.get(b"Subtype").and_then(Object::as_name).and_then(AnnotationKind::from_subtype)
}

fn resolve_dict(file: &File<'_>, object: &Object) -> Option<Dict> {
    file.resolve(object).and_then(|object| object.as_dict().cloned())
}

fn popup(dict: &Dict) -> Option<Object> {
    dict.get(b"Popup").filter(|popup| matches!(popup, Object::Ref(..))).cloned()
}

pub(crate) fn check_contents(contents: &str) -> Result<()> {
    if contents.encode_utf16().count() > MAX_CONTENTS {
        return Err(Error::InvalidEdit("a note holds at most 20,000 characters"));
    }
    Ok(())
}

pub(crate) fn check_color(color: [f32; 3]) -> Result<()> {
    if color.iter().any(|c| !(0.0..=1.0).contains(c)) {
        return Err(Error::InvalidEdit("colour components run from 0 to 1"));
    }
    Ok(())
}

pub(crate) fn in_range((x, y): (f32, f32)) -> bool {
    x.abs() <= MAX_COORDINATE && y.abs() <= MAX_COORDINATE
}

/// Adds annotations to an update.
struct Writer<'a, 'f> {
    file: &'a File<'f>,
    password: Option<&'a str>,
    update: Update,
    next_number: &'a mut u32,
}

impl Writer<'_, '_> {
    /// Appends the appearance stream and the annotation, and adds the
    /// annotation to the page's `/Annots`. `entries` go after `/Rect`.
    fn add(
        &mut self,
        leaf: &Leaf,
        subtype: &[u8],
        color: [f32; 3],
        appearance: &Appearance,
        entries: Vec<(Vec<u8>, Object)>,
    ) -> Result<()> {
        let crypt = Crypt::of(self.file, self.password)?;
        let too_many = || Error::EditUnsupported("it has too many objects");
        let annotation = *self.next_number;
        let stream = annotation.checked_add(1).ok_or_else(too_many)?;
        *self.next_number = stream.checked_add(1).ok_or_else(too_many)?;

        let data = match &crypt {
            Some(crypt) => crypt.encrypt_stream(stream, 0, &appearance.content),
            None => appearance.content.clone(),
        };
        self.update.stream(stream, 0, appearance.form(), &data);

        let real = |value: f32| Object::Real(f64::from(value));
        let now = pdf_date(SystemTime::now());
        let mut dict = vec![
            (b"Type".to_vec(), Object::Name(b"Annot".to_vec())),
            (b"Subtype".to_vec(), Object::Name(subtype.to_vec())),
            (b"Rect".to_vec(), Object::Array(appearance.rect.iter().map(|v| real(*v)).collect())),
        ];
        dict.extend(entries);
        dict.extend([
            (b"C".to_vec(), Object::Array(color.iter().map(|v| real(*v)).collect())),
            // Print: annotations print with the page, as in other viewers.
            (b"F".to_vec(), Object::Int(4)),
            (b"P".to_vec(), Object::Ref(leaf.number, leaf.generation)),
            (b"NM".to_vec(), Object::Str(unique_name())),
            (b"M".to_vec(), Object::Str(now.clone())),
            (b"CreationDate".to_vec(), Object::Str(now)),
            (b"AP".to_vec(), Object::Dict(Dict(vec![(b"N".to_vec(), Object::Ref(stream, 0))]))),
        ]);
        let mut dict = Object::Dict(Dict(dict));
        if let Some(crypt) = &crypt {
            crypt.encrypt_strings(annotation, 0, &mut dict);
        }
        self.update.object(annotation, 0, &dict);

        let mut annots = Annots::read(self.file, leaf)?;
        annots.items.push(Object::Ref(annotation, 0));
        annots.write(&mut self.update, leaf);
        Ok(())
    }
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

/// An annotation's appearance: a form XObject. Its `/BBox`, mapped through
/// its `/Matrix`, is fitted to the annotation's `/Rect` (ISO 32000-2,
/// 12.5.5); with an identity matrix and `/BBox` = `/Rect`, it is drawn in
/// page space.
pub(crate) struct Appearance {
    pub(crate) rect: [f32; 4],
    pub(crate) bbox: [f32; 4],
    pub(crate) matrix: Option<[f32; 6]>,
    pub(crate) resources: Dict,
    pub(crate) content: Vec<u8>,
}

impl Appearance {
    /// Drawn in page space over `rect`.
    pub(crate) fn in_page_space(rect: [f32; 4], resources: Dict, content: Vec<u8>) -> Self {
        Self { rect, bbox: rect, matrix: None, resources, content }
    }

    fn form(&self) -> Dict {
        let real = |value: f32| Object::Real(f64::from(value));
        let mut form = Dict(vec![
            (b"Type".to_vec(), Object::Name(b"XObject".to_vec())),
            (b"Subtype".to_vec(), Object::Name(b"Form".to_vec())),
            (b"BBox".to_vec(), Object::Array(self.bbox.iter().map(|v| real(*v)).collect())),
        ]);
        if let Some(matrix) = self.matrix {
            form.0.push((b"Matrix".to_vec(), Object::Array(matrix.iter().map(|v| real(*v)).collect())));
        }
        form.0.push((b"Resources".to_vec(), Object::Dict(self.resources.clone())));
        form
    }
}

/// The bounding box of `points` (left, bottom, right, top).
pub(crate) fn bounds(points: impl Iterator<Item = (f32, f32)>) -> [f32; 4] {
    let mut rect = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
    for (x, y) in points {
        rect = [rect[0].min(x), rect[1].min(y), rect[2].max(x), rect[3].max(y)];
    }
    rect
}

/// A content stream being written.
#[derive(Default)]
pub(crate) struct Content(pub(crate) Vec<u8>);

impl Content {
    pub(crate) fn op(&mut self, text: &str) {
        self.0.extend_from_slice(text.as_bytes());
        self.0.push(b'\n');
    }

    pub(crate) fn move_to(&mut self, (x, y): (f32, f32)) {
        self.op(&format!("{} {} m", num(x), num(y)));
    }

    pub(crate) fn line_to(&mut self, (x, y): (f32, f32)) {
        self.op(&format!("{} {} l", num(x), num(y)));
    }
}

pub(crate) fn offset((x, y): (f32, f32), (dx, dy): (f32, f32), distance: f32) -> (f32, f32) {
    (x + dx * distance, y + dy * distance)
}

/// A number for a content stream: three decimals at most, no exponent.
pub(crate) fn num(value: f32) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-0" || text == "-" { "0".into() } else { text.into() }
}

/// A PDF text string: ASCII as is (it is PDFDocEncoding too), anything else
/// as UTF-16BE with a byte-order mark.
pub(crate) fn text_string(text: &str) -> Vec<u8> {
    let plain = text.chars().all(|c| c.is_ascii() && (c >= ' ' || matches!(c, '\n' | '\r' | '\t')));
    if plain {
        return text.as_bytes().to_vec();
    }
    let mut out = vec![0xFE, 0xFF];
    for unit in text.encode_utf16() {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out
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
    fn text_strings_are_ascii_or_utf16() {
        assert_eq!(text_string("Call me\n"), b"Call me\n");
        // Arabic, and anything else outside ASCII, is UTF-16BE with a BOM.
        assert_eq!(text_string("نعم"), [0xFE, 0xFF, 0x06, 0x46, 0x06, 0x39, 0x06, 0x45]);
        assert_eq!(text_string("é")[..2], [0xFE, 0xFF]);
        assert_eq!(text_string("😀").len(), 2 + 4, "a surrogate pair");
    }

    #[test]
    fn subtypes_map_to_kinds() {
        assert_eq!(AnnotationKind::from_subtype(b"Text"), Some(AnnotationKind::Note));
        assert_eq!(AnnotationKind::from_subtype(b"Ink"), Some(AnnotationKind::Ink));
        assert_eq!(AnnotationKind::from_subtype(b"StrikeOut"), Some(AnnotationKind::StrikeOut));
        assert_eq!(AnnotationKind::from_subtype(b"Link"), None);
    }
}
