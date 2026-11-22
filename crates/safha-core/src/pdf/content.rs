//! Reading a page's content streams (ADR 0023): every text-showing operation
//! (`Tj`, `TJ`, `'`, `"`), with where it is in the content and the state it
//! draws in. In-place editing removes a block's operations and writes new
//! ones (spec §7), so it has to know exactly which bytes drew which text.
//!
//! This is a small interpreter, not a renderer:
//!
//! - **Graphics state:** `q`/`Q` and `cm` (the CTM), and the fill colour
//!   (`g`, `rg`, `k`, `sc`/`scn` by their number of components).
//! - **Text state:** `Tf`, `Tc`, `Tw`, `Tz`, `TL`, `Ts`, `Tr`; the text and
//!   line matrices (`BT`, `Tm`, `Td`, `TD`, `T*`).
//! - **Marked content:** `BMC`/`BDC`/`EMC`, to know the tags in force.
//!
//! An operation's starting point (its first glyph's origin, which PDFium
//! reports per character) needs no font metrics: it is `(0, rise)` mapped
//! through the text matrix and the CTM. Only after a show operation does the
//! pen move by glyph widths this reader does not know; a show operation that
//! follows another without a positioning operator in between continues it
//! (`origin: None`). Form XObjects (`Do`) are not entered, and inline images
//! are skipped.

use std::ops::Range;

use super::{Object, Parser};

/// One text-showing operation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ShowOp {
    /// Which of the page's content streams it is in.
    pub(crate) stream: usize,
    /// Its bytes in that stream (decoded): from its first operand to the end
    /// of its operator.
    pub(crate) range: Range<usize>,
    /// The `BT` that opened its text object, in the same terms.
    pub(crate) text_object: (usize, usize),
    /// The font's resource name (`/F1` → `F1`) and the `Tf` size.
    pub(crate) font: Vec<u8>,
    pub(crate) size: f32,
    /// The first glyph's origin in page space; `None` when it continues the
    /// previous operation's pen position.
    pub(crate) origin: Option<(f32, f32)>,
    /// The size as drawn: `Tf` size scaled by the text matrix and the CTM.
    pub(crate) drawn_size: f32,
    /// Fill colour, as RGB (grey and CMYK converted), each 0 to 1.
    pub(crate) fill: [f32; 3],
    /// Inside `/MirsamText` marked content (text this app wrote, ADR 0021).
    pub(crate) mirsam: bool,
    /// The string bytes shown (concatenated for `TJ`).
    pub(crate) bytes: Vec<u8>,
}

