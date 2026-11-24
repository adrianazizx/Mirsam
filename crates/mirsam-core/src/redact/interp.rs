//! The content-stream rewriter (ADR 0042).
//!
//! It walks a page's operations keeping the graphics and text state, works
//! out where every glyph, image, and path lands on the page, and rewrites
//! what falls in a redaction area (or, for sanitizing, what is hidden):
//!
//! - **Text**: the character codes of glyphs touching an area are cut out of
//!   the string, and the gap they leave is closed with a `TJ` adjustment, so
//!   the text around them stays where it was. A glyph is cut by its code, so
//!   its shape, and the text its ToUnicode map gives, leave the file.
//! - **Images**: pixels whose centres fall in an area are overwritten
//!   (`image.rs`), in a copy of the image used by that one drawing.
//! - **Paths**: a path with a point in an area is dropped; a plain rectangle
//!   fill is cut into the pieces outside it.
//! - **Form XObjects** are rewritten the same way, in copies.
//!
//! Everything else is copied byte for byte.

use std::collections::{HashMap, HashSet};

use super::content::{Op, Output, number, tokenize};
use super::fonts::FontInfo;
use super::image::{ImageOutcome, redact_image};
use crate::pdf::{Dict, Entry, File, Object, decode_stream};

pub(crate) type Matrix = [f64; 6];
/// A rectangle of page space: left, bottom, right, top.
pub(crate) type Area = [f64; 4];

pub(crate) const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `m` applied first, then `n`.
pub(crate) fn mul(m: &Matrix, n: &Matrix) -> Matrix {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

pub(crate) fn apply(m: &Matrix, (x, y): (f64, f64)) -> (f64, f64) {
    (x * m[0] + y * m[2] + m[4], x * m[1] + y * m[3] + m[5])
}

fn bounds(points: &[(f64, f64)]) -> Area {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for &(x, y) in points {
        b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
    }
    b
}

/// Whether two areas share more than their edge (`eps` is the least overlap,
/// in points, in each direction).
fn overlaps(a: &Area, b: &Area, eps: f64) -> bool {
    a[0] < b[2] - eps && b[0] < a[2] - eps && a[1] < b[3] - eps && b[1] < a[3] - eps
}

fn inside(area: &Area, (x, y): (f64, f64)) -> bool {
    area[0] <= x && x <= area[2] && area[1] <= y && y <= area[3]
}

/// What a rewrite removes.
#[derive(Debug, Clone, Default)]
pub(crate) struct Settings {
    /// Page-space rectangles whose content is removed.
    pub(crate) areas: Vec<Area>,
    /// Sanitizing: remove text drawn invisibly (render mode 3).
    pub(crate) invisible_text: bool,
    /// Sanitizing: remove text that lies wholly outside this box.
    pub(crate) page_box: Option<Area>,
    /// Sanitizing: optional-content groups switched off; what they hold goes.
    pub(crate) off_layers: HashSet<u32>,
    /// Rewrite every form XObject, not only those that meet an area.
    pub(crate) every_form: bool,
}

/// What was removed, for the report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    /// Glyphs cut out of text.
    pub glyphs: usize,
    /// Images with pixels overwritten.
    pub images_blanked: usize,
    /// Images that could not be edited pixel by pixel and were dropped whole.
    pub images_dropped: usize,
    /// Paths dropped or cut.
    pub paths: usize,
    /// Drawings dropped because they were hidden (sanitizing).
    pub hidden: usize,
}

impl Counts {
    pub(crate) fn add(&mut self, other: Counts) {
        self.glyphs += other.glyphs;
        self.images_blanked += other.images_blanked;
        self.images_dropped += other.images_dropped;
        self.paths += other.paths;
        self.hidden += other.hidden;
    }
}

/// A stream object to write into the update: number, dictionary, data.
pub(crate) type Write = (u32, Dict, Vec<u8>);

/// The rewriter's shared state for one update.
pub(crate) struct Rewriter<'f, 'b> {
    pub(crate) file: &'f File<'b>,
    pub(crate) settings: Settings,
    pub(crate) next_number: u32,
    pub(crate) writes: Vec<Write>,
    pub(crate) counts: Counts,
    fonts: HashMap<u32, FontInfo>,
    visiting: Vec<u32>,
    names: u32,
}

