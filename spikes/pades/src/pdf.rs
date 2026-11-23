//! The PDF side of a signature (ISO 32000-1, 12.8; ETSI EN 319 142-1): an
//! incremental update that adds a signature field, a `/Sig` dictionary with
//! a `/ByteRange` and a `/Contents` placeholder, and changes nothing before it.
//!
//! The spike reads the file with lopdf and writes the update by hand. The
//! product would build on `mirsam-core`'s own `pdf::write::Update`, which
//! already writes updates in both cross-reference forms.

use std::fmt::Write as _;

use lopdf::{Dictionary, Document, Object, ObjectId};

/// Bytes reserved for the CMS signature (hex doubles it in the file).
pub const DEFAULT_RESERVED: usize = 16 * 1024;

pub struct Prepared {
    /// The whole file with the placeholder in it.
    pub bytes: Vec<u8>,
    /// Where `<` and the byte after `>` of /Contents are.
    pub contents: (usize, usize),
    /// Where the four /ByteRange numbers sit (10 digits each).
    byte_range_at: usize,
}

pub struct Appearance<'a> {
    pub name: &'a str,
    pub reason: &'a str,
    pub location: &'a str,
}

/// PDF text string: PDFDocEncoding when ASCII, else UTF-16BE with a BOM, as
/// hex (so any bytes survive).
fn text(value: &str) -> String {
    let mut bytes: Vec<u8> = Vec::new();
    if value.is_ascii() {
        bytes.extend_from_slice(value.as_bytes());
    } else {
        bytes.extend_from_slice(&[0xFE, 0xFF]);
        for unit in value.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
    }
    let mut out = String::from("<");
    for byte in bytes {
        let _ = write!(out, "{byte:02X}");
    }
    out.push('>');
    out
}

fn object(out: &mut String, value: &Object) {
    match value {
        Object::Null => out.push_str("null"),
        Object::Boolean(b) => out.push_str(if *b { "true" } else { "false" }),
        Object::Integer(i) => {
            let _ = write!(out, "{i}");
        }
        Object::Real(r) => {
            let _ = write!(out, "{r}");
        }
        Object::Name(name) => {
            out.push('/');
            for &b in name {
                if b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_' {
                    out.push(char::from(b));
                } else {
                    let _ = write!(out, "#{b:02X}");
                }
            }
        }
        Object::String(bytes, _) => {
            out.push('<');
            for b in bytes {
                let _ = write!(out, "{b:02X}");
            }
            out.push('>');
        }
        Object::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                object(out, item);
            }
            out.push(']');
        }
        Object::Dictionary(dict) => dictionary(out, dict),
        Object::Reference((n, g)) => {
            let _ = write!(out, "{n} {g} R");
        }
        Object::Stream(_) => out.push_str("null"),
    }
}

fn dictionary(out: &mut String, dict: &Dictionary) {
    out.push_str("<<");
    for (key, value) in dict.iter() {
        object(out, &Object::Name(key.clone()));
        out.push(' ');
        object(out, value);
        out.push(' ');
    }
    out.push_str(">>");
}

fn find_last(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).rposition(|w| w == needle)
}

