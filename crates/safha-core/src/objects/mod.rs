//! Page objects: images, paths, text blocks and form XObjects, traced to
//! the bytes that draw them, and moved, resized, rotated, cropped, removed,
//! reordered and replaced by byte-range edits of the page's content, as
//! incremental updates (ADR 0051). PDFium never regenerates the content
//! (ADR 0005's amendment): every byte this module does not edit stays.
//!
//! - **Move, resize, rotate:** the piece goes inside a wrapper of ours with
//!   one `cm`, where it was, so its z-order and the clip in force keep. A
//!   text piece inside a text object is split out of it (`ET`, the wrapper,
//!   `BT` and the line and position it left), so the text after it stays.
//! - **Crop:** a clip inside the wrapper, in its own space, so it moves with it.
//! - **Remove:** the bytes go; text leaves its advance behind; a path that also
//!   clipped leaves its clip.
//! - **Reorder:** the piece is drawn again elsewhere from its recipe (the state
//!   it was drawn in), front and back always, one step where the clip and
//!   `gs` there allow it.
//! - **Images:** a new image is an XObject placed in its own wrapper on top;
//!   a replacement is fitted in the old one's box, at the old one's place.

mod trace;

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use serde::{Deserialize, Serialize};

pub(crate) use trace::{Content, Piece, PieceKind, Snap, Traced, fmt, fmt_matrix};
use trace::{intersect, trace};

use crate::edit::{PageTree, Start, contains_string, set};
use crate::error::{Error, Result};
use crate::pdf::content::{Matrix, Operand, tokenize};
use crate::pdf::crypt::Crypt;
use crate::pdf::image::{ImageLimits, image_size, image_to_embed, write_image};
use crate::pdf::write::Update;
use crate::pdf::{Dict, Entry, File, Object, decode_stream};
use crate::text_edit::{WRAP_KEY, page_contents};
use crate::view::{ViewRect, ViewTransform};

/// What an object on the page is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ObjectKind {
    Image,
    Path,
    Text,
    /// A form XObject: a group drawn as one.
    Form,
    Shading,
}

/// An image's own size in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageSize {
    pub width: u32,
    pub height: u32,
}

/// Why an object can be seen but not changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ObjectRefusal {
    /// Text whose text object changes the matrix inside it, which splitting
    /// it would lose.
    TextState,
}

/// One object of a page, in drawing order (the last is on top).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageObject {
    pub index: usize,
    /// Changes when the object's bytes do: an edit names the object by index
    /// and fingerprint, and one that no longer matches is refused.
    pub fingerprint: u32,
    pub kind: ObjectKind,
    /// Its bounds on the displayed page, within any clip.
    pub rect: ViewRect,
    pub image: Option<ImageSize>,
    /// Cropped by this app (it can be uncropped).
    pub cropped: bool,
    pub refusal: Option<ObjectRefusal>,
}

/// The object an edit is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectRef {
    pub index: usize,
    pub fingerprint: u32,
}

/// Where a reorder takes an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Arrange {
    /// Over everything on the page.
    Front,
    /// Over the next object above it that it overlaps.
    Forward,
    /// Under the next object below it that it overlaps.
    Backward,
    /// Under everything on the page.
    Back,
}

/// A change to one object.
#[derive(Debug, Clone)]
pub enum ObjectEdit {
    /// An affine map of the displayed page (`[a b c d e f]`, a point goes to
    /// `(a·x + c·y + e, b·x + d·y + f)`): a move, a resize, a rotation.
    Transform {
        matrix: [f64; 6],
    },
    Delete,
    Arrange(Arrange),
    /// Shows only this part of the object (displayed page); `None` shows all of it again.
    Crop {
        rect: Option<ViewRect>,
    },
    /// Another PNG or JPEG in the image's place, fitted in its box.
    ReplaceImage {
        data: Vec<u8>,
    },
}

/// An edit of one object of one page.
#[derive(Debug, Clone)]
pub struct ObjectChange {
    pub page: usize,
    pub object: ObjectRef,
    pub edit: ObjectEdit,
}

/// A new image on a page: a PNG or JPEG, and where it goes on the displayed page.
#[derive(Debug, Clone)]
pub struct NewImage {
    pub page: usize,
    pub rect: ViewRect,
    pub data: Vec<u8>,
}

/// What images may be inserted or put in another's place.
pub(crate) const IMAGE_LIMITS: ImageLimits = ImageLimits {
    max_side: 20_000,
    max_pixels: 100_000_000,
    too_large: "an image is at most 20,000 pixels a side and 100 million pixels",
    unreadable: "only PNG and JPEG images can be placed",
};

/// The largest image file read for inserting or replacing.
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;

/// The size in pixels of a PNG or JPEG that may be placed.
pub fn image_pixels(data: &[u8]) -> Result<(u32, u32)> {
    image_size(data, &IMAGE_LIMITS)
}

/// Paths, shadings and forms covering this much of the page are page
/// backgrounds: not offered, so a press on an empty spot selects nothing.
const BACKGROUND_SHARE: f64 = 0.9;

/// An object: one or more pieces (text pieces of one block).
struct Obj {
    kind: ObjectKind,
    pieces: Vec<usize>,
    bounds: [f64; 4],
}

/// A page, read.
struct PageRead<'a> {
    file: File<'a>,
    tree: PageTree,
    leaf: usize,
    resources: Dict,
    content: Content,
    traced: Traced,
    objects: Vec<Obj>,
    crypt: Option<Crypt>,
    /// Text pieces by where their operator starts (stream, offset).
    shows: HashMap<(usize, usize), usize>,
}

fn unreadable() -> Error {
    Error::EditUnsupported("the page's content cannot be read")
}

/// The page's content streams, decrypted and decoded.
fn read_content(file: &File<'_>, page: &Dict, crypt: Option<&Crypt>) -> Result<Content> {
    let items = page_contents(file, page)?;
    let mut content =
        Content { items: items.clone(), data: Vec::new(), ops: Vec::new(), wrap_streams: Vec::new() };
    for item in &items {
        let &Object::Ref(number, generation) = item else { return Err(unreadable()) };
        let Some(Entry::Offset(offset)) = file.entries.get(&number) else { return Err(unreadable()) };
        let (dict, raw) = file.indirect_at(*offset).ok_or_else(unreadable)?;
        let raw = raw.ok_or_else(unreadable)?;
        let raw = match crypt {
            Some(crypt) => crypt.decrypt_stream_of(number, generation, &dict, raw).ok_or_else(unreadable)?,
            None => raw.to_vec(),
        };
        let data = decode_stream(&dict, &raw, file)
            .ok_or(Error::EditUnsupported("the page's content uses a filter the editor cannot read"))?;
        let ops = tokenize(&data).ok_or_else(unreadable)?;
        content.wrap_streams.push(dict.get(WRAP_KEY).is_some());
        content.data.push(data);
        content.ops.push(ops);
    }
    Ok(content)
}