/// A rewritten stream, and the resources it now needs.
pub(crate) struct Rewritten {
    pub(crate) bytes: Vec<u8>,
    /// New `/XObject` entries to add to the stream's resources.
    pub(crate) xobjects: Vec<(Vec<u8>, Object)>,
    /// Names of the XObjects those replace.
    pub(crate) replaced: Vec<Vec<u8>>,
}

const MAX_FORM_DEPTH: usize = 12;

#[derive(Clone)]
struct State {
    ctm: Matrix,
    font: Option<FontInfo>,
    size: f64,
    char_space: f64,
    word_space: f64,
    scale: f64,
    leading: f64,
    render: i64,
    rise: f64,
}

impl State {
    fn new(ctm: Matrix) -> Self {
        Self {
            ctm,
            font: None,
            size: 0.0,
            char_space: 0.0,
            word_space: 0.0,
            scale: 1.0,
            leading: 0.0,
            render: 0,
            rise: 0.0,
        }
    }
}

/// The path being built.
#[derive(Default)]
struct Pending {
    ops: Vec<usize>,
    points: Vec<(f64, f64)>,
    /// Set when the path is one `re` and nothing else: its rectangle in the
    /// coordinates it was written in.
    rect: Option<[f64; 4]>,
    constructs: usize,
    clip: bool,
}

impl<'f, 'b> Rewriter<'f, 'b> {
    pub(crate) fn new(file: &'f File<'b>, settings: Settings, next_number: u32) -> Self {
        Self {
            file,
            settings,
            next_number,
            writes: Vec::new(),
            counts: Counts::default(),
            fonts: HashMap::new(),
            visiting: Vec::new(),
            names: 0,
        }
    }

    pub(crate) fn alloc(&mut self) -> Option<u32> {
        let number = self.next_number;
        self.next_number = number.checked_add(1)?;
        Some(number)
    }

    fn category(&self, resources: &Dict, category: &[u8], name: &[u8]) -> Option<Object> {
        let dict = self.file.resolve(resources.get(category)?)?;
        dict.as_dict()?.get(name).cloned()
    }

    fn font(&mut self, resources: &Dict, name: &[u8]) -> FontInfo {
        let Some(font) = self.category(resources, b"Font", name) else { return FontInfo::fallback() };
        if let Object::Ref(number, _) = font {
            if let Some(info) = self.fonts.get(&number) {
                return info.clone();
            }
            let info = FontInfo::read(self.file, &font);
            self.fonts.insert(number, info.clone());
            return info;
        }
        FontInfo::read(self.file, &font)
    }

    fn layer_off(&self, properties: Option<Object>) -> bool {
        matches!(properties, Some(Object::Ref(n, _)) if self.settings.off_layers.contains(&n))
    }

    fn glyph_goes(&self, page_box: &Area, state: &State, hidden: bool) -> bool {
        if hidden || (self.settings.invisible_text && state.render == 3) {
            return true;
        }
        if let Some(outside) = &self.settings.page_box
            && !overlaps(outside, page_box, -1.0)
        {
            return true;
        }
        let eps_x = (page_box[2] - page_box[0]) * 0.03;
        let eps_x = eps_x.max(0.05);
        self.settings.areas.iter().any(|area| overlaps(area, page_box, eps_x.min(0.5)))
    }

