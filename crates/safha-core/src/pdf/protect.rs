//! Rewriting a whole file with another protection (ADR 0043).
//!
//! PDFium writes a file with the protection it was opened with and cannot
//! change it. So a save that sets, changes or removes a password reads
//! PDFium's output back object by object, decrypts it with the password it
//! was opened with, and writes it again encrypted for the new settings, or in
//! the clear. Object streams are unpacked on the way: the new file holds every
//! object directly, under a classic cross-reference table.

use std::collections::BTreeMap;
use std::io::Write as _;

use super::crypt::{Crypt, new_handler};
use super::{Dict, Entry, File, Object, Parser, decode_stream, write};
use crate::error::{Error, Result};
use crate::security::Protection;

/// One object of the file, in the clear.
struct Plain {
    generation: u16,
    object: Object,
    /// The stream's data as stored (filtered), if it is a stream.
    stream: Option<Vec<u8>>,
}

/// `bytes` rewritten with `target`'s protection, or with none for `None`.
/// `password` opens `bytes` if they are encrypted.
pub(crate) fn rewrite(bytes: &[u8], password: Option<&str>, target: Option<&Protection>) -> Result<Vec<u8>> {
    let file = File::new(bytes).ok_or(Error::InvalidPdf)?;
    let old = Crypt::of(&file, password)?;
    let encrypt_number = match file.trailer.get(b"Encrypt") {
        Some(Object::Ref(number, _)) => Some(*number),
        _ => None,
    };

    let mut objects: BTreeMap<u32, Plain> = BTreeMap::new();
    let mut numbers: Vec<u32> = file.entries.keys().copied().collect();
    numbers.sort_unstable();
    for &number in &numbers {
        if Some(number) == encrypt_number {
            continue;
        }
        let Some(&Entry::Offset(offset)) = file.entries.get(&number) else { continue };
        if offset == 0 {
            continue;
        }
        let mut header = Parser::new(bytes, offset);
        header.unsigned();
        let generation = header.unsigned().and_then(|g| u16::try_from(g).ok()).unwrap_or(0);
        let Some((mut object, raw)) = file.indirect_object_at(offset) else { continue };
        let kind =
            object.as_dict().and_then(|d| d.get(b"Type")).and_then(Object::as_name).map(<[u8]>::to_vec);
        if kind.as_deref() == Some(b"XRef") {
            continue;
        }
        let mut stream = raw.map(<[u8]>::to_vec);
        if let Some(old) = &old {
            let _ = old.decrypt_strings(number, generation, &mut object);
            if let (Some(data), Some(dict)) = (&stream, object.as_dict()) {
                stream = Some(
                    old.decrypt_stream_of(number, generation, dict, data).unwrap_or_else(|| data.clone()),
                );
            }
        }
        if kind.as_deref() == Some(b"ObjStm") {
            if let (Some(dict), Some(data)) = (object.as_dict(), &stream) {
                for (inner, value) in unpack(&file, dict, data) {
                    // Only the entries the cross-reference table still points here.
                    if matches!(file.entries.get(&inner), Some(Entry::InStream { stream, .. }) if *stream == number)
                    {
                        objects.insert(inner, Plain { generation: 0, object: value, stream: None });
                    }
                }
            }
            continue;
        }
        objects.insert(number, Plain { generation, object, stream });
    }

    let root = file.trailer.get(b"Root").cloned().ok_or(Error::InvalidPdf)?;
    let info = file.trailer.get(b"Info").cloned();
    let id = match file.trailer.get(b"ID").and_then(Object::as_array) {
        Some([Object::Str(first), Object::Str(second), ..]) => [first.clone(), second.clone()],
        _ => {
            let mut fresh = [0u8; 32];
            getrandom::fill(&mut fresh).map_err(|_| Error::EditUnsupported("no source of randomness"))?;
            [fresh[..16].to_vec(), fresh[16..].to_vec()]
        }
    };
    let mut next = objects.keys().next_back().map_or(1, |last| last + 1);

    // The new handler, and the dictionary that describes it.
    let mut encrypt_dict = None;
    let mut handler = None;
    if let Some(protection) = target {
        protection.check()?;
        let made = new_handler(
            protection.handler(),
            &protection.user_password,
            &protection.owner_password,
            protection.permissions.bits(),
            &id[0],
        )?;
        encrypt_dict = Some((next, made.dict));
        handler = Some(made.crypt);
        next += 1;
    }

    let mut out = Vec::new();
    let header: &[u8] = match target {
        None => bytes.get(..8).filter(|h| h.starts_with(b"%PDF-")).unwrap_or(b"%PDF-1.7"),
        Some(p) if p.cipher == crate::security::Cipher::Aes256 => b"%PDF-2.0",
        Some(_) => b"%PDF-1.6",
    };
    out.extend_from_slice(header);
    out.extend_from_slice(b"\n%\xE2\xE3\xCF\xD3\n");

    let mut offsets: BTreeMap<u32, (u16, usize)> = BTreeMap::new();
    for (number, mut plain) in objects {
        if let Some(handler) = &handler {
            handler.encrypt_strings(number, plain.generation, &mut plain.object);
            if let Some(data) = &plain.stream {
                plain.stream = Some(handler.encrypt_stream(number, plain.generation, data));
            }
        }
        offsets.insert(number, (plain.generation, out.len()));
        let _ = writeln!(out, "{number} {} obj", plain.generation);
        match (&plain.object, &plain.stream) {
            (Object::Dict(dict), Some(data)) => {
                let mut dict = dict.clone();
                dict.0.retain(|(key, _)| key != b"Length");
                dict.0.push((b"Length".to_vec(), Object::Int(i64::try_from(data.len()).unwrap_or(i64::MAX))));
                write::dict(&mut out, &dict);
                out.extend_from_slice(b"\nstream\n");
                out.extend_from_slice(data);
                out.extend_from_slice(b"\nendstream\nendobj\n");
            }
            (object, _) => {
                write::object(&mut out, object);
                out.extend_from_slice(b"\nendobj\n");
            }
        }
    }
    if let Some((number, dict)) = &encrypt_dict {
        offsets.insert(*number, (0, out.len()));
        let _ = writeln!(out, "{number} 0 obj");
        write::dict(&mut out, dict);
        out.extend_from_slice(b"\nendobj\n");
    }

    let xref = out.len();
    let _ = writeln!(out, "xref\n0 {next}");
    for number in 0..next {
        match offsets.get(&number) {
            Some((generation, offset)) => {
                let _ = write!(out, "{offset:010} {generation:05} n\r\n");
            }
            None if number == 0 => out.extend_from_slice(b"0000000000 65535 f\r\n"),
            None => out.extend_from_slice(b"0000000000 00000 f\r\n"),
        }
    }
    let mut trailer = vec![(b"Size".to_vec(), Object::Int(i64::from(next))), (b"Root".to_vec(), root)];
    if let Some(info) = info {
        trailer.push((b"Info".to_vec(), info));
    }
    trailer
        .push((b"ID".to_vec(), Object::Array(vec![Object::Str(id[0].clone()), Object::Str(id[1].clone())])));
    if let Some((number, _)) = &encrypt_dict {
        trailer.push((b"Encrypt".to_vec(), Object::Ref(*number, 0)));
    }
    out.extend_from_slice(b"trailer\n");
    write::dict(&mut out, &Dict(trailer));
    let _ = write!(out, "\nstartxref\n{xref}\n%%EOF\n");
    Ok(out)
}

/// The objects an object stream holds, as (number, value).
fn unpack(file: &File<'_>, dict: &Dict, data: &[u8]) -> Vec<(u32, Object)> {
    let Some(data) = decode_stream(dict, data, file) else { return Vec::new() };
    let count = dict.get(b"N").and_then(Object::as_int).and_then(|n| usize::try_from(n).ok());
    let first = dict.get(b"First").and_then(Object::as_int).and_then(|n| usize::try_from(n).ok());
    let (Some(count), Some(first)) = (count, first) else { return Vec::new() };
    let mut header = Parser::new(&data, 0);
    let mut out = Vec::new();
    for _ in 0..count {
        let (Some(number), Some(at)) = (header.unsigned(), header.unsigned()) else { break };
        let (Ok(number), Ok(at)) = (u32::try_from(number), usize::try_from(at)) else { break };
        if let Some(value) = Parser::new(&data, first.saturating_add(at)).object(0) {
            out.push((number, value));
        }
    }
    out
}
