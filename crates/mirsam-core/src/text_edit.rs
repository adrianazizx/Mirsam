//! Text written into a page's content (M3, ADR 0021): not an annotation, but
//! part of the page, so it prints, extracts, and searches like the page's own
//! text in every viewer.
//!
//! New text is laid out by the text engine (`typeset`, ADR 0020) and drawn by
//! the content-stream writer (`pdf::text`, ADR 0019) with embedded font
//! subsets, as an incremental update (ADR 0014):
//!
//! - **Its own content stream**, appended to the page's `/Contents`. It draws
//!   inside `q … Q`, under a `cm` that maps the box's displayed top-left
//!   corner (y up) to page space, so it stands upright on a turned page.
//! - **The page's own content is wrapped once** in a `q` stream before it and
//!   a `Q` stream after it (both marked `/MirsamWrap`), so a page that leaves
//!   its graphics state changed (an unbalanced `cm`) cannot move the new text.
//! - **The page gets a `/Resources` of its own:** what it used (its own, or an
//!   ancestor's) plus the new fonts, under names it does not use yet. Other
//!   pages that shared the old resources keep them.
//! - **The text is marked** (`/MirsamText` marked content), so later edits can
//!   find the blocks this app wrote.
//!
//! In an encrypted file the new streams and fonts are encrypted under their own
//! numbers (`pdf::crypt`). Copied resources that hold strings would no longer
//! decrypt under the page's number; such a page is refused.

use crate::edit::{PageTree, Start, contains_string, set};
use crate::error::{Error, Result};
use crate::fonts::{FontFamily, for_text};
use crate::pdf::crypt::Crypt;
use crate::pdf::text::{draw_text, write_font};
use crate::pdf::write::Update;
use crate::pdf::{Dict, File, Object};
use crate::typeset::{Align, BaseDirection, FontFace, LineHeight, ParagraphStyle, Span, TextLayout, layout};
use crate::view::ViewTransform;

/// The longest text added at once, in UTF-16 code units.
pub const MAX_TEXT: usize = 20_000;

/// Text to add to a page.
#[derive(Debug, Clone, PartialEq)]
pub struct NewText {
    pub page: usize,
    /// The box's top-left corner on the page as displayed: points from the
    /// displayed page's top-left corner, y down.
    pub x: f32,
    pub y: f32,
    /// The box's width as displayed; lines wrap inside it.
    pub width: f32,
    pub text: String,
    pub family: FontFamily,
    /// Points.
    pub size: f32,
    /// RGB, each 0 to 1.
    pub color: [f32; 3],
    pub align: Align,
    /// Auto: the text's first strong character decides (spec §6.3).
    pub direction: BaseDirection,
}

impl NewText {
    pub const DEFAULT_SIZE: f32 = 12.0;

    fn check(&self) -> Result<()> {
        if self.text.trim().is_empty() {
            return Err(Error::InvalidEdit("new text needs some text"));
        }
        if self.text.encode_utf16().count() > MAX_TEXT {
            return Err(Error::InvalidEdit("text is added at most 20,000 characters at a time"));
        }
        if !(1.0..=400.0).contains(&self.size) {
            return Err(Error::InvalidEdit("text is 1 to 400 points in size"));
        }
        if !(self.width >= 1.0 && self.width <= 14_400.0) {
            return Err(Error::InvalidEdit("text needs a width of 1 to 14,400 points"));
        }
        if !(self.x.is_finite() && self.y.is_finite() && self.x.abs() <= 14_400.0 && self.y.abs() <= 14_400.0)
        {
            return Err(Error::InvalidEdit("text needs a place on the page"));
        }
        if self.color.iter().any(|c| !(0.0..=1.0).contains(c)) {
            return Err(Error::InvalidEdit("colour components run from 0 to 1"));
        }
        Ok(())
    }

