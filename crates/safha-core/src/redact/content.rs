//! Content streams as a list of operations, for rewriting (ADR 0042).
//!
//! The tokenizer keeps every operation's byte range in the source, so what a
//! rewrite does not touch is copied byte for byte, and what it does touch is
//! re-serialized with the crate's own writer.

use std::ops::Range;

use crate::pdf::{Dict, Object, Parser, is_delim, is_white, write};

/// One operation of a content stream.
#[derive(Debug, Clone)]
pub(crate) struct Op {
    pub(crate) operator: Vec<u8>,
    pub(crate) operands: Vec<Object>,
    /// Where it sits in the source, operands included (for an inline image,
    /// from `BI` through `EI`).
    pub(crate) span: Range<usize>,
}

impl Op {
    pub(crate) fn is(&self, operator: &[u8]) -> bool {
        self.operator == operator
    }

    pub(crate) fn number(&self, index: usize) -> Option<f64> {
        number(self.operands.get(index)?)
    }
}

pub(crate) fn number(object: &Object) -> Option<f64> {
    match object {
        Object::Int(i) => Some(*i as f64),
        Object::Real(r) if r.is_finite() => Some(*r),
        _ => None,
    }
}

/// Most operations a stream may hold before it is refused (a bound for
/// hostile input).
const MAX_OPS: usize = 20_000_000;

/// Splits a content stream into its operations. Stray delimiters are skipped,
/// as viewers do.
pub(crate) fn tokenize(data: &[u8]) -> Option<Vec<Op>> {
    let mut ops = Vec::new();
    let mut operands: Vec<Object> = Vec::new();
    let mut operand_start = 0;
    let mut parser = Parser::new(data, 0);
    loop {
        parser.skip_space();
        let Some(byte) = parser.peek() else { break };
        if ops.len() > MAX_OPS {
            return None;
        }
        let start = parser.pos;
        let is_operand = matches!(byte, b'/' | b'(' | b'<' | b'[' | b'0'..=b'9' | b'+' | b'-' | b'.');
        if is_operand {
            match parser.object(0) {
                Some(object) => {
                    if operands.is_empty() {
                        operand_start = start;
                    }
                    operands.push(object);
                    // A long run of operands is not a real operation.
                    if operands.len() > 10_000 {
                        return None;
                    }
                }
                None => {
                    if parser.pos == start {
                        parser.pos += 1;
                    }
                }
            }
            continue;
        }
        if is_delim(byte) {
            parser.pos += 1;
            continue;
        }
        let word = parser.word();
        if word.is_empty() {
            parser.pos += 1;
            continue;
        }
        match word {
            b"true" => {
                if operands.is_empty() {
                    operand_start = start;
                }
                operands.push(Object::Bool(true));
            }
            b"false" => {
                if operands.is_empty() {
                    operand_start = start;
                }
                operands.push(Object::Bool(false));
            }
            b"null" => {
                if operands.is_empty() {
                    operand_start = start;
                }
                operands.push(Object::Null);
            }
            b"BI" => {
                let begin = if operands.is_empty() { start } else { operand_start };
                let (dict, end) = inline_image(data, &mut parser)?;
                let mut args = std::mem::take(&mut operands);
                args.push(Object::Dict(dict));
                ops.push(Op { operator: b"BI".to_vec(), operands: args, span: begin..end });
            }
            _ => {
                let begin = if operands.is_empty() { start } else { operand_start };
                ops.push(Op {
                    operator: word.to_vec(),
                    operands: std::mem::take(&mut operands),
                    span: begin..parser.pos,
                });
            }
        }
    }
    Some(ops)
}