fn area(b: &[f64; 4]) -> f64 {
    (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0)
}

fn union(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

/// Whether two text pieces drawn one after the other belong to one block:
/// they touch once grown by one and a half font sizes along the line (word
/// spaces, a justified line's too) and half of one across it (the gap
/// between lines), but not by a column's gutter or a paragraph's space.
fn near(a: &[f64; 4], b: &[f64; 4], em: f64, horizontal: bool) -> bool {
    let (along, across) = ((em * 1.5).max(0.5), (em * 0.5).max(0.5));
    let (gx, gy) = if horizontal { (along, across) } else { (across, along) };
    let grown = [a[0] - gx, a[1] - gy, a[2] + gx, a[3] + gy];
    intersect(grown, *b).is_some()
}

/// Pieces → objects: each image, path, form or shading on its own; text
/// pieces drawn one after another and near each other as one block, or by
/// the wrapper group this app gave them.
fn group(traced: &Traced, page_area: f64) -> Vec<Obj> {
    let mut objects: Vec<Obj> = Vec::new();
    // The open text block: its object index, its group, its last piece's bounds and font size.
    let mut block: Option<(usize, u32, [f64; 4], f64)> = None;
    for (i, piece) in traced.pieces.iter().enumerate() {
        let Some(bounds) = piece.bounds else { continue };
        let kind = match piece.kind {
            PieceKind::Image | PieceKind::InlineImage => ObjectKind::Image,
            PieceKind::Path => ObjectKind::Path,
            PieceKind::Form => ObjectKind::Form,
            PieceKind::Shading => ObjectKind::Shading,
            PieceKind::Text => ObjectKind::Text,
        };
        if kind != ObjectKind::Text {
            block = None;
            if kind != ObjectKind::Image && area(&bounds) >= BACKGROUND_SHARE * page_area {
                continue;
            }
            objects.push(Obj { kind, pieces: vec![i], bounds });
            continue;
        }
        if piece.text.as_ref().is_some_and(|t| t.invisible) {
            continue;
        }
        let group = own_wrapper(traced, piece).map_or(0, |w| traced.wrappers[w].group);
        let em = piece.text.as_ref().map_or(0.0, |t| t.em);
        let horizontal = piece.text.as_ref().is_none_or(|t| t.horizontal);
        let joins = match block {
            Some((_, g, last, last_em)) if g == group => {
                group != 0 || near(&last, &bounds, em.max(last_em), horizontal)
            }
            _ => false,
        };
        // A group of ours may be drawn apart (pieces moved with it stay one).
        let earlier = (group != 0 && !joins).then(|| {
            objects.iter().position(|o| {
                o.kind == ObjectKind::Text && {
                    let p = &traced.pieces[o.pieces[0]];
                    own_wrapper(traced, p).is_some_and(|w| traced.wrappers[w].group == group)
                }
            })
        });
        match (joins, earlier.flatten(), block) {
            (true, _, Some((at, ..))) | (_, Some(at), _) => {
                objects[at].pieces.push(i);
                objects[at].bounds = union(objects[at].bounds, bounds);
                block = Some((at, group, bounds, em));
            }
            _ => {
                objects.push(Obj { kind, pieces: vec![i], bounds });
                block = Some((objects.len() - 1, group, bounds, em));
            }
        }
    }
    objects
}

/// The wrapper of ours a piece is in.
fn own_wrapper(_traced: &Traced, piece: &Piece) -> Option<usize> {
    piece.wrapper
}

fn fingerprint(content: &Content, traced: &Traced, object: &Obj) -> u32 {
    let mut hash: u32 = 0x811C_9DC5;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            hash ^= u32::from(*b);
            hash = hash.wrapping_mul(0x0100_0193);
        }
    };
    eat(&[object.kind as u8]);
    for &i in &object.pieces {
        let piece = &traced.pieces[i];
        eat(&(piece.stream as u64).to_le_bytes());
        eat(&(piece.range.start as u64).to_le_bytes());
        eat(&content.data[piece.stream][piece.range.clone()]);
        if let Some(w) = own_wrapper(traced, piece) {
            let wrapper = &traced.wrappers[w];
            eat(&content.data[wrapper.stream][wrapper.range.clone()]);
        }
    }
    hash
}

fn read_page<'a>(
    bytes: &'a [u8],
    password: Option<&str>,
    page: usize,
    page_area: f64,
) -> Result<PageRead<'a>> {
    let Start { file, .. } = Start::read(bytes)?;
    let tree = PageTree::read(&file)?;
    let leaf =
        tree.leaves.get(page).ok_or(Error::PageOutOfRange { index: page, count: tree.leaves.len() })?;
    let crypt = Crypt::of(&file, password)?;
    let resources = tree
        .attribute(leaf, b"Resources")
        .and_then(|r| file.resolve(&r))
        .and_then(|r| r.as_dict().cloned())
        .unwrap_or_default();
    let content = read_content(&file, &leaf.dict, crypt.as_ref())?;
    let traced = trace(&file, &resources, &content).ok_or_else(unreadable)?;
    let objects = group(&traced, page_area);
    let shows = traced
        .pieces
        .iter()
        .enumerate()
        .filter(|(_, p)| p.kind == PieceKind::Text)
        .map(|(i, p)| ((p.stream, p.range.start), i))
        .collect();
    Ok(PageRead { file, tree, leaf: page, resources, content, traced, objects, crypt, shows })
}

fn displayed(view: &ViewTransform, b: [f64; 4]) -> ViewRect {
    view.rect(crate::text::Rect {
        left: b[0] as f32,
        bottom: b[1] as f32,
        right: b[2] as f32,
        top: b[3] as f32,
    })
}

/// The objects of a page, in drawing order. `page_area` is the page's area
/// in square points; `view` maps it to the page as displayed.
pub fn page_objects(
    bytes: &[u8],
    password: Option<&str>,
    view: &ViewTransform,
    page: usize,
    page_area: f64,
) -> Result<Vec<PageObject>> {
    let read = read_page(bytes, password, page, page_area)?;
    Ok(describe(&read, view))
}

