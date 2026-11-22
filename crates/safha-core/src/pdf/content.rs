//! Reading page content (M3, ADR 0023): a tokenizer for content streams that
//! keeps every operator's byte range, and an interpreter that follows the
//! graphics and text state to say where each piece of shown text lands.
//!
//! Editing text in place removes some text-showing operators and writes new
//! ones. PDFium must never regenerate existing content (ADR 0005's
//! amendment), so this is our own reader: it finds the operators to remove by
//! range and the graphics state (matrix, clip, colour) they were drawn in.
//!
//! Bounded for untrusted input: operators, operand nesting, and the
//! operand stack are limited, and anything unreadable yields `None`.

use std::ops::Range;
use std::rc::Rc;

use super::fontinfo::FontInfo;
use super::{is_delim, is_white};

const MAX_OPS: usize = 4_000_000;
const MAX_OPERANDS: usize = 4_096;
const MAX_NESTING: usize = 16;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Operand {
    Num(f64),
    Name(Vec<u8>),
    Str(Vec<u8>),
    Array(Vec<Operand>),
    Dict(Vec<(Vec<u8>, Operand)>),
}

impl Operand {
    fn num(&self) -> Option<f64> {
        match self {
            Operand::Num(n) => Some(*n),
            _ => None,
        }
    }
}

/// One operator with its operands and where they all sit in the stream.
#[derive(Debug, Clone)]
pub(crate) struct Op {
    pub(crate) name: Vec<u8>,
    pub(crate) operands: Vec<Operand>,
    /// From the first operand's first byte to the operator's last byte.
    pub(crate) range: Range<usize>,
}

