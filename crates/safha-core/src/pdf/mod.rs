//! PDF syntax: objects, a tolerant parser, and the cross-reference chain.
//!
//! Read-only and bounded, for untrusted input: every loop, nesting level, and
//! decompressed size is limited, and anything unexpected yields `None`. Used
//! by the catalog reader (reading preferences) and the editor (page edits as
//! incremental updates, `write`).

use std::collections::{HashMap, HashSet};

pub(crate) mod content;
pub(crate) mod crypt;
pub(crate) mod fontinfo;
pub(crate) mod image;
pub(crate) mod protect;
pub(crate) mod text;
pub(crate) mod write;

pub(crate) const MAX_DEPTH: usize = 64;
const MAX_XREF_SECTIONS: usize = 256;
const MAX_INFLATED: usize = 64 * 1024 * 1024;

// ---------------------------------------------------------------- objects

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Object {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    Name(Vec<u8>),
    Str(Vec<u8>),
    Array(Vec<Object>),
    Dict(Dict),
    Ref(u32, u16),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Dict(pub(crate) Vec<(Vec<u8>, Object)>);

impl Dict {
    pub(crate) fn get(&self, key: &[u8]) -> Option<&Object> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

impl Object {
    pub(crate) fn as_dict(&self) -> Option<&Dict> {
        match self {
            Object::Dict(d) => Some(d),
            _ => None,
        }
    }
    pub(crate) fn as_name(&self) -> Option<&[u8]> {
        match self {
            Object::Name(n) => Some(n),
            _ => None,
        }
    }
    pub(crate) fn as_int(&self) -> Option<i64> {
        match self {
            Object::Int(i) => Some(*i),
            Object::Real(r) if r.fract() == 0.0 => Some(*r as i64),
            _ => None,
        }
    }
    pub(crate) fn as_array(&self) -> Option<&[Object]> {
        match self {
            Object::Array(a) => Some(a),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------- lexer and parser

pub(crate) fn is_white(b: u8) -> bool {
    matches!(b, 0 | b'\t' | b'\n' | 0x0C | b'\r' | b' ')
}

pub(crate) fn is_delim(b: u8) -> bool {
    matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

pub(crate) struct Parser<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) pos: usize,
}

impl<'a> Parser<'a> {
    pub(crate) fn new(bytes: &'a [u8], pos: usize) -> Self {
        Self { bytes, pos }
    }

    pub(crate) fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    pub(crate) fn skip_space(&mut self) {
        while let Some(b) = self.peek() {
            if is_white(b) {
                self.pos += 1;
            } else if b == b'%' {
                while let Some(c) = self.peek() {
                    if c == b'\n' || c == b'\r' {
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    /// A run of regular characters (a keyword or a number).
    pub(crate) fn word(&mut self) -> &'a [u8] {
        self.skip_space();
        let start = self.pos;
        while let Some(b) = self.peek() {
            if is_white(b) || is_delim(b) {
                break;
            }
            self.pos += 1;
        }
        &self.bytes[start..self.pos]
    }

    pub(crate) fn keyword(&mut self, expected: &[u8]) -> Option<()> {
        (self.word() == expected).then_some(())
    }

    pub(crate) fn unsigned(&mut self) -> Option<u64> {
        let word = self.word();
        if word.is_empty() || !word.iter().all(u8::is_ascii_digit) {
            return None;
        }
        std::str::from_utf8(word).ok()?.parse().ok()
    }

    pub(crate) fn object(&mut self, depth: usize) -> Option<Object> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.skip_space();
        match self.peek()? {
            b'/' => {
                self.pos += 1;
                Some(Object::Name(self.name_body()))
            }
            b'(' => self.literal_string().map(Object::Str),
            b'<' if self.bytes.get(self.pos + 1) == Some(&b'<') => {
                self.pos += 2;
                let mut dict = Dict::default();
                loop {
                    self.skip_space();
                    if self.bytes.get(self.pos..self.pos + 2) == Some(b">>") {
                        self.pos += 2;
                        return Some(Object::Dict(dict));
                    }
                    let key = match self.object(depth + 1)? {
                        Object::Name(name) => name,
                        _ => return None,
                    };
                    let value = self.object(depth + 1)?;
                    dict.0.push((key, value));
                }
            }
            b'<' => self.hex_string().map(Object::Str),
            b'[' => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_space();
                    if self.peek()? == b']' {
                        self.pos += 1;
                        return Some(Object::Array(items));
                    }
                    items.push(self.object(depth + 1)?);
                }
            }
            _ => {
                let start = self.pos;
                let word = self.word();
                match word {
                    b"null" => Some(Object::Null),
                    b"true" => Some(Object::Bool(true)),
                    b"false" => Some(Object::Bool(false)),
                    _ => {
                        let text = std::str::from_utf8(word).ok()?;
                        if text.is_empty() {
                            self.pos = start + 1; // an unexpected delimiter
                            return None;
                        }
                        if let Ok(int) = text.parse::<i64>() {
                            // `n g R` is a reference.
                            let save = self.pos;
                            if let (Ok(number), Some(generation)) = (u32::try_from(int), self.unsigned())
                                && self.word() == b"R"
                                && let Ok(generation) = u16::try_from(generation)
                            {
                                return Some(Object::Ref(number, generation));
                            }
                            self.pos = save;
                            Some(Object::Int(int))
                        } else {
                            text.parse::<f64>().ok().map(Object::Real)
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn name_body(&mut self) -> Vec<u8> {
        let mut name = Vec::new();
        while let Some(b) = self.peek() {
            if is_white(b) || is_delim(b) {
                break;
            }
            self.pos += 1;
            if b == b'#'
                && let Some(hex) = self.bytes.get(self.pos..self.pos + 2)
                && let Ok(text) = std::str::from_utf8(hex)
                && let Ok(value) = u8::from_str_radix(text, 16)
            {
                name.push(value);
                self.pos += 2;
            } else {
                name.push(b);
            }
        }
        name
    }

    pub(crate) fn literal_string(&mut self) -> Option<Vec<u8>> {
        self.pos += 1; // (
        let mut out = Vec::new();
        let mut depth = 1usize;
        while let Some(b) = self.peek() {
            self.pos += 1;
            match b {
                b'\\' => {
                    let escaped = self.peek()?;
                    self.pos += 1;
                    out.push(match escaped {
                        b'n' => b'\n',
                        b'r' => b'\r',
                        b't' => b'\t',
                        b'b' => 8,
                        b'f' => 12,
                        other => other,
                    });
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

    pub(crate) fn hex_string(&mut self) -> Option<Vec<u8>> {
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
}

// ---------------------------------------------------------------- the file

#[derive(Debug, Clone, Copy)]
pub(crate) enum Entry {
    Offset(usize),
    InStream { stream: u32, index: usize },
}

pub(crate) struct File<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) entries: HashMap<u32, Entry>,
    pub(crate) trailer: Dict,
}

impl<'a> File<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Option<Self> {
        let start = startxref(bytes)?;
        let mut file = File { bytes, entries: HashMap::new(), trailer: Dict::default() };
        let mut next = Some(start);
        let mut seen = HashSet::new();
        let mut newest = true;
        while let Some(offset) = next.take() {
            if !seen.insert(offset) || seen.len() > MAX_XREF_SECTIONS {
                break;
            }
            let trailer = file.read_section(offset)?;
            // Hybrid files: a classic table whose trailer points at a
            // cross-reference stream for the objects in object streams.
            if let Some(stream) = trailer.get(b"XRefStm").and_then(Object::as_int)
                && let Ok(stream) = usize::try_from(stream)
                && seen.insert(stream)
            {
                file.read_section(stream);
            }
            next = trailer.get(b"Prev").and_then(Object::as_int).and_then(|p| usize::try_from(p).ok());
            if newest {
                file.trailer = trailer;
                newest = false;
            }
        }
        Some(file)
    }

    /// Reads one cross-reference section; newer sections were read first, so
    /// an entry already known wins.
    pub(crate) fn read_section(&mut self, offset: usize) -> Option<Dict> {
        let mut parser = Parser::new(self.bytes, offset);
        if parser.word() == b"xref" {
            loop {
                let save = parser.pos;
                let word = parser.word();
                if word == b"trailer" {
                    return parser.object(0)?.as_dict().cloned();
                }
                parser.pos = save;
                let first = parser.unsigned()?;
                let count = parser.unsigned()?;
                for number in first..first.checked_add(count)? {
                    let field = parser.unsigned()?;
                    let _generation = parser.unsigned()?;
                    let kind = parser.word();
                    let number = u32::try_from(number).ok()?;
                    if kind == b"n" && field > 0 {
                        self.entries.entry(number).or_insert(Entry::Offset(usize::try_from(field).ok()?));
                    }
                }
            }
        }
        // A cross-reference stream: `n g obj << /Type /XRef … >> stream`.
        let (dict, data) = self.indirect_at(offset)?;
        let stream = data?;
        if dict.get(b"Type").and_then(Object::as_name) != Some(b"XRef") {
            return None;
        }
        let data = decode_stream(&dict, stream, self)?;
        let widths: Vec<usize> = dict
            .get(b"W")?
            .as_array()?
            .iter()
            .map(|w| w.as_int().and_then(|w| usize::try_from(w).ok()).filter(|w| *w <= 8))
            .collect::<Option<_>>()?;
        if widths.len() != 3 {
            return None;
        }
        let row = widths.iter().sum::<usize>();
        if row == 0 {
            return None;
        }
        let size = dict.get(b"Size").and_then(Object::as_int).unwrap_or(0);
        let index: Vec<i64> = match dict.get(b"Index").and_then(Object::as_array) {
            Some(index) => index.iter().map(Object::as_int).collect::<Option<_>>()?,
            None => vec![0, size],
        };
        let mut rows = data.chunks_exact(row);
        for pair in index.chunks(2) {
            let (&first, &count) = (pair.first()?, pair.get(1)?);
            for number in first..first.checked_add(count)? {
                let Some(row) = rows.next() else { break };
                let field = |i: usize, default: u64| -> u64 {
                    let start: usize = widths[..i].iter().sum();
                    if widths[i] == 0 {
                        return default;
                    }
                    row[start..start + widths[i]].iter().fold(0u64, |acc, b| acc << 8 | u64::from(*b))
                };
                let number = u32::try_from(number).ok()?;
                let entry = match field(0, 1) {
                    1 => Entry::Offset(usize::try_from(field(1, 0)).ok()?),
                    2 => Entry::InStream {
                        stream: u32::try_from(field(1, 0)).ok()?,
                        index: usize::try_from(field(2, 0)).ok()?,
                    },
                    _ => continue,
                };
                self.entries.entry(number).or_insert(entry);
            }
        }
        Some(dict)
    }

    /// The object at `offset` (`n g obj … endobj`): its value, and the byte
    /// range of its stream data if it has one.
    #[allow(clippy::type_complexity)]
    pub(crate) fn indirect_at(&self, offset: usize) -> Option<(Dict, Option<&'a [u8]>)> {
        let (object, stream) = self.indirect_object_at(offset)?;
        Some((object.as_dict()?.clone(), stream))
    }

    pub(crate) fn indirect_object_at(&self, offset: usize) -> Option<(Object, Option<&'a [u8]>)> {
        let mut parser = Parser::new(self.bytes, offset);
        parser.unsigned()?;
        parser.unsigned()?;
        parser.keyword(b"obj")?;
        let object = parser.object(0)?;
        if parser.word() != b"stream" {
            return Some((object, None));
        }
        // The data starts after the end-of-line that follows `stream`.
        let mut start = parser.pos;
        if self.bytes.get(start) == Some(&b'\r') {
            start += 1;
        }
        if self.bytes.get(start) == Some(&b'\n') {
            start += 1;
        }
        let declared = object
            .as_dict()
            .and_then(|d| d.get(b"Length"))
            .and_then(|length| match length {
                Object::Ref(..) => self.resolve_shallow(length),
                other => Some(other.clone()),
            })
            .and_then(|length| length.as_int())
            .and_then(|length| usize::try_from(length).ok());
        let end = match declared {
            Some(length) if start.checked_add(length).is_some_and(|end| end <= self.bytes.len()) => {
                start + length
            }
            _ => start + find(&self.bytes[start..], b"endstream")?,
        };
        Some((object, Some(&self.bytes[start..end])))
    }

    /// Resolves a reference to an object stored directly in the file (used
    /// for stream lengths, which cannot live in object streams).
    pub(crate) fn resolve_shallow(&self, object: &Object) -> Option<Object> {
        let Object::Ref(number, _) = object else { return Some(object.clone()) };
        match self.entries.get(number)? {
            Entry::Offset(offset) => Some(self.indirect_object_at(*offset)?.0),
            Entry::InStream { .. } => None,
        }
    }

    /// Follows a reference (or a chain of them) to its value.
    pub(crate) fn resolve(&self, object: &Object) -> Option<Object> {
        let mut current = object.clone();
        for _ in 0..MAX_DEPTH {
            let Object::Ref(number, _) = current else { return Some(current) };
            current = match *self.entries.get(&number)? {
                Entry::Offset(offset) => self.indirect_object_at(offset)?.0,
                Entry::InStream { stream, index } => self.in_object_stream(stream, index, number)?,
            };
        }
        None
    }

    pub(crate) fn in_object_stream(&self, stream: u32, index: usize, number: u32) -> Option<Object> {
        // Object streams are encrypted along with everything else; this
        // reader does not decrypt.
        if self.trailer.get(b"Encrypt").is_some() {
            return None;
        }
        let Entry::Offset(offset) = *self.entries.get(&stream)? else { return None };
        let (dict, data) = self.indirect_at(offset)?;
        if dict.get(b"Type").and_then(Object::as_name) != Some(b"ObjStm") {
            return None;
        }
        let data = decode_stream(&dict, data?, self)?;
        let count = usize::try_from(dict.get(b"N")?.as_int()?).ok()?;
        let first = usize::try_from(dict.get(b"First")?.as_int()?).ok()?;
        if index >= count {
            return None;
        }
        let mut header = Parser::new(&data, 0);
        let mut found = None;
        for i in 0..count {
            let n = header.unsigned()?;
            let at = usize::try_from(header.unsigned()?).ok()?;
            if i == index {
                if u64::from(number) != n {
                    return None;
                }
                found = Some(at);
                break;
            }
        }
        Parser::new(&data, first.checked_add(found?)?).object(0)
    }
}

pub(crate) fn startxref(bytes: &[u8]) -> Option<usize> {
    let tail = &bytes[bytes.len().saturating_sub(2048)..];
    let at = rfind(tail, b"startxref")?;
    let mut parser = Parser::new(tail, at + b"startxref".len());
    usize::try_from(parser.unsigned()?).ok().filter(|offset| *offset < bytes.len())
}

pub(crate) fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

pub(crate) fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).rposition(|window| window == needle)
}

/// Applies the stream's filters. Only Flate (with PNG predictors, as
/// cross-reference streams use) is supported; anything else gives up.
pub(crate) fn decode_stream(dict: &Dict, data: &[u8], file: &File<'_>) -> Option<Vec<u8>> {
    let filter = dict.get(b"Filter").and_then(|f| file.resolve_shallow(f));
    let filters: Vec<Vec<u8>> = match filter {
        None | Some(Object::Null) => Vec::new(),
        Some(Object::Name(name)) => vec![name],
        Some(Object::Array(items)) => {
            items.iter().map(|i| i.as_name().map(<[u8]>::to_vec)).collect::<Option<_>>()?
        }
        _ => return None,
    };
    let params = dict.get(b"DecodeParms").and_then(|p| file.resolve_shallow(p));
    let params = match params {
        Some(Object::Array(items)) => items.first().and_then(Object::as_dict).cloned(),
        Some(Object::Dict(d)) => Some(d),
        _ => None,
    };
    let mut data = data.to_vec();
    for filter in filters {
        if filter != b"FlateDecode" && filter != b"Fl" {
            return None;
        }
        data = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&data, MAX_INFLATED).ok()?;
        if let Some(params) = &params {
            data = unpredict(&data, params)?;
        }
    }
    Some(data)
}

/// Reverses a PNG predictor (`/Predictor` 10–15), one byte per component.
fn unpredict(data: &[u8], params: &Dict) -> Option<Vec<u8>> {
    let predictor = params.get(b"Predictor").and_then(Object::as_int).unwrap_or(1);
    if predictor < 10 {
        return (predictor == 1).then(|| data.to_vec());
    }
    let colors = params.get(b"Colors").and_then(Object::as_int).unwrap_or(1);
    let bits = params.get(b"BitsPerComponent").and_then(Object::as_int).unwrap_or(8);
    let columns = params.get(b"Columns").and_then(Object::as_int).unwrap_or(1);
    if bits != 8 || !(1..=4).contains(&colors) || columns < 1 {
        return None;
    }
    let bpp = usize::try_from(colors).ok()?;
    let width = usize::try_from(columns).ok()?.checked_mul(bpp)?;
    let mut out = Vec::with_capacity(data.len());
    let mut previous = vec![0u8; width];
    for chunk in data.chunks(width + 1) {
        let (&kind, row) = chunk.split_first()?;
        if row.len() != width {
            break;
        }
        let mut current = vec![0u8; width];
        for i in 0..width {
            let left = if i >= bpp { current[i - bpp] } else { 0 };
            let up = previous[i];
            let up_left = if i >= bpp { previous[i - bpp] } else { 0 };
            current[i] = row[i].wrapping_add(match kind {
                0 => 0,
                1 => left,
                2 => up,
                3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
                4 => paeth(left, up, up_left),
                _ => return None,
            });
        }
        out.extend_from_slice(&current);
        previous = current;
    }
    Some(out)
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let (pa, pb, pc) = ((p - i16::from(a)).abs(), (p - i16::from(b)).abs(), (p - i16::from(c)).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_strings_parse_with_escapes() {
        let mut parser = Parser::new(b"/A#20B (a\\)b(c)) <48 49> [1 2 0 R 3.5]", 0);
        assert_eq!(parser.object(0), Some(Object::Name(b"A B".to_vec())));
        assert_eq!(parser.object(0), Some(Object::Str(b"a)b(c)".to_vec())));
        assert_eq!(parser.object(0), Some(Object::Str(b"HI".to_vec())));
        assert_eq!(
            parser.object(0),
            Some(Object::Array(vec![Object::Int(1), Object::Ref(2, 0), Object::Real(3.5)]))
        );
    }
}