fn describe(read: &PageRead<'_>, view: &ViewTransform) -> Vec<PageObject> {
    read.objects
        .iter()
        .enumerate()
        .map(|(index, object)| {
            let pieces: Vec<&Piece> = object.pieces.iter().map(|&i| &read.traced.pieces[i]).collect();
            let image =
                pieces.iter().find_map(|p| p.image).map(|m| ImageSize { width: m.width, height: m.height });
            let cropped = pieces.iter().any(|p| {
                own_wrapper(&read.traced, p).is_some_and(|w| read.traced.wrappers[w].crop.is_some())
            });
            let refusal = pieces
                .iter()
                .any(|p| {
                    p.kind == PieceKind::Text && p.text.is_none() && own_wrapper(&read.traced, p).is_none()
                })
                .then_some(ObjectRefusal::TextState);
            PageObject {
                index,
                fingerprint: fingerprint(&read.content, &read.traced, object),
                kind: object.kind,
                rect: displayed(view, object.bounds),
                image,
                cropped,
                refusal,
            }
        })
        .collect()
}

/// One change to a content stream's bytes.
struct Splice {
    stream: usize,
    range: Range<usize>,
    with: Vec<u8>,
}

/// What an edit does to the page.
#[derive(Default)]
struct Plan {
    splices: Vec<Splice>,
    /// New streams drawn over everything (after the page's own content).
    front: Vec<Vec<u8>>,
    /// New streams drawn under everything.
    back: Vec<Vec<u8>>,
    /// XObjects to add to the page's resources.
    images: Vec<crate::pdf::image::ImageToEmbed>,
}

fn matrix_of(m: [f64; 6]) -> Matrix {
    Matrix(m)
}

fn inverse(m: &Matrix) -> Result<Matrix> {
    m.inverse().ok_or(Error::EditUnsupported("the object is drawn at no size"))
}

/// A displayed-page map as a page-space one.
fn page_map(view: &ViewTransform, displayed: Matrix) -> Result<Matrix> {
    let v = Matrix(view.matrix());
    Ok(v.then(displayed).then(inverse(&v)?))
}

/// The wrapper opening for group `group` with matrix `m`.
fn wrapper_open(group: u32, m: &Matrix) -> String {
    format!("\n/MirsamObj <</G {group}>> BDC q {} cm\n", fmt_matrix(m))
}

const WRAPPER_CLOSE: &str = "\nQ EMC\n";

/// A quadrilateral path (`m l l l h`) through `corners`, as a crop.
fn crop_ops(corners: [(f64, f64); 4]) -> String {
    let p = |(x, y): (f64, f64)| format!("{} {}", fmt(x), fmt(y));
    format!("{} m {} l {} l {} l h W n\n", p(corners[0]), p(corners[1]), p(corners[2]), p(corners[3]))
}

/// The displayed rectangle's corners in a space whose map to page space is `space`.
fn rect_in(view: &ViewTransform, rect: &ViewRect, space: &Matrix) -> Result<[(f64, f64); 4]> {
    let inverse = inverse(space)?;
    let corner = |x: f32, y: f32| {
        let (px, py) = view.page_point(x, y);
        inverse.apply(f64::from(px), f64::from(py))
    };
    let (x0, y0, x1, y1) = (rect.x, rect.y, rect.x + rect.width, rect.y + rect.height);
    let corners = [corner(x0, y1), corner(x1, y1), corner(x1, y0), corner(x0, y0)];
    if corners.iter().any(|(x, y)| !x.is_finite() || !y.is_finite())
        || rect.width <= 0.0
        || rect.height <= 0.0
    {
        return Err(Error::InvalidEdit("a crop needs an area"));
    }
    Ok(corners)
}

/// Operators inside a text object that change the graphics state for what
/// comes after it: drawn again after a run taken out of it.
const TEXT_STATE_OPS: [&[u8]; 22] = [
    b"Tf", b"Tc", b"Tw", b"Tz", b"TL", b"Ts", b"Tr", b"g", b"rg", b"k", b"G", b"RG", b"K", b"cs", b"CS",
    b"sc", b"scn", b"SC", b"SCN", b"gs", b"w", b"d",
];

fn is_show(name: &[u8]) -> bool {
    matches!(name, b"Tj" | b"TJ" | b"'" | b"\"")
}

/// A `[n] TJ` that moves the text position `advance` along the line, when it moves it.
fn spacer(advance: f64, scale: f64) -> Result<String> {
    if advance.abs() < 1e-9 {
        return Ok(String::new());
    }
    if scale.abs() < 1e-9 {
        return Err(Error::EditUnsupported("this text is drawn at no size"));
    }
    Ok(format!(" [{}] TJ", fmt(-advance * 1000.0 / scale)))
}

/// Consecutive text-showing operators of one object in one text object,
/// with what taking them out of it takes: the marked content open around
/// them (closed before, opened again after), the line and position before
/// and after them, and the state they change for what follows.
struct Run {
    stream: usize,
    range: Range<usize>,
    open_before: Vec<Vec<u8>>,
    open_after: Vec<Vec<u8>>,
    /// `Tm` (and a `TJ` move) for the start of the run, and for after it.
    start: String,
    end: String,
    replay: String,
    snap: Snap,
}

impl Run {
    fn reopen(ops: &[Vec<u8>]) -> String {
        ops.iter().map(|op| format!("{} ", String::from_utf8_lossy(op))).collect()
    }

    /// Ends the text object the run was in.
    fn before(&self) -> String {
        format!("{}ET", "EMC ".repeat(self.open_before.len()))
    }

    /// The run in a text object of its own.
    fn alone(&self, data: &[u8]) -> Vec<u8> {
        let mut out = format!("BT {}{}\n", Self::reopen(&self.open_before), self.start).into_bytes();
        out.extend_from_slice(&data[self.range.clone()]);
        out.extend_from_slice(format!("\n{}ET", "EMC ".repeat(self.open_after.len())).as_bytes());
        out
    }

    /// Starts the text object again where the text after the run expects it.
    fn after(&self) -> String {
        format!("\n{}BT {}{}\n", self.replay, Self::reopen(&self.open_after), self.end)
    }
}

/// What an object is made of, as edits handle it.
enum Unit {
    /// A wrapper of ours (one image, path or form, or one run of text).
    Wrapped(usize),
    Run(Box<Run>),
    /// An image, path, form or shading drawn as the page has it.
    Piece(usize),
}