/// Reads an inline image after its `BI`: the dictionary, then the data up to
/// and including `EI`. Returns the dictionary and the end of the whole image.
fn inline_image(data: &[u8], parser: &mut Parser<'_>) -> Option<(Dict, usize)> {
    let mut dict = Dict::default();
    loop {
        parser.skip_space();
        let save = parser.pos;
        if parser.word() == b"ID" {
            break;
        }
        parser.pos = save;
        let Object::Name(key) = parser.object(1)? else { return None };
        let value = parser.object(1)?;
        dict.0.push((key, value));
    }
    // One white-space byte follows ID.
    if parser.peek().is_some_and(is_white) {
        parser.pos += 1;
    }
    let start = parser.pos;
    let get = |long: &[u8], short: &[u8]| dict.get(long).or_else(|| dict.get(short));
    let unfiltered = get(b"Filter", b"F").is_none();
    let known = unfiltered
        .then(|| {
            let (w, h) = (get(b"Width", b"W")?.as_int()?, get(b"Height", b"H")?.as_int()?);
            let bpc = if matches!(get(b"ImageMask", b"IM"), Some(Object::Bool(true))) {
                1
            } else {
                get(b"BitsPerComponent", b"BPC")?.as_int()?
            };
            let comps = match get(b"ColorSpace", b"CS") {
                _ if bpc == 1 && matches!(get(b"ImageMask", b"IM"), Some(Object::Bool(true))) => 1,
                Some(Object::Name(n)) => match n.as_slice() {
                    b"G" | b"DeviceGray" | b"I" | b"Indexed" => 1,
                    b"RGB" | b"DeviceRGB" => 3,
                    b"CMYK" | b"DeviceCMYK" => 4,
                    _ => return None,
                },
                Some(Object::Array(a))
                    if a.first().and_then(Object::as_name).is_some_and(|n| n == b"I" || n == b"Indexed") =>
                {
                    1
                }
                _ => return None,
            };
            let row = usize::try_from(w)
                .ok()?
                .checked_mul(comps)?
                .checked_mul(usize::try_from(bpc).ok()?)?
                .div_ceil(8);
            row.checked_mul(usize::try_from(h).ok()?)
        })
        .flatten();
    if let Some(length) = known
        && let Some(after) = start.checked_add(length)
        && after <= data.len()
    {
        let mut p = Parser::new(data, after);
        if p.word() == b"EI" {
            parser.pos = p.pos;
            return Some((dict, p.pos));
        }
    }
    // Otherwise the first `EI` standing alone.
    let mut at = start;
    while at + 2 <= data.len() {
        if &data[at..at + 2] == b"EI"
            && (at == start || is_white(data[at - 1]))
            && data.get(at + 2).is_none_or(|b| is_white(*b) || is_delim(*b))
        {
            parser.pos = at + 2;
            return Some((dict, at + 2));
        }
        at += 1;
    }
    None
}

/// Output being assembled: untouched spans copied from the source, and new
/// operations written out.
pub(crate) struct Output<'a> {
    source: &'a [u8],
    pub(crate) bytes: Vec<u8>,
}

impl<'a> Output<'a> {
    pub(crate) fn new(source: &'a [u8]) -> Self {
        Self { source, bytes: Vec::with_capacity(source.len()) }
    }

    pub(crate) fn keep(&mut self, op: &Op) {
        self.bytes.extend_from_slice(&self.source[op.span.clone()]);
        self.bytes.push(b'\n');
    }

    /// Writes `operands operator`.
    pub(crate) fn op(&mut self, operator: &str, operands: &[Object]) {
        for operand in operands {
            write::object(&mut self.bytes, operand);
            self.bytes.push(b' ');
        }
        self.bytes.extend_from_slice(operator.as_bytes());
        self.bytes.push(b'\n');
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operators(data: &[u8]) -> Vec<String> {
        tokenize(data)
            .expect("tokenizes")
            .iter()
            .map(|op| String::from_utf8_lossy(&op.operator).into_owned())
            .collect()
    }

    #[test]
    fn operations_carry_their_operands_and_ranges() {
        let data = b"q 1 0 0 1 10 20 cm BT /F1 12 Tf (Hi \\) there) Tj [(A) -20 (B)] TJ ET Q";
        assert_eq!(operators(data), ["q", "cm", "BT", "Tf", "Tj", "TJ", "ET", "Q"]);
        let ops = tokenize(data).expect("tokenizes");
        assert_eq!(&data[ops[1].span.clone()], b"1 0 0 1 10 20 cm");
        assert_eq!(ops[1].number(4), Some(10.0));
        assert_eq!(ops[4].operands, [Object::Str(b"Hi ) there".to_vec())]);
    }

    #[test]
    fn inline_images_are_one_operation_even_with_binary_data() {
        let mut data = b"q BI /W 2 /H 2 /BPC 8 /CS /G ID ".to_vec();
        // Four bytes of pixels that spell out an `EI` of their own.
        data.extend_from_slice(&[b'E', b'I', b' ', 0]);
        data.extend_from_slice(b"\nEI Q (x) Tj");
        assert_eq!(operators(&data), ["q", "BI", "Q", "Tj"]);

        let filtered = b"BI /W 1 /H 1 /F /AHx ID 00> EI (y) Tj";
        assert_eq!(operators(filtered), ["BI", "Tj"]);
    }

    #[test]
    fn comments_and_stray_delimiters_are_skipped() {
        assert_eq!(operators(b"q % note (\n) ] Q"), ["q", "Q"]);
    }
}
