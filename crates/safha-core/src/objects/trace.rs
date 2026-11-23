//! Tracing a page's content to its objects (ADR 0051).
//!
//! One walk over the page's operators, keeping the graphics state, finds
//! every *piece* that draws something: an image (`Do` of an image, or an
//! inline image), a form XObject, a painted path, a shading, a text-showing
//! operator. Each piece keeps its byte range, the matrix it was drawn under,
//! where it lands on the page, and a *recipe* of the state it was drawn in
//! (the operators that set colours, line style, text state, `gs`, and the
//! clip, in page space), so it can be drawn again elsewhere in the stream
//! exactly as it was. Text pieces are grouped into blocks afterwards.
//!
//! Pieces this app moved, resized, cropped or reordered are wrapped in
//! `/MirsamObj <</G n>> BDC q a b c d e f cm [crop W n] … Q EMC`; the walk
//! recognises the wrapper, so a second edit changes its numbers instead of
//! wrapping again.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use crate::pdf::content::{Matrix, Op, Operand, ShowKind};
use crate::pdf::fontinfo::{FontInfo, FontKind};
use crate::pdf::{Dict, File, Object};

/// The marked-content tag of the wrapper around a piece this app edited.
pub(crate) const WRAPPER_TAG: &[u8] = b"MirsamObj";
/// The most pieces read from one page.
const MAX_PIECES: usize = 50_000;
/// The most `gs` operators a recipe keeps.
const MAX_GS: usize = 64;
/// The most nested `q` the walk follows.
const MAX_DEPTH: usize = 256;

/// What a state operator sets, so a later one of the same kind replaces it
/// in the recipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Param {
    LineWidth,
    Cap,
    Join,
    Miter,
    Dash,
    Intent,
    Flatness,
    /// `gs`: never replaced (each sets some parameters only).
    ExtGState,
    StrokeSpace,
    StrokeColor,
    FillSpace,
    FillColor,
    Font,
    CharSpacing,
    WordSpacing,
    Scaling,
    Leading,
    Rise,
    Render,
}

impl Param {
    /// The operator that puts a parameter back to its initial value.
    pub(crate) fn default_op(self) -> Option<&'static [u8]> {
        Some(match self {
            Param::LineWidth => b"1 w",
            Param::Cap => b"0 J",
            Param::Join => b"0 j",
            Param::Miter => b"10 M",
            Param::Dash => b"[] 0 d",
            Param::Intent => b"/RelativeColorimetric ri",
            Param::Flatness => b"1 i",
            Param::StrokeSpace | Param::StrokeColor => b"0 G",
            Param::FillSpace | Param::FillColor => b"0 g",
            Param::CharSpacing => b"0 Tc",
            Param::WordSpacing => b"0 Tw",
            Param::Scaling => b"100 Tz",
            Param::Leading => b"0 TL",
            Param::Rise => b"0 Ts",
            Param::Render => b"0 Tr",
            Param::ExtGState | Param::Font => return None,
        })
    }
}

/// One operator of a recipe, as written in the stream.
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub(crate) param: Param,
    pub(crate) op: Vec<u8>,
    /// The matrix in force when it ran (a soft mask set by `gs` is placed by it).
    pub(crate) ctm: Matrix,
    /// Unique per `gs` operator on the page (0 for others).
    pub(crate) id: u32,
}

/// The state at one point of the page's content: what drawing something
/// there again takes.
#[derive(Debug, Clone)]
pub(crate) struct Snap {
    pub(crate) ctm: Matrix,
    /// Clips in force, oldest first (indices into `Traced::clips`).
    pub(crate) clips: Vec<usize>,
    pub(crate) log: Vec<Entry>,
}

impl Snap {
    fn initial() -> Self {
        Snap { ctm: Matrix::IDENTITY, clips: Vec::new(), log: Vec::new() }
    }

    /// The `gs` operators in force, in order.
    pub(crate) fn gs_ids(&self) -> Vec<u32> {
        self.log.iter().filter(|e| e.param == Param::ExtGState).map(|e| e.id).collect()
    }
}