impl PageRead<'_> {
    fn data(&self, piece: &Piece) -> &[u8] {
        &self.content.data[piece.stream][piece.range.clone()]
    }

    /// What stays where a path that clipped was: its clip.
    fn kept_clip(&self, piece: &Piece) -> Vec<u8> {
        let Some(range) = &piece.clip_ops else { return Vec::new() };
        let data = &self.content.data[piece.stream];
        // The painting operator is `W` or `W*` followed by the paint.
        let tail = &data[range.end..piece.range.end];
        let even_odd = tail.windows(2).any(|w| w == b"W*");
        let mut out = b"\n".to_vec();
        out.extend_from_slice(&data[range.clone()]);
        out.extend_from_slice(if even_odd { b" W* n\n" } else { b" W n\n" });
        out
    }

    /// The units of an object, in drawing order.
    fn units(&self, object: &Obj) -> Result<Vec<Unit>> {
        let mut units = Vec::new();
        let mut run: Vec<usize> = Vec::new();
        let flush = |run: &mut Vec<usize>, units: &mut Vec<Unit>| -> Result<()> {
            if !run.is_empty() {
                units.push(Unit::Run(Box::new(self.run(run)?)));
                run.clear();
            }
            Ok(())
        };
        for &i in &object.pieces {
            let piece = &self.traced.pieces[i];
            if let Some(w) = piece.wrapper {
                flush(&mut run, &mut units)?;
                if !matches!(units.last(), Some(Unit::Wrapped(x)) if *x == w) {
                    units.push(Unit::Wrapped(w));
                }
            } else if piece.kind == PieceKind::Text {
                if let Some(&last) = run.last()
                    && !(last + 1 == i && self.traced.pieces[last].start.0 == piece.start.0)
                {
                    flush(&mut run, &mut units)?;
                }
                run.push(i);
            } else {
                flush(&mut run, &mut units)?;
                units.push(Unit::Piece(i));
            }
        }
        flush(&mut run, &mut units)?;
        Ok(units)
    }

    /// The run of text-showing operators `pieces` (consecutive, in one text object).
    fn run(&self, pieces: &[usize]) -> Result<Run> {
        let odd = || Error::EditUnsupported("this text changes its matrix mid-way");
        let split = || Error::EditUnsupported("this text object runs across content streams");
        let first = &self.traced.pieces[pieces[0]];
        let last = &self.traced.pieces[*pieces.last().ok_or_else(odd)?];
        if pieces.iter().any(|&i| self.traced.pieces[i].text.is_none()) {
            return Err(odd());
        }
        let stream = first.stream;
        let ((bt_stream, bt_offset), _) = first.start;
        if bt_stream != stream || last.stream != stream {
            return Err(split());
        }
        let ops = &self.content.ops[stream];
        let data = &self.content.data[stream];
        let find = |offset: usize| ops.binary_search_by_key(&offset, |op| op.range.start).map_err(|_| odd());
        let (bt, f, l) = (find(bt_offset)?, find(first.range.start)?, find(last.range.start)?);
        let et = (l + 1..ops.len()).find(|&j| ops[j].name == b"ET").ok_or_else(split)?;
        // From just after the show before it (and the marked content that
        // show closes) to just after its own last show and what that closes.
        let previous = (bt + 1..f).rev().find(|&j| is_show(&ops[j].name));
        let mut k = previous.map_or(bt + 1, |p| p + 1);
        while k < f && ops[k].name == b"EMC" {
            k += 1;
        }
        let mut e = l;
        while e + 1 < et && ops[e + 1].name == b"EMC" {
            e += 1;
        }
        let open_at = |upto: usize| -> Vec<Vec<u8>> {
            let mut stack: Vec<Vec<u8>> = Vec::new();
            for op in &ops[bt + 1..upto] {
                match op.name.as_slice() {
                    b"BDC" | b"BMC" => stack.push(data[op.range.clone()].to_vec()),
                    b"EMC" => {
                        stack.pop();
                    }
                    _ => {}
                }
            }
            stack
        };
        let start = match previous {
            Some(p) => {
                let t = self
                    .shows
                    .get(&(stream, ops[p].range.start))
                    .and_then(|&i| self.traced.pieces[i].text.as_ref())
                    .ok_or_else(odd)?;
                format!("{} Tm{}", fmt_matrix(&t.tlm_after), spacer(t.line_advance, t.size * t.scaling)?)
            }
            None => String::new(),
        };
        let t = last.text.as_ref().ok_or_else(odd)?;
        let end = format!("{} Tm{}", fmt_matrix(&t.tlm_after), spacer(t.line_advance, t.size * t.scaling)?);
        let mut replay = String::new();
        for op in &ops[k..=e] {
            if TEXT_STATE_OPS.contains(&op.name.as_slice()) {
                replay.push_str(&String::from_utf8_lossy(&data[op.range.clone()]));
                replay.push('\n');
            } else if op.name == b"\"" {
                let nums: Vec<f64> = op
                    .operands
                    .iter()
                    .filter_map(|o| match o {
                        Operand::Num(n) => Some(*n),
                        _ => None,
                    })
                    .collect();
                if let [aw, ac] = nums[..] {
                    replay.push_str(&format!("{} Tw {} Tc\n", fmt(aw), fmt(ac)));
                }
            }
        }
        Ok(Run {
            stream,
            range: ops[k].range.start..ops[e].range.end,
            open_before: open_at(k),
            open_after: open_at(e + 1),
            start,
            end,
            replay,
            snap: first.snap.clone(),
        })
    }

    /// Removes a unit.
    fn delete(&self, unit: &Unit) -> Splice {
        match unit {
            Unit::Wrapped(w) => {
                let wrapper = &self.traced.wrappers[*w];
                Splice { stream: wrapper.stream, range: wrapper.range.clone(), with: Vec::new() }
            }
            Unit::Run(run) => Splice {
                stream: run.stream,
                range: run.range.clone(),
                with: format!("{}{}", run.before(), run.after()).into_bytes(),
            },
            Unit::Piece(i) => {
                let piece = &self.traced.pieces[*i];
                Splice { stream: piece.stream, range: piece.range.clone(), with: self.kept_clip(piece) }
            }
        }
    }

    /// Draws a unit through `t`, a page-space map, where it is; `crop` sets
    /// or clears the crop of an image, path or form.
    fn wrap(
        &self,
        unit: &Unit,
        group: u32,
        t: &Matrix,
        crop: Option<Option<(&ViewTransform, &ViewRect)>>,
    ) -> Result<Vec<Splice>> {
        match unit {
            Unit::Wrapped(w) => {
                let wrapper = &self.traced.wrappers[*w];
                let c = wrapper.outer.ctm;
                let m = wrapper.matrix.then(c).then(*t).then(inverse(&c)?);
                let mut splices = vec![Splice {
                    stream: wrapper.stream,
                    range: wrapper.cm.clone(),
                    with: format!("{} cm", fmt_matrix(&m)).into_bytes(),
                }];
                if let Some(crop) = crop {
                    let ops = match crop {
                        Some((view, rect)) => crop_ops(rect_in(view, rect, &m.then(c))?),
                        None => String::new(),
                    };
                    splices.push(match &wrapper.crop {
                        Some(range) => {
                            Splice { stream: wrapper.stream, range: range.clone(), with: ops.into_bytes() }
                        }
                        None => Splice {
                            stream: wrapper.stream,
                            range: wrapper.cm.end..wrapper.cm.end,
                            with: format!("\n{ops}").into_bytes(),
                        },
                    });
                }
                Ok(splices)
            }
            Unit::Run(run) => {
                let c = run.snap.ctm;
                let m = c.then(*t).then(inverse(&c)?);
                let data = &self.content.data[run.stream];
                let mut with = run.before().into_bytes();
                with.extend_from_slice(wrapper_open(group, &m).as_bytes());
                with.extend_from_slice(&run.alone(data));
                with.extend_from_slice(WRAPPER_CLOSE.trim_end().as_bytes());
                with.extend_from_slice(run.after().as_bytes());
                Ok(vec![Splice { stream: run.stream, range: run.range.clone(), with }])
            }
            Unit::Piece(i) => {
                let piece = &self.traced.pieces[*i];
                let c = piece.ctm;
                let m = c.then(*t).then(inverse(&c)?);
                let crop = match crop.flatten() {
                    Some((view, rect)) => crop_ops(rect_in(view, rect, &m.then(c))?),
                    None => String::new(),
                };
                let mut with = wrapper_open(group, &m).into_bytes();
                with.extend_from_slice(crop.as_bytes());
                with.extend_from_slice(self.data(piece));
                with.extend_from_slice(WRAPPER_CLOSE.as_bytes());
                with.extend_from_slice(&self.kept_clip(piece));
                Ok(vec![Splice { stream: piece.stream, range: piece.range.clone(), with }])
            }
        }
    }

    /// A unit as something to draw elsewhere — its wrapper, or a new one
    /// around it — with the state it needs, and the splice that takes it
    /// away from where it is.
    fn movable(&self, unit: &Unit, group: u32) -> (Snap, Vec<u8>, Splice) {
        let wrapped = |inner: &[u8]| {
            let mut bytes = wrapper_open(group, &Matrix::IDENTITY).trim_start().as_bytes().to_vec();
            bytes.extend_from_slice(inner);
            bytes.extend_from_slice(WRAPPER_CLOSE.trim_end().as_bytes());
            bytes
        };
        let removal = self.delete(unit);
        match unit {
            Unit::Wrapped(w) => {
                let wrapper = &self.traced.wrappers[*w];
                let bytes = self.content.data[wrapper.stream][wrapper.range.clone()].to_vec();
                (wrapper.outer.clone(), bytes, removal)
            }
            Unit::Run(run) => {
                (run.snap.clone(), wrapped(&run.alone(&self.content.data[run.stream])), removal)
            }
            Unit::Piece(i) => {
                let piece = &self.traced.pieces[*i];
                (piece.snap.clone(), wrapped(self.data(piece)), removal)
            }
        }
    }

    /// The operators that take the state at `at` to the state `to`, inside a
    /// `q` the caller closes. Refused when `at` has a clip or `gs` `to` does
    /// not start from.
    fn transition(&self, at: &Snap, to: &Snap) -> Result<String> {
        let refuse = || Error::EditUnsupported("the object is clipped or blended differently there");
        if at.clips.len() > to.clips.len() || at.clips != to.clips[..at.clips.len()] {
            return Err(refuse());
        }
        let (gs_at, gs_to) = (at.gs_ids(), to.gs_ids());
        if gs_at.len() > gs_to.len() || gs_at != gs_to[..gs_at.len()] {
            return Err(refuse());
        }
        let mut out = String::from("q\n");
        let mut current = at.ctm;
        if current != Matrix::IDENTITY {
            out.push_str(&format!("{} cm\n", fmt_matrix(&inverse(&current)?)));
            current = Matrix::IDENTITY;
        }
        for &clip in &to.clips[at.clips.len()..] {
            let clip = &self.traced.clips[clip];
            let path = clip.path.as_ref().ok_or(Error::EditUnsupported("the object is clipped by text"))?;
            out.push_str(&String::from_utf8_lossy(path));
            out.push_str(if clip.even_odd { "W* n\n" } else { "W n\n" });
        }
        // What `at` set that `to` did not goes back to its initial value.
        let has = |snap: &Snap, param: trace::Param| snap.log.iter().any(|e| e.param == param);
        let mut reset: Vec<&'static [u8]> = Vec::new();
        for entry in &at.log {
            if entry.param == trace::Param::ExtGState || has(to, entry.param) {
                continue;
            }
            let sibling = match entry.param {
                trace::Param::StrokeSpace => Some(trace::Param::StrokeColor),
                trace::Param::StrokeColor => Some(trace::Param::StrokeSpace),
                trace::Param::FillSpace => Some(trace::Param::FillColor),
                trace::Param::FillColor => Some(trace::Param::FillSpace),
                _ => None,
            };
            if sibling.is_some_and(|s| has(to, s)) {
                continue;
            }
            if let Some(op) = entry.param.default_op()
                && !reset.contains(&op)
            {
                reset.push(op);
            }
        }
        for op in reset {
            out.push_str(&String::from_utf8_lossy(op));
            out.push('\n');
        }
        for entry in &to.log {
            if entry.param == trace::Param::ExtGState {
                if gs_at.contains(&entry.id) {
                    continue;
                }
                if entry.ctm != current {
                    out.push_str(&format!("{} cm\n", fmt_matrix(&entry.ctm.then(inverse(&current)?))));
                    current = entry.ctm;
                }
            }
            out.push_str(&String::from_utf8_lossy(&entry.op));
            out.push('\n');
        }
        if to.ctm != current {
            out.push_str(&format!("{} cm\n", fmt_matrix(&to.ctm.then(inverse(&current)?))));
        }
        Ok(out)
    }

    fn object(&self, at: ObjectRef) -> Result<&Obj> {
        let object = self.objects.get(at.index).ok_or(Error::InvalidEdit("no such object on the page"))?;
        if fingerprint(&self.content, &self.traced, object) != at.fingerprint {
            return Err(Error::InvalidEdit("the page's objects changed since they were read"));
        }
        Ok(object)
    }

    /// The group number an edited object's wrappers get: its own, or a new one.
    fn group_of(&self, object: &Obj) -> u32 {
        object
            .pieces
            .iter()
            .find_map(|&i| own_wrapper(&self.traced, &self.traced.pieces[i]))
            .map(|w| self.traced.wrappers[w].group)
            .filter(|g| *g != 0)
            .unwrap_or(self.traced.max_group + 1)
    }
}