    /// Rewrites one content stream. `ctm` maps its space to the page.
    pub(crate) fn rewrite(
        &mut self,
        data: &[u8],
        resources: &Dict,
        ctm: Matrix,
        depth: usize,
    ) -> Option<Rewritten> {
        let ops = tokenize(data)?;
        let mut out = Output::new(data);
        let mut xobjects: Vec<(Vec<u8>, Object)> = Vec::new();
        let mut replaced: Vec<Vec<u8>> = Vec::new();
        let mut stack: Vec<State> = Vec::new();
        let mut state = State::new(ctm);
        let mut tm = IDENTITY;
        let mut tlm = IDENTITY;
        let mut pending = Pending::default();
        // One entry per open marked-content sequence: whether it is hidden.
        let mut marked: Vec<bool> = Vec::new();

        for (i, op) in ops.iter().enumerate() {
            let hidden = marked.iter().any(|hidden| *hidden);
            let name = op.operator.as_slice();

            // Path construction, clipping, and painting.
            match name {
                b"m" | b"l" | b"c" | b"v" | b"y" | b"re" | b"h" => {
                    pending.ops.push(i);
                    pending.constructs += 1;
                    let n = |k: usize| op.number(k).unwrap_or(0.0);
                    match name {
                        b"m" | b"l" => pending.points.push(apply(&state.ctm, (n(0), n(1)))),
                        b"c" => {
                            for k in 0..3 {
                                pending.points.push(apply(&state.ctm, (n(2 * k), n(2 * k + 1))));
                            }
                        }
                        b"v" | b"y" => {
                            for k in 0..2 {
                                pending.points.push(apply(&state.ctm, (n(2 * k), n(2 * k + 1))));
                            }
                        }
                        b"re" => {
                            let (x, y, w, h) = (n(0), n(1), n(2), n(3));
                            for corner in [(x, y), (x + w, y), (x + w, y + h), (x, y + h)] {
                                pending.points.push(apply(&state.ctm, corner));
                            }
                            pending.rect = Some([x, y, w, h]);
                        }
                        _ => {}
                    }
                    if name != b"re" || pending.constructs != 1 {
                        pending.rect = None;
                    }
                    continue;
                }
                b"W" | b"W*" => {
                    pending.ops.push(i);
                    pending.clip = true;
                    continue;
                }
                b"S" | b"s" | b"f" | b"F" | b"f*" | b"B" | b"B*" | b"b" | b"b*" | b"n" => {
                    self.paint(&ops, op, &mut pending, &state, hidden, &mut out);
                    pending = Pending::default();
                    continue;
                }
                _ => {}
            }
            // Anything else ends a path that never got painted.
            for &k in &pending.ops {
                out.keep(&ops[k]);
            }
            pending = Pending::default();

            match name {
                b"q" => {
                    stack.push(state.clone());
                    out.keep(op);
                }
                b"Q" => {
                    if let Some(previous) = stack.pop() {
                        state = previous;
                    }
                    out.keep(op);
                }
                b"cm" => {
                    let m = [0, 1, 2, 3, 4, 5].map(|k| op.number(k).unwrap_or(0.0));
                    state.ctm = mul(&m, &state.ctm);
                    out.keep(op);
                }
                b"BT" => {
                    tm = IDENTITY;
                    tlm = IDENTITY;
                    out.keep(op);
                }
                b"Tc" => {
                    state.char_space = op.number(0).unwrap_or(0.0);
                    out.keep(op);
                }
                b"Tw" => {
                    state.word_space = op.number(0).unwrap_or(0.0);
                    out.keep(op);
                }
                b"Tz" => {
                    state.scale = op.number(0).unwrap_or(100.0) / 100.0;
                    out.keep(op);
                }
                b"TL" => {
                    state.leading = op.number(0).unwrap_or(0.0);
                    out.keep(op);
                }
                b"Tr" => {
                    state.render = op.number(0).unwrap_or(0.0) as i64;
                    out.keep(op);
                }
                b"Ts" => {
                    state.rise = op.number(0).unwrap_or(0.0);
                    out.keep(op);
                }
                b"Tf" => {
                    if let Some(Object::Name(font)) = op.operands.first() {
                        state.font = Some(self.font(resources, font));
                    }
                    state.size = op.number(1).unwrap_or(0.0);
                    out.keep(op);
                }
                b"Td" | b"TD" => {
                    let (tx, ty) = (op.number(0).unwrap_or(0.0), op.number(1).unwrap_or(0.0));
                    if name == b"TD" {
                        state.leading = -ty;
                    }
                    tlm = mul(&[1.0, 0.0, 0.0, 1.0, tx, ty], &tlm);
                    tm = tlm;
                    out.keep(op);
                }
                b"Tm" => {
                    let m: Vec<f64> = (0..6).map(|k| op.number(k).unwrap_or(0.0)).collect();
                    tlm = [m[0], m[1], m[2], m[3], m[4], m[5]];
                    tm = tlm;
                    out.keep(op);
                }
                b"T*" => {
                    tlm = mul(&[1.0, 0.0, 0.0, 1.0, 0.0, -state.leading], &tlm);
                    tm = tlm;
                    out.keep(op);
                }
                b"Tj" | b"TJ" | b"'" | b"\"" => {
                    // `'` and `"` move to the next line first.
                    if name == b"'" || name == b"\"" {
                        if name == b"\"" {
                            state.word_space = op.number(0).unwrap_or(0.0);
                            state.char_space = op.number(1).unwrap_or(0.0);
                        }
                        tlm = mul(&[1.0, 0.0, 0.0, 1.0, 0.0, -state.leading], &tlm);
                        tm = tlm;
                    }
                    let elements: Vec<Object> = match (name, op.operands.last()) {
                        (b"TJ", Some(Object::Array(items))) => items.clone(),
                        (_, Some(string @ Object::Str(_))) => vec![string.clone()],
                        _ => Vec::new(),
                    };
                    let shown = self.show(&elements, &state, &mut tm, hidden);
                    match shown {
                        Some(new) => {
                            self.counts.glyphs += new.removed;
                            match name {
                                b"'" => out.op("T*", &[]),
                                b"\"" => {
                                    out.op("Tw", &[Object::Real(state.word_space)]);
                                    out.op("Tc", &[Object::Real(state.char_space)]);
                                    out.op("T*", &[]);
                                }
                                _ => {}
                            }
                            out.op("TJ", &[Object::Array(new.elements)]);
                        }
                        None => out.keep(op),
                    }
                }
                b"BMC" => {
                    marked.push(false);
                    out.keep(op);
                }
                b"BDC" => {
                    let off = matches!(op.operands.first(), Some(Object::Name(tag)) if tag == b"OC")
                        && match op.operands.get(1) {
                            Some(Object::Name(property)) => {
                                self.layer_off(self.category(resources, b"Properties", property))
                            }
                            _ => false,
                        };
                    marked.push(off);
                    out.keep(op);
                }
                b"EMC" => {
                    marked.pop();
                    out.keep(op);
                }
                b"BI" => {
                    let area = bounds(&unit_square(&state.ctm));
                    if hidden || self.settings.areas.iter().any(|a| overlaps(a, &area, 0.0)) {
                        if hidden {
                            self.counts.hidden += 1;
                        } else {
                            self.counts.images_dropped += 1;
                        }
                    } else {
                        out.keep(op);
                    }
                }
                b"sh" if hidden => self.counts.hidden += 1,
                b"Do" => {
                    self.draw(op, resources, &state, hidden, depth, &mut out, &mut xobjects, &mut replaced);
                }
                _ => out.keep(op),
            }
        }
        for &k in &pending.ops {
            out.keep(&ops[k]);
        }
        Some(Rewritten { bytes: out.finish(), xobjects, replaced })
    }

