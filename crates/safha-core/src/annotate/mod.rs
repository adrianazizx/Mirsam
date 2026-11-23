//! Annotations — markup (highlight, underline, strikethrough, squiggly),
//! ink, sticky notes, and text boxes — added, changed, and removed as
//! incremental updates (ADRs 0017–0019), so undo covers them like page edits
//! (ADR 0014).
//!
//! Each is a standard annotation (ISO 32000-2, 12.5) with its own appearance
//! stream, so every viewer draws it the same way instead of improvising one.
//! Adding one appends three objects' worth: the appearance stream, the
//! annotation, and the page's `/Annots` (the page dictionary, or the array
//! when it is an object of its own). In an encrypted file the new objects,
//! and new strings in changed ones, are encrypted (`pdf::crypt`).

mod comments;
mod fill;
mod ink;
mod markup;
mod note;
mod text_box;

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub(crate) use comments::threads;
pub use comments::{Comment, Reply};
pub use fill::{
    DrawnSignature, FillText, MAX_SIGNATURE_IMAGE_BYTES, Mark, MarkKind, Signature, SignaturePoint,
    SignatureStamp, TypedSignature,
};
pub(crate) use fill::{mark_in, upright};
pub use ink::{Ink, InkPoint};
pub use markup::{Markup, MarkupKind, Quad};
pub use note::{NOTE_SIZE, Note};
pub use text_box::{TEXT_BOX_PADDING, TextBox};

use crate::edit::{Leaf, PageTree, Start, set};
use crate::error::{Error, Result};
use crate::pdf::crypt::Crypt;
use crate::pdf::image::{ImageToEmbed, write_image};
use crate::pdf::text::{FontToEmbed, write_font};
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
    /// Text on the page (`/FreeText`).
    TextBox,
    /// Fill & Sign text or a date (a `/Stamp` of ours, ADR 0045).
    FillText,
    /// Fill & Sign tick, cross, or dot (a `/Stamp` of ours).
    Mark,
    /// A drawn, typed, or image signature (a `/Stamp` of ours).
    Signature,
}

impl AnnotationKind {
    /// Whether it is a Fill & Sign item (ADR 0045), not a comment.
    pub fn is_fill(self) -> bool {
        matches!(self, AnnotationKind::FillText | AnnotationKind::Mark | AnnotationKind::Signature)
    }