/// Where an edited object is afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
    /// In wrappers of this group.
    Group(u32),
    /// At the same index.
    Index(usize),
    /// Removed.
    Gone,
}

/// The bytes to append to `bytes` to make `change`, and where the object is
/// afterwards.
pub fn object_update(
    bytes: &[u8],
    password: Option<&str>,
    view: &ViewTransform,
    page_area: f64,
    change: &ObjectChange,
) -> Result<(Vec<u8>, Kept)> {
    let read = read_page(bytes, password, change.page, page_area)?;
    let object = read.object(change.object)?;
    let pieces: Vec<&Piece> = object.pieces.iter().map(|&i| &read.traced.pieces[i]).collect();
    let group = read.group_of(object);
    let units = read.units(object)?;
    let mut plan = Plan::default();
    let mut kept = Kept::Group(group);
    match &change.edit {
        ObjectEdit::Transform { matrix } => {
            if matrix.iter().any(|v| !v.is_finite()) {
                return Err(Error::InvalidEdit("an object is moved by finite amounts"));
            }
            let t = page_map(view, matrix_of(*matrix))?;
            inverse(&t)?;
            for unit in &units {
                plan.splices.extend(read.wrap(unit, group, &t, None)?);
            }
        }
        ObjectEdit::Delete => {
            kept = Kept::Gone;
            for unit in &units {
                plan.splices.push(read.delete(unit));
            }
        }
        ObjectEdit::Crop { rect } => {
            if object.kind == ObjectKind::Text {
                return Err(Error::InvalidEdit("text is not cropped"));
            }
            let crop = rect.as_ref().map(|r| (view, r));
            for unit in &units {
                plan.splices.extend(read.wrap(unit, group, &Matrix::IDENTITY, Some(crop))?);
            }
        }
        ObjectEdit::ReplaceImage { data } => {
            if object.kind != ObjectKind::Image {
                return Err(Error::InvalidEdit("only an image is replaced by an image"));
            }
            let piece = pieces[0];
            let (width, height) = image_size(data, &IMAGE_LIMITS)?;
            let name = free_image_name(&read, &plan);
            plan.images.push(image_to_embed(data, &name, &IMAGE_LIMITS)?);
            // Fitted in the old image's box, in proportion, centred.
            let m = piece.ctm.0;
            let (lu, lv) = ((m[0] * m[0] + m[1] * m[1]).sqrt(), (m[2] * m[2] + m[3] * m[3]).sqrt());
            if lu < 1e-9 || lv < 1e-9 {
                return Err(Error::EditUnsupported("the object is drawn at no size"));
            }
            let (want, have) = (f64::from(width) / f64::from(height), lu / lv);
            let (sx, sy) = if want >= have { (1.0, have / want) } else { (want / have, 1.0) };
            let fit = Matrix([sx, 0.0, 0.0, sy, (1.0 - sx) / 2.0, (1.0 - sy) / 2.0]);
            match piece.wrapper {
                Some(w) => {
                    let wrapper = &read.traced.wrappers[w];
                    let inner = wrapper.matrix.then(wrapper.outer.ctm);
                    let x = fit.then(piece.ctm).then(inverse(&inner)?);
                    plan.splices.push(Splice {
                        stream: wrapper.stream,
                        range: wrapper.inner.clone(),
                        with: format!("q {} cm /{name} Do Q\n", fmt_matrix(&x)).into_bytes(),
                    });
                }
                None => {
                    kept = Kept::Index(change.object.index);
                    plan.splices.push(Splice {
                        stream: piece.stream,
                        range: piece.range.clone(),
                        with: format!("q {} cm /{name} Do Q", fmt_matrix(&fit)).into_bytes(),
                    });
                }
            }
        }
        ObjectEdit::Arrange(to) => {
            let index = change.object.index;
            let mut drawn = Vec::new();
            for unit in &units {
                let (snap, bytes, removal) = read.movable(unit, group);
                plan.splices.push(removal);
                drawn.push((snap, bytes));
            }
            let draw = |at: &Snap| -> Result<Vec<u8>> {
                let mut out = Vec::new();
                for (snap, bytes) in &drawn {
                    out.extend_from_slice(read.transition(at, snap)?.as_bytes());
                    out.extend_from_slice(bytes);
                    out.extend_from_slice(b"\nQ\n");
                }
                Ok(out)
            };
            let overlaps = |other: &Obj| intersect(other.bounds, object.bounds).is_some();
            match to {
                Arrange::Front => plan.front.push(draw(&initial())?),
                Arrange::Back => plan.back.push(draw(&initial())?),
                Arrange::Forward => {
                    let above = read.objects[index + 1..]
                        .iter()
                        .find(|o| overlaps(o))
                        .ok_or(Error::InvalidEdit("nothing it overlaps is above it"))?;
                    let last = &read.traced.pieces[*above.pieces.last().unwrap_or(&0)];
                    let ((stream, offset), snap) = &last.end;
                    let mut with = b"\n".to_vec();
                    with.extend_from_slice(&draw(snap)?);
                    plan.splices.push(Splice { stream: *stream, range: *offset..*offset, with });
                }
                Arrange::Backward => {
                    let below = read.objects[..index]
                        .iter()
                        .rev()
                        .find(|o| overlaps(o))
                        .ok_or(Error::InvalidEdit("nothing it overlaps is below it"))?;
                    let first = &read.traced.pieces[below.pieces[0]];
                    let ((stream, offset), snap) = &first.start;
                    let mut with = draw(snap)?;
                    with.push(b'\n');
                    plan.splices.push(Splice { stream: *stream, range: *offset..*offset, with });
                }
            }
        }
    }
    let update = apply(&read, bytes, plan)?;
    Ok((update, kept))
}