struct Lexer<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Lexer<'_> {
    fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    fn skip_space(&mut self) {
        while let Some(b) = self.peek() {
            if is_white(b) {
                self.pos += 1;
            } else if b == b'%' {
                while self.peek().is_some_and(|c| c != b'\n' && c != b'\r') {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn word(&mut self) -> &[u8] {
        let start = self.pos;
        while self.peek().is_some_and(|b| !is_white(b) && !is_delim(b)) {
            self.pos += 1;
        }
        &self.data[start..self.pos]
    }

    /// A literal string, with octal escapes and line continuations.
    fn literal(&mut self) -> Option<Vec<u8>> {
        self.pos += 1; // (
        let mut out = Vec::new();
        let mut depth = 1usize;
        while let Some(b) = self.peek() {
            self.pos += 1;
            match b {
                b'\\' => {
                    let escaped = self.peek()?;
                    self.pos += 1;
                    match escaped {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'0'..=b'7' => {
                            let mut value = u32::from(escaped - b'0');
                            for _ in 0..2 {
                                match self.peek() {
                                    Some(d @ b'0'..=b'7') => {
                                        value = value * 8 + u32::from(d - b'0');
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push((value & 0xFF) as u8);
                        }
                        b'\r' => {
                            if self.peek() == Some(b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        other => out.push(other),
                    }
                }
                b'(' => {
                    depth += 1;
                    out.push(b);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(out);
                    }
                    out.push(b);
                }
                _ => out.push(b),
            }
        }
        None
    }

    fn hex(&mut self) -> Option<Vec<u8>> {
        self.pos += 1; // <
        let mut digits = Vec::new();
        while let Some(b) = self.peek() {
            self.pos += 1;
            if b == b'>' {
                if digits.len() % 2 == 1 {
                    digits.push(b'0');
                }
                return digits
                    .chunks(2)
                    .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
                    .collect();
            }
            if b.is_ascii_hexdigit() {
                digits.push(b);
            } else if !is_white(b) {
                return None;
            }
        }
        None
    }

    /// An operand, or `None` at an operator (the cursor stays before it).
    fn operand(&mut self, depth: usize) -> Option<Option<Operand>> {
        if depth > MAX_NESTING {
            return None;
        }
        self.skip_space();
        let Some(b) = self.peek() else { return Some(None) };
        Some(match b {
            b'/' => {
                self.pos += 1;
                let name = self.word().to_vec();
                Some(Operand::Name(name))
            }
            b'(' => Some(Operand::Str(self.literal()?)),
            b'<' if self.data.get(self.pos + 1) == Some(&b'<') => {
                self.pos += 2;
                let mut entries = Vec::new();
                loop {
                    self.skip_space();
                    self.peek()?;
                    if self.data.get(self.pos..self.pos + 2) == Some(b">>") {
                        self.pos += 2;
                        break;
                    }
                    let Some(Some(Operand::Name(key))) = self.operand(depth + 1) else {
                        return None;
                    };
                    // A keyword value (`true`, `null`) is skipped.
                    match self.operand(depth + 1)? {
                        Some(value) => entries.push((key, value)),
                        None => {
                            if self.word().is_empty() {
                                return None;
                            }
                        }
                    }
                }
                Some(Operand::Dict(entries))
            }
            b'<' => Some(Operand::Str(self.hex()?)),
            b'[' => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_space();
                    if self.peek()? == b']' {
                        self.pos += 1;
                        break;
                    }
                    items.push(self.operand(depth + 1)??);
                }
                Some(Operand::Array(items))
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => {
                let word = self.word();
                let number =
                    std::str::from_utf8(word).ok().and_then(|text| text.parse::<f64>().ok()).unwrap_or(0.0);
                Some(Operand::Num(if number.is_finite() { number } else { 0.0 }))
            }
            _ => None,
        })
    }
}

/// Every operator of a content stream, in order.
pub(crate) fn tokenize(data: &[u8]) -> Option<Vec<Op>> {
    let mut lexer = Lexer { data, pos: 0 };
    let mut ops = Vec::new();
    let mut pending: Vec<Operand> = Vec::new();
    let mut start = None;
    loop {
        lexer.skip_space();
        let here = lexer.pos;
        match lexer.operand(0)? {
            Some(operand) => {
                start.get_or_insert(here);
                if pending.len() >= MAX_OPERANDS {
                    pending.remove(0);
                }
                pending.push(operand);
            }
            None => {
                if lexer.peek().is_none() {
                    break;
                }
                let word = lexer.word().to_vec();
                if word.is_empty() {
                    lexer.pos += 1; // a stray delimiter
                    continue;
                }
                let begin = start.take().unwrap_or(here);
                if word == b"BI" {
                    // Inline image: skip to `EI`.
                    let id = find_word(data, lexer.pos, b"ID")?;
                    let mut at = id + 2;
                    let end = loop {
                        let found = find(data, at, b"EI")?;
                        let before = found == 0 || is_white(data[found - 1]);
                        let after = data.get(found + 2).is_none_or(|b| is_white(*b));
                        if before && after {
                            break found + 2;
                        }
                        at = found + 2;
                    };
                    lexer.pos = end;
                    pending.clear();
                    ops.push(Op { name: word, operands: Vec::new(), range: begin..end });
                } else {
                    ops.push(Op {
                        name: word,
                        operands: std::mem::take(&mut pending),
                        range: begin..lexer.pos,
                    });
                }
                if ops.len() > MAX_OPS {
                    return None;
                }
            }
        }
    }
    Some(ops)
}

fn find(data: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    data.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

fn find_word(data: &[u8], from: usize, word: &[u8]) -> Option<usize> {
    let mut at = from;
    loop {
        let found = find(data, at, word)?;
        let before = found == 0 || is_white(data[found - 1]);
        let after = data.get(found + word.len()).is_none_or(|b| is_white(*b));
        if before && after {
            return Some(found);
        }
        at = found + word.len();
    }
}

// ------------------------------------------------------------ the interpreter

/// An affine matrix `[a b c d e f]`; a point maps to `(ax + cy + e, bx + dy + f)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Matrix(pub(crate) [f64; 6]);

impl Matrix {
    pub(crate) const IDENTITY: Matrix = Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    pub(crate) fn translate(x: f64, y: f64) -> Matrix {
        Matrix([1.0, 0.0, 0.0, 1.0, x, y])
    }

    /// `self` applied first, then `then`.
    pub(crate) fn then(self, then: Matrix) -> Matrix {
        let [a1, b1, c1, d1, e1, f1] = self.0;
        let [a2, b2, c2, d2, e2, f2] = then.0;
        Matrix([
            a1 * a2 + b1 * c2,
            a1 * b2 + b1 * d2,
            c1 * a2 + d1 * c2,
            c1 * b2 + d1 * d2,
            e1 * a2 + f1 * c2 + e2,
            e1 * b2 + f1 * d2 + f2,
        ])
    }

    pub(crate) fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        let [a, b, c, d, e, f] = self.0;
        (a * x + c * y + e, b * x + d * y + f)
    }

    pub(crate) fn inverse(&self) -> Option<Matrix> {
        let [a, b, c, d, e, f] = self.0;
        let det = a * d - b * c;
        if det.abs() < 1e-12 || !det.is_finite() {
            return None;
        }
        Some(Matrix([d / det, -b / det, -c / det, a / det, (c * f - d * e) / det, (b * e - a * f) / det]))
    }
}

/// One glyph of shown text.
#[derive(Debug, Clone)]
pub(crate) struct Glyph {
    /// The font's advance, in thousandths of an em.
    pub(crate) width: f64,
    /// Where its origin is along the text line, in text space units, from
    /// the start of the show (font size, `Tc`, `Tw`, `Tz` and `TJ`
    /// adjustments applied).
    pub(crate) x: f64,
    pub(crate) text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShowKind {
    Tj,
    TJ,
    /// `'`: moves to the next line, then shows.
    NextLine,
    /// `"`: sets word and character spacing, moves to the next line, shows.
    NextLineSpaced,
}

/// A text-showing operator and the state it ran in.
#[derive(Debug, Clone)]
pub(crate) struct Shown {
    pub(crate) stream: usize,
    pub(crate) range: Range<usize>,
    pub(crate) kind: ShowKind,
    /// Which `BT … ET` it is in (counting from 0 over the page).
    pub(crate) object: usize,
    /// No later show in its text object depends on this one's advance.
    pub(crate) last_in_object: bool,
    /// Where the text object's `BT` is: (stream, byte offset).
    pub(crate) object_start: (usize, usize),
    pub(crate) font_name: Vec<u8>,
    pub(crate) font: Option<Rc<FontInfo>>,
    pub(crate) size: f64,
    pub(crate) ctm: Matrix,
    /// The text matrix when the show starts.
    pub(crate) tm: Matrix,
    pub(crate) render: i64,
    pub(crate) tz: f64,
    /// The word and character spacing in force (`Tw`, `Tc`).
    pub(crate) word_char: (f64, f64),
    pub(crate) fill: Option<[f32; 3]>,
    pub(crate) glyphs: Vec<Glyph>,
    /// Total advance along the text line, in the same units as `Glyph::x`.
    pub(crate) advance: f64,
    /// In a text object with a `cm` or an inline unreadable state.
    pub(crate) odd_state: bool,
}

#[derive(Clone)]
struct GState {
    ctm: Matrix,
    fill: Option<[f32; 3]>,
    font_name: Vec<u8>,
    size: f64,
    tc: f64,
    tw: f64,
    tz: f64,
    tl: f64,
    rise: f64,
    render: i64,
}

impl GState {
    fn initial() -> Self {
        GState {
            ctm: Matrix::IDENTITY,
            fill: Some([0.0; 3]),
            font_name: Vec::new(),
            size: 0.0,
            tc: 0.0,
            tw: 0.0,
            tz: 1.0,
            tl: 0.0,
            rise: 0.0,
            render: 0,
        }
    }
}

/// Everything on a page that shows text, in drawing order. `streams` are the
/// decoded content streams in order; `font` resolves a font's resource name.
pub(crate) fn interpret(
    streams: &[Vec<Op>],
    font: &mut dyn FnMut(&[u8]) -> Option<Rc<FontInfo>>,
) -> Vec<Shown> {
    let mut shown: Vec<Shown> = Vec::new();
    let mut state = GState::initial();
    let mut stack: Vec<GState> = Vec::new();
    let mut tm = Matrix::IDENTITY;
    let mut tlm = Matrix::IDENTITY;
    let mut in_text = false;
    let mut object = 0usize;
    let mut object_start = (0usize, 0usize);
    let mut first_of_object = 0usize;
    let mut odd = false;
    // Marked content: the `/ActualText` of each open sequence, and whether a glyph took it.
    let mut marked: Vec<Option<(String, bool)>> = Vec::new();
    let mut cache: Vec<(Vec<u8>, Option<Rc<FontInfo>>)> = Vec::new();

    for (stream, ops) in streams.iter().enumerate() {
        for op in ops {
            let nums: Vec<f64> = op.operands.iter().filter_map(Operand::num).collect();
            let all_nums = nums.len() == op.operands.len();
            match op.name.as_slice() {
                b"q" if stack.len() < 256 => stack.push(state.clone()),
                b"Q" => {
                    if let Some(previous) = stack.pop() {
                        state = previous;
                    }
                }
                b"cm" if nums.len() == 6 && all_nums => {
                    state.ctm =
                        Matrix([nums[0], nums[1], nums[2], nums[3], nums[4], nums[5]]).then(state.ctm);
                    if in_text {
                        odd = true;
                    }
                }
                b"BMC" => marked.push(None),
                b"BDC" => marked.push(op.operands.last().and_then(actual_text).map(|t| (t, false))),
                b"EMC" => {
                    marked.pop();
                }
                b"g" if nums.len() == 1 => state.fill = Some([nums[0] as f32; 3]),
                b"rg" if nums.len() == 3 => {
                    state.fill = Some([nums[0] as f32, nums[1] as f32, nums[2] as f32]);
                }
                b"k" if nums.len() == 4 => state.fill = Some(cmyk(&nums)),
                b"cs" => state.fill = Some([0.0; 3]),
                b"sc" | b"scn" => {
                    state.fill = match (all_nums, nums.len()) {
                        (true, 1) => Some([nums[0] as f32; 3]),
                        (true, 3) => Some([nums[0] as f32, nums[1] as f32, nums[2] as f32]),
                        (true, 4) => Some(cmyk(&nums)),
                        _ => None,
                    }
                }
                b"BT" => {
                    in_text = true;
                    tm = Matrix::IDENTITY;
                    tlm = Matrix::IDENTITY;
                    object_start = (stream, op.range.start);
                    first_of_object = shown.len();
                    odd = false;
                }
                b"ET" => {
                    if in_text {
                        if let Some(last) = shown.get_mut(first_of_object..).and_then(<[Shown]>::last_mut) {
                            last.last_in_object = true;
                        }
                        for item in shown.get_mut(first_of_object..).unwrap_or_default() {
                            item.odd_state |= odd;
                        }
                        object += 1;
                    }
                    in_text = false;
                }
                b"Tc" if nums.len() == 1 => state.tc = nums[0],
                b"Tw" if nums.len() == 1 => state.tw = nums[0],
                b"Tz" if nums.len() == 1 => state.tz = nums[0] / 100.0,
                b"TL" if nums.len() == 1 => state.tl = nums[0],
                b"Ts" if nums.len() == 1 => state.rise = nums[0],
                b"Tr" if nums.len() == 1 => state.render = nums[0] as i64,
                b"Tf" => {
                    if let (Some(Operand::Name(name)), Some(size)) = (op.operands.first(), nums.first()) {
                        state.font_name = name.clone();
                        state.size = *size;
                    }
                }
                b"Td" if nums.len() == 2 => {
                    tlm = Matrix::translate(nums[0], nums[1]).then(tlm);
                    tm = tlm;
                }
                b"TD" if nums.len() == 2 => {
                    state.tl = -nums[1];
                    tlm = Matrix::translate(nums[0], nums[1]).then(tlm);
                    tm = tlm;
                }
                b"Tm" if nums.len() == 6 && all_nums => {
                    tlm = Matrix([nums[0], nums[1], nums[2], nums[3], nums[4], nums[5]]);
                    tm = tlm;
                }
                b"T*" => {
                    tlm = Matrix::translate(0.0, -state.tl).then(tlm);
                    tm = tlm;
                }
                name @ (b"Tj" | b"TJ" | b"'" | b"\"") if in_text => {
                    let kind = match name {
                        b"Tj" => ShowKind::Tj,
                        b"TJ" => ShowKind::TJ,
                        b"'" => ShowKind::NextLine,
                        _ => ShowKind::NextLineSpaced,
                    };
                    if kind == ShowKind::NextLineSpaced && nums.len() >= 2 {
                        state.tw = nums[0];
                        state.tc = nums[1];
                    }
                    if matches!(kind, ShowKind::NextLine | ShowKind::NextLineSpaced) {
                        tlm = Matrix::translate(0.0, -state.tl).then(tlm);
                        tm = tlm;
                    }
                    let info = match cache.iter().find(|(n, _)| *n == state.font_name) {
                        Some((_, info)) => info.clone(),
                        None => {
                            let info = font(&state.font_name);
                            cache.push((state.font_name.clone(), info.clone()));
                            info
                        }
                    };
                    let pieces: Vec<&Operand> = match op.operands.last() {
                        Some(Operand::Array(items)) if kind == ShowKind::TJ => items.iter().collect(),
                        Some(string @ Operand::Str(_)) => vec![string],
                        _ => Vec::new(),
                    };
                    let mut glyphs = Vec::new();
                    let mut x = 0.0f64;
                    for piece in pieces {
                        match piece {
                            Operand::Num(adjust) => x -= adjust / 1000.0 * state.size * state.tz,
                            Operand::Str(bytes) => {
                                let Some(info) = &info else { continue };
                                for (code, length) in info.codes(bytes) {
                                    let width = info.width(code);
                                    glyphs.push(Glyph { width, x, text: info.text(code).map(str::to_owned) });
                                    let space = length == 1 && code == 32;
                                    x += (width / 1000.0 * state.size
                                        + state.tc
                                        + if space { state.tw } else { 0.0 })
                                        * state.tz;
                                }
                            }
                            _ => {}
                        }
                    }
                    // Text an `/ActualText` span says it stands for goes on its first glyph.
                    if let Some(Some((text, taken))) = marked.iter_mut().rev().find(|m| m.is_some()) {
                        for glyph in &mut glyphs {
                            glyph.text = Some(if *taken { String::new() } else { text.clone() });
                            *taken = true;
                        }
                    }
                    shown.push(Shown {
                        stream,
                        range: op.range.clone(),
                        kind,
                        object,
                        last_in_object: false,
                        object_start,
                        font_name: state.font_name.clone(),
                        font: info,
                        size: state.size,
                        ctm: state.ctm,
                        tm,
                        render: state.render,
                        tz: state.tz,
                        word_char: (state.tw, state.tc),
                        fill: state.fill,
                        glyphs,
                        advance: x,
                        odd_state: odd,
                    });
                    tm = Matrix::translate(x, 0.0).then(tm);
                }
                _ => {}
            }
        }
    }
    shown
}

/// The `/ActualText` of a marked-content property dictionary.
fn actual_text(operand: &Operand) -> Option<String> {
    let Operand::Dict(entries) = operand else { return None };
    let Some((_, Operand::Str(bytes))) = entries.iter().find(|(k, _)| k == b"ActualText") else {
        return None;
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> =
            rest.as_chunks::<2>().0.iter().map(|p| u16::from(p[0]) << 8 | u16::from(p[1])).collect();
        String::from_utf16(&units).ok()
    } else {
        Some(bytes.iter().map(|&b| char::from(b)).collect())
    }
}

fn cmyk(n: &[f64]) -> [f32; 3] {
    let k = 1.0 - n[3];
    [((1.0 - n[0]) * k) as f32, ((1.0 - n[1]) * k) as f32, ((1.0 - n[2]) * k) as f32]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operators_keep_their_ranges() {
        let data = b"q 1 0 0 1 10 20 cm\nBT /F1 12 Tf (a\\(b\\101) Tj ET Q";
        let ops = tokenize(data).expect("tokenizes");
        let names: Vec<&[u8]> = ops.iter().map(|op| op.name.as_slice()).collect();
        assert_eq!(names, [&b"q"[..], b"cm", b"BT", b"Tf", b"Tj", b"ET", b"Q"]);
        let show = &ops[4];
        assert_eq!(&data[show.range.clone()], b"(a\\(b\\101) Tj");
        assert_eq!(show.operands, [Operand::Str(b"a(bA".to_vec())]);
        assert_eq!(&data[ops[1].range.clone()], b"1 0 0 1 10 20 cm");
    }

    #[test]
    fn inline_images_and_dictionaries_are_skipped_whole() {
        let data = b"BI /W 1 /H 1 /BPC 8 /CS /G ID \x01EI \nEI\n/Span << /ActualText (x)>> BDC (hi) Tj EMC";
        let ops = tokenize(data).expect("tokenizes");
        let names: Vec<&[u8]> = ops.iter().map(|op| op.name.as_slice()).collect();
        assert_eq!(names, [&b"BI"[..], b"BDC", b"Tj", b"EMC"]);
    }

    #[test]
    fn matrices_compose_and_invert() {
        let m = Matrix([2.0, 0.0, 0.0, -3.0, 5.0, 7.0]);
        let p = m.apply(1.0, 1.0);
        let q = m.inverse().map(|inv| inv.apply(p.0, p.1)).expect("invertible");
        assert!((q.0 - 1.0).abs() < 1e-9 && (q.1 - 1.0).abs() < 1e-9);
        let moved = Matrix::translate(10.0, 0.0).then(Matrix([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]));
        assert_eq!(moved.apply(0.0, 0.0), (20.0, 0.0));
    }

    #[test]
    fn unbalanced_input_gives_up_instead_of_looping() {
        assert!(tokenize(b"(never closed").is_none());
        assert!(tokenize(b"[1 2").is_none());
    }
}
