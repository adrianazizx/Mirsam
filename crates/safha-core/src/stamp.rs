//! Headers and footers, page numbers, and watermarks (M6, ADR 0050).
//!
//! All three are *stamps*: something drawn on a set of pages, written as one
//! incremental update (ADR 0014), so one undo takes it back. Nothing already
//! on a page is regenerated (ADR 0005's amendment); each page gets one new
//! content stream, in the way text is added (ADR 0021):
//!
//! - the page's own content is wrapped once in `q … Q` (`/MirsamWrap`);
//! - the stamp's stream goes after it (over the page) or just inside the
//!   wrapper's opening `q`, before the page's content (behind it);
//! - the page gets a `/Resources` of its own with the new fonts, the image,
//!   and the transparency state, under names it does not use yet.
//!
//! Header and footer text is laid out by the text engine (`typeset`) and
//! drawn by our own writer (`pdf::text`) with embedded subsets, so Arabic is
//! shaped and copies out in logical order. A text watermark is laid out the
//! same way but drawn as glyph outlines (`pdf::outline`): it is decoration,
//! and as text it would be read into copy, search, and export. Every stamp is marked as a pagination artifact
//! (`/Artifact << /Type /Pagination /Subtype /Header|/Footer|/Watermark >>`,
//! ISO 32000-1 §14.8.2.2), which tagged-PDF readers and screen readers skip,
//! with `/MirsamStamp true` so a later version can find and remove it.
//!
//! A band writes one set of font subsets for all its pages, holding the glyphs
//! of every text it draws, so page numbers on a thousand pages share one small
//! font.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::edit::{PageTree, Start, contains_string, set};
use crate::error::{Error, Result};
use crate::fonts::{FontFamily, for_text};
use crate::pdf::crypt::Crypt;
use crate::pdf::outline::draw_outlines;
use crate::pdf::text::{draw_with_subsets, fonts_to_embed, write_font};
use crate::pdf::write::Update;
use crate::pdf::{Dict, Object};
use crate::text_edit::{WRAP_KEY, num, page_contents};
use crate::typeset::{
    Align, BaseDirection, FontFace, LineHeight, ParagraphStyle, Span, layout, subset_fonts,
};
use crate::view::ViewTransform;

/// The longest text in one slot or watermark, in characters.
pub const MAX_STAMP_TEXT: usize = 500;
/// The largest image a watermark takes, in pixels.
const MAX_IMAGE_PIXELS: u64 = 40_000_000;

/// The digits numbers are written in (spec §6: Western or Arabic-Indic).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Digits {
    #[default]
    Western,
    /// ٠١٢٣٤٥٦٧٨٩ (U+0660 to U+0669).
    ArabicIndic,
}

impl Digits {
    pub fn format(self, number: u64) -> String {
        let western = number.to_string();
        match self {
            Digits::Western => western,
            Digits::ArabicIndic => western
                .chars()
                .map(|c| c.to_digit(10).and_then(|d| char::from_u32(0x0660 + d)).unwrap_or(c))
                .collect(),
        }
    }
}

/// Which edge a band runs along.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Edge {
    Top,
    Bottom,
}

/// A header or a footer: up to three pieces of text along one edge.
///
/// `{page}` in the text becomes the page's number and `{pages}` the last
/// number, counting from `first_number` over the stamped pages, in `digits`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Band {
    pub edge: Edge,
    /// At the start of the line: the left edge, or the right in a
    /// right-to-left document (`rtl`).
    pub start: String,
    pub center: String,
    pub end: String,
    pub rtl: bool,
    pub family: FontFamily,
    /// Points.
    pub size: f32,
    /// RGB, each 0 to 1.
    pub color: [f32; 3],
    /// From the page's edges to the text, in points.
    pub margin: f32,
    pub digits: Digits,
    pub first_number: u32,
}