fn initial() -> Snap {
    Snap { ctm: Matrix::IDENTITY, clips: Vec::new(), log: Vec::new() }
}

/// A resource name for a new image no XObject of the page uses (`MI0`, `MI1`, …).
fn free_image_name(read: &PageRead<'_>, plan: &Plan) -> String {
    let taken: HashSet<Vec<u8>> = read
        .resources
        .get(b"XObject")
        .and_then(|x| read.file.resolve(x))
        .and_then(|x| x.as_dict().map(|d| d.0.iter().map(|(k, _)| k.clone()).collect()))
        .unwrap_or_default();
    (0..)
        .map(|n: u32| format!("MI{n}"))
        .find(|name| !taken.contains(name.as_bytes()) && !plan.images.iter().any(|i| i.resource == *name))
        .unwrap_or_else(|| "MI".into())
}

/// Whether a resource name is one this app gave a placed image.
fn is_our_image(name: &[u8]) -> bool {
    name.strip_prefix(b"MI").is_some_and(|rest| !rest.is_empty() && rest.iter().all(u8::is_ascii_digit))
}

/// Writes the plan as an update: edited streams as new objects, new
/// streams in front or behind, new images, the page.
fn apply(read: &PageRead<'_>, bytes: &[u8], plan: Plan) -> Result<Vec<u8>> {
    let Start { section, form, mut next_number, .. } = Start::read(bytes)?;
    let file = &read.file;
    let leaf = &read.tree.leaves[read.leaf];
    let crypt = read.crypt.as_ref();
    let mut update = Update::new(bytes);

    // ---- The splices, stream by stream.
    let mut changed: HashMap<usize, Vec<u8>> = HashMap::new();
    let mut touched: Vec<usize> = plan.splices.iter().map(|s| s.stream).collect();
    touched.sort_unstable();
    touched.dedup();
    for stream in touched {
        let mut mine: Vec<&Splice> = plan.splices.iter().filter(|s| s.stream == stream).collect();
        mine.sort_by_key(|s| (s.range.start, s.range.end));
        if mine.windows(2).any(|w| w[0].range.end > w[1].range.start) {
            return Err(Error::EditUnsupported("the object's operators overlap"));
        }
        let data = &read.content.data[stream];
        let mut out = Vec::with_capacity(data.len() + 64);
        let mut at = 0;
        for splice in mine {
            out.extend_from_slice(&data[at..splice.range.start]);
            out.extend_from_slice(&splice.with);
            at = splice.range.end;
        }
        out.extend_from_slice(&data[at..]);
        changed.insert(stream, out);
    }

    let mut take = || -> Result<u32> {
        let number = next_number;
        next_number = number.checked_add(1).ok_or(Error::EditUnsupported("it has too many objects"))?;
        Ok(number)
    };
    let write_stream = |update: &mut Update, number: u32, dict: Dict, data: &[u8]| {
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(data, 6);
        let mut dict = dict;
        dict.0.push((b"Filter".to_vec(), Object::Name(b"FlateDecode".to_vec())));
        let data = match crypt {
            Some(crypt) => crypt.encrypt_stream(number, 0, &compressed),
            None => compressed,
        };
        update.stream(number, 0, dict, &data);
    };

    // ---- The page's streams: edited ones replaced, emptied ones dropped.
    let mut contents: Vec<Object> = Vec::new();
    let mut all_data: Vec<Vec<u8>> = Vec::new();
    for (i, item) in read.content.items.iter().enumerate() {
        match changed.remove(&i) {
            Some(data) => {
                if data.iter().all(|b| crate::pdf::is_white(*b)) && !read.content.wrap_streams[i] {
                    continue;
                }
                let number = take()?;
                write_stream(&mut update, number, Dict::default(), &data);
                contents.push(Object::Ref(number, 0));
                all_data.push(data);
            }
            None => {
                contents.push(item.clone());
                all_data.push(read.content.data[i].clone());
            }
        }
    }
    if !plan.front.is_empty() {
        let wrapped = read.content.wrap_streams.first().copied().unwrap_or(false);
        if !wrapped && !contents.is_empty() {
            let marker = || Dict(vec![(WRAP_KEY.to_vec(), Object::Bool(true))]);
            let (open, close) = (take()?, take()?);
            write_stream(&mut update, open, marker(), b"q\n");
            write_stream(&mut update, close, marker(), b"\nQ\n");
            contents.insert(0, Object::Ref(open, 0));
            contents.push(Object::Ref(close, 0));
        }
        for data in &plan.front {
            let number = take()?;
            write_stream(&mut update, number, Dict::default(), data);
            contents.push(Object::Ref(number, 0));
            all_data.push(data.clone());
        }
    }
    for data in plan.back.iter().rev() {
        let number = take()?;
        write_stream(&mut update, number, Dict::default(), data);
        contents.insert(0, Object::Ref(number, 0));
        all_data.push(data.clone());
    }

    // ---- Resources: new images in; images of ours no stream draws out.
    let mut page = leaf.dict.clone();
    let our_images_in_use = read
        .resources
        .get(b"XObject")
        .and_then(|x| file.resolve(x))
        .and_then(|x| x.as_dict().cloned())
        .is_some_and(|d| d.0.iter().any(|(k, _)| is_our_image(k)));
    if !plan.images.is_empty() || our_images_in_use {
        let mut resources = read.resources.clone();
        let mut xobjects = resources
            .get(b"XObject")
            .and_then(|x| file.resolve(x))
            .and_then(|x| x.as_dict().cloned())
            .unwrap_or_default();
        for image in &plan.images {
            let number = write_image(&mut update, crypt, &mut next_number, image)?;
            set(&mut xobjects, image.resource.as_bytes(), Object::Ref(number, 0));
        }
        let mut drawn: HashSet<Vec<u8>> = HashSet::new();
        for data in &all_data {
            for op in tokenize(data).ok_or_else(unreadable)?.iter().filter(|op| op.name == b"Do") {
                if let Some(Operand::Name(name)) = op.operands.first() {
                    drawn.insert(name.clone());
                }
            }
        }
        let before = xobjects.0.len();
        xobjects.0.retain(|(name, _)| !is_our_image(name) || drawn.contains(name));
        if !plan.images.is_empty() || xobjects.0.len() != before {
            set(&mut resources, b"XObject", Object::Dict(xobjects));
            if crypt.is_some() && contains_string(&Object::Dict(resources.clone())) {
                return Err(Error::EditUnsupported(
                    "this page's resources in an encrypted file cannot be copied yet",
                ));
            }
            set(&mut page, b"Resources", Object::Dict(resources));
        }
    }

    set(&mut page, b"Contents", Object::Array(contents));
    update.object(leaf.number, leaf.generation, &Object::Dict(page));
    Ok(update.finish(form, file.trailer.clone(), section, next_number))
}

