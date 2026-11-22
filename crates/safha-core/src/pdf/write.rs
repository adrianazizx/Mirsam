//! Writing PDF syntax: objects, and incremental updates (ISO 32000-1, 7.5.6).
//!
//! An incremental update appends changed and new objects after the end of the
//! file, then a cross-reference section for just those objects and a trailer
//! whose `/Prev` points at the previous section. Everything before stays byte
//! for byte as it was, so earlier revisions (and the signatures over them)
//! remain valid, and dropping the update restores the previous revision.
//!
//! The new section has the same form as the file's newest one: a classic table
//! after a classic table, an uncompressed cross-reference stream after a
//! stream (a file that uses streams may keep objects in object streams, which
//! a classic table cannot describe for readers that look only at the newest
//! section).

use std::io::Write as _;

use super::{Dict, Object};

/// Appends `object` in PDF syntax.
///
/// Strings are written as hexadecimal so their bytes come out exactly as they
/// were read; in an encrypted file they stay valid as long as they keep their
/// object number, which is what an update that rewrites an object does.
pub(crate) fn object(out: &mut Vec<u8>, object: &Object) {
    match object {
        Object::Null => out.extend_from_slice(b"null"),
        Object::Bool(value) => out.extend_from_slice(if *value { b"true" } else { b"false" }),
        Object::Int(value) => {
            let _ = write!(out, "{value}");
        }
        Object::Real(value) => real(out, *value),
        Object::Name(name) => self::name(out, name),
        Object::Str(bytes) => {
            out.push(b'<');
            for byte in bytes {
                let _ = write!(out, "{byte:02X}");
            }
            out.push(b'>');
        }
        Object::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b' ');
                }
                self::object(out, item);
            }
            out.push(b']');
        }
        Object::Dict(dict) => self::dict(out, dict),
        Object::Ref(number, generation) => {
            let _ = write!(out, "{number} {generation} R");
        }
    }
}

pub(crate) fn dict(out: &mut Vec<u8>, dict: &Dict) {
    out.extend_from_slice(b"<<");
    for (key, value) in &dict.0 {
        name(out, key);
        out.push(b' ');
        object(out, value);
        out.push(b' ');
    }
    out.extend_from_slice(b">>");
}

fn name(out: &mut Vec<u8>, name: &[u8]) {
    out.push(b'/');
    for &byte in name {
        let regular = byte.is_ascii_graphic() && !super::is_delim(byte) && byte != b'#';
        if regular {
            out.push(byte);
        } else {
            let _ = write!(out, "#{byte:02X}");
        }
    }
}

/// Reals without exponents (PDF has none), trailing zeros trimmed.
fn real(out: &mut Vec<u8>, value: f64) {
    if !value.is_finite() {
        out.push(b'0');
        return;
    }
    let text = format!("{value:.6}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    out.extend_from_slice(if text.is_empty() || text == "-" { b"0" } else { text.as_bytes() });
}

/// The form of a file's newest cross-reference section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum XrefForm {
    Table,
    Stream,
}

/// An incremental update being assembled: objects, then the section.
pub(crate) struct Update {
    /// Bytes appended after the original file.
    bytes: Vec<u8>,
    /// Length of the original file (offsets count from its start).
    base: usize,
    /// (object number, generation, offset) of every object written.
    entries: Vec<(u32, u16, usize)>,
}

impl Update {
    pub(crate) fn new(original: &[u8]) -> Self {
        // Updates start on a line of their own.
        let mut bytes = Vec::new();
        if !original.ends_with(b"\n") && !original.ends_with(b"\r") {
            bytes.push(b'\n');
        }
        Self { bytes, base: original.len(), entries: Vec::new() }
    }

    /// Writes `n g obj … endobj`.
    pub(crate) fn object(&mut self, number: u32, generation: u16, value: &Object) {
        self.entries.push((number, generation, self.base + self.bytes.len()));
        let _ = writeln!(self.bytes, "{number} {generation} obj");
        object(&mut self.bytes, value);
        self.bytes.extend_from_slice(b"\nendobj\n");
    }