    fn from_subtype(name: &[u8]) -> Option<Self> {
        match name {
            b"Ink" => Some(AnnotationKind::Ink),
            b"Text" => Some(AnnotationKind::Note),
            b"FreeText" => Some(AnnotationKind::TextBox),
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
    /// A note's or a text box's text.
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
    AddTextBox {
        page: usize,
        text_box: TextBox,
    },
    /// Fill & Sign text or a date (ADR 0045).
    AddFillText {
        page: usize,
        text: FillText,
    },
    /// A Fill & Sign tick, cross, or dot.
    AddMark {
        page: usize,
        mark: Mark,
    },
    /// A signature placed on the page.
    AddSignature {
        page: usize,
        stamp: SignatureStamp,
    },
    /// Moves or resizes the Fill & Sign item at `index` of the page's
    /// `/Annots`: its appearance is fitted to the new `/Rect`. Fill text
    /// keeps its new size for later changes (`/DA`).
    SetRect {
        page: usize,
        index: usize,
        rect: [f32; 4],
    },
    /// Moves the Fill & Sign item at `index` of `page`'s `/Annots` to
    /// `to_page`, at `rect` in that page's space (ADR 0045). It leaves one
    /// page's `/Annots` for the other's, keeping its object; on a page turned
    /// otherwise, its appearance is turned to stand upright there.
    MoveToPage {
        page: usize,
        index: usize,
        to_page: usize,
        rect: [f32; 4],
    },
    /// Recolours the Fill & Sign text or mark at `index` of the page's
    /// `/Annots` (ADR 0045): its appearance is drawn again in `color`, in
    /// the same box.
    SetColor {
        page: usize,
        index: usize,
        color: [f32; 3],
    },
    /// Replaces the text of the note or text box at `index` of the page's
    /// `/Annots`. A text box is set again and its height fitted to the text,
    /// its displayed top edge kept.
    SetContents {
        page: usize,
        index: usize,
        contents: String,
    },
    /// Removes the annotation at `index` of the page's `/Annots` (as
    /// [`crate::Document::page_annotations`] numbers them), with its pop-up
    /// and its thread (replies, replies to them, review states). Only the
    /// kinds in [`AnnotationKind`].
    Remove {
        page: usize,
        index: usize,
    },
    /// Answers the comment at `index` (ADR 0041): a `/Text` annotation
    /// with `/IRT`, drawn nowhere on the page.
    Reply {
        page: usize,
        index: usize,
        contents: String,
    },
    /// Marks the comment at `index` resolved (`/State /Completed`) or open
    /// again (`/State /None`), with a new review state in its thread.
    SetResolved {
        page: usize,
        index: usize,
        resolved: bool,
    },
}

/// Far outside any page (PDF's limit is 14,400 points), but finite.
pub(crate) const MAX_COORDINATE: f32 = 1.0e6;
/// The longest note, in UTF-16 code units.
pub const MAX_CONTENTS: usize = 20_000;
/// The longest author name (`/T`), in UTF-16 code units.
pub const MAX_AUTHOR: usize = 256;

/// The bytes to append to `bytes` to apply `edit`. `password` is the one the
/// document was opened with, needed to encrypt new objects and strings.
/// `author` names who made new annotations (`/T`); none when empty.
pub fn annotation_update(
    bytes: &[u8],
    password: Option<&str>,
    author: Option<&str>,
    edit: &AnnotationEdit,
) -> Result<Vec<u8>> {
    let author = author.map(str::trim).filter(|author| !author.is_empty());
    if author.is_some_and(|author| author.encode_utf16().count() > MAX_AUTHOR) {
        return Err(Error::InvalidEdit("an author's name holds at most 256 characters"));
    }
    let Start { file, section, form, mut next_number } = Start::read(bytes)?;
    let tree = PageTree::read(&file)?;
    let leaf = |page: usize| {
        tree.leaves.get(page).ok_or(Error::PageOutOfRange { index: page, count: tree.leaves.len() })
    };
    let mut writer = Writer {
        file: &file,
        password,
        author,
        crypt: None,
        update: Update::new(bytes),
        next_number: &mut next_number,
    };
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
        AnnotationEdit::AddTextBox { page, text_box } => {
            let leaf = leaf(*page)?;
            let appearance = text_box.appearance(leaf.rotate)?;
            writer.add(leaf, b"FreeText", text_box.color, &appearance, text_box.entries())?;
        }
        AnnotationEdit::AddFillText { page, text } => {
            let leaf = leaf(*page)?;
            let appearance = text.appearance(leaf.rotate)?;
            writer.add(leaf, b"Stamp", text.color, &appearance, text.entries())?;
        }
        AnnotationEdit::AddMark { page, mark } => {
            let leaf = leaf(*page)?;
            let appearance = mark.appearance(leaf.rotate)?;
            writer.add(leaf, b"Stamp", mark.color, &appearance, mark.entries())?;
        }
        AnnotationEdit::AddSignature { page, stamp } => {
            let leaf = leaf(*page)?;
            let appearance = stamp.appearance(leaf.rotate)?;
            writer.add(leaf, b"Stamp", stamp.color, &appearance, stamp.entries())?;
        }
        AnnotationEdit::SetRect { page, index, rect } => {
            let leaf = leaf(*page)?;
            check_item_rect(*rect)?;
            let [_, bottom, _, top] = *rect;
            let annots = Annots::read(&file, leaf)?;
            let target = annots.items.get(*index).ok_or(Error::InvalidEdit("no such annotation"))?;
            let &Object::Ref(number, generation) = target else {
                return Err(Error::EditUnsupported("this annotation is not an object of its own"));
            };
            let mut dict = resolve_dict(&file, target)
                .ok_or(Error::EditUnsupported("this annotation cannot be read"))?;
            let kind = kind_of(&dict);
            if !kind.is_some_and(AnnotationKind::is_fill) {
                return Err(Error::InvalidEdit("only Fill & Sign items are moved or resized"));
            }
            if kind == Some(AnnotationKind::FillText) {
                // Text keeps the size it is shown at, for when it is changed.
                let old = rect_of(&file, &dict).ok_or(Error::EditUnsupported("this item has no box"))?;
                let scale = (top - bottom) / (old[3] - old[1]).max(0.01);
                let da = decrypted_da(&mut writer, &dict, number, generation)?;
                let (size, color) = text_box::parse_da(&da);
                let mut da = Object::Str(fill::da(size * scale, color).into_bytes());
                if let Some(crypt) = writer.crypt()? {
                    crypt.encrypt_strings(number, generation, &mut da);
                }
                set(&mut dict, b"DA", da);
            }
            let real = |value: f32| Object::Real(f64::from(value));
            set(&mut dict, b"Rect", Object::Array(rect.iter().map(|v| real(*v)).collect()));
            let mut date = Object::Str(pdf_date(SystemTime::now()));
            if let Some(crypt) = writer.crypt()? {
                crypt.encrypt_strings(number, generation, &mut date);
            }
            set(&mut dict, b"M", date);
            writer.update.object(number, generation, &Object::Dict(dict));
        }
        AnnotationEdit::MoveToPage { page, index, to_page, rect } => {
            let (from, to) = (leaf(*page)?, leaf(*to_page)?);
            check_item_rect(*rect)?;
            let mut annots = Annots::read(&file, from)?;
            let target = annots.items.get(*index).cloned().ok_or(Error::InvalidEdit("no such annotation"))?;
            let Object::Ref(number, generation) = target else {
                return Err(Error::EditUnsupported("this annotation is not an object of its own"));
            };
            let mut dict = resolve_dict(&file, &target)
                .ok_or(Error::EditUnsupported("this annotation cannot be read"))?;
            if !kind_of(&dict).is_some_and(AnnotationKind::is_fill) {
                return Err(Error::InvalidEdit("only Fill & Sign items move to another page"));
            }
            if page != to_page {
                annots.items.remove(*index);
                annots.write(&mut writer.update, from);
                let mut arrived = Annots::read(&file, to)?;
                arrived.items.push(target);
                arrived.write(&mut writer.update, to);
                // Upright on the new page: its appearance turns with that page's `/Rotate`.
                if from.rotate.rem_euclid(360) != to.rotate.rem_euclid(360) {
                    turn_appearance(&file, &mut writer.update, &dict, to.rotate)?;
                }
            }
            let real = |value: f32| Object::Real(f64::from(value));
            set(&mut dict, b"P", Object::Ref(to.number, to.generation));
            set(&mut dict, b"Rect", Object::Array(rect.iter().map(|v| real(*v)).collect()));
            let mut date = Object::Str(pdf_date(SystemTime::now()));
            if let Some(crypt) = writer.crypt()? {
                crypt.encrypt_strings(number, generation, &mut date);
            }
            set(&mut dict, b"M", date);
            writer.update.object(number, generation, &Object::Dict(dict));
        }
        AnnotationEdit::SetColor { page, index, color } => {
            check_color(*color)?;
            let leaf = leaf(*page)?;
            let annots = Annots::read(&file, leaf)?;
            let target = annots.items.get(*index).ok_or(Error::InvalidEdit("no such annotation"))?;
            let &Object::Ref(number, generation) = target else {
                return Err(Error::EditUnsupported("this annotation is not an object of its own"));
            };
            let mut dict = resolve_dict(&file, target)
                .ok_or(Error::EditUnsupported("this annotation cannot be read"))?;
            let rect = rect_of(&file, &dict).ok_or(Error::EditUnsupported("this item has no box"))?;
            let appearance = match kind_of(&dict) {
                Some(AnnotationKind::FillText) => {
                    let da = decrypted_da(&mut writer, &dict, number, generation)?;
                    let (size, _) = text_box::parse_da(&da);
                    let contents = decrypted_string(&mut writer, &dict, b"Contents", number, generation)?
                        .map(|bytes| crate::forms::decode_text(&bytes))
                        .ok_or(Error::EditUnsupported("this text has lost its contents"))?;
                    let mut da = Object::Str(fill::da(size, *color).into_bytes());
                    if let Some(crypt) = writer.crypt()? {
                        crypt.encrypt_strings(number, generation, &mut da);
                    }
                    set(&mut dict, b"DA", da);
                    FillText { rect, contents, size, color: *color }.appearance(leaf.rotate)?
                }
                Some(AnnotationKind::Mark) => {
                    let kind = dict
                        .get(b"Name")
                        .and_then(Object::as_name)
                        .and_then(fill::mark_kind_of)
                        .ok_or(Error::EditUnsupported("this mark cannot be read"))?;
                    Mark { rect, kind, color: *color }.appearance(leaf.rotate)?
                }
                _ => return Err(Error::InvalidEdit("only Fill & Sign text and marks change colour")),
            };
            let stream = writer.write_appearance(&appearance)?;
            let real = |value: f32| Object::Real(f64::from(value));
            set(&mut dict, b"AP", Object::Dict(Dict(vec![(b"N".to_vec(), Object::Ref(stream, 0))])));
            set(&mut dict, b"C", Object::Array(color.iter().map(|v| real(*v)).collect()));
            let mut date = Object::Str(pdf_date(SystemTime::now()));
            if let Some(crypt) = writer.crypt()? {
                crypt.encrypt_strings(number, generation, &mut date);
            }
            set(&mut dict, b"M", date);
            writer.update.object(number, generation, &Object::Dict(dict));
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
            match kind_of(&dict) {
                Some(AnnotationKind::Note) => {}
                Some(AnnotationKind::FillText) => {
                    // Set again at its size (`/DA`), its displayed top-left corner kept.
                    let da = decrypted_da(&mut writer, &dict, number, generation)?;
                    let (size, color) = text_box::parse_da(&da);
                    let rect = rect_of(&file, &dict).ok_or(Error::EditUnsupported("this item has no box"))?;
                    let text = FillText::replacing(rect, leaf.rotate, contents, size, color)?;
                    let appearance = text.appearance(leaf.rotate)?;
                    let stream = writer.write_appearance(&appearance)?;
                    let real = |value: f32| Object::Real(f64::from(value));
                    set(&mut dict, b"Rect", Object::Array(text.rect.iter().map(|v| real(*v)).collect()));
                    set(&mut dict, b"AP", Object::Dict(Dict(vec![(b"N".to_vec(), Object::Ref(stream, 0))])));
                }
                Some(AnnotationKind::TextBox) => {
                    // Set again from its `/DA` (size and colour) and `/Rect`.
                    let da = decrypted_da(&mut writer, &dict, number, generation)?;
                    let (size, color) = text_box::parse_da(&da);
                    let rect =
                        rect_of(&file, &dict).ok_or(Error::EditUnsupported("this text box has no box"))?;
                    let width = {
                        let [l, b, r, t] = rect;
                        if leaf.rotate.rem_euclid(180) == 90 { t - b } else { r - l }
                    };
                    let height = TextBox::height_for(contents, width, size)?;
                    let text_box = TextBox {
                        rect: text_box::resized(rect, leaf.rotate, height),
                        contents: contents.clone(),
                        size,
                        color,
                    };
                    let appearance = text_box.appearance(leaf.rotate)?;
                    let stream = writer.write_appearance(&appearance)?;
                    let real = |value: f32| Object::Real(f64::from(value));
                    set(&mut dict, b"Rect", Object::Array(text_box.rect.iter().map(|v| real(*v)).collect()));
                    set(&mut dict, b"AP", Object::Dict(Dict(vec![(b"N".to_vec(), Object::Ref(stream, 0))])));
                    for (key, value) in text_box.entries() {
                        if key == b"Q" {
                            set(&mut dict, &key, value);
                        }
                    }
                }
                _ => {
                    return Err(Error::InvalidEdit(
                        "only a note's, a text box's, or Fill & Sign text can be changed",
                    ));
                }
            }
            // The object keeps its number: strings already in it stay as
            // read; only the new ones are encrypted, under that number.
            let mut text = Object::Str(text_string(contents));
            let mut date = Object::Str(pdf_date(SystemTime::now()));
            if let Some(crypt) = writer.crypt()? {
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
            kind_of(&dict).ok_or(Error::InvalidEdit(
                "only markup, ink, notes, text boxes, and Fill & Sign items can be removed so far",
            ))?;
            // Its pop-up, and its thread (replies, replies to those, review
            // states, each with its pop-up), go with it.
            let mut doomed = vec![target.clone()];
            doomed.extend(popup(&dict));
            if matches!(target, Object::Ref(..)) {
                let entries: Vec<(Object, Dict)> = annots
                    .items
                    .iter()
                    .filter_map(|item| Some((item.clone(), resolve_dict(&file, item)?)))
                    .filter(|(_, dict)| dict.get(b"IRT").is_some())
                    .collect();
                let mut thread = vec![target.clone()];
                while let Some(parent) = thread.pop() {
                    for (item, reply) in &entries {
                        if reply.get(b"IRT") == Some(&parent) && !doomed.contains(item) {
                            doomed.push(item.clone());
                            doomed.extend(popup(reply));
                            thread.push(item.clone());
                        }
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
        AnnotationEdit::Reply { page, index, contents } => {
            check_contents(contents)?;
            if contents.trim().is_empty() {
                return Err(Error::InvalidEdit("a reply needs some text"));
            }
            let leaf = leaf(*page)?;
            let (target, rect) = thread_target(&file, leaf, *index)?;
            let entries = comments::thread_entries(&target, Some(contents), None);
            writer.add_with(
                leaf,
                b"Text",
                Note::DEFAULT_COLOR,
                &comments::empty_appearance(rect),
                entries,
                comments::REPLY_FLAGS,
            )?;
        }
        AnnotationEdit::SetResolved { page, index, resolved } => {
            let leaf = leaf(*page)?;
            let (target, rect) = thread_target(&file, leaf, *index)?;
            let entries = comments::thread_entries(&target, None, Some(*resolved));
            writer.add_with(
                leaf,
                b"Text",
                Note::DEFAULT_COLOR,
                &comments::empty_appearance(rect),
                entries,
                comments::STATE_FLAGS,
            )?;
        }
    }
    let update = writer.update;
    Ok(update.finish(form, file.trailer.clone(), section, next_number))
}

/// The pages that have annotations (a non-empty `/Annots`), or `None` where
/// our parser cannot read the page tree (then any page may have some).
pub(crate) fn annotated_pages(bytes: &[u8]) -> Option<std::collections::HashSet<usize>> {
    let Start { file, .. } = Start::read(bytes).ok()?;
    let tree = PageTree::read(&file).ok()?;
    let mut out = std::collections::HashSet::new();
    for (index, leaf) in tree.leaves.iter().enumerate() {
        match Annots::read(&file, leaf) {
            Ok(annots) if annots.items.is_empty() => {}
            _ => {
                out.insert(index);
            }
        }
    }
    Some(out)
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

/// The comment at `index` of the page's `/Annots` that a reply or a review
/// state can answer: an object of its own (`/IRT` names it), of a kind the
/// app knows, that answers nothing itself. Its reference and `/Rect`.
fn thread_target(file: &File<'_>, leaf: &Leaf, index: usize) -> Result<(Object, [f32; 4])> {
    let annots = Annots::read(file, leaf)?;
    let target = annots.items.get(index).cloned().ok_or(Error::InvalidEdit("no such annotation"))?;
    if !matches!(target, Object::Ref(..)) {
        return Err(Error::EditUnsupported("this annotation is not an object of its own"));
    }
    let dict = resolve_dict(file, &target).ok_or(Error::EditUnsupported("this annotation cannot be read"))?;
    if !kind_of(&dict).is_some_and(|kind| !kind.is_fill()) {
        return Err(Error::InvalidEdit("only markup, ink, notes, and text boxes take replies so far"));
    }
    if dict.get(b"IRT").is_some() {
        return Err(Error::InvalidEdit("a reply is answered in its comment's thread"));
    }
    let rect = rect_of(file, &dict).ok_or(Error::EditUnsupported("this annotation has no place"))?;
    Ok((target, rect))
}

pub(crate) fn number(object: &Object) -> Option<f32> {
    match object {
        Object::Int(value) => Some(*value as f32),
        Object::Real(value) => Some(*value as f32),
        _ => None,
    }
}

/// An annotation's `/Rect`, normalized (left, bottom, right, top).
pub(crate) fn rect_of(file: &File<'_>, dict: &Dict) -> Option<[f32; 4]> {
    let Object::Array(values) = file.resolve(dict.get(b"Rect")?)? else { return None };
    let numbers: Vec<f32> = values.iter().filter_map(|v| number(&file.resolve(v)?)).collect();
    let [x0, y0, x1, y1] = numbers.as_slice() else { return None };
    Some([x0.min(*x1), y0.min(*y1), x0.max(*x1), y0.max(*y1)])
}

fn kind_of(dict: &Dict) -> Option<AnnotationKind> {
    let subtype = dict.get(b"Subtype").and_then(Object::as_name)?;
    if subtype == b"Stamp" {
        return fill::kind_of_stamp(dict.get(b"Name").and_then(Object::as_name)?);
    }
    AnnotationKind::from_subtype(subtype)
}

/// A Fill & Sign item's new box: finite, and at least a point each way.
fn check_item_rect([left, bottom, right, top]: [f32; 4]) -> Result<()> {
    if !in_range((left, bottom)) || !in_range((right, top)) || right - left < 1.0 || top - bottom < 1.0 {
        return Err(Error::InvalidEdit("an item needs a place on the page"));
    }
    Ok(())
}

/// Gives a Fill & Sign item's appearance (`/AP /N`) the `/Matrix` that
/// stands it upright on a page turned by `rotate`: the stream object is
/// written again under its own number with its data as it is (encrypted
/// data stays valid), only its dictionary changed.
fn turn_appearance(file: &File<'_>, update: &mut Update, annotation: &Dict, rotate: i64) -> Result<()> {
    let unreadable = || Error::EditUnsupported("this item's appearance cannot be read");
    let ap = annotation.get(b"AP").and_then(|ap| resolve_dict(file, ap)).ok_or_else(unreadable)?;
    let Some(&Object::Ref(number, generation)) = ap.get(b"N") else { return Err(unreadable()) };
    let Some(&crate::pdf::Entry::Offset(offset)) = file.entries.get(&number) else {
        return Err(unreadable());
    };
    let (mut form, data) = file.indirect_at(offset).ok_or_else(unreadable)?;
    let data = data.ok_or_else(unreadable)?;
    form.0.retain(|(key, _)| key != b"Matrix");
    if let Some(matrix) = fill::upright(rotate) {
        let real = |value: f32| Object::Real(f64::from(value));
        form.0.push((b"Matrix".to_vec(), Object::Array(matrix.iter().map(|v| real(*v)).collect())));
    }
    update.stream(number, generation, form, data);
    Ok(())
}

/// An annotation's `/DA`, decrypted (empty when it has none).
fn decrypted_da(writer: &mut Writer<'_, '_>, dict: &Dict, number: u32, generation: u16) -> Result<Vec<u8>> {
    Ok(decrypted_string(writer, dict, b"DA", number, generation)?.unwrap_or_default())
}

/// A string entry of annotation `number`, decrypted.
fn decrypted_string(
    writer: &mut Writer<'_, '_>,
    dict: &Dict,
    key: &[u8],
    number: u32,
    generation: u16,
) -> Result<Option<Vec<u8>>> {
    Ok(match dict.get(key) {
        Some(Object::Str(text)) => Some(match writer.crypt()? {
            Some(crypt) => crypt.decrypt_string(number, generation, text).unwrap_or_default(),
            None => text.clone(),
        }),
        _ => None,
    })
}

/// The Fill & Sign items (our `/Stamp`s, ADR 0045) on `page`, by position
/// in its `/Annots`. Empty where the page cannot be read.
pub(crate) fn fill_kinds(bytes: &[u8], page: usize) -> HashMap<usize, AnnotationKind> {
    let mut out = HashMap::new();
    let Ok(Start { file, .. }) = Start::read(bytes) else { return out };
    let Ok(tree) = PageTree::read(&file) else { return out };
    let Some(leaf) = tree.leaves.get(page) else { return out };
    let Ok(annots) = Annots::read(&file, leaf) else { return out };
    for (index, item) in annots.items.iter().enumerate() {
        if let Some(kind) = resolve_dict(&file, item).as_ref().and_then(kind_of).filter(|k| k.is_fill()) {
            out.insert(index, kind);
        }
    }
    out
}

pub(crate) fn resolve_dict(file: &File<'_>, object: &Object) -> Option<Dict> {
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
pub(crate) struct Writer<'a, 'f> {
    file: &'a File<'f>,
    password: Option<&'a str>,
    /// `/T` of new annotations.
    author: Option<&'a str>,
    /// The file's security handler, once asked for (`None` before).
    crypt: Option<Option<Crypt>>,
    pub(crate) update: Update,
    next_number: &'a mut u32,
}

impl<'a, 'f> Writer<'a, 'f> {
    /// A writer of an update to `bytes` (whose newest revision `file` is).
    pub(crate) fn new(
        file: &'a File<'f>,
        bytes: &[u8],
        password: Option<&'a str>,
        next_number: &'a mut u32,
    ) -> Self {
        Writer { file, password, author: None, crypt: None, update: Update::new(bytes), next_number }
    }

    /// The file's unlocked security handler, if it is encrypted.
    pub(crate) fn crypt(&mut self) -> Result<Option<&Crypt>> {
        if self.crypt.is_none() {
            self.crypt = Some(Crypt::of(self.file, self.password)?);
        }
        Ok(self.crypt.as_ref().and_then(Option::as_ref))
    }

    pub(crate) fn take_number(&mut self) -> Result<u32> {
        let number = *self.next_number;
        *self.next_number = number.checked_add(1).ok_or(Error::EditUnsupported("it has too many objects"))?;
        Ok(number)
    }

    /// Writes an appearance stream, with the fonts it draws text in; returns
    /// its object number.
    pub(crate) fn write_appearance(&mut self, appearance: &Appearance) -> Result<u32> {
        self.crypt()?;
        let mut fonts = Vec::new();
        for font in &appearance.fonts {
            let crypt = self.crypt.as_ref().and_then(Option::as_ref);
            let number = write_font(&mut self.update, crypt, self.next_number, font)?;
            fonts.push((font.resource.as_bytes().to_vec(), Object::Ref(number, 0)));
        }
        let mut images = Vec::new();
        for image in &appearance.images {
            let crypt = self.crypt.as_ref().and_then(Option::as_ref);
            let number = write_image(&mut self.update, crypt, self.next_number, image)?;
            images.push((image.resource.as_bytes().to_vec(), Object::Ref(number, 0)));
        }
        let stream = self.take_number()?;
        let data = match self.crypt.as_ref().and_then(Option::as_ref) {
            Some(crypt) => crypt.encrypt_stream(stream, 0, &appearance.content),
            None => appearance.content.clone(),
        };
        let mut form = appearance.form();
        if let Some((_, Object::Dict(resources))) = form.0.iter_mut().find(|(key, _)| key == b"Resources") {
            if !fonts.is_empty() {
                resources.0.push((b"Font".to_vec(), Object::Dict(Dict(fonts))));
            }
            if !images.is_empty() {
                resources.0.push((b"XObject".to_vec(), Object::Dict(Dict(images))));
            }
        }
        self.update.stream(stream, 0, form, &data);
        Ok(stream)
    }

    /// Appends the appearance stream and the annotation, and adds the
    /// annotation to the page's `/Annots`. `entries` go after `/Rect`. It
    /// prints with the page, as in other viewers.
    fn add(
        &mut self,
        leaf: &Leaf,
        subtype: &[u8],
        color: [f32; 3],
        appearance: &Appearance,
        entries: Vec<(Vec<u8>, Object)>,
    ) -> Result<()> {
        self.add_with(leaf, subtype, color, appearance, entries, 4)
    }

    /// [`Writer::add`] with its annotation flags (`/F`).
    fn add_with(
        &mut self,
        leaf: &Leaf,
        subtype: &[u8],
        color: [f32; 3],
        appearance: &Appearance,
        entries: Vec<(Vec<u8>, Object)>,
        flags: i64,
    ) -> Result<()> {
        let annotation = self.take_number()?;
        let stream = self.write_appearance(appearance)?;

        let real = |value: f32| Object::Real(f64::from(value));
        let now = pdf_date(SystemTime::now());
        let mut dict = vec![
            (b"Type".to_vec(), Object::Name(b"Annot".to_vec())),
            (b"Subtype".to_vec(), Object::Name(subtype.to_vec())),
            (b"Rect".to_vec(), Object::Array(appearance.rect.iter().map(|v| real(*v)).collect())),
        ];
        dict.extend(entries);
        if let Some(author) = self.author {
            dict.push((b"T".to_vec(), Object::Str(text_string(author))));
        }
        dict.extend([
            (b"C".to_vec(), Object::Array(color.iter().map(|v| real(*v)).collect())),
            (b"F".to_vec(), Object::Int(flags)),
            (b"P".to_vec(), Object::Ref(leaf.number, leaf.generation)),
            (b"NM".to_vec(), Object::Str(unique_name())),
            (b"M".to_vec(), Object::Str(now.clone())),
            (b"CreationDate".to_vec(), Object::Str(now)),
            (b"AP".to_vec(), Object::Dict(Dict(vec![(b"N".to_vec(), Object::Ref(stream, 0))]))),
        ]);
        let mut dict = Object::Dict(Dict(dict));
        if let Some(crypt) = self.crypt()? {
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
pub(crate) struct Annots {
    /// The array's own object, when it is not in the page dictionary.
    own: Option<(u32, u16)>,
    pub(crate) items: Vec<Object>,
}

impl Annots {
    pub(crate) fn read(file: &File<'_>, leaf: &Leaf) -> Result<Self> {
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
    pub(crate) fn write(self, update: &mut Update, leaf: &Leaf) {
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
    /// Fonts the content draws text in, embedded with it (`/Font` resources).
    pub(crate) fonts: Vec<FontToEmbed>,
    /// Images the content draws, embedded with it (`/XObject` resources).
    pub(crate) images: Vec<ImageToEmbed>,
}

impl Appearance {
    /// Drawn in page space over `rect`.
    pub(crate) fn in_page_space(rect: [f32; 4], resources: Dict, content: Vec<u8>) -> Self {
        Self { rect, bbox: rect, matrix: None, resources, content, fonts: Vec::new(), images: Vec::new() }
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
pub(crate) fn pdf_date(time: SystemTime) -> Vec<u8> {
    let seconds = time.duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());
    let (year, month, day) = civil_from_days(i64::try_from(seconds / 86_400).unwrap_or(0));
    let rest = seconds % 86_400;
    format!("D:{year:04}{month:02}{day:02}{:02}{:02}{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
        .into_bytes()
}

/// A PDF date (`D:YYYYMMDDHHmmSSOHH'mm`, ISO 32000-2, 7.9.4) in Unix
/// seconds. Everything after the year is optional; a missing offset is UTC.
pub(crate) fn parse_pdf_date(text: &str) -> Option<i64> {
    let text = text.trim();
    let text = text.strip_prefix("D:").unwrap_or(text);
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    if digits < 4 {
        return None;
    }
    let field = |at: usize, len: usize, default: i64| -> Option<i64> {
        if at + len > digits { Some(default) } else { text.get(at..at + len)?.parse().ok() }
    };
    let (year, month, day) = (field(0, 4, 0)?, field(4, 2, 1)?, field(6, 2, 1)?);
    let (hour, minute, second) = (field(8, 2, 0)?, field(10, 2, 0)?, field(12, 2, 0)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let rest = text.get(digits..).unwrap_or("");
    let offset = match rest.as_bytes().first() {
        Some(sign @ (b'+' | b'-')) => {
            let numbers: Vec<i64> =
                rest[1..].split(|c: char| !c.is_ascii_digit()).filter_map(|n| n.parse().ok()).collect();
            let minutes = numbers.first().copied().unwrap_or(0) * 60 + numbers.get(1).copied().unwrap_or(0);
            if *sign == b'+' { minutes } else { -minutes }
        }
        _ => 0,
    };
    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset * 60)
}

/// Days from 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
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
pub(crate) fn unique_name() -> Vec<u8> {
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
    fn dates_are_read_back_with_their_offsets() {
        assert_eq!(parse_pdf_date("D:20010909014640Z"), Some(1_000_000_000));
        let written = pdf_date(UNIX_EPOCH + std::time::Duration::from_secs(1_795_000_000));
        assert_eq!(parse_pdf_date(std::str::from_utf8(&written).expect("ascii")), Some(1_795_000_000));
        // 03:00 in Riyadh is midnight UTC.
        assert_eq!(parse_pdf_date("D:20261122030000+03'00'"), parse_pdf_date("D:20261122000000Z"));
        assert_eq!(parse_pdf_date("D:20261121190000-05'00"), parse_pdf_date("D:20261122000000"));
        assert_eq!(parse_pdf_date("D:2026"), parse_pdf_date("D:20260101000000Z"));
        assert_eq!(parse_pdf_date("1970"), Some(0));
        assert_eq!(parse_pdf_date("D:20261322"), None, "month 13");
        assert_eq!(parse_pdf_date("yesterday"), None);
        for days in [-1, 0, 59, 19_782, 20_779] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
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