/// The bytes to append to `bytes` to place a new image, and its group number.
pub fn insert_image_update(
    bytes: &[u8],
    password: Option<&str>,
    view: &ViewTransform,
    page_area: f64,
    image: &NewImage,
) -> Result<(Vec<u8>, u32)> {
    let read = read_page(bytes, password, image.page, page_area)?;
    let rect = image.rect;
    if ![rect.x, rect.y, rect.width, rect.height].iter().all(|v| v.is_finite())
        || rect.width < 1.0
        || rect.height < 1.0
    {
        return Err(Error::InvalidEdit("an image needs a place on the page"));
    }
    let mut plan = Plan::default();
    let name = free_image_name(&read, &plan);
    plan.images.push(image_to_embed(&image.data, &name, &IMAGE_LIMITS)?);
    // The unit square, y up, onto the rectangle as displayed.
    let corner = |x: f32, y: f32| {
        let (px, py) = view.page_point(x, y);
        (f64::from(px), f64::from(py))
    };
    let origin = corner(rect.x, rect.y + rect.height);
    let right = corner(rect.x + rect.width, rect.y + rect.height);
    let up = corner(rect.x, rect.y);
    let m = Matrix([
        right.0 - origin.0,
        right.1 - origin.1,
        up.0 - origin.0,
        up.1 - origin.1,
        origin.0,
        origin.1,
    ]);
    let group = read.traced.max_group + 1;
    let content = format!("{}/{name} Do{WRAPPER_CLOSE}", wrapper_open(group, &m));
    plan.front.push(content.into_bytes());
    Ok((apply(&read, bytes, plan)?, group))
}