    /// Ends the update with its cross-reference section and trailer, and
    /// returns the bytes to append. `trailer` carries the previous trailer's
    /// keys to keep (`/Root`, `/Info`, `/ID`, `/Encrypt`); `/Size` and `/Prev`
    /// are set here. `next_number` is the first object number not in use,
    /// counting the stream object this may write.
    pub(crate) fn finish(
        mut self,
        form: XrefForm,
        mut trailer: Dict,
        previous_section: usize,
        next_number: u32,
    ) -> Vec<u8> {
        trailer.0.retain(|(key, _)| matches!(key.as_slice(), b"Root" | b"Info" | b"ID" | b"Encrypt"));
        let start = self.base + self.bytes.len();
        match form {
            XrefForm::Table => {
                self.entries.sort_by_key(|(number, ..)| *number);
                self.bytes.extend_from_slice(b"xref\n");
                for run in runs(&self.entries) {
                    let _ = writeln!(self.bytes, "{} {}", run[0].0, run.len());
                    for (_, generation, offset) in run {
                        let _ = write!(self.bytes, "{offset:010} {generation:05} n\r\n");
                    }
                }
                trailer.0.push((b"Size".to_vec(), Object::Int(i64::from(next_number))));
                trailer.0.push((b"Prev".to_vec(), Object::Int(offset_int(previous_section))));
                self.bytes.extend_from_slice(b"trailer\n");
                dict(&mut self.bytes, &trailer);
                self.bytes.push(b'\n');
            }
            XrefForm::Stream => {
                // The stream object itself takes the last number.
                let own = next_number;
                self.entries.push((own, 0, start));
                self.entries.sort_by_key(|(number, ..)| *number);
                let mut index = Vec::new();
                let mut rows = Vec::new();
                for run in runs(&self.entries) {
                    index.push(Object::Int(i64::from(run[0].0)));
                    index.push(Object::Int(i64::try_from(run.len()).unwrap_or(i64::MAX)));
                    for (_, generation, offset) in run {
                        rows.push(1u8);
                        rows.extend_from_slice(&u64::try_from(*offset).unwrap_or(0).to_be_bytes());
                        rows.extend_from_slice(&generation.to_be_bytes());
                    }
                }
                trailer.0.extend([
                    (b"Type".to_vec(), Object::Name(b"XRef".to_vec())),
                    (b"Size".to_vec(), Object::Int(i64::from(own) + 1)),
                    (b"Prev".to_vec(), Object::Int(offset_int(previous_section))),
                    (b"W".to_vec(), Object::Array(vec![Object::Int(1), Object::Int(8), Object::Int(2)])),
                    (b"Index".to_vec(), Object::Array(index)),
                    (b"Length".to_vec(), Object::Int(i64::try_from(rows.len()).unwrap_or(i64::MAX))),
                ]);
                let _ = writeln!(self.bytes, "{own} 0 obj");
                dict(&mut self.bytes, &trailer);
                self.bytes.extend_from_slice(b"\nstream\n");
                self.bytes.extend_from_slice(&rows);
                self.bytes.extend_from_slice(b"\nendstream\nendobj\n");
            }
        }
        let _ = write!(self.bytes, "startxref\n{start}\n%%EOF\n");
        self.bytes
    }
}

fn offset_int(offset: usize) -> i64 {
    i64::try_from(offset).unwrap_or(i64::MAX)
}

/// Consecutive object numbers, for the subsections of a cross-reference section.
fn runs(entries: &[(u32, u16, usize)]) -> Vec<&[(u32, u16, usize)]> {
    let mut runs = Vec::new();
    let mut start = 0;
    for i in 1..=entries.len() {
        if i == entries.len() || entries[i].0 != entries[i - 1].0 + 1 {
            runs.push(&entries[start..i]);
            start = i;
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::Parser;

    fn written(value: &Object) -> Vec<u8> {
        let mut out = Vec::new();
        object(&mut out, value);
        out
    }

    #[test]
    fn objects_round_trip_through_the_parser() {
        let value = Object::Dict(Dict(vec![
            (b"Type".to_vec(), Object::Name(b"Page".to_vec())),
            (b"Odd Name/x".to_vec(), Object::Bool(true)),
            (b"Kids".to_vec(), Object::Array(vec![Object::Ref(3, 0), Object::Ref(12, 1)])),
            (b"Rotate".to_vec(), Object::Int(-90)),
            (b"Scale".to_vec(), Object::Real(0.25)),
            (b"Title".to_vec(), Object::Str(vec![0xFE, 0xFF, 0x06, 0x27, b')', b'\\'])),
            (b"Nothing".to_vec(), Object::Null),
        ]));
        let bytes = written(&value);
        assert_eq!(Parser::new(&bytes, 0).object(0), Some(value));
    }

    #[test]
    fn reals_have_no_exponent() {
        assert_eq!(written(&Object::Real(1e-7)), b"0");
        assert_eq!(written(&Object::Real(612.0)), b"612");
        assert_eq!(written(&Object::Real(-0.5)), b"-0.5");
        assert_eq!(written(&Object::Real(123_456_789.125)), b"123456789.125");
    }

    #[test]
    fn subsections_group_consecutive_numbers() {
        let entries = [(2, 0, 10), (3, 0, 20), (7, 0, 30), (8, 0, 40), (9, 0, 50)];
        let lengths: Vec<usize> = runs(&entries).iter().map(|run| run.len()).collect();
        assert_eq!(lengths, [2, 3]);
    }
}