    /// The text laid out in its box, and the fonts it uses.
    fn laid_out(&self) -> Result<(TextLayout, Vec<crate::pdf::text::NamedFont>)> {
        self.check()?;
        let fonts = for_text(self.family, &self.text)?;
        let faces: Vec<FontFace> = fonts.iter().map(|font| font.face.clone()).collect();
        let style = ParagraphStyle {
            direction: self.direction,
            align: self.align,
            width: Some(self.width),
            line_height: LineHeight::Normal,
        };
        let laid = layout(&self.text, &[Span::new(0..self.text.len(), 0, self.size)], &faces, &style)
            .map_err(|err| Error::Typeset(err.code()))?;
        Ok((laid, fonts))
    }

    /// The box's height as displayed, in points: what the text needs.
    pub fn height(&self) -> Result<f32> {
        Ok(self.laid_out()?.0.height)
    }
}

/// Marks the streams that wrap a page's original content in `q … Q`.
pub(crate) const WRAP_KEY: &[u8] = b"MirsamWrap";

/// The bytes to append to `bytes` to add `text` to its page: an incremental
/// update. `view` maps the page to the page as displayed; `password` is the
/// one the document was opened with.
pub fn add_text_update(
    bytes: &[u8],
    password: Option<&str>,
    view: &ViewTransform,
    text: &NewText,
) -> Result<Vec<u8>> {
    let (laid, fonts) = text.laid_out()?;
    let Start { file, section, form, mut next_number } = Start::read(bytes)?;
    let tree = PageTree::read(&file)?;
    let leaf = tree
        .leaves
        .get(text.page)
        .ok_or(Error::PageOutOfRange { index: text.page, count: tree.leaves.len() })?;
    let crypt = Crypt::of(&file, password)?;
    let mut update = Update::new(bytes);

    // The page's resources, made its own, with names free for the new fonts.
    let mut resources = tree
        .attribute(leaf, b"Resources")
        .and_then(|value| file.resolve(&value))
        .and_then(|value| value.as_dict().cloned())
        .unwrap_or_default();
    let mut font_names = resources
        .get(b"Font")
        .and_then(|value| file.resolve(value))
        .and_then(|value| value.as_dict().cloned())
        .unwrap_or_default();
    let prefix = free_prefix(&font_names);

    // The text, drawn from the box's top-left corner (y up) and mapped onto the page.
    let drawing = draw_text(&text.text, &laid, &fonts, (0.0, 0.0), text.color, &prefix)?;
    for font in &drawing.fonts {
        let number = write_font(&mut update, crypt.as_ref(), &mut next_number, font)?;
        set(&mut font_names, font.resource.as_bytes(), Object::Ref(number, 0));
    }
    set(&mut resources, b"Font", Object::Dict(font_names));
    if crypt.is_some() && contains_string(&Object::Dict(resources.clone())) {
        return Err(Error::EditUnsupported(
            "this page's resources in an encrypted file cannot be copied yet",
        ));
    }

    let [a, b, c, d, e, f] = placement(view, text.x, text.y)?;
    let mut content =
        format!("q\n{} {} {} {} {} {} cm\n/MirsamText BMC\n", num(a), num(b), num(c), num(d), num(e), num(f))
            .into_bytes();
    content.extend_from_slice(&drawing.content);
    content.extend_from_slice(b"EMC\nQ\n");

    let mut take = || -> Result<u32> {
        let number = next_number;
        next_number = number.checked_add(1).ok_or(Error::EditUnsupported("it has too many objects"))?;
        Ok(number)
    };
    let stream = |update: &mut Update, number: u32, dict: Dict, data: &[u8]| {
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(data, 6);
        let mut dict = dict;
        dict.0.push((b"Filter".to_vec(), Object::Name(b"FlateDecode".to_vec())));
        let data = match crypt.as_ref() {
            Some(crypt) => crypt.encrypt_stream(number, 0, &compressed),
            None => compressed,
        };
        update.stream(number, 0, dict, &data);
    };

    // The page's content, wrapped once in `q … Q`, then the new text.
    let mut contents = page_contents(&file, &leaf.dict)?;
    let wrapped = contents
        .first()
        .and_then(|first| file.resolve(first))
        .and_then(|first| first.as_dict().cloned())
        .is_some_and(|dict| dict.get(WRAP_KEY).is_some());
    if !wrapped && !contents.is_empty() {
        let marker = || Dict(vec![(WRAP_KEY.to_vec(), Object::Bool(true))]);
        let (open, close) = (take()?, take()?);
        stream(&mut update, open, marker(), b"q\n");
        stream(&mut update, close, marker(), b"\nQ\n");
        contents.insert(0, Object::Ref(open, 0));
        contents.push(Object::Ref(close, 0));
    }
    let number = take()?;
    stream(&mut update, number, Dict::default(), &content);
    contents.push(Object::Ref(number, 0));

    let mut page = leaf.dict.clone();
    set(&mut page, b"Contents", Object::Array(contents));
    set(&mut page, b"Resources", Object::Dict(resources));
    update.object(leaf.number, leaf.generation, &Object::Dict(page));
    Ok(update.finish(form, file.trailer.clone(), section, next_number))
}