/// The index of the object whose wrappers have group `group`.
pub fn object_with_group(
    bytes: &[u8],
    password: Option<&str>,
    page: usize,
    page_area: f64,
    group: u32,
) -> Result<Option<usize>> {
    let read = read_page(bytes, password, page, page_area)?;
    Ok(read.objects.iter().position(|o| {
        o.pieces.iter().any(|&i| {
            own_wrapper(&read.traced, &read.traced.pieces[i])
                .is_some_and(|w| read.traced.wrappers[w].group == group)
        })
    }))
}

/// Which of the page's content streams are this app's OCR layer or text
/// converted from one (ADR 0035, ADR 0039).
fn our_streams(read: &PageRead<'_>) -> Vec<bool> {
    read.content
        .items
        .iter()
        .map(|item| {
            read.file.resolve(item).and_then(|o| o.as_dict().cloned()).is_some_and(|d| {
                d.get(crate::ocr::LAYER_KEY).is_some() || d.get(crate::ocr::CONVERTED_KEY).is_some()
            })
        })
        .collect()
}

/// What text a page draws, for checking another tool's text layer (ADR 0038).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TextCensus {
    /// Text-showing operators drawn invisibly (render modes 3 and 7), not by us.
    pub invisible: usize,
    /// Text-showing operators drawn visibly, not by us.
    pub visible: usize,
    /// The page holds a text layer of ours, or text converted from one.
    pub ours: bool,
    /// The largest image's share of the page's area (0–1).
    pub image_share: f64,
}

/// Counts what text page `page` draws, and how much of it an image covers.
pub fn text_census(bytes: &[u8], password: Option<&str>, page: usize, page_area: f64) -> Result<TextCensus> {
    let read = read_page(bytes, password, page, page_area)?;
    let ours = our_streams(&read);
    let mut census = TextCensus { ours: ours.iter().any(|&o| o), ..TextCensus::default() };
    for piece in &read.traced.pieces {
        match piece.kind {
            PieceKind::Text if ours.get(piece.stream).copied().unwrap_or(false) => {}
            PieceKind::Text if piece.text.as_ref().is_some_and(|t| t.invisible) => census.invisible += 1,
            PieceKind::Text => census.visible += 1,
            PieceKind::Image | PieceKind::InlineImage => {
                if let Some(b) = piece.bounds {
                    census.image_share = census.image_share.max(area(&b) / page_area.max(1.0));
                }
            }
            _ => {}
        }
    }
    Ok(census)
}

/// The bytes to append to `bytes` to remove another tool's invisible text
/// (render modes 3 and 7: an OCR layer) from page `page` by byte range: all of
/// it, or the operators whose glyphs' centre lies in `within` (page space,
/// `[x0, y0, x1, y1]`). Each run leaves its advance behind, so text drawn
/// after it in the same text object stays where it was (ADR 0038). This
/// app's own layers are left alone: the layer writer replaces them whole.
/// `None` when there is nothing to remove.
pub fn invisible_text_update(
    bytes: &[u8],
    password: Option<&str>,
    page: usize,
    page_area: f64,
    within: Option<[f64; 4]>,
) -> Result<Option<Vec<u8>>> {
    let read = read_page(bytes, password, page, page_area)?;
    let ours = our_streams(&read);
    let pieces: Vec<usize> = read
        .traced
        .pieces
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            p.kind == PieceKind::Text
                && p.text.as_ref().is_some_and(|t| t.invisible)
                && !ours.get(p.stream).copied().unwrap_or(false)
                && within.is_none_or(|w| {
                    p.bounds.is_some_and(|b| {
                        let (x, y) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
                        x >= w[0] && x <= w[2] && y >= w[1] && y <= w[3]
                    })
                })
        })
        .map(|(i, _)| i)
        .collect();
    if pieces.is_empty() {
        return Ok(None);
    }
    let object = Obj { kind: ObjectKind::Text, pieces, bounds: [0.0; 4] };
    let mut plan = Plan::default();
    for unit in &read.units(&object)? {
        plan.splices.push(read.delete(unit));
    }
    Ok(Some(apply(&read, bytes, plan)?))
}

#[cfg(test)]
mod tests;