/// A clip, in page space.
#[derive(Debug, Clone)]
pub(crate) struct Clip {
    /// The path as operators in page space; `None` for a clip that cannot be
    /// drawn again (text used as a clip).
    pub(crate) path: Option<Vec<u8>>,
    pub(crate) even_odd: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PieceKind {
    Image,
    InlineImage,
    Form,
    Path,
    Shading,
    Text,
}

/// A text-showing operator, and what splitting it out of its text object takes.
#[derive(Debug, Clone)]
pub(crate) struct TextPiece {
    /// The line matrix after the operator.
    pub(crate) tlm_after: Matrix,
    /// From the line's start to where the next show starts, in text space.
    pub(crate) line_advance: f64,
    pub(crate) size: f64,
    pub(crate) scaling: f64,
    /// The font's size on the page, in points.
    pub(crate) em: f64,
    /// Its baseline runs along the page's x axis (else along y).
    pub(crate) horizontal: bool,
    /// Drawn invisibly (render mode 3, OCR layers): not offered as an object.
    pub(crate) invisible: bool,
}

/// An image's own size, for the inspector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImageMeta {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/// One thing the page draws.
#[derive(Debug, Clone)]
pub(crate) struct Piece {
    pub(crate) kind: PieceKind,
    pub(crate) stream: usize,
    /// The drawing operator(s): a path from its first construction operator
    /// to its painting operator.
    pub(crate) range: Range<usize>,
    /// The matrix the piece was drawn under.
    pub(crate) ctm: Matrix,
    /// Where it lands, in page space, within the clip.
    pub(crate) bounds: Option<[f64; 4]>,
    /// The state at the piece (inside its wrapper, if it has one).
    pub(crate) snap: Snap,
    /// Where something drawn just before it goes (its text object's `BT`,
    /// its wrapper's start), and the state there.
    pub(crate) start: ((usize, usize), Snap),
    /// Where something drawn just after it goes, and the state there.
    pub(crate) end: ((usize, usize), Snap),
    /// A path that also clips (`W` before its painting operator): the bytes
    /// of its construction, which must stay to keep the clip.
    pub(crate) clip_ops: Option<Range<usize>>,
    pub(crate) text: Option<TextPiece>,
    pub(crate) wrapper: Option<usize>,
    pub(crate) image: Option<ImageMeta>,
}

/// A wrapper this app wrote around a piece.
#[derive(Debug, Clone)]
pub(crate) struct Wrapper {
    pub(crate) stream: usize,
    /// From its `BDC` to its `EMC`.
    pub(crate) range: Range<usize>,
    /// Its `cm` operator, and the matrix.
    pub(crate) cm: Range<usize>,
    pub(crate) matrix: Matrix,
    /// Its crop (`… W n`), when it has one.
    pub(crate) crop: Option<Range<usize>>,
    /// What it wraps: after the `cm` (and crop) up to its closing `Q`.
    pub(crate) inner: Range<usize>,
    pub(crate) group: u32,
    /// The state before its `BDC`.
    pub(crate) outer: Snap,
    pub(crate) pieces: usize,
}

/// A page's content, read.
pub(crate) struct Content {
    /// `/Contents` entries.
    pub(crate) items: Vec<Object>,
    /// Each stream's data, decoded and decrypted.
    pub(crate) data: Vec<Vec<u8>>,
    pub(crate) ops: Vec<Vec<Op>>,
    /// Which streams are the `q`/`Q` this app wraps the page's content in.
    pub(crate) wrap_streams: Vec<bool>,
}

/// What the walk found.
pub(crate) struct Traced {
    pub(crate) pieces: Vec<Piece>,
    pub(crate) wrappers: Vec<Wrapper>,
    pub(crate) clips: Vec<Clip>,
    /// The largest wrapper group number in use.
    pub(crate) max_group: u32,
}

/// How a font measures its codes: the font reader's widths, or a Type 3
/// font's own (`/Widths` through `/FontMatrix`).
struct Metrics {
    info: Option<Rc<FontInfo>>,
    type3: Option<(u32, Vec<f64>)>,
    /// Above and below the baseline, in em.
    ascent: f64,
    descent: f64,
}

impl Metrics {
    fn read(file: &File<'_>, dict: &Dict) -> Metrics {
        let resolve = |key: &[u8]| dict.get(key).and_then(|v| file.resolve(v));
        let number = |o: &Object| match o {
            Object::Int(i) => Some(*i as f64),
            Object::Real(r) => Some(*r),
            _ => None,
        };
        let info = FontInfo::read(file, dict).map(Rc::new);
        let subtype = resolve(b"Subtype");
        let mut ascent = 0.8;
        let mut descent = -0.2;
        let mut type3 = None;
        if subtype.as_ref().and_then(Object::as_name) == Some(b"Type3") {
            let scale = resolve(b"FontMatrix")
                .and_then(|m| m.as_array().and_then(|a| a.first().and_then(number)))
                .filter(|s| s.is_finite() && *s != 0.0)
                .unwrap_or(0.001);
            let first = resolve(b"FirstChar").and_then(|v| v.as_int()).unwrap_or(0).max(0) as u32;
            let widths = resolve(b"Widths")
                .and_then(|w| w.as_array().map(<[Object]>::to_vec))
                .unwrap_or_default()
                .iter()
                .map(|w| file.resolve(w).as_ref().and_then(number).unwrap_or(0.0) * scale * 1000.0)
                .collect();
            if let Some(bbox) = resolve(b"FontBBox").and_then(|b| b.as_array().map(<[Object]>::to_vec))
                && bbox.len() == 4
            {
                let y0 = file.resolve(&bbox[1]).as_ref().and_then(number).unwrap_or(0.0) * scale;
                let y1 = file.resolve(&bbox[3]).as_ref().and_then(number).unwrap_or(0.0) * scale;
                if y1 > y0 && y1 - y0 < 4.0 {
                    ascent = y1.max(0.1);
                    descent = y0.min(0.0);
                }
            }
            type3 = Some((first, widths));
        } else {
            let descriptor = resolve(b"FontDescriptor")
                .or_else(|| {
                    let descendant = resolve(b"DescendantFonts")?.as_array()?.first().cloned()?;
                    file.resolve(&descendant)?.as_dict()?.get(b"FontDescriptor").and_then(|d| file.resolve(d))
                })
                .and_then(|d| d.as_dict().cloned());
            if let Some(d) = descriptor {
                let get = |k: &[u8]| d.get(k).and_then(|v| file.resolve(v)).as_ref().and_then(number);
                if let (Some(a), Some(b)) = (get(b"Ascent"), get(b"Descent"))
                    && a > 0.0
                    && a < 4000.0
                    && b <= 0.0
                    && b > -4000.0
                {
                    ascent = a / 1000.0;
                    descent = b / 1000.0;
                }
            }
        }
        Metrics { info, type3, ascent, descent }
    }

    /// Each code in `bytes` with its byte length and advance (thousandths of an em).
    fn codes(&self, bytes: &[u8]) -> Vec<(u32, usize, f64)> {
        match (&self.type3, &self.info) {
            (Some((first, widths)), _) => bytes
                .iter()
                .map(|&b| {
                    let code = u32::from(b);
                    let w = code.checked_sub(*first).and_then(|i| widths.get(i as usize)).copied();
                    (code, 1, w.unwrap_or(0.0))
                })
                .collect(),
            (None, Some(info)) => {
                info.codes(bytes).map(|(code, len)| (code, len, info.width(code))).collect()
            }
            (None, None) => bytes.iter().map(|&b| (u32::from(b), 1, 500.0)).collect(),
        }
    }