    /// Shows `elements` (the strings and adjustments of a `Tj` or `TJ`),
    /// advancing `tm`. `Some` when glyphs were cut: the new `TJ` elements.
    fn show(&self, elements: &[Object], state: &State, tm: &mut Matrix, hidden: bool) -> Option<Shown> {
        let fallback = FontInfo::fallback();
        let font = state.font.as_ref().unwrap_or(&fallback);
        let size = state.size;
        let mut shown = Shown { elements: Vec::new(), removed: 0 };
        let mut kept: Vec<u8> = Vec::new();
        // The distance cut out since the last kept glyph, in thousandths of
        // an em (TJ's unit, positive moving left).
        let mut gap = 0.0_f64;
        let flush = |shown: &mut Shown, kept: &mut Vec<u8>, gap: &mut f64| {
            if !kept.is_empty() {
                shown.elements.push(Object::Str(std::mem::take(kept)));
            }
            if *gap != 0.0 {
                shown.elements.push(Object::Real(-*gap));
                *gap = 0.0;
            }
        };
        for element in elements {
            match element {
                Object::Str(bytes) => {
                    let step = font.code_bytes.max(1);
                    let mut at = 0;
                    while at < bytes.len() {
                        let len = step.min(bytes.len() - at);
                        let code = bytes[at..at + len].iter().fold(0u32, |acc, b| acc << 8 | u32::from(*b));
                        let width = font.width(code);
                        let word = len == 1 && code == 32;
                        let advance = (width * size / 1000.0
                            + state.char_space
                            + if word { state.word_space } else { 0.0 })
                            * state.scale;
                        // Where the glyph's ink is, in text space: a Type 3
                        // glyph's declared box; else its advance, and for a
                        // zero-advance mark (dots, tashkeel) a span around
                        // its origin, since where its ink falls is unknown.
                        let (mut x0, mut x1, y0, y1) = match font.ink(code) {
                            Some([l, b, r, t]) => (
                                l * size * state.scale,
                                r * size * state.scale,
                                state.rise + b * size,
                                state.rise + t * size,
                            ),
                            None => {
                                let (y0, y1) = (
                                    state.rise + font.descent * size / 1000.0,
                                    state.rise + font.ascent * size / 1000.0,
                                );
                                if width.abs() < 10.0 {
                                    (-0.3 * size * state.scale, 0.5 * size * state.scale, y0, y1)
                                } else {
                                    (0.0, width * size / 1000.0 * state.scale, y0, y1)
                                }
                            }
                        };
                        if x0 > x1 {
                            std::mem::swap(&mut x0, &mut x1);
                        }
                        let to_page = mul(tm, &state.ctm);
                        let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|p| apply(&to_page, p));
                        let page_box = bounds(&corners);
                        if self.glyph_goes(&page_box, state, hidden) {
                            if !kept.is_empty() {
                                flush(&mut shown, &mut kept, &mut gap);
                            }
                            shown.removed += 1;
                            // What the glyph moved the pen by, as a TJ number.
                            gap += if size != 0.0 {
                                width
                                    + (state.char_space + if word { state.word_space } else { 0.0 }) * 1000.0
                                        / size
                            } else {
                                0.0
                            };
                        } else {
                            if gap != 0.0 {
                                flush(&mut shown, &mut kept, &mut gap);
                            }
                            kept.extend_from_slice(&bytes[at..at + len]);
                        }
                        *tm = mul(&[1.0, 0.0, 0.0, 1.0, advance, 0.0], tm);
                        at += len;
                    }
                }
                other => {
                    if let Some(n) = number(other) {
                        flush(&mut shown, &mut kept, &mut gap);
                        shown.elements.push(other.clone());
                        *tm = mul(&[1.0, 0.0, 0.0, 1.0, -n / 1000.0 * size * state.scale, 0.0], tm);
                    }
                }
            }
        }
        if shown.removed == 0 {
            return None;
        }
        flush(&mut shown, &mut kept, &mut gap);
        Some(shown)
    }

    fn paint(
        &mut self,
        ops: &[Op],
        paint: &Op,
        pending: &mut Pending,
        state: &State,
        hidden: bool,
        out: &mut Output<'_>,
    ) {
        let keep_path = |out: &mut Output<'_>| {
            for &k in &pending.ops {
                out.keep(&ops[k]);
            }
        };
        if paint.is(b"n") || pending.ops.is_empty() {
            keep_path(out);
            out.keep(paint);
            return;
        }
        let areas = self.settings.areas.clone();
        let fills_only = matches!(paint.operator.as_slice(), b"f" | b"F" | b"f*");
        let axis_aligned =
            state.ctm[1] == 0.0 && state.ctm[2] == 0.0 && state.ctm[0] != 0.0 && state.ctm[3] != 0.0;
        if !hidden
            && fills_only
            && !pending.clip
            && axis_aligned
            && let Some([x, y, w, h]) = pending.rect
        {
            let page = bounds(&[apply(&state.ctm, (x, y)), apply(&state.ctm, (x + w, y + h))]);
            if !areas.iter().any(|a| overlaps(a, &page, 0.0)) {
                keep_path(out);
                out.keep(paint);
                return;
            }
            self.counts.paths += 1;
            let mut pieces = vec![page];
            for area in &areas {
                pieces = pieces.into_iter().flat_map(|piece| subtract(piece, *area)).collect();
            }
            let ctm = state.ctm;
            for piece in pieces {
                let (x0, y0) = ((piece[0] - ctm[4]) / ctm[0], (piece[1] - ctm[5]) / ctm[3]);
                let (x1, y1) = ((piece[2] - ctm[4]) / ctm[0], (piece[3] - ctm[5]) / ctm[3]);
                out.op(
                    "re",
                    &[Object::Real(x0), Object::Real(y0), Object::Real(x1 - x0), Object::Real(y1 - y0)],
                );
                out.keep(paint);
            }
            return;
        }
        let goes = hidden || pending.points.iter().any(|p| areas.iter().any(|a| inside(a, *p)));
        if !goes {
            keep_path(out);
            out.keep(paint);
            return;
        }
        self.counts.paths += 1;
        if pending.clip {
            // The clip is kept (it holds no content), the painting is not.
            keep_path(out);
            out.op("n", &[]);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        op: &Op,
        resources: &Dict,
        state: &State,
        hidden: bool,
        depth: usize,
        out: &mut Output<'_>,
        xobjects: &mut Vec<(Vec<u8>, Object)>,
        replaced: &mut Vec<Vec<u8>>,
    ) {
        let Some(Object::Name(name)) = op.operands.first() else { return out.keep(op) };
        let Some(Object::Ref(object_number, _)) = self.category(resources, b"XObject", name) else {
            return out.keep(op);
        };
        let Some(Entry::Offset(offset)) = self.file.entries.get(&object_number).copied() else {
            return out.keep(op);
        };
        let Some((dict, raw)) = self.file.indirect_at(offset) else { return out.keep(op) };
        if hidden || self.layer_off(dict.get(b"OC").cloned()) {
            self.counts.hidden += 1;
            return;
        }
        let subtype = dict.get(b"Subtype").and_then(Object::as_name).unwrap_or(b"");
        let area = bounds(&unit_square(&state.ctm));
        match subtype {
            b"Image" => {
                if !self.settings.areas.iter().any(|a| overlaps(a, &area, 0.0)) {
                    return out.keep(op);
                }
                let Some(raw) = raw else { return out.keep(op) };
                match redact_image(self.file, &dict, raw, &state.ctm, &self.settings.areas) {
                    ImageOutcome::Untouched => out.keep(op),
                    ImageOutcome::Dropped => self.counts.images_dropped += 1,
                    ImageOutcome::Blanked(new_dict, data) => {
                        let Some(new_number) = self.alloc() else { return };
                        self.writes.push((new_number, new_dict, data));
                        let fresh = self.fresh_name(b"RdImg", xobjects);
                        xobjects.push((fresh.clone(), Object::Ref(new_number, 0)));
                        replaced.push(name.clone());
                        out.op("Do", &[Object::Name(fresh)]);
                        self.counts.images_blanked += 1;
                    }
                }
            }
            b"Form" => {
                let matrix = dict
                    .get(b"Matrix")
                    .and_then(|m| self.file.resolve(m))
                    .and_then(|m| matrix_of(m.as_array()?))
                    .unwrap_or(IDENTITY);
                let ctm = mul(&matrix, &state.ctm);
                let bbox = dict
                    .get(b"BBox")
                    .and_then(|b| self.file.resolve(b))
                    .and_then(|b| b.as_array().map(<[Object]>::to_vec))
                    .and_then(|b| {
                        let v: Vec<f64> = b.iter().filter_map(number).collect();
                        (v.len() == 4).then(|| [v[0], v[1], v[2], v[3]])
                    });
                let touched = match bbox {
                    Some([l, b, r, t]) => {
                        let page = bounds(&[(l, b), (r, b), (r, t), (l, t)].map(|p| apply(&ctm, p)));
                        self.settings.areas.iter().any(|a| overlaps(a, &page, 0.0))
                    }
                    None => true,
                };
                if !touched && !self.settings.every_form {
                    return out.keep(op);
                }
                if depth >= MAX_FORM_DEPTH || self.visiting.contains(&object_number) {
                    // Too deep, or a form that draws itself: dropped, not guessed at.
                    self.counts.images_dropped += 1;
                    return;
                }
                let Some(data) = raw.and_then(|raw| decode_stream(&dict, raw, self.file)) else {
                    self.counts.images_dropped += 1;
                    return;
                };
                let own = dict
                    .get(b"Resources")
                    .and_then(|r| self.file.resolve(r))
                    .and_then(|r| r.as_dict().cloned());
                let inner = own.clone().unwrap_or_else(|| resources.clone());
                self.visiting.push(object_number);
                let rewritten = self.rewrite(&data, &inner, ctm, depth + 1);
                self.visiting.pop();
                let Some(rewritten) = rewritten else {
                    self.counts.images_dropped += 1;
                    return;
                };
                if rewritten.bytes == data && rewritten.xobjects.is_empty() {
                    return out.keep(op);
                }
                let mut new_dict = dict.clone();
                new_dict.0.retain(|(k, _)| !matches!(k.as_slice(), b"Filter" | b"DecodeParms" | b"Length"));
                if !rewritten.xobjects.is_empty() {
                    new_dict.0.retain(|(k, _)| k != b"Resources");
                    new_dict.0.push((
                        b"Resources".to_vec(),
                        Object::Dict(with_xobjects(self.file, &inner, &rewritten)),
                    ));
                }
                let Some(new_number) = self.alloc() else { return };
                self.writes.push((new_number, new_dict, rewritten.bytes));
                let fresh = self.fresh_name(b"RdForm", xobjects);
                xobjects.push((fresh.clone(), Object::Ref(new_number, 0)));
                replaced.push(name.clone());
                out.op("Do", &[Object::Name(fresh)]);
            }
            _ => out.keep(op),
        }
    }

    fn fresh_name(&mut self, prefix: &[u8], _taken: &[(Vec<u8>, Object)]) -> Vec<u8> {
        self.names += 1;
        let mut name = prefix.to_vec();
        name.extend_from_slice(self.names.to_string().as_bytes());
        name
    }
}

