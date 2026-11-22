//! A minimal, read-only reader for a PDF's document catalog: just enough to
//! learn the document's own reading preferences, which PDFium exposes only
//! through an `unsafe` call (`FPDF_VIEWERREF_GetName`).
//!
//! - `/ViewerPreferences /Direction /R2L`: pages are bound on the right, so a
//!   two-page spread puts the earlier page on the right (spec section 6.5;
//!   most Arabic books).
//! - `/PageLayout`: the layout the author asked for (single, continuous,
//!   two-page with the cover alone or not).
//!
//! It follows the cross-reference chain from the end of the file (classic
//! tables, cross-reference streams, hybrid files, incremental updates) to the
//! catalog, decompressing object streams with Flate when the catalog lives in
//! one. Input is untrusted: every loop, nesting level, and decompressed size is
//! bounded, and anything unexpected (damage, other filters, encrypted object
//! streams) yields "no preference" rather than an error. PDFium still renders
//! such files; they just open left to right.

use std::collections::{HashMap, HashSet};

/// The document's reading preferences, where it states them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CatalogHints {
    pub direction: Option<ReadingDirection>,
    pub page_layout: Option<PageLayout>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadingDirection {
    LeftToRight,
    RightToLeft,
}

/// `/PageLayout` values (ISO 32000-1, table 28).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageLayout {
    SinglePage,
    OneColumn,
    /// Two columns, odd pages on the left.
    TwoColumnLeft,
    /// Two columns, odd pages on the right (the first page alone).
    TwoColumnRight,
    TwoPageLeft,
    TwoPageRight,
}

/// Reads the catalog's hints; empty when the file does not state them or
/// cannot be read this way.
pub fn catalog_hints(bytes: &[u8]) -> CatalogHints {
    read_hints(bytes).unwrap_or_default()
}

const MAX_DEPTH: usize = 64;
const MAX_XREF_SECTIONS: usize = 256;
const MAX_INFLATED: usize = 64 * 1024 * 1024;

fn read_hints(bytes: &[u8]) -> Option<CatalogHints> {
    let file = File::new(bytes)?;
    let catalog = file.resolve(file.trailer.get(b"Root")?)?;
    let catalog = catalog.as_dict()?;
    let direction = catalog
        .get(b"ViewerPreferences")
        .and_then(|prefs| file.resolve(prefs))
        .and_then(|prefs| prefs.as_dict().and_then(|d| d.get(b"Direction")).cloned())
        .and_then(|direction| match file.resolve(&direction)?.as_name()? {
            b"R2L" => Some(ReadingDirection::RightToLeft),
            b"L2R" => Some(ReadingDirection::LeftToRight),
            _ => None,
        });
    let page_layout = catalog.get(b"PageLayout").and_then(|layout| {
        Some(match file.resolve(layout)?.as_name()? {
            b"SinglePage" => PageLayout::SinglePage,
            b"OneColumn" => PageLayout::OneColumn,
            b"TwoColumnLeft" => PageLayout::TwoColumnLeft,
            b"TwoColumnRight" => PageLayout::TwoColumnRight,
            b"TwoPageLeft" => PageLayout::TwoPageLeft,
            b"TwoPageRight" => PageLayout::TwoPageRight,
            _ => return None,
        })
    });
    Some(CatalogHints { direction, page_layout })
}

// ---------------------------------------------------------------- objects