    fn two_byte(&self) -> bool {
        self.type3.is_none() && self.info.as_ref().is_some_and(|i| i.kind == FontKind::Identity)
    }
}

#[derive(Clone)]
struct State {
    snap: Snap,
    line_width: f64,
    /// The clip's bounds in page space.
    clip_bounds: Option<[f64; 4]>,
    font: Vec<u8>,
    size: f64,
    tc: f64,
    tw: f64,
    tz: f64,
    tl: f64,
    rise: f64,
    render: i64,
}

impl State {
    fn initial() -> Self {
        State {
            snap: Snap::initial(),
            line_width: 1.0,
            clip_bounds: None,
            font: Vec::new(),
            size: 0.0,
            tc: 0.0,
            tw: 0.0,
            tz: 1.0,
            tl: 0.0,
            rise: 0.0,
            render: 0,
        }
    }

    fn record(&mut self, param: Param, op: Vec<u8>, id: u32) {
        let log = &mut self.snap.log;
        match param {
            Param::ExtGState => {
                if log.iter().filter(|e| e.param == Param::ExtGState).count() >= MAX_GS
                    && let Some(at) = log.iter().position(|e| e.param == Param::ExtGState)
                {
                    log.remove(at);
                }
            }
            // A device colour operator sets the space too.
            Param::StrokeColor if !op.ends_with(b"SC") && !op.ends_with(b"SCN") => {
                log.retain(|e| e.param != Param::StrokeSpace && e.param != Param::StrokeColor);
            }
            Param::FillColor if !op.ends_with(b"sc") && !op.ends_with(b"scn") => {
                log.retain(|e| e.param != Param::FillSpace && e.param != Param::FillColor);
            }
            Param::StrokeSpace => {
                log.retain(|e| e.param != Param::StrokeSpace && e.param != Param::StrokeColor)
            }
            Param::FillSpace => log.retain(|e| e.param != Param::FillSpace && e.param != Param::FillColor),
            _ => log.retain(|e| e.param != param),
        }
        log.push(Entry { param, op, ctm: self.snap.ctm, id });
    }
}

fn bounds_of(points: impl IntoIterator<Item = (f64, f64)>) -> Option<[f64; 4]> {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    let mut any = false;
    for (x, y) in points {
        if !x.is_finite() || !y.is_finite() {
            continue;
        }
        any = true;
        b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
    }
    any.then_some(b)
}

pub(crate) fn intersect(a: [f64; 4], b: [f64; 4]) -> Option<[f64; 4]> {
    let r = [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])];
    (r[0] <= r[2] && r[1] <= r[3]).then_some(r)
}

/// The corners of the unit square through `m`.
pub(crate) fn unit_square(m: &Matrix) -> [(f64, f64); 4] {
    [m.apply(0.0, 0.0), m.apply(1.0, 0.0), m.apply(1.0, 1.0), m.apply(0.0, 1.0)]
}