struct Shown {
    elements: Vec<Object>,
    removed: usize,
}

fn unit_square(ctm: &Matrix) -> [(f64, f64); 4] {
    [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|p| apply(ctm, p))
}

fn matrix_of(items: &[Object]) -> Option<Matrix> {
    let v: Vec<f64> = items.iter().filter_map(number).collect();
    (v.len() == 6).then(|| [v[0], v[1], v[2], v[3], v[4], v[5]])
}

/// `resources` with the rewrite's new `/XObject` entries added, and the
/// entries it replaced taken out unless the rewritten stream still draws them
/// (the original image must not stay reachable through its old name).
pub(crate) fn with_xobjects(file: &File<'_>, resources: &Dict, rewritten: &Rewritten) -> Dict {
    let mut resources = resources.clone();
    let mut xobject = resources
        .get(b"XObject")
        .and_then(|x| file.resolve(x))
        .and_then(|x| x.as_dict().cloned())
        .unwrap_or_default();
    let still_drawn: HashSet<Vec<u8>> = tokenize(&rewritten.bytes)
        .unwrap_or_default()
        .iter()
        .filter(|op| op.is(b"Do"))
        .filter_map(|op| op.operands.first().and_then(Object::as_name).map(<[u8]>::to_vec))
        .collect();
    xobject.0.retain(|(name, _)| !rewritten.replaced.contains(name) || still_drawn.contains(name));
    xobject.0.extend(rewritten.xobjects.iter().cloned());
    resources.0.retain(|(k, _)| k != b"XObject");
    resources.0.push((b"XObject".to_vec(), Object::Dict(xobject)));
    resources
}