/// What a watermark shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Mark {
    Text {
        text: String,
        family: FontFamily,
        size: f32,
        color: [f32; 3],
    },
    /// A PNG or JPEG file's bytes, drawn `width` of the page's width across
    /// (0.05 to 1).
    Image {
        #[serde(skip)]
        data: Vec<u8>,
        width: f32,
    },
}

/// Text or an image across the middle of each page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Watermark {
    pub mark: Mark,
    /// 0 (invisible) to 1 (opaque).
    pub opacity: f32,
    /// Degrees counter-clockwise, as displayed.
    pub rotation: f32,
    /// Drawn before the page's content instead of over it.
    pub behind: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StampKind {
    Band(Band),
    Watermark(Watermark),
}

/// A stamp and the pages (zero-based) it goes on.
#[derive(Debug, Clone, PartialEq)]
pub struct Stamp {
    pub pages: Vec<usize>,
    pub kind: StampKind,
}

/// A page to stamp: its index, how it is displayed, and its displayed size.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Target {
    pub(crate) index: usize,
    pub(crate) view: ViewTransform,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

fn check_text_style(size: f32, color: [f32; 3]) -> Result<()> {
    if !(1.0..=400.0).contains(&size) {
        return Err(Error::InvalidEdit("text is 1 to 400 points in size"));
    }
    if color.iter().any(|c| !(0.0..=1.0).contains(c)) {
        return Err(Error::InvalidEdit("colour components run from 0 to 1"));
    }
    Ok(())
}