type Matrix = [f32; 6];
const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `a` then `b`: a point goes through `a` first.
fn multiply(a: Matrix, b: Matrix) -> Matrix {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

fn apply(m: Matrix, (x, y): (f32, f32)) -> (f32, f32) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

#[derive(Clone)]
struct Graphics {
    ctm: Matrix,
    fill: [f32; 3],
    /// Components of the colour space set by `cs` (1, 3, or 4; else 0).
    fill_components: usize,
}

/// The text-showing operations of `streams` (a page's content, decoded, in
/// order), and whether the whole content could be read.
pub(crate) fn show_ops(streams: &[Vec<u8>]) -> (Vec<ShowOp>, bool) {
    let mut ops = Vec::new();
    let mut graphics = Graphics { ctm: IDENTITY, fill: [0.0; 3], fill_components: 1 };
    let mut stack: Vec<Graphics> = Vec::new();
    let (mut tm, mut tlm) = (IDENTITY, IDENTITY);
    let mut pen_known = true;
    let (mut font, mut size, mut leading, mut rise) = (Vec::new(), 0.0_f32, 0.0_f32, 0.0_f32);
    let mut text_object = (0, 0);
    let mut tags: Vec<Vec<u8>> = Vec::new();
    let mut complete = true;

    for (stream, data) in streams.iter().enumerate() {
        let mut parser = Parser::new(data, 0);
        let mut operands: Vec<Object> = Vec::new();
        let mut operands_start: Option<usize> = None;
        loop {
            parser.skip_space();
            let Some(byte) = parser.peek() else { break };
            let start = parser.pos;
            let is_operand = matches!(byte, b'/' | b'(' | b'<' | b'[' | b'+' | b'-' | b'.' | b'0'..=b'9');
            if is_operand {
                match parser.object(0) {
                    Some(object) => {
                        operands_start.get_or_insert(start);
                        operands.push(object);
                    }
                    None => {
                        complete = false;
                        parser.pos = start + 1;
                    }
                }
                continue;
            }
            let operator = parser.word();
            if operator.is_empty() {
                // A stray delimiter (`]`, `)`, `>`, `{`, `}`): not content.
                complete = false;
                parser.pos = start + 1;
                operands.clear();
                operands_start = None;
                continue;
            }
            let number = |i: usize| -> f32 {
                match operands.get(i) {
                    Some(Object::Int(v)) => *v as f32,
                    Some(Object::Real(v)) => *v as f32,
                    _ => 0.0,
                }
            };
            let numbers = |count: usize| -> Option<Vec<f32>> {
                (operands.len() >= count)
                    .then(|| (operands.len() - count..operands.len()).map(number).collect())
            };
            let range = operands_start.unwrap_or(start)..parser.pos;
            let mut show = |bytes: Vec<u8>, origin: Option<(f32, f32)>, tm: Matrix, graphics: &Graphics| {
                let trm = multiply(tm, graphics.ctm);
                let scale = (trm[0] * trm[3] - trm[1] * trm[2]).abs().sqrt();
                ops.push(ShowOp {
                    stream,
                    range: range.clone(),
                    text_object,
                    font: font.clone(),
                    size,
                    origin,
                    drawn_size: size * scale,
                    fill: graphics.fill,
                    mirsam: tags.iter().any(|tag| tag == b"MirsamText"),
                    bytes,
                });
            };
            let origin = |tm: Matrix, graphics: &Graphics| apply(multiply(tm, graphics.ctm), (0.0, rise));
            match operator {
                b"q" => stack.push(graphics.clone()),
                b"Q" => {
                    if let Some(saved) = stack.pop() {
                        graphics = saved;
                    }
                }
                b"cm" => {
                    if let Some(m) = numbers(6) {
                        graphics.ctm = multiply([m[0], m[1], m[2], m[3], m[4], m[5]], graphics.ctm);
                    }
                }
                b"BT" => {
                    tm = IDENTITY;
                    tlm = IDENTITY;
                    pen_known = true;
                    text_object = (stream, start);
                }
                b"Tf" => {
                    if let Some(Object::Name(name)) = operands.first() {
                        font = name.clone();
                    }
                    size = number(1);
                }
                b"TL" => leading = number(0),
                b"Ts" => rise = number(0),
                b"Tm" => {
                    if let Some(m) = numbers(6) {
                        tm = [m[0], m[1], m[2], m[3], m[4], m[5]];
                        tlm = tm;
                        pen_known = true;
                    }
                }
                b"Td" | b"TD" => {
                    let (tx, ty) = (number(0), number(1));
                    if operator == b"TD" {
                        leading = -ty;
                    }
                    tlm = multiply([1.0, 0.0, 0.0, 1.0, tx, ty], tlm);
                    tm = tlm;
                    pen_known = true;
                }
                b"T*" => {
                    tlm = multiply([1.0, 0.0, 0.0, 1.0, 0.0, -leading], tlm);
                    tm = tlm;
                    pen_known = true;
                }
                b"Tj" | b"'" | b"\"" | b"TJ" => {
                    if operator == b"'" || operator == b"\"" {
                        tlm = multiply([1.0, 0.0, 0.0, 1.0, 0.0, -leading], tlm);
                        tm = tlm;
                        pen_known = true;
                    }
                    let bytes = match operands.last() {
                        Some(Object::Str(bytes)) => bytes.clone(),
                        Some(Object::Array(items)) => items
                            .iter()
                            .filter_map(|item| match item {
                                Object::Str(bytes) => Some(bytes.as_slice()),
                                _ => None,
                            })
                            .flatten()
                            .copied()
                            .collect(),
                        _ => Vec::new(),
                    };
                    let start_origin = pen_known.then(|| origin(tm, &graphics));
                    show(bytes, start_origin, tm, &graphics);
                    pen_known = false;
                }
                b"g" => {
                    let v = number(0);
                    graphics.fill = [v, v, v];
                    graphics.fill_components = 1;
                }
                b"rg" => {
                    graphics.fill = [number(0), number(1), number(2)];
                    graphics.fill_components = 3;
                }
                b"k" => {
                    graphics.fill = cmyk(number(0), number(1), number(2), number(3));
                    graphics.fill_components = 4;
                }
                b"cs" => {
                    graphics.fill_components = match operands.first() {
                        Some(Object::Name(name)) => match name.as_slice() {
                            b"DeviceGray" | b"CalGray" | b"G" => 1,
                            b"DeviceRGB" | b"CalRGB" | b"RGB" => 3,
                            b"DeviceCMYK" | b"CMYK" => 4,
                            _ => 0,
                        },
                        _ => 0,
                    };
                    graphics.fill = [0.0; 3];
                }
                b"sc" | b"scn" => {
                    let values: Vec<f32> = (0..operands.len())
                        .filter(|&i| matches!(operands[i], Object::Int(_) | Object::Real(_)))
                        .map(number)
                        .collect();
                    graphics.fill = match (graphics.fill_components, values.as_slice()) {
                        (1, [v]) => [*v, *v, *v],
                        (3, [r, g, b]) => [*r, *g, *b],
                        (4, [c, m, y, k]) => cmyk(*c, *m, *y, *k),
                        _ => graphics.fill,
                    };
                }
                b"BMC" | b"BDC" => {
                    if let Some(Object::Name(tag)) = operands.first() {
                        tags.push(tag.clone());
                    } else {
                        tags.push(Vec::new());
                    }
                }
                b"EMC" => {
                    tags.pop();
                }
                b"BI" if !skip_inline_image(&mut parser) => {
                    complete = false;
                    break;
                }
                _ => {}
            }
            operands.clear();
            operands_start = None;
        }
    }
    (ops, complete)
}

/// CMYK as RGB, the naive way: good enough to tell colours apart.
fn cmyk(c: f32, m: f32, y: f32, k: f32) -> [f32; 3] {
    [(1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)]
}

/// Skips an inline image after `BI`: its dictionary, `ID`, its binary data,
/// and `EI` (preceded by white space, followed by white space or the end).
fn skip_inline_image(parser: &mut Parser<'_>) -> bool {
    let Some(id) = super::find(&parser.bytes[parser.pos..], b"ID") else { return false };
    let mut at = parser.pos + id + 2;
    while at + 2 <= parser.bytes.len() {
        let Some(found) = super::find(&parser.bytes[at..], b"EI") else { return false };
        let pos = at + found;
        let before =
            pos.checked_sub(1).and_then(|i| parser.bytes.get(i)).is_some_and(|b| super::is_white(*b));
        let after = parser.bytes.get(pos + 2).is_none_or(|b| super::is_white(*b) || super::is_delim(*b));
        if before && after {
            parser.pos = pos + 2;
            return true;
        }
        at = pos + 2;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ops(content: &str) -> Vec<ShowOp> {
        let (ops, complete) = show_ops(&[content.as_bytes().to_vec()]);
        assert!(complete, "{content}");
        ops
    }

    #[test]
    fn origins_come_from_the_text_matrix_and_the_ctm() {
        let found = ops("q 2 0 0 2 10 20 cm BT /F1 12 Tf 5 6 Td (Hi) Tj ET Q");
        assert_eq!(found.len(), 1);
        let op = &found[0];
        assert_eq!(op.origin, Some((20.0, 32.0)), "Td 5 6 scaled by 2, then moved by 10 20");
        assert_eq!((op.font.as_slice(), op.size, op.drawn_size), (b"F1".as_slice(), 12.0, 24.0));
        assert_eq!(op.bytes, b"Hi");
        assert_eq!(&"q 2 0 0 2 10 20 cm BT /F1 12 Tf 5 6 Td (Hi) Tj ET Q"[op.range.clone()], "(Hi) Tj");
    }

    #[test]
    fn a_show_after_a_show_continues_it_until_the_next_move() {
        let found = ops("BT /F1 10 Tf 14 TL 100 700 Td (one) Tj (two) Tj T* [(thr) -20 (ee)] TJ ET");
        let origins: Vec<_> = found.iter().map(|op| op.origin).collect();
        assert_eq!(origins, [Some((100.0, 700.0)), None, Some((100.0, 686.0))]);
        assert_eq!(found[2].bytes, b"three");
        // ' moves to the next line first; " sets spacing too.
        let found = ops("BT /F1 10 Tf 12 TL 0 0 Td (a) ' 1 2 (b) \" ET");
        assert_eq!(
            found.iter().map(|op| op.origin).collect::<Vec<_>>(),
            [Some((0.0, -12.0)), Some((0.0, -24.0))]
        );
    }

    #[test]
    fn colours_marks_and_rises_are_tracked() {
        let found =
            ops("0.5 g BT /F1 9 Tf (a) Tj ET /MirsamText BMC 1 0 0 rg BT /F2 9 Tf 3 Ts (b) Tj ET EMC \
             0 0 0 1 k BT (c) Tj ET /DeviceRGB cs 0 0 1 sc BT (d) Tj ET");
        assert_eq!(found[0].fill, [0.5, 0.5, 0.5]);
        assert!(!found[0].mirsam);
        assert_eq!(
            (found[1].fill, found[1].mirsam, found[1].origin),
            ([1.0, 0.0, 0.0], true, Some((0.0, 3.0)))
        );
        assert_eq!(found[2].fill, [0.0, 0.0, 0.0]);
        assert_eq!(found[3].fill, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn inline_images_and_strings_with_operator_names_are_skipped() {
        let found = ops("BI /W 2 /H 1 /BPC 8 /CS /G ID \u{1}Tj EI\nBT /F1 8 Tf (Tj TJ ET) Tj ET");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].bytes, b"Tj TJ ET");
    }

    #[test]
    fn content_split_across_streams_keeps_its_state() {
        let (found, complete) =
            show_ops(&[b"q 1 0 0 1 50 0 cm BT /F1 8 Tf".to_vec(), b"10 0 Td (x) Tj ET Q".to_vec()]);
        assert!(complete);
        assert_eq!((found[0].stream, found[0].origin), (1, Some((60.0, 0.0))));
    }
}