/// Adds the signature field to `original`, leaving a placeholder. Classic
/// cross-reference tables only (the spike's fixtures).
pub fn prepare(original: &[u8], reserved: usize, appearance: &Appearance<'_>, when: &str) -> Prepared {
    let doc = Document::load_mem(original).expect("lopdf reads the file");
    assert!(
        !original.windows(5).rev().take(2048).any(|w| w == b"/XRef"),
        "the spike handles classic cross-reference tables only"
    );
    let root: ObjectId = doc.trailer.get(b"Root").and_then(Object::as_reference).expect("root");
    let info = doc.trailer.get(b"Info").ok().cloned();
    let id = doc.trailer.get(b"ID").ok().cloned();
    let page_id = *doc.get_pages().values().next().expect("a page");
    let mut next = doc.max_id + 1;
    let (sig_n, widget_n) = (next, next + 1);
    next += 2;

    let tail = &original[original.len().saturating_sub(1024)..];
    let at = find_last(tail, b"startxref").expect("startxref");
    let prev: usize = std::str::from_utf8(&tail[at + 9..])
        .expect("ascii")
        .split_whitespace()
        .next()
        .and_then(|n| n.parse().ok())
        .expect("startxref offset");

    let mut out: Vec<u8> = original.to_vec();
    if !out.ends_with(b"\n") {
        out.push(b'\n');
    }
    let mut offsets: Vec<(u32, usize)> = Vec::new();

    // The signature dictionary, with the placeholders.
    offsets.push((sig_n, out.len()));
    let mut sig = String::new();
    let _ = write!(
        sig,
        "{sig_n} 0 obj\n<</Type/Sig/Filter/Adobe.PPKLite/SubFilter/ETSI.CAdES.detached/Name {} /Reason {} /Location {} /M (D:{when}Z) /ByteRange ",
        text(appearance.name),
        text(appearance.reason),
        text(appearance.location),
    );
    out.extend_from_slice(sig.as_bytes());
    let byte_range_at = out.len() + 1;
    out.extend_from_slice(b"[0000000000 0000000000 0000000000 0000000000] /Contents ");
    let contents_start = out.len();
    out.push(b'<');
    out.extend(std::iter::repeat_n(b'0', reserved * 2));
    out.push(b'>');
    let contents_end = out.len();
    out.extend_from_slice(b">>\nendobj\n");

    // The widget (an invisible signature field) on the first page.
    offsets.push((widget_n, out.len()));
    let widget = format!(
        "{widget_n} 0 obj\n<</Type/Annot/Subtype/Widget/FT/Sig/T {} /V {sig_n} 0 R /F 132 /Rect [0 0 0 0] /P {} {} R>>\nendobj\n",
        text("Signature1"),
        page_id.0,
        page_id.1
    );
    out.extend_from_slice(widget.as_bytes());

    // The page, with the widget in /Annots.
    let mut page = doc.get_dictionary(page_id).expect("page").clone();
    let annots = match page.get(b"Annots") {
        Ok(Object::Array(items)) => Some(items.clone()),
        Ok(Object::Reference(r)) => doc.get_object(*r).ok().and_then(|o| o.as_array().ok()).cloned(),
        _ => None,
    };
    let mut annots = annots.unwrap_or_default();
    annots.push(Object::Reference((widget_n, 0)));
    page.set("Annots", Object::Array(annots));
    offsets.push((page_id.0, out.len()));
    let mut text_page = format!("{} {} obj\n", page_id.0, page_id.1);
    dictionary(&mut text_page, &page);
    text_page.push_str("\nendobj\n");
    out.extend_from_slice(text_page.as_bytes());

    // The catalog, with the field in /AcroForm and SigFlags 3 (signatures
    // exist; append only).
    let mut catalog = doc.get_dictionary(root).expect("catalog").clone();
    let mut form = match catalog.get(b"AcroForm") {
        Ok(Object::Dictionary(d)) => d.clone(),
        Ok(Object::Reference(r)) => doc.get_dictionary(*r).cloned().unwrap_or_default(),
        _ => Dictionary::new(),
    };
    let mut fields = match form.get(b"Fields") {
        Ok(Object::Array(items)) => items.clone(),
        _ => Vec::new(),
    };
    fields.push(Object::Reference((widget_n, 0)));
    form.set("Fields", Object::Array(fields));
    form.set("SigFlags", Object::Integer(3));
    catalog.set("AcroForm", Object::Dictionary(form));
    offsets.push((root.0, out.len()));
    let mut text_catalog = format!("{} {} obj\n", root.0, root.1);
    dictionary(&mut text_catalog, &catalog);
    text_catalog.push_str("\nendobj\n");
    out.extend_from_slice(text_catalog.as_bytes());

    // Cross-reference section and trailer.
    offsets.sort_unstable();
    let xref = out.len();
    let mut table = String::from("xref\n");
    let mut i = 0;
    while i < offsets.len() {
        let mut j = i + 1;
        while j < offsets.len() && offsets[j].0 == offsets[j - 1].0 + 1 {
            j += 1;
        }
        let _ = writeln!(table, "{} {}", offsets[i].0, j - i);
        for (n, offset) in &offsets[i..j] {
            let generation = if *n == root.0 {
                root.1
            } else if *n == page_id.0 {
                page_id.1
            } else {
                0
            };
            let _ = write!(table, "{offset:010} {generation:05} n\r\n");
        }
        i = j;
    }
    let mut trailer = Dictionary::new();
    trailer.set("Size", Object::Integer(i64::from(next)));
    trailer.set("Root", Object::Reference(root));
    if let Some(info) = info {
        trailer.set("Info", info);
    }
    if let Some(id) = id {
        trailer.set("ID", id);
    }
    trailer.set("Prev", Object::Integer(i64::try_from(prev).expect("fits")));
    let mut text_trailer = String::from("trailer\n");
    dictionary(&mut text_trailer, &trailer);
    let _ = write!(text_trailer, "\nstartxref\n{xref}\n%%EOF\n");
    out.extend_from_slice(table.as_bytes());
    out.extend_from_slice(text_trailer.as_bytes());

    Prepared { bytes: out, contents: (contents_start, contents_end), byte_range_at }
}