impl Stamp {
    fn check(&self) -> Result<()> {
        if self.pages.is_empty() {
            return Err(Error::InvalidEdit("no pages given"));
        }
        let long = |text: &str| text.chars().count() > MAX_STAMP_TEXT;
        match &self.kind {
            StampKind::Band(band) => {
                check_text_style(band.size, band.color)?;
                let texts = [&band.start, &band.center, &band.end];
                if texts.iter().all(|text| text.trim().is_empty()) {
                    return Err(Error::InvalidEdit("a header or footer needs some text"));
                }
                if texts.iter().any(|text| long(text)) {
                    return Err(Error::InvalidEdit("header and footer text is at most 500 characters"));
                }
                if !(0.0..=500.0).contains(&band.margin) {
                    return Err(Error::InvalidEdit("the margin is 0 to 500 points"));
                }
            }
            StampKind::Watermark(watermark) => {
                if !(0.0..=1.0).contains(&watermark.opacity) {
                    return Err(Error::InvalidEdit("opacity runs from 0 to 1"));
                }
                if !(-360.0..=360.0).contains(&watermark.rotation) {
                    return Err(Error::InvalidEdit("rotation is -360 to 360 degrees"));
                }
                match &watermark.mark {
                    Mark::Text { text, size, color, .. } => {
                        check_text_style(*size, *color)?;
                        if text.trim().is_empty() {
                            return Err(Error::InvalidEdit("a watermark needs some text"));
                        }
                        if long(text) {
                            return Err(Error::InvalidEdit("watermark text is at most 500 characters"));
                        }
                    }
                    Mark::Image { width, .. } => {
                        if !(0.05..=1.0).contains(width) {
                            return Err(Error::InvalidEdit(
                                "an image watermark is 5 to 100% of the page wide",
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// The text of a band's slot on one page: tokens replaced.
pub fn fill_tokens(text: &str, number: u64, last: u64, digits: Digits) -> String {
    text.replace("{page}", &digits.format(number)).replace("{pages}", &digits.format(last))
}

/// Laid-out text, its drawing ready (its fonts are the band's).
struct Drawn {
    content: Vec<u8>,
    width: f32,
    height: f32,
}

/// What one page gets: its index, the content, the fonts it uses, and
/// whether it goes behind the page's own content.
type PageDrawing = (usize, Vec<u8>, Vec<FontRef>, bool);

/// A font's resource name and object number.
type FontRef = (String, u32);

/// Writes a stream object (number, dictionary, data, whether to compress).
type StreamWriter<'a> = dyn Fn(&mut Update, u32, Dict, &[u8], bool) + 'a;

/// An image XObject written into the update.
struct Image {
    number: u32,
    width: u32,
    height: u32,
}

/// Builds the update. Each target's `view` and size come from the document.
pub(crate) fn stamp_update(
    bytes: &[u8],
    password: Option<&str>,
    targets: &[Target],
    stamp: &Stamp,
) -> Result<Vec<u8>> {
    stamp.check()?;
    let Start { file, section, form, mut next_number } = Start::read(bytes)?;
    let tree = PageTree::read(&file)?;
    let crypt = Crypt::of(&file, password)?;
    let mut update = Update::new(bytes);
    for target in targets {
        if target.index >= tree.leaves.len() {
            return Err(Error::PageOutOfRange { index: target.index, count: tree.leaves.len() });
        }
    }
    let mut seen = std::collections::HashSet::new();
    if !targets.iter().all(|target| seen.insert(target.index)) {
        return Err(Error::InvalidEdit("a page is listed twice"));
    }

    // Names no stamped page uses yet: fonts, the image, the transparency state.
    let resources_of = |index: usize| -> Dict {
        tree.leaves
            .get(index)
            .and_then(|leaf| tree.attribute(leaf, b"Resources"))
            .and_then(|value| file.resolve(&value))
            .and_then(|value| value.as_dict().cloned())
            .unwrap_or_default()
    };
    let sub = |resources: &Dict, key: &[u8]| -> Dict {
        resources
            .get(key)
            .and_then(|value| file.resolve(value))
            .and_then(|value| value.as_dict().cloned())
            .unwrap_or_default()
    };
    let mut font_base = String::from("MS");
    let mut other_base = String::from("MS");
    for target in targets {
        let resources = resources_of(target.index);
        let fonts = sub(&resources, b"Font");
        while fonts.0.iter().any(|(key, _)| key.starts_with(font_base.as_bytes())) {
            font_base.push('x');
        }
        for key in [b"XObject".as_slice(), b"ExtGState"] {
            let names = sub(&resources, key);
            while names.0.iter().any(|(name, _)| name.starts_with(other_base.as_bytes())) {
                other_base.push('x');
            }
        }
    }
    let image_name = format!("{other_base}Im");
    let state_name = format!("{other_base}Gs");

    let take = |next: &mut u32| -> Result<u32> {
        let number = *next;
        *next = number.checked_add(1).ok_or(Error::EditUnsupported("it has too many objects"))?;
        Ok(number)
    };
    let write_stream = |update: &mut Update, number: u32, mut dict: Dict, data: &[u8], compress: bool| {
        let data = if compress {
            dict.0.push((b"Filter".to_vec(), Object::Name(b"FlateDecode".to_vec())));
            miniz_oxide::deflate::compress_to_vec_zlib(data, 6)
        } else {
            data.to_vec()
        };
        let data = match crypt.as_ref() {
            Some(crypt) => crypt.encrypt_stream(number, 0, &data),
            None => data,
        };
        update.stream(number, 0, dict, &data);
    };

    // What each page draws: (content, fonts used, whether behind the page).
    let mut pages: Vec<PageDrawing> = Vec::new();
    let mut image: Option<Image> = None;
    let mut state: Option<u32> = None;
    match &stamp.kind {
        StampKind::Band(band) => {
            let last = u64::from(band.first_number) + targets.len() as u64 - 1;
            // Physical sides: start is on the right in a right-to-left document.
            let (left, right) = if band.rtl { (&band.end, &band.start) } else { (&band.start, &band.end) };
            let texts: Vec<[String; 3]> = (0..targets.len())
                .map(|position| {
                    let number = u64::from(band.first_number) + position as u64;
                    [left, &band.center, right]
                        .map(|text| fill_tokens(text.trim(), number, last, band.digits))
                })
                .collect();
            let (set, fonts) = set_texts(
                texts.iter().flatten(),
                band,
                &font_base,
                &mut update,
                crypt.as_ref(),
                &mut next_number,
            )?;
            let subtype = match band.edge {
                Edge::Top => "Header",
                Edge::Bottom => "Footer",
            };
            for (target, texts) in targets.iter().zip(&texts) {
                let mut content =
                    format!("/Artifact << /Type /Pagination /Subtype /{subtype} /MirsamStamp true >> BDC\n");
                for (slot, text) in texts.iter().enumerate() {
                    let Some(drawn) = set.get(text) else { continue };
                    let x = match slot {
                        0 => band.margin,
                        1 => (target.width - drawn.width) / 2.0,
                        _ => target.width - band.margin - drawn.width,
                    };
                    let y = match band.edge {
                        Edge::Top => band.margin,
                        Edge::Bottom => target.height - band.margin - drawn.height,
                    };
                    let matrix = local_to_page(&target.view, |u, v| (x + u, y - v))?;
                    let _ = writeln!(content, "q {} cm", matrix_text(&matrix));
                    content.push_str(&String::from_utf8_lossy(&drawn.content));
                    content.push_str("Q\n");
                }
                content.push_str("EMC\n");
                pages.push((target.index, content.into_bytes(), fonts.clone(), false));
            }
        }
        StampKind::Watermark(watermark) => {
            let opacity = Object::Real(f64::from((watermark.opacity * 1000.0).round() / 1000.0));
            let number = take(&mut next_number)?;
            let mut dict = Dict(vec![
                (b"Type".to_vec(), Object::Name(b"ExtGState".to_vec())),
                (b"ca".to_vec(), opacity.clone()),
                (b"CA".to_vec(), opacity),
            ]);
            if let Some(crypt) = crypt.as_ref() {
                let mut object = Object::Dict(dict);
                crypt.encrypt_strings(number, 0, &mut object);
                dict = object.as_dict().cloned().unwrap_or_default();
            }
            update.object(number, 0, &Object::Dict(dict));
            state = Some(number);

            let (angle_sin, angle_cos) = watermark.rotation.to_radians().sin_cos();
            // Local space: the mark's box, origin at its top-left corner, y up.
            let around_centre = |target: &Target, width: f32, height: f32| {
                let (cx, cy) = (target.width / 2.0, target.height / 2.0);
                move |u: f32, v: f32| {
                    let (dx, dy) = (u - width / 2.0, v + height / 2.0);
                    (cx + dx * angle_cos - dy * angle_sin, cy - (dx * angle_sin + dy * angle_cos))
                }
            };
            let head = "/Artifact << /Type /Pagination /Subtype /Watermark /MirsamStamp true >> BDC\n";
            match &watermark.mark {
                Mark::Text { text, family, size, color } => {
                    // Outlines, not text: a watermark is not part of what the page says.
                    let text = text.trim();
                    let fonts = for_text(*family, text)?;
                    let faces: Vec<FontFace> = fonts.iter().map(|font| font.face.clone()).collect();
                    let laid = layout(text, &[Span::new(0..text.len(), 0, *size)], &faces, &single_line())
                        .map_err(|err| Error::Typeset(err.code()))?;
                    let shapes = draw_outlines(text, &laid, &faces, (0.0, 0.0), *color)?;
                    for target in targets {
                        let matrix =
                            local_to_page(&target.view, around_centre(target, laid.width, laid.height))?;
                        let mut content = format!("{head}q /{state_name} gs {} cm\n", matrix_text(&matrix));
                        content.push_str(&String::from_utf8_lossy(&shapes));
                        content.push_str("Q\nEMC\n");
                        pages.push((target.index, content.into_bytes(), Vec::new(), watermark.behind));
                    }
                }
                Mark::Image { data, width } => {
                    let written = write_image(data, &mut update, &mut next_number, &write_stream)?;
                    for target in targets {
                        let w = target.width * width;
                        let h = w * written.height as f32 / written.width as f32;
                        let matrix = local_to_page(&target.view, around_centre(target, w, h))?;
                        let content = format!(
                            "{head}q /{state_name} gs {} cm {} 0 0 {} 0 {} cm /{image_name} Do Q\nEMC\n",
                            matrix_text(&matrix),
                            num(w),
                            num(h),
                            num(-h)
                        );
                        pages.push((target.index, content.into_bytes(), Vec::new(), watermark.behind));
                    }
                    image = Some(written);
                }
            }
        }
    }

    for (index, content, fonts, behind) in pages {
        let leaf = &tree.leaves[index];
        let mut resources = resources_of(index);
        if !fonts.is_empty() {
            let mut names = sub(&resources, b"Font");
            for (name, number) in fonts {
                set(&mut names, name.as_bytes(), Object::Ref(number, 0));
            }
            set(&mut resources, b"Font", Object::Dict(names));
        }
        if let Some(image) = &image {
            let mut names = sub(&resources, b"XObject");
            set(&mut names, image_name.as_bytes(), Object::Ref(image.number, 0));
            set(&mut resources, b"XObject", Object::Dict(names));
        }
        if let Some(state) = state {
            let mut names = sub(&resources, b"ExtGState");
            set(&mut names, state_name.as_bytes(), Object::Ref(state, 0));
            set(&mut resources, b"ExtGState", Object::Dict(names));
        }
        if crypt.is_some() && contains_string(&Object::Dict(resources.clone())) {
            return Err(Error::EditUnsupported(
                "this page's resources in an encrypted file cannot be copied yet",
            ));
        }

        let mut contents = page_contents(&file, &leaf.dict)?;
        let wrapped = contents
            .first()
            .and_then(|first| file.resolve(first))
            .and_then(|first| first.as_dict().cloned())
            .is_some_and(|dict| dict.get(WRAP_KEY).is_some());
        if !wrapped && !contents.is_empty() {
            let marker = || Dict(vec![(WRAP_KEY.to_vec(), Object::Bool(true))]);
            let (open, close) = (take(&mut next_number)?, take(&mut next_number)?);
            write_stream(&mut update, open, marker(), b"q\n", true);
            write_stream(&mut update, close, marker(), b"\nQ\n", true);
            contents.insert(0, Object::Ref(open, 0));
            contents.push(Object::Ref(close, 0));
        }
        let number = take(&mut next_number)?;
        let mut drawing = b"q\n".to_vec();
        drawing.extend_from_slice(&content);
        drawing.extend_from_slice(b"Q\n");
        write_stream(&mut update, number, Dict::default(), &drawing, true);
        if behind && !contents.is_empty() {
            // Inside the wrapper's opening `q`, before the page's own content.
            contents.insert(1, Object::Ref(number, 0));
        } else {
            contents.push(Object::Ref(number, 0));
        }

        let mut page = leaf.dict.clone();
        set(&mut page, b"Contents", Object::Array(contents));
        set(&mut page, b"Resources", Object::Dict(resources));
        update.object(leaf.number, leaf.generation, &Object::Dict(page));
    }
    Ok(update.finish(form, file.trailer.clone(), section, next_number))
}

/// Text on one line (hard breaks only), its direction from its first strong character.
fn single_line() -> ParagraphStyle {
    ParagraphStyle {
        direction: BaseDirection::Auto,
        align: Align::Start,
        width: None,
        line_height: LineHeight::Normal,
    }
}

/// Lays out every distinct text of a band and writes one set of font subsets
/// that holds the glyphs of them all (made from the texts joined as
/// paragraphs, so each is shaped as it is alone): page numbers on a thousand
/// pages share one small font instead of a thousand. Returns each text's
/// drawing and the fonts' resource names and object numbers.
fn set_texts<'t>(
    texts: impl Iterator<Item = &'t String>,
    band: &Band,
    font_base: &str,
    update: &mut Update,
    crypt: Option<&Crypt>,
    next: &mut u32,
) -> Result<(HashMap<String, Drawn>, Vec<FontRef>)> {
    let distinct: BTreeSet<&String> = texts.filter(|text| !text.is_empty()).collect();
    let all = distinct.iter().map(|text| text.as_str()).collect::<Vec<_>>().join("\n");
    let fonts = for_text(band.family, &all)?;
    let faces: Vec<FontFace> = fonts.iter().map(|font| font.face.clone()).collect();
    let set = |text: &str| {
        layout(text, &[Span::new(0..text.len(), 0, band.size)], &faces, &single_line())
            .map_err(|err| Error::Typeset(err.code()))
    };
    let subsets = subset_fonts(&all, &set(&all)?, &faces).map_err(|err| Error::Typeset(err.code()))?;
    let prefix = format!("{font_base}_");
    let mut names = Vec::new();
    for font in fonts_to_embed(subsets.clone(), &fonts, &prefix) {
        names.push((font.resource.clone(), write_font(update, crypt, next, &font)?));
    }
    let mut drawn = HashMap::new();
    for text in distinct {
        let laid = set(text)?;
        let content = draw_with_subsets(text, &laid, &subsets, (0.0, 0.0), band.color, &prefix);
        drawn.insert(text.clone(), Drawn { content, width: laid.width, height: laid.height });
    }
    Ok((drawn, names))
}

/// Writes a PNG or JPEG as an image XObject (a PNG's alpha as its soft mask).
fn write_image(
    data: &[u8],
    update: &mut Update,
    next: &mut u32,
    write_stream: &StreamWriter<'_>,
) -> Result<Image> {
    let unreadable = || Error::InvalidEdit("the image is not a PNG or JPEG file that can be read");
    let mut take = || -> Result<u32> {
        let number = *next;
        *next = number.checked_add(1).ok_or(Error::EditUnsupported("it has too many objects"))?;
        Ok(number)
    };
    let name = |n: &[u8]| Object::Name(n.to_vec());
    let image_dict = |width: u32, height: u32, space: &[u8]| {
        Dict(vec![
            (b"Type".to_vec(), name(b"XObject")),
            (b"Subtype".to_vec(), name(b"Image")),
            (b"Width".to_vec(), Object::Int(i64::from(width))),
            (b"Height".to_vec(), Object::Int(i64::from(height))),
            (b"ColorSpace".to_vec(), name(space)),
            (b"BitsPerComponent".to_vec(), Object::Int(8)),
        ])
    };
    let too_big = |width: u32, height: u32| u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS;

    if let Some((width, height, components)) = crate::jpeg::frame(data) {
        if too_big(width, height) {
            return Err(Error::InvalidEdit("the image is too large"));
        }
        let number = take()?;
        let space: &[u8] = if components == 1 { b"DeviceGray" } else { b"DeviceRGB" };
        let mut dict = image_dict(width, height, space);
        dict.0.push((b"Filter".to_vec(), name(b"DCTDecode")));
        write_stream(update, number, dict, data, false);
        return Ok(Image { number, width, height });
    }

    let mut decoder = png::Decoder::new_with_limits(
        std::io::Cursor::new(data),
        png::Limits { bytes: usize::try_from(MAX_IMAGE_PIXELS * 4).unwrap_or(usize::MAX) },
    );
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|_| unreadable())?;
    let mut buffer = vec![0u8; reader.output_buffer_size().ok_or_else(unreadable)?];
    let info = reader.next_frame(&mut buffer).map_err(|_| unreadable())?;
    let (width, height) = (info.width, info.height);
    if too_big(width, height) || info.bit_depth != png::BitDepth::Eight {
        return Err(Error::InvalidEdit("the image is too large"));
    }
    let channels = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err(unreadable()),
    };
    let (color_channels, has_alpha) = match channels {
        1 => (1, false),
        2 => (1, true),
        3 => (3, false),
        _ => (3, true),
    };
    let mut color = Vec::with_capacity(width as usize * height as usize * color_channels);
    let mut alpha = Vec::new();
    for row in buffer.chunks(info.line_size).take(height as usize) {
        for pixel in row.chunks(channels).take(width as usize) {
            color.extend_from_slice(&pixel[..color_channels]);
            if has_alpha {
                alpha.push(pixel[channels - 1]);
            }
        }
    }
    let number = take()?;
    let space: &[u8] = if color_channels == 1 { b"DeviceGray" } else { b"DeviceRGB" };
    let mut dict = image_dict(width, height, space);
    if has_alpha && alpha.iter().any(|&a| a != 255) {
        let mask = take()?;
        write_stream(update, mask, image_dict(width, height, b"DeviceGray"), &alpha, true);
        dict.0.push((b"SMask".to_vec(), Object::Ref(mask, 0)));
    }
    write_stream(update, number, dict, &color, true);
    Ok(Image { number, width, height })
}

/// The `cm` matrix for a local space whose points `map` sends to the
/// displayed page (an affine map), mapped on to page space.
pub(crate) fn local_to_page(view: &ViewTransform, map: impl Fn(f32, f32) -> (f32, f32)) -> Result<[f32; 6]> {
    let page = |(x, y): (f32, f32)| view.page_point(x, y);
    let origin = page(map(0.0, 0.0));
    let ex = page(map(1.0, 0.0));
    let ey = page(map(0.0, 1.0));
    let matrix = [ex.0 - origin.0, ex.1 - origin.1, ey.0 - origin.0, ey.1 - origin.1, origin.0, origin.1];
    if matrix.iter().any(|v| !v.is_finite()) {
        return Err(Error::InvalidEdit("the stamp needs a place on the page"));
    }
    Ok(matrix)
}

fn matrix_text(matrix: &[f32; 6]) -> String {
    matrix.iter().map(|v| num(*v)).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_take_the_chosen_digits() {
        assert_eq!(Digits::Western.format(120), "120");
        assert_eq!(Digits::ArabicIndic.format(120), "١٢٠");
        assert_eq!(fill_tokens("صفحة {page} من {pages}", 3, 12, Digits::ArabicIndic), "صفحة ٣ من ١٢");
        assert_eq!(fill_tokens("Page {page} of {pages}", 3, 12, Digits::Western), "Page 3 of 12");
    }

    #[test]
    fn the_local_space_lands_where_it_is_displayed() {
        // A 600 x 800 page shown as it is: displayed y runs down from the top.
        let view = ViewTransform::unrotated(800.0);
        let matrix = local_to_page(&view, |u, v| (50.0 + u, 30.0 - v)).expect("matrix");
        assert_eq!(matrix, [1.0, 0.0, 0.0, 1.0, 50.0, 770.0]);
    }

    #[test]
    fn stamps_are_checked() {
        let band = Band {
            edge: Edge::Bottom,
            start: String::new(),
            center: "{page}".into(),
            end: String::new(),
            rtl: false,
            family: FontFamily::Sans,
            size: 10.0,
            color: [0.0; 3],
            margin: 36.0,
            digits: Digits::Western,
            first_number: 1,
        };
        let stamp = |kind| Stamp { pages: vec![0], kind };
        assert!(stamp(StampKind::Band(band.clone())).check().is_ok());
        let empty = Band { center: " ".into(), ..band.clone() };
        assert!(stamp(StampKind::Band(empty)).check().is_err());
        assert!(Stamp { pages: vec![], kind: StampKind::Band(band) }.check().is_err());
        let watermark = Watermark {
            mark: Mark::Text {
                text: "سري".into(), family: FontFamily::Naskh, size: 60.0, color: [0.5; 3]
            },
            opacity: 1.5,
            rotation: 45.0,
            behind: false,
        };
        assert!(stamp(StampKind::Watermark(watermark)).check().is_err());
    }
}