/// The page's content streams, in order: `/Contents` is one stream, an array
/// of them (possibly an object of its own), or absent.
pub(crate) fn page_contents(file: &File<'_>, page: &Dict) -> Result<Vec<Object>> {
    Ok(match page.get(b"Contents") {
        None | Some(Object::Null) => Vec::new(),
        Some(Object::Array(items)) => items.clone(),
        Some(reference @ Object::Ref(..)) => match file.resolve(reference) {
            Some(Object::Array(items)) => items,
            Some(Object::Dict(_)) => vec![reference.clone()],
            _ => return Err(Error::EditUnsupported("the page's content cannot be read")),
        },
        Some(_) => return Err(Error::EditUnsupported("the page's content cannot be read")),
    })
}

/// A prefix (`MT`, `MT2`, …) under which no font of the page is named.
pub(crate) fn free_prefix(fonts: &Dict) -> String {
    (1..)
        .map(|n: u32| if n == 1 { "MT".to_string() } else { format!("MT{n}x") })
        .find(|prefix| !fonts.0.iter().any(|(key, _)| key.starts_with(prefix.as_bytes())))
        .unwrap_or_else(|| "MT".to_string())
}

/// The `cm` matrix mapping a box whose top-left corner is displayed at
/// (`x`, `y`) — its own space y up, origin at that corner — to page space.
pub(crate) fn placement(view: &ViewTransform, x: f32, y: f32) -> Result<[f32; 6]> {
    let origin = view.page_point(x, y);
    let right = view.page_point(x + 1.0, y);
    let up = view.page_point(x, y - 1.0);
    let matrix =
        [right.0 - origin.0, right.1 - origin.1, up.0 - origin.0, up.1 - origin.1, origin.0, origin.1];
    if matrix.iter().any(|v| !v.is_finite()) {
        return Err(Error::InvalidEdit("text needs a place on the page"));
    }
    Ok(matrix)
}

/// A number for a content stream: three decimals at most, no exponent.
pub(crate) fn num(value: f32) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-0" || text == "-" { "0".into() } else { text.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_font_names_never_collide() {
        let names =
            |keys: &[&str]| Dict(keys.iter().map(|k| (k.as_bytes().to_vec(), Object::Null)).collect());
        assert_eq!(free_prefix(&names(&["F1", "F2"])), "MT");
        assert_eq!(free_prefix(&names(&["F1", "MT0"])), "MT2x");
        assert_eq!(free_prefix(&names(&["MT0", "MT2x0"])), "MT3x");
    }

    #[test]
    fn text_is_checked_before_it_is_written() {
        let text = NewText {
            page: 0,
            x: 10.0,
            y: 10.0,
            width: 200.0,
            text: "نص".into(),
            family: FontFamily::Sans,
            size: 12.0,
            color: [0.0, 0.0, 0.0],
            align: Align::Start,
            direction: BaseDirection::Auto,
        };
        assert!(text.height().is_ok_and(|h| h > 12.0 && h < 30.0));
        for broken in [
            NewText { text: "  ".into(), ..text.clone() },
            NewText { size: 0.5, ..text.clone() },
            NewText { width: 0.0, ..text.clone() },
            NewText { x: f32::NAN, ..text.clone() },
            NewText { color: [2.0, 0.0, 0.0], ..text.clone() },
        ] {
            assert!(matches!(broken.height(), Err(Error::InvalidEdit(_))), "{broken:?}");
        }
    }
}