#[derive(Debug, Clone, PartialEq)]
enum Object {
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
struct Dict(Vec<(Vec<u8>, Object)>);

impl Dict {
    fn get(&self, key: &[u8]) -> Option<&Object> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

impl Object {
    fn as_dict(&self) -> Option<&Dict> {
        match self {
            Object::Dict(d) => Some(d),
            _ => None,
        }
    }
    fn as_name(&self) -> Option<&[u8]> {
        match self {
            Object::Name(n) => Some(n),
            _ => None,
        }
    }
    fn as_int(&self) -> Option<i64> {
        match self {
            Object::Int(i) => Some(*i),
            Object::Real(r) if r.fract() == 0.0 => Some(*r as i64),
            _ => None,
        }
    }
    fn as_array(&self) -> Option<&[Object]> {
        match self {
            Object::Array(a) => Some(a),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------- lexer and parser

fn is_white(b: u8) -> bool {
    matches!(b, 0 | b'\t' | b'\n' | 0x0C | b'\r' | b' ')
}

fn is_delim(b: u8) -> bool {
    matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(bytes: &'a [u8], pos: usize) -> Self {
        Self { bytes, pos }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_space(&mut self) {
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
    fn word(&mut self) -> &'a [u8] {
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

    fn keyword(&mut self, expected: &[u8]) -> Option<()> {
        (self.word() == expected).then_some(())
    }

    fn unsigned(&mut self) -> Option<u64> {
        let word = self.word();
        if word.is_empty() || !word.iter().all(u8::is_ascii_digit) {
            return None;
        }
        std::str::from_utf8(word).ok()?.parse().ok()
    }

    fn object(&mut self, depth: usize) -> Option<Object> {
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

    fn name_body(&mut self) -> Vec<u8> {
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

    fn literal_string(&mut self) -> Option<Vec<u8>> {
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

    fn hex_string(&mut self) -> Option<Vec<u8>> {
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
enum Entry {
    Offset(usize),
    InStream { stream: u32, index: usize },
}

struct File<'a> {
    bytes: &'a [u8],
    entries: HashMap<u32, Entry>,
    trailer: Dict,
}

impl<'a> File<'a> {
    fn new(bytes: &'a [u8]) -> Option<Self> {
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
    fn read_section(&mut self, offset: usize) -> Option<Dict> {
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
    fn indirect_at(&self, offset: usize) -> Option<(Dict, Option<&'a [u8]>)> {
        let (object, stream) = self.indirect_object_at(offset)?;
        Some((object.as_dict()?.clone(), stream))
    }

    fn indirect_object_at(&self, offset: usize) -> Option<(Object, Option<&'a [u8]>)> {
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
    fn resolve_shallow(&self, object: &Object) -> Option<Object> {
        let Object::Ref(number, _) = object else { return Some(object.clone()) };
        match self.entries.get(number)? {
            Entry::Offset(offset) => Some(self.indirect_object_at(*offset)?.0),
            Entry::InStream { .. } => None,
        }
    }

    /// Follows a reference (or a chain of them) to its value.
    fn resolve(&self, object: &Object) -> Option<Object> {
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

    fn in_object_stream(&self, stream: u32, index: usize, number: u32) -> Option<Object> {
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

fn startxref(bytes: &[u8]) -> Option<usize> {
    let tail = &bytes[bytes.len().saturating_sub(2048)..];
    let at = rfind(tail, b"startxref")?;
    let mut parser = Parser::new(tail, at + b"startxref".len());
    usize::try_from(parser.unsigned()?).ok().filter(|offset| *offset < bytes.len())
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).rposition(|window| window == needle)
}

/// Applies the stream's filters. Only Flate (with PNG predictors, as
/// cross-reference streams use) is supported; anything else gives up.
fn decode_stream(dict: &Dict, data: &[u8], file: &File<'_>) -> Option<Vec<u8>> {
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

    /// A one-page PDF with a classic cross-reference table. `catalog` is the
    /// catalog dictionary's body; `extra` are further objects from number 4.
    fn classic(catalog: &str, extra: &[&str]) -> Vec<u8> {
        let mut objects = vec![
            format!("<< /Type /Catalog /Pages 2 0 R {catalog} >>"),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 3 2] >>".to_string(),
        ];
        objects.extend(extra.iter().map(|s| s.to_string()));
        let mut pdf = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1)
                .as_bytes(),
        );
        pdf
    }

    /// Appends an incremental update replacing the catalog.
    fn update_catalog(mut pdf: Vec<u8>, catalog: &str) -> Vec<u8> {
        let previous = startxref(&pdf).expect("startxref");
        let offset = pdf.len();
        pdf.extend_from_slice(
            format!("1 0 obj\n<< /Type /Catalog /Pages 2 0 R {catalog} >>\nendobj\n").as_bytes(),
        );
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!(
                "xref\n1 1\n{offset:010} 00000 n \ntrailer\n<< /Size 4 /Root 1 0 R /Prev {previous} >>\nstartxref\n{xref}\n%%EOF\n"
            )
            .as_bytes(),
        );
        pdf
    }

    /// The same document with the catalog in a Flate-compressed object stream,
    /// indexed by a cross-reference stream with a PNG Up predictor.
    fn with_object_stream(catalog: &str, encrypted: bool) -> Vec<u8> {
        let catalog = format!("<< /Type /Catalog /Pages 2 0 R {catalog} >>");
        let header = "1 0 ";
        let objstm_data = format!("{header}{catalog}");
        let objstm = miniz_oxide::deflate::compress_to_vec_zlib(objstm_data.as_bytes(), 6);
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = HashMap::new();
        offsets.insert(2u32, pdf.len());
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        offsets.insert(3, pdf.len());
        pdf.extend_from_slice(b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 3 2] >>\nendobj\n");
        offsets.insert(4, pdf.len());
        pdf.extend_from_slice(
            format!(
                "4 0 obj\n<< /Type /ObjStm /N 1 /First {} /Filter /FlateDecode /Length {} >>\nstream\n",
                header.len(),
                objstm.len()
            )
            .as_bytes(),
        );
        pdf.extend_from_slice(&objstm);
        pdf.extend_from_slice(b"\nendstream\nendobj\n");
        let xref_offset = pdf.len();
        // Rows: type (1 byte), field 2 (4 bytes), field 3 (1 byte), each
        // with a PNG "Up" filter byte, for objects 0..=5.
        let rows: Vec<[u8; 6]> = vec![
            [0, 0, 0, 0, 0, 0],
            [2, 0, 0, 0, 4, 0],
            row(1, offsets[&2]),
            row(1, offsets[&3]),
            row(1, offsets[&4]),
            row(1, xref_offset),
        ];
        let mut raw = Vec::new();
        let mut previous = [0u8; 6];
        for current in &rows {
            raw.push(2); // Up
            for i in 0..6 {
                raw.push(current[i].wrapping_sub(previous[i]));
            }
            previous = *current;
        }
        let data = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
        let encrypt = if encrypted { "/Encrypt << /Filter /Standard >>" } else { "" };
        pdf.extend_from_slice(
            format!(
                "5 0 obj\n<< /Type /XRef /Size 6 /W [1 4 1] /Root 1 0 R {encrypt} /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 6 >> /Length {} >>\nstream\n",
                data.len()
            )
            .as_bytes(),
        );
        pdf.extend_from_slice(&data);
        pdf.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref_offset}\n%%EOF\n").as_bytes());
        pdf
    }

    fn row(kind: u8, offset: usize) -> [u8; 6] {
        let o = u32::try_from(offset).expect("small").to_be_bytes();
        [kind, o[0], o[1], o[2], o[3], 0]
    }

    const RTL: CatalogHints = CatalogHints {
        direction: Some(ReadingDirection::RightToLeft),
        page_layout: Some(PageLayout::TwoPageRight),
    };

    #[test]
    fn reads_direction_and_layout_from_a_classic_file() {
        let pdf = classic("/ViewerPreferences << /Direction /R2L >> /PageLayout /TwoPageRight", &[]);
        assert_eq!(catalog_hints(&pdf), RTL);
        let pdf = classic("/ViewerPreferences 4 0 R", &["<< /Direction /L2R >>"]);
        assert_eq!(catalog_hints(&pdf).direction, Some(ReadingDirection::LeftToRight));
        assert_eq!(catalog_hints(&classic("", &[])), CatalogHints::default());
    }

    #[test]
    fn the_newest_incremental_update_wins() {
        let pdf = classic("/ViewerPreferences << /Direction /L2R >>", &[]);
        let pdf = update_catalog(pdf, "/ViewerPreferences << /Direction /R2L >> /PageLayout /TwoPageRight");
        assert_eq!(catalog_hints(&pdf), RTL);
    }

    #[test]
    fn follows_cross_reference_and_object_streams() {
        let pdf =
            with_object_stream("/ViewerPreferences << /Direction /R2L >> /PageLayout /TwoPageRight", false);
        assert_eq!(catalog_hints(&pdf), RTL);
        // Encrypted object streams cannot be read without the key: no hints.
        let pdf = with_object_stream("/ViewerPreferences << /Direction /R2L >>", true);
        assert_eq!(catalog_hints(&pdf), CatalogHints::default());
    }

    #[test]
    fn damaged_or_hostile_input_gives_no_hints() {
        assert_eq!(catalog_hints(b""), CatalogHints::default());
        assert_eq!(catalog_hints(b"%PDF-1.7\nstartxref\n999999\n%%EOF"), CatalogHints::default());
        let pdf = classic("/ViewerPreferences << /Direction /R2L >>", &[]);
        assert_eq!(catalog_hints(&pdf[..pdf.len() / 2]), CatalogHints::default());
        // A /Prev that points at itself, and deep nesting, terminate.
        let mut looped = classic("", &[]);
        let at = startxref(&looped).expect("startxref");
        let tail = format!("trailer\n<< /Size 4 /Root 1 0 R /Prev {at} >>\nstartxref\n{at}\n%%EOF\n");
        looped.extend_from_slice(tail.as_bytes());
        let _ = catalog_hints(&looped);
        let deep = format!("/X {}{}", "[".repeat(10_000), "]".repeat(10_000));
        let _ = catalog_hints(&classic(&deep, &[]));
    }

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