impl Prepared {
    /// Fills in /ByteRange and returns the two ranges the signature covers.
    pub fn byte_range(&mut self) -> [usize; 4] {
        let (start, end) = self.contents;
        let range = [0, start, end, self.bytes.len() - end];
        let text = format!("{:010} {:010} {:010} {:010}", range[0], range[1], range[2], range[3]);
        self.bytes[self.byte_range_at..self.byte_range_at + text.len()].copy_from_slice(text.as_bytes());
        range
    }

    /// The bytes the signature covers: everything but the /Contents value.
    pub fn signed_content(&self) -> Vec<u8> {
        let (start, end) = self.contents;
        let mut out = Vec::with_capacity(self.bytes.len() - (end - start));
        out.extend_from_slice(&self.bytes[..start]);
        out.extend_from_slice(&self.bytes[end..]);
        out
    }

    /// Writes the DER signature into the placeholder (hex, zero padded).
    pub fn insert(&mut self, der: &[u8]) -> Result<(), usize> {
        let (start, end) = self.contents;
        let room = (end - start - 2) / 2;
        if der.len() > room {
            return Err(der.len());
        }
        let mut hex = String::with_capacity(der.len() * 2);
        for b in der {
            let _ = write!(hex, "{b:02X}");
        }
        self.bytes[start + 1..start + 1 + hex.len()].copy_from_slice(hex.as_bytes());
        Ok(())
    }
}

/// Reads /ByteRange and /Contents out of the last signature in a file.
pub fn extract(bytes: &[u8]) -> Option<([usize; 4], Vec<u8>)> {
    let at = find_last(bytes, b"/ByteRange")?;
    let rest = std::str::from_utf8(&bytes[at + 10..at + 10 + 80]).ok()?;
    let open = rest.find('[')?;
    let close = rest.find(']')?;
    let numbers: Vec<usize> =
        rest[open + 1..close].split_whitespace().filter_map(|n| n.parse().ok()).collect();
    let range: [usize; 4] = numbers.try_into().ok()?;
    let contents = bytes[range[1]..range[2]].to_vec();
    let hex = std::str::from_utf8(&contents[1..contents.len() - 1]).ok()?;
    let mut der = Vec::with_capacity(hex.len() / 2);
    for pair in hex.as_bytes().chunks(2) {
        der.push(u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?);
    }
    // The placeholder is zero padded: keep the DER structure only.
    let length = der_length(&der)?;
    der.truncate(length);
    Some((range, der))
}

/// The length of the DER value at the start of `bytes`, header included.
fn der_length(bytes: &[u8]) -> Option<usize> {
    let first = *bytes.get(1)?;
    if first < 0x80 {
        return Some(2 + usize::from(first));
    }
    let count = usize::from(first & 0x7F);
    let body = bytes.get(2..2 + count)?.iter().fold(0usize, |n, b| (n << 8) | usize::from(*b));
    Some(2 + count + body)
}

/// The bytes a `[a b c d]` range covers.
pub fn covered(bytes: &[u8], range: [usize; 4]) -> Vec<u8> {
    let mut out = bytes[range[0]..range[0] + range[1]].to_vec();
    out.extend_from_slice(&bytes[range[2]..range[2] + range[3]]);
    out
}