/// A number for a content stream: at most six decimals, no exponent.
pub(crate) fn fmt(value: f64) -> String {
    if !value.is_finite() {
        return "0".into();
    }
    let text = format!("{value:.6}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-0" || text == "-" { "0".into() } else { text.into() }
}

pub(crate) fn fmt_matrix(m: &Matrix) -> String {
    m.0.iter().map(|v| fmt(*v)).collect::<Vec<_>>().join(" ")
}

/// A wrapper's shape in one stream's operators, if `ops[at]` starts one.
struct Shape {
    cm: usize,
    crop: Option<(usize, usize)>,
    inner_start: usize,
    close_q: usize,
    emc: usize,
    group: u32,
    matrix: Matrix,
}

fn wrapper_shape(ops: &[Op], at: usize) -> Option<Shape> {
    let op = ops.get(at)?;
    let group = match (op.name.as_slice(), op.operands.as_slice()) {
        (b"BMC", [Operand::Name(tag)]) if tag == WRAPPER_TAG => 0,
        (b"BDC", [Operand::Name(tag), properties]) if tag == WRAPPER_TAG => match properties {
            Operand::Dict(entries) => entries
                .iter()
                .find(|(k, _)| k == b"G")
                .and_then(|(_, v)| match v {
                    Operand::Num(n) if *n >= 0.0 && *n < f64::from(u32::MAX) => Some(*n as u32),
                    _ => None,
                })
                .unwrap_or(0),
            _ => 0,
        },
        _ => return None,
    };
    if ops.get(at + 1)?.name != b"q" {
        return None;
    }
    let cm = ops.get(at + 2)?;
    let nums: Vec<f64> = cm
        .operands
        .iter()
        .filter_map(|o| match o {
            Operand::Num(n) => Some(*n),
            _ => None,
        })
        .collect();
    if cm.name != b"cm" || nums.len() != 6 {
        return None;
    }
    let matrix = Matrix([nums[0], nums[1], nums[2], nums[3], nums[4], nums[5]]);
    let names: Vec<&[u8]> =
        ops.get(at + 3..at + 10).map(|s| s.iter().map(|o| o.name.as_slice()).collect()).unwrap_or_default();
    let crop = (names == [&b"m"[..], b"l", b"l", b"l", b"h", b"W", b"n"]).then_some((at + 3, at + 9));
    let inner_start = crop.map_or(at + 3, |(_, n)| n + 1);
    let mut depth = 1usize;
    let mut i = inner_start;
    while let Some(op) = ops.get(i) {
        match op.name.as_slice() {
            b"q" => depth += 1,
            b"Q" => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    if depth != 0 || ops.get(i + 1)?.name != b"EMC" {
        return None;
    }
    Some(Shape { cm: at + 2, crop, inner_start, close_q: i, emc: i + 1, group, matrix })
}

/// An XObject on the page, by resource name.
enum XObject {
    Image(ImageMeta),
    Form { bbox: [f64; 4], matrix: Matrix },
    Other,
}

fn xobject(file: &File<'_>, resources: &Dict, name: &[u8]) -> XObject {
    let dict = resources
        .get(b"XObject")
        .and_then(|x| file.resolve(x))
        .and_then(|x| x.as_dict().and_then(|x| x.get(name).cloned()))
        .and_then(|r| match r {
            Object::Ref(number, _) => match file.entries.get(&number) {
                Some(crate::pdf::Entry::Offset(offset)) => file.indirect_at(*offset).map(|(d, _)| d),
                _ => None,
            },
            _ => None,
        });
    let Some(dict) = dict else { return XObject::Other };
    let number = |o: Option<Object>| match o {
        Some(Object::Int(i)) => Some(i as f64),
        Some(Object::Real(r)) => Some(r),
        _ => None,
    };
    let get = |k: &[u8]| dict.get(k).and_then(|v| file.resolve(v));
    match get(b"Subtype").as_ref().and_then(Object::as_name) {
        Some(b"Image") => {
            let w = number(get(b"Width")).unwrap_or(0.0).clamp(0.0, 1e9) as u32;
            let h = number(get(b"Height")).unwrap_or(0.0).clamp(0.0, 1e9) as u32;
            XObject::Image(ImageMeta { width: w, height: h })
        }
        Some(b"Form") => {
            let array = |k: &[u8], n: usize| -> Option<Vec<f64>> {
                let items = get(k)?.as_array()?.to_vec();
                let values: Vec<f64> = items.iter().filter_map(|i| number(file.resolve(i))).collect();
                (values.len() == n).then_some(values)
            };
            let bbox = array(b"BBox", 4).map_or([0.0, 0.0, 0.0, 0.0], |b| {
                [b[0].min(b[2]), b[1].min(b[3]), b[0].max(b[2]), b[1].max(b[3])]
            });
            let matrix = array(b"Matrix", 6)
                .map_or(Matrix::IDENTITY, |m| Matrix([m[0], m[1], m[2], m[3], m[4], m[5]]));
            XObject::Form { bbox, matrix }
        }
        _ => XObject::Other,
    }
}

/// An inline image's size, from its dictionary (`/W`, `/H`).
fn inline_size(data: &[u8]) -> Option<ImageMeta> {
    let end = crate::pdf::find(data, b"ID")?;
    let head = String::from_utf8_lossy(&data[..end]);
    let mut words = head.split(|c: char| c.is_whitespace() || c == '/').filter(|w| !w.is_empty());
    let (mut w, mut h) = (None, None);
    while let Some(word) = words.next() {
        match word {
            "W" | "Width" => w = words.next().and_then(|v| v.parse::<u32>().ok()),
            "H" | "Height" => h = words.next().and_then(|v| v.parse::<u32>().ok()),
            _ => {}
        }
    }
    Some(ImageMeta { width: w?, height: h? })
}

fn nums_of(op: &Op) -> Vec<f64> {
    op.operands
        .iter()
        .filter_map(|o| match o {
            Operand::Num(n) => Some(*n),
            _ => None,
        })
        .collect()
}

/// Construction operators of the current path, kept to know where it lands
/// and to write it again in page space.
#[derive(Default)]
struct PathBuild {
    start: Option<(usize, usize)>,
    /// The first construction operator's index (in its stream's ops).
    points: Vec<(f64, f64)>,
    page_ops: Vec<u8>,
    clip: Option<bool>,
    /// Ends of the construction operators (for `clip_ops`).
    construction_end: usize,
    spans_streams: bool,
}

impl PathBuild {
    fn push(&mut self, ctm: &Matrix, op: &Op, stream: usize) {
        let n = nums_of(op);
        let pt = |x: f64, y: f64| ctm.apply(x, y);
        let emit = |points: &[(f64, f64)], name: &str, out: &mut Vec<u8>, all: &mut Vec<(f64, f64)>| {
            for (x, y) in points {
                out.extend_from_slice(format!("{} {} ", fmt(*x), fmt(*y)).as_bytes());
                all.push((*x, *y));
            }
            out.extend_from_slice(name.as_bytes());
            out.push(b'\n');
        };
        match (op.name.as_slice(), n.len()) {
            (b"m", 2) => emit(&[pt(n[0], n[1])], "m", &mut self.page_ops, &mut self.points),
            (b"l", 2) => emit(&[pt(n[0], n[1])], "l", &mut self.page_ops, &mut self.points),
            (b"c", 6) => emit(
                &[pt(n[0], n[1]), pt(n[2], n[3]), pt(n[4], n[5])],
                "c",
                &mut self.page_ops,
                &mut self.points,
            ),
            (b"v" | b"y", 4) => emit(
                &[pt(n[0], n[1]), pt(n[2], n[3])],
                if op.name == b"v" { "v" } else { "y" },
                &mut self.page_ops,
                &mut self.points,
            ),
            (b"re", 4) => {
                let (x, y, w, h) = (n[0], n[1], n[2], n[3]);
                emit(&[pt(x, y)], "m", &mut self.page_ops, &mut self.points);
                emit(&[pt(x + w, y)], "l", &mut self.page_ops, &mut self.points);
                emit(&[pt(x + w, y + h)], "l", &mut self.page_ops, &mut self.points);
                emit(&[pt(x, y + h)], "l", &mut self.page_ops, &mut self.points);
                self.page_ops.extend_from_slice(b"h\n");
            }
            (b"h", _) => self.page_ops.extend_from_slice(b"h\n"),
            _ => {}
        }
        match self.start {
            None => self.start = Some((stream, op.range.start)),
            Some((s, _)) if s != stream => self.spans_streams = true,
            _ => {}
        }
        self.construction_end = op.range.end;
    }
}

/// A text object being walked.
struct TextObject {
    tm: Matrix,
    tlm: Matrix,
    /// Where its `BT` is, and the state there.
    start: ((usize, usize), Snap),
    /// Pieces it holds (their `end` is set at `ET`).
    pieces: Vec<usize>,
    /// A `cm` inside it, or an operator the splitter cannot follow.
    odd: bool,
}

/// Walks the page's content. `resources` are the page's; `font` resolves a
/// font resource name.
pub(crate) fn trace(file: &File<'_>, resources: &Dict, content: &Content) -> Option<Traced> {
    let font_dict = resources
        .get(b"Font")
        .and_then(|f| file.resolve(f))
        .and_then(|f| f.as_dict().cloned())
        .unwrap_or_default();
    let mut fonts: HashMap<Vec<u8>, Rc<Metrics>> = HashMap::new();
    let mut metrics = |name: &[u8]| -> Rc<Metrics> {
        if let Some(m) = fonts.get(name) {
            return m.clone();
        }
        let m = font_dict
            .get(name)
            .and_then(|f| file.resolve(f))
            .and_then(|f| f.as_dict().cloned())
            .map_or(Metrics { info: None, type3: None, ascent: 0.8, descent: -0.2 }, |d| {
                Metrics::read(file, &d)
            });
        let m = Rc::new(m);
        fonts.insert(name.to_vec(), m.clone());
        m
    };

    let mut traced = Traced { pieces: Vec::new(), wrappers: Vec::new(), clips: Vec::new(), max_group: 0 };
    let mut state = State::initial();
    let mut stack: Vec<State> = Vec::new();
    let mut path = PathBuild::default();
    let mut text: Option<TextObject> = None;
    let mut gs_ids = 0u32;

    for (stream, ops) in content.ops.iter().enumerate() {
        let data = &content.data[stream];
        let bytes = |op: &Op| data[op.range.clone()].to_vec();
        let mut index = 0usize;
        while index < ops.len() {
            let op = &ops[index];
            // A wrapper of ours starts here.
            if matches!(op.name.as_slice(), b"BDC" | b"BMC")
                && let Some(shape) = wrapper_shape(ops, index)
            {
                let matrix = shape.matrix;
                let wrapper = Wrapper {
                    stream,
                    range: op.range.start..ops[shape.emc].range.end,
                    cm: ops[shape.cm].range.clone(),
                    matrix,
                    crop: shape.crop.map(|(a, b)| ops[a].range.start..ops[b].range.end),
                    inner: ops
                        .get(shape.inner_start)
                        .map_or(ops[shape.close_q].range.start, |o| o.range.start)
                        ..ops[shape.close_q].range.start,
                    group: shape.group,
                    outer: state.snap.clone(),
                    pieces: 0,
                };
                traced.max_group = traced.max_group.max(shape.group);
                traced.wrappers.push(wrapper);
                // The `q` and `cm` (and crop) run as usual below.
            }
            let nums = nums_of(op);
            let all_nums = nums.len() == op.operands.len();
            match op.name.as_slice() {
                b"q" => {
                    if stack.len() >= MAX_DEPTH {
                        return None;
                    }
                    stack.push(state.clone());
                }
                b"Q" => {
                    if let Some(previous) = stack.pop() {
                        state = previous;
                    }
                }
                b"cm" if nums.len() == 6 && all_nums => {
                    state.snap.ctm =
                        Matrix([nums[0], nums[1], nums[2], nums[3], nums[4], nums[5]]).then(state.snap.ctm);
                    if let Some(t) = &mut text {
                        t.odd = true;
                    }
                }
                b"w" if nums.len() == 1 => {
                    state.line_width = nums[0];
                    state.record(Param::LineWidth, bytes(op), 0);
                }
                b"J" => state.record(Param::Cap, bytes(op), 0),
                b"j" => state.record(Param::Join, bytes(op), 0),
                b"M" => state.record(Param::Miter, bytes(op), 0),
                b"d" => state.record(Param::Dash, bytes(op), 0),
                b"ri" => state.record(Param::Intent, bytes(op), 0),
                b"i" => state.record(Param::Flatness, bytes(op), 0),
                b"gs" => {
                    gs_ids += 1;
                    state.record(Param::ExtGState, bytes(op), gs_ids);
                }
                b"CS" => state.record(Param::StrokeSpace, bytes(op), 0),
                b"cs" => state.record(Param::FillSpace, bytes(op), 0),
                b"SC" | b"SCN" | b"G" | b"RG" | b"K" => state.record(Param::StrokeColor, bytes(op), 0),
                b"sc" | b"scn" | b"g" | b"rg" | b"k" => state.record(Param::FillColor, bytes(op), 0),
                b"Tf" => {
                    if let (Some(Operand::Name(name)), Some(size)) = (op.operands.first(), nums.first()) {
                        state.font = name.clone();
                        state.size = *size;
                    }
                    state.record(Param::Font, bytes(op), 0);
                }
                b"Tc" if nums.len() == 1 => {
                    state.tc = nums[0];
                    state.record(Param::CharSpacing, bytes(op), 0);
                }
                b"Tw" if nums.len() == 1 => {
                    state.tw = nums[0];
                    state.record(Param::WordSpacing, bytes(op), 0);
                }
                b"Tz" if nums.len() == 1 => {
                    state.tz = nums[0] / 100.0;
                    state.record(Param::Scaling, bytes(op), 0);
                }
                b"TL" if nums.len() == 1 => {
                    state.tl = nums[0];
                    state.record(Param::Leading, bytes(op), 0);
                }
                b"Ts" if nums.len() == 1 => {
                    state.rise = nums[0];
                    state.record(Param::Rise, bytes(op), 0);
                }
                b"Tr" if nums.len() == 1 => {
                    state.render = nums[0] as i64;
                    state.record(Param::Render, bytes(op), 0);
                }
                // ---- paths
                b"m" | b"l" | b"c" | b"v" | b"y" | b"h" | b"re" => path.push(&state.snap.ctm, op, stream),
                b"W" => path.clip = Some(false),
                b"W*" => path.clip = Some(true),
                b"S" | b"s" | b"f" | b"F" | b"f*" | b"B" | b"B*" | b"b" | b"b*" | b"n" => {
                    let built = std::mem::take(&mut path);
                    let stroked = matches!(op.name.as_slice(), b"S" | b"s" | b"B" | b"B*" | b"b" | b"b*");
                    let start = built.start.unwrap_or((stream, op.range.start));
                    let snap_before = state.snap.clone();
                    let clip_before = state.clip_bounds;
                    if let Some(even_odd) = built.clip {
                        let bounds = bounds_of(built.points.iter().copied()).unwrap_or([0.0; 4]);
                        traced.clips.push(Clip { path: Some(built.page_ops.clone()), even_odd });
                        state.snap.clips.push(traced.clips.len() - 1);
                        state.clip_bounds = Some(
                            state.clip_bounds.map_or(bounds, |c| intersect(c, bounds).unwrap_or([0.0; 4])),
                        );
                    }
                    if op.name != b"n"
                        && !built.spans_streams
                        && start.0 == stream
                        && traced.pieces.len() < MAX_PIECES
                    {
                        let grow = if stroked {
                            let m = state.snap.ctm.0;
                            let scale = ((m[0] * m[0] + m[1] * m[1]).sqrt()
                                + (m[2] * m[2] + m[3] * m[3]).sqrt())
                                / 2.0;
                            state.line_width.max(1.0) * scale / 2.0
                        } else {
                            0.0
                        };
                        let bounds = bounds_of(built.points.iter().copied())
                            .map(|b| [b[0] - grow, b[1] - grow, b[2] + grow, b[3] + grow])
                            .and_then(|b| match clip_before {
                                Some(c) => intersect(b, c),
                                None => Some(b),
                            });
                        let range = start.1..op.range.end;
                        let end_snap = state.snap.clone();
                        traced.pieces.push(Piece {
                            kind: PieceKind::Path,
                            stream,
                            range: range.clone(),
                            ctm: state.snap.ctm,
                            bounds,
                            snap: snap_before.clone(),
                            start: ((stream, range.start), snap_before),
                            end: ((stream, range.end), end_snap),
                            clip_ops: built.clip.map(|_| start.1..built.construction_end),
                            text: None,
                            wrapper: None,
                            image: None,
                        });
                    }
                }
                // ---- images, forms, shadings
                b"Do" => {
                    if let Some(Operand::Name(name)) = op.operands.first() {
                        let (kind, bounds, image) = match xobject(file, resources, name) {
                            XObject::Image(meta) => {
                                (PieceKind::Image, bounds_of(unit_square(&state.snap.ctm)), Some(meta))
                            }
                            XObject::Form { bbox, matrix } => {
                                let m = matrix.then(state.snap.ctm);
                                let corners = [
                                    (bbox[0], bbox[1]),
                                    (bbox[2], bbox[1]),
                                    (bbox[2], bbox[3]),
                                    (bbox[0], bbox[3]),
                                ];
                                (
                                    PieceKind::Form,
                                    bounds_of(corners.iter().map(|&(x, y)| m.apply(x, y))),
                                    None,
                                )
                            }
                            XObject::Other => {
                                index += 1;
                                continue;
                            }
                        };
                        let bounds = match (bounds, state.clip_bounds) {
                            (Some(b), Some(c)) => intersect(b, c),
                            (b, _) => b,
                        };
                        push_simple(&mut traced, &state, stream, op, kind, bounds, image);
                    }
                }
                b"BI" => {
                    let bounds =
                        bounds_of(unit_square(&state.snap.ctm)).and_then(|b| match state.clip_bounds {
                            Some(c) => intersect(b, c),
                            None => Some(b),
                        });
                    let meta = inline_size(&data[op.range.clone()]);
                    push_simple(&mut traced, &state, stream, op, PieceKind::InlineImage, bounds, meta);
                }
                b"sh" => {
                    push_simple(&mut traced, &state, stream, op, PieceKind::Shading, state.clip_bounds, None);
                }
                // ---- text
                b"BT" => {
                    text = Some(TextObject {
                        tm: Matrix::IDENTITY,
                        tlm: Matrix::IDENTITY,
                        start: ((stream, op.range.start), state.snap.clone()),
                        pieces: Vec::new(),
                        odd: false,
                    });
                }
                b"ET" => {
                    if let Some(object) = text.take() {
                        // Text drawn as a clip adds a clip no path can stand for.
                        if state.render >= 4 {
                            traced.clips.push(Clip { path: None, even_odd: false });
                            state.snap.clips.push(traced.clips.len() - 1);
                        }
                        for &i in &object.pieces {
                            traced.pieces[i].end = ((stream, op.range.end), state.snap.clone());
                            if object.odd {
                                traced.pieces[i].text = None;
                            }
                        }
                    }
                }
                b"Td" | b"TD" if nums.len() == 2 => {
                    if let Some(t) = &mut text {
                        if op.name == b"TD" {
                            state.tl = -nums[1];
                        }
                        t.tlm = Matrix::translate(nums[0], nums[1]).then(t.tlm);
                        t.tm = t.tlm;
                    }
                }
                b"Tm" if nums.len() == 6 && all_nums => {
                    if let Some(t) = &mut text {
                        t.tlm = Matrix([nums[0], nums[1], nums[2], nums[3], nums[4], nums[5]]);
                        t.tm = t.tlm;
                    }
                }
                b"T*" => {
                    if let Some(t) = &mut text {
                        t.tlm = Matrix::translate(0.0, -state.tl).then(t.tlm);
                        t.tm = t.tlm;
                    }
                }
                name @ (b"Tj" | b"TJ" | b"'" | b"\"") => {
                    let Some(t) = &mut text else {
                        index += 1;
                        continue;
                    };
                    let kind = match name {
                        b"Tj" => ShowKind::Tj,
                        b"TJ" => ShowKind::TJ,
                        b"'" => ShowKind::NextLine,
                        _ => ShowKind::NextLineSpaced,
                    };
                    let snap = state.snap.clone();
                    if kind == ShowKind::NextLineSpaced && nums.len() >= 2 {
                        state.tw = nums[0];
                        state.tc = nums[1];
                        let id = 0;
                        state.record(Param::WordSpacing, format!("{} Tw", fmt(nums[0])).into_bytes(), id);
                        state.record(Param::CharSpacing, format!("{} Tc", fmt(nums[1])).into_bytes(), id);
                    }
                    if matches!(kind, ShowKind::NextLine | ShowKind::NextLineSpaced) {
                        t.tlm = Matrix::translate(0.0, -state.tl).then(t.tlm);
                        t.tm = t.tlm;
                    }
                    let font = metrics(&state.font);
                    let pieces: Vec<&Operand> = match op.operands.last() {
                        Some(Operand::Array(items)) if kind == ShowKind::TJ => items.iter().collect(),
                        Some(string @ Operand::Str(_)) => vec![string],
                        _ => Vec::new(),
                    };
                    let mut x = 0.0f64;
                    let (mut lo, mut hi) = (0.0f64, 0.0f64);
                    let mut glyphs = 0usize;
                    for piece in pieces {
                        match piece {
                            Operand::Num(adjust) => x -= adjust / 1000.0 * state.size * state.tz,
                            Operand::Str(bytes) => {
                                let one_byte = !font.two_byte();
                                for (code, length, width) in font.codes(bytes) {
                                    let space = one_byte && length == 1 && code == 32;
                                    let advance = (width / 1000.0 * state.size
                                        + state.tc
                                        + if space { state.tw } else { 0.0 })
                                        * state.tz;
                                    lo = lo.min(x).min(x + advance);
                                    hi = hi.max(x).max(x + advance);
                                    x += advance;
                                    glyphs += 1;
                                }
                            }
                            _ => {}
                        }
                    }
                    let trm = t.tm.then(state.snap.ctm);
                    let (y0, y1) =
                        (font.descent * state.size + state.rise, font.ascent * state.size + state.rise);
                    let corners = [(lo, y0), (hi, y0), (hi, y1), (lo, y1)];
                    let bounds = bounds_of(corners.iter().map(|&(px, py)| trm.apply(px, py)))
                        .filter(|b| glyphs > 0 && (b[2] > b[0] || b[3] > b[1]))
                        .and_then(|b| match state.clip_bounds {
                            Some(c) => intersect(b, c),
                            None => Some(b),
                        });
                    // The text matrix after the show, from the line's start.
                    let line_start = t.tlm;
                    let line_offset = {
                        let (dx, dy) = (t.tm.0[4] - line_start.0[4], t.tm.0[5] - line_start.0[5]);
                        let (a, b) = (line_start.0[0], line_start.0[1]);
                        let len = (a * a + b * b).sqrt();
                        if len > 1e-12 { (dx * a + dy * b) / (len * len) } else { 0.0 }
                    };
                    t.tm = Matrix::translate(x, 0.0).then(t.tm);
                    let piece = Piece {
                        kind: PieceKind::Text,
                        stream,
                        range: op.range.clone(),
                        ctm: state.snap.ctm,
                        bounds,
                        snap,
                        start: t.start.clone(),
                        end: ((stream, op.range.end), state.snap.clone()),
                        clip_ops: None,
                        text: (state.size.abs() > 1e-9 && state.tz.abs() > 1e-9).then(|| TextPiece {
                            tlm_after: t.tlm,
                            line_advance: line_offset + x,
                            size: state.size,
                            scaling: state.tz,
                            em: state.size.abs() * (trm.0[2] * trm.0[2] + trm.0[3] * trm.0[3]).sqrt(),
                            horizontal: trm.0[0].abs() >= trm.0[1].abs(),
                            invisible: state.render == 3 || state.render == 7,
                        }),
                        wrapper: None,
                        image: None,
                    };
                    if traced.pieces.len() < MAX_PIECES {
                        traced.pieces.push(piece);
                        t.pieces.push(traced.pieces.len() - 1);
                    }
                }
                b"d0" | b"d1" | b"BX" | b"EX" => {}
                _ => {}
            }
            index += 1;
        }
    }

    // Which wrapper each piece is in (the innermost).
    for w in 0..traced.wrappers.len() {
        let wrapper = &traced.wrappers[w];
        let inner = wrapper.inner.clone();
        let stream = wrapper.stream;
        let range = wrapper.range.clone();
        let outer = wrapper.outer.clone();
        let mut count = 0;
        for piece in &mut traced.pieces {
            if piece.stream == stream && piece.range.start >= inner.start && piece.range.end <= inner.end {
                piece.wrapper = Some(w);
                piece.start = ((stream, range.start), outer.clone());
                count += 1;
            }
        }
        traced.wrappers[w].pieces = count;
    }
    for piece in &mut traced.pieces {
        if let Some(w) = piece.wrapper {
            let wrapper = &traced.wrappers[w];
            // The state after the wrapper is the state before it.
            piece.end = ((wrapper.stream, wrapper.range.end), wrapper.outer.clone());
        }
    }
    Some(traced)
}

#[allow(clippy::too_many_arguments)]
fn push_simple(
    traced: &mut Traced,
    state: &State,
    stream: usize,
    op: &Op,
    kind: PieceKind,
    bounds: Option<[f64; 4]>,
    image: Option<ImageMeta>,
) {
    if traced.pieces.len() >= MAX_PIECES {
        return;
    }
    traced.pieces.push(Piece {
        kind,
        stream,
        range: op.range.clone(),
        ctm: state.snap.ctm,
        bounds,
        snap: state.snap.clone(),
        start: ((stream, op.range.start), state.snap.clone()),
        end: ((stream, op.range.end), state.snap.clone()),
        clip_ops: None,
        text: None,
        wrapper: None,
        image,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::content::tokenize;

    fn content(data: &[u8]) -> Content {
        Content {
            items: Vec::new(),
            data: vec![data.to_vec()],
            ops: vec![tokenize(data).expect("tokenizes")],
            wrap_streams: vec![false],
        }
    }

    fn file() -> Vec<u8> {
        let mut pdf = b"%PDF-1.7\n1 0 obj << /Type /Catalog >> endobj\n".to_vec();
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!(
                "xref\n0 2\n0000000000 65535 f \n0000000009 00000 n \ntrailer << /Size 2 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
            )
            .as_bytes(),
        );
        pdf
    }

    #[test]
    fn paths_and_clips_are_told_apart() {
        let bytes = file();
        let file = File::new(&bytes).expect("file");
        let data = b"q 0 0 100 100 re W n 1 0 0 rg 10 10 20 20 re f Q 2 w 0 0 m 50 50 l S";
        let traced = trace(&file, &Dict::default(), &content(data)).expect("traced");
        assert_eq!(traced.pieces.len(), 2);
        let fill = &traced.pieces[0];
        assert_eq!(&data[fill.range.clone()], b"10 10 20 20 re f");
        assert_eq!(fill.bounds, Some([10.0, 10.0, 30.0, 30.0]));
        assert_eq!(fill.snap.clips.len(), 1);
        assert!(fill.snap.log.iter().any(|e| e.op == b"1 0 0 rg"));
        let stroke = &traced.pieces[1];
        assert!(stroke.snap.clips.is_empty(), "the clip ended with its q … Q");
        assert_eq!(stroke.bounds, Some([-1.0, -1.0, 51.0, 51.0]));
    }

    #[test]
    fn the_recipe_keeps_the_last_of_each_kind_and_every_gs() {
        let bytes = file();
        let file = File::new(&bytes).expect("file");
        let data = b"1 w 2 w /G1 gs 0 g /CS0 cs 0.5 sc /G2 gs 0 0 10 10 re f";
        let traced = trace(&file, &Dict::default(), &content(data)).expect("traced");
        let ops: Vec<String> =
            traced.pieces[0].snap.log.iter().map(|e| String::from_utf8_lossy(&e.op).into_owned()).collect();
        assert_eq!(ops, ["2 w", "/G1 gs", "/CS0 cs", "0.5 sc", "/G2 gs"]);
    }

    #[test]
    fn our_wrapper_is_recognised() {
        let bytes = file();
        let file = File::new(&bytes).expect("file");
        let data =
            b"/MirsamObj <</G 7>> BDC q 2 0 0 2 5 5 cm 0 0 m 1 0 l 1 1 l 0 1 l h W n 0 0 1 1 re f Q EMC";
        let traced = trace(&file, &Dict::default(), &content(data)).expect("traced");
        assert_eq!(traced.wrappers.len(), 1);
        let w = &traced.wrappers[0];
        assert_eq!(w.group, 7);
        assert_eq!(w.matrix, Matrix([2.0, 0.0, 0.0, 2.0, 5.0, 5.0]));
        assert!(w.crop.is_some());
        assert_eq!(&data[w.inner.clone()], b"0 0 1 1 re f ");
        let piece = &traced.pieces[0];
        assert_eq!(piece.wrapper, Some(0));
        assert_eq!(piece.bounds, Some([5.0, 5.0, 7.0, 7.0]));
        assert_eq!(traced.max_group, 7);
    }

    #[test]
    fn a_show_knows_its_line() {
        let bytes = file();
        let file = File::new(&bytes).expect("file");
        let data = b"BT /F1 10 Tf 12 TL 100 700 Td (ab) Tj (cd) Tj T* (ef) ' ET";
        let traced = trace(&file, &Dict::default(), &content(data)).expect("traced");
        assert_eq!(traced.pieces.len(), 3);
        let second = traced.pieces[1].text.as_ref().expect("text");
        // Unknown font: 500 thousandths per code; two codes before it, two in it.
        assert!((second.line_advance - 20.0).abs() < 1e-9, "{}", second.line_advance);
        assert_eq!(second.tlm_after, Matrix([1.0, 0.0, 0.0, 1.0, 100.0, 700.0]));
        let third = traced.pieces[2].text.as_ref().expect("text");
        assert_eq!(third.tlm_after, Matrix([1.0, 0.0, 0.0, 1.0, 100.0, 676.0]));
        assert_eq!(traced.pieces[2].start.0, (0, 0));
    }
}