/// `rect` without `hole`: up to four rectangles.
fn subtract(rect: Area, hole: Area) -> Vec<Area> {
    if !overlaps(&rect, &hole, 0.0) {
        return vec![rect];
    }
    let mut pieces = Vec::new();
    if hole[1] > rect[1] {
        pieces.push([rect[0], rect[1], rect[2], hole[1]]);
    }
    if hole[3] < rect[3] {
        pieces.push([rect[0], hole[3], rect[2], rect[3]]);
    }
    let (low, high) = (rect[1].max(hole[1]), rect[3].min(hole[3]));
    if hole[0] > rect[0] {
        pieces.push([rect[0], low, hole[0], high]);
    }
    if hole[2] < rect[2] {
        pieces.push([hole[2], low, rect[2], high]);
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rectangle_is_cut_into_what_lies_outside_the_hole() {
        let pieces = subtract([0.0, 0.0, 10.0, 10.0], [4.0, 4.0, 6.0, 6.0]);
        assert_eq!(pieces.len(), 4);
        let area: f64 = pieces.iter().map(|p| (p[2] - p[0]) * (p[3] - p[1])).sum();
        assert_eq!(area, 100.0 - 4.0);
        assert!(subtract([0.0, 0.0, 10.0, 10.0], [-1.0, -1.0, 11.0, 11.0]).is_empty());
        assert_eq!(subtract([0.0, 0.0, 10.0, 10.0], [20.0, 20.0, 30.0, 30.0]), [[0.0, 0.0, 10.0, 10.0]]);
    }

    #[test]
    fn matrices_compose_in_pdf_order() {
        let scale = [2.0, 0.0, 0.0, 2.0, 0.0, 0.0];
        let shift = [1.0, 0.0, 0.0, 1.0, 5.0, 0.0];
        // Scale first, then shift.
        assert_eq!(apply(&mul(&scale, &shift), (1.0, 1.0)), (7.0, 2.0));
    }
}
