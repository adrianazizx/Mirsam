//! The PDF half of a signature (ISO 32000-2, 12.8; ETSI EN 319 142-1): one
//! incremental update that adds a signature field and its widget, a
//! signature dictionary with a `/ByteRange` and a `/Contents` placeholder,
//! and changes nothing before it; and reading signatures back, with what was
//! changed in the file after each.
//!
//! In an encrypted file the update's strings are encrypted like any edit's,
//! except the signature's `/Contents`, which never is (ISO 32000-2, 7.6.2):
//! a validator reads it as written.

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use serde::Serialize;

use super::appearance::VisibleSignature;
use crate::annotate::{Annots, Appearance, Writer, pdf_date, text_string};
use crate::edit::{PageTree, Start, set};
use crate::error::{Error, Result};
use crate::pdf::crypt::Crypt;
use crate::pdf::write;
use crate::pdf::{Dict, Entry, File, Object, Parser, find, startxref};

/// `/ByteRange`'s value before it is known; as long as any real one.
const BYTE_RANGE_PLACEHOLDER: &[u8] = b"[0 ********** ********** **********]";
/// Annotation flags of a signature widget: Print and Locked.
const WIDGET_FLAGS: i64 = 4 | 128;
/// The most signature dictionaries looked at in one file.
const MAX_SIGNATURES: usize = 256;

/// What the signature dictionary says besides the signature itself.
pub(crate) struct SigEntries<'a> {
    pub(crate) name: Option<&'a str>,
    pub(crate) reason: Option<&'a str>,
    pub(crate) location: Option<&'a str>,
    pub(crate) contact: Option<&'a str>,
    pub(crate) time: SystemTime,
}

/// The signed file with its placeholders, before the signature goes in.
pub(crate) struct Prepared {
    pub(crate) bytes: Vec<u8>,
    /// `<` of `/Contents`, and the byte after its `>`.
    contents: (usize, usize),
    /// Where the `/ByteRange` array is.
    byte_range: usize,
}

impl Prepared {
    /// Fills in `/ByteRange` (everything but the `/Contents` value) and
    /// returns it.
    pub(crate) fn fill_byte_range(&mut self) -> [usize; 4] {
        let (start, end) = self.contents;
        let range = [0, start, end, self.bytes.len() - end];
        let mut text = format!("[0 {} {} {}", range[1], range[2], range[3]).into_bytes();
        text.resize(BYTE_RANGE_PLACEHOLDER.len() - 1, b' ');
        text.push(b']');
        self.bytes[self.byte_range..self.byte_range + text.len()].copy_from_slice(&text);
        range
    }

    /// The bytes the signature covers, as the two parts of the range.
    pub(crate) fn covered(&self) -> [&[u8]; 2] {
        let (start, end) = self.contents;
        [&self.bytes[..start], &self.bytes[end..]]
    }

    /// Room for the signature's DER, in bytes.
    pub(crate) fn room(&self) -> usize {
        (self.contents.1 - self.contents.0 - 2) / 2
    }

    /// Writes the DER signature into the placeholder, in hex, zero padded.
    pub(crate) fn insert(&mut self, der: &[u8]) {
        let start = self.contents.0 + 1;
        for (i, byte) in der.iter().enumerate().take(self.room()) {
            let hex = format!("{byte:02X}");
            self.bytes[start + 2 * i..start + 2 * i + 2].copy_from_slice(hex.as_bytes());
        }
    }
}

/// Appends a signature field named `field_name` to `base`, invisible on the
/// first page or `visible`, with room for `reserved` bytes of signature.
pub(crate) fn prepare(
    base: &[u8],
    password: Option<&str>,
    entries: &SigEntries<'_>,
    visible: Option<&VisibleSignature>,
    field_name: &str,
    reserved: usize,
) -> Result<Prepared> {
    let unreadable = |what| Error::EditUnsupported(what);
    let Start { file, section, form, mut next_number } = Start::read(base)?;
    let tree = PageTree::read(&file)?;
    let page = visible.map_or(0, |v| v.page);
    let count = tree.leaves.len();
    let leaf = tree.leaves.get(page).ok_or(Error::PageOutOfRange { index: page, count })?;
    let Some(&Object::Ref(root_number, root_generation)) = file.trailer.get(b"Root") else {
        return Err(unreadable("its catalog cannot be read"));
    };
    let catalog = file
        .resolve(&Object::Ref(root_number, root_generation))
        .and_then(|catalog| catalog.as_dict().cloned())
        .ok_or_else(|| unreadable("its catalog cannot be read"))?;

    let mut writer = Writer::new(&file, base, password, &mut next_number);
    let crypt: Option<Crypt> = writer.crypt()?.cloned();
    let encrypt = |number: u32, object: &mut Object| {
        if let Some(crypt) = &crypt {
            crypt.encrypt_strings(number, 0, object);
        }
    };

    // The widget's look: the visible signature, or nothing at all.
    let (rect, appearance) = match visible {
        Some(visible) => (visible.rect, visible.appearance(leaf.rotate)?),
        None => ([0.0; 4], Appearance::in_page_space([0.0; 4], Dict::default(), Vec::new())),
    };
    let appearance = writer.write_appearance(&appearance)?;
    let signature = writer.take_number()?;
    let widget = writer.take_number()?;

    // The signature dictionary, with its placeholders.
    let mut dict = vec![
        (b"Type".to_vec(), Object::Name(b"Sig".to_vec())),
        (b"Filter".to_vec(), Object::Name(b"Adobe.PPKLite".to_vec())),
        (b"SubFilter".to_vec(), Object::Name(b"ETSI.CAdES.detached".to_vec())),
        (b"M".to_vec(), Object::Str(pdf_date(entries.time))),
    ];
    for (key, value) in [
        (b"Name".as_slice(), entries.name),
        (b"Reason", entries.reason),
        (b"Location", entries.location),
        (b"ContactInfo", entries.contact),
    ] {
        if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
            dict.push((key.to_vec(), Object::Str(text_string(value))));
        }
    }
    let app = Dict(vec![
        (b"Name".to_vec(), Object::Name(b"Mirsam".to_vec())),
        (b"REx".to_vec(), Object::Str(env!("CARGO_PKG_VERSION").as_bytes().to_vec())),
    ]);
    dict.push((b"Prop_Build".to_vec(), Object::Dict(Dict(vec![(b"App".to_vec(), Object::Dict(app))]))));
    let mut dict = Object::Dict(Dict(dict));
    encrypt(signature, &mut dict);
    let mut body = Vec::new();
    write::object(&mut body, &dict);
    body.truncate(body.len() - 2); // the closing ">>"
    body.extend_from_slice(b"/ByteRange ");
    let byte_range_at = body.len();
    body.extend_from_slice(BYTE_RANGE_PLACEHOLDER);
    body.extend_from_slice(b" /Contents ");
    let contents_at = body.len();
    body.push(b'<');
    body.resize(body.len() + 2 * reserved, b'0');
    body.push(b'>');
    body.extend_from_slice(b">>");
    let body_at = writer.update.raw_object(signature, 0, &body);

    // The field and its widget, one dictionary.
    let real = |value: f32| Object::Real(f64::from(value));
    let mut field = Object::Dict(Dict(vec![
        (b"Type".to_vec(), Object::Name(b"Annot".to_vec())),
        (b"Subtype".to_vec(), Object::Name(b"Widget".to_vec())),
        (b"FT".to_vec(), Object::Name(b"Sig".to_vec())),
        (b"T".to_vec(), Object::Str(text_string(field_name))),
        (b"V".to_vec(), Object::Ref(signature, 0)),
        (b"F".to_vec(), Object::Int(WIDGET_FLAGS)),
        (b"Rect".to_vec(), Object::Array(rect.iter().map(|v| real(*v)).collect())),
        (b"P".to_vec(), Object::Ref(leaf.number, leaf.generation)),
        (b"AP".to_vec(), Object::Dict(Dict(vec![(b"N".to_vec(), Object::Ref(appearance, 0))]))),
    ]));
    encrypt(widget, &mut field);
    writer.update.object(widget, 0, &field);

    let mut annots = Annots::read(&file, leaf)?;
    annots.items.push(Object::Ref(widget, 0));
    annots.write(&mut writer.update, leaf);

    // The form: the field in /Fields, and SigFlags SignaturesExist and
    // AppendOnly. Objects rewritten keep their numbers, so strings already
    // in them stay valid in an encrypted file.
    let mut catalog = catalog;
    let add_field = |writer: &mut Writer<'_, '_>, acro: &mut Dict| -> Result<()> {
        match acro.get(b"Fields") {
            Some(Object::Ref(number, generation)) => {
                let (number, generation) = (*number, *generation);
                let mut fields = match file.resolve(&Object::Ref(number, generation)) {
                    Some(Object::Array(fields)) => fields,
                    _ => return Err(unreadable("its form's fields cannot be read")),
                };
                fields.push(Object::Ref(widget, 0));
                writer.update.object(number, generation, &Object::Array(fields));
            }
            Some(Object::Array(fields)) => {
                let mut fields = fields.clone();
                fields.push(Object::Ref(widget, 0));
                set(acro, b"Fields", Object::Array(fields));
            }
            _ => set(acro, b"Fields", Object::Array(vec![Object::Ref(widget, 0)])),
        }
        let flags = acro.get(b"SigFlags").and_then(Object::as_int).unwrap_or(0);
        set(acro, b"SigFlags", Object::Int(flags | 3));
        Ok(())
    };
    match catalog.get(b"AcroForm").cloned() {
        Some(Object::Ref(number, generation)) => {
            let mut acro = file
                .resolve(&Object::Ref(number, generation))
                .and_then(|acro| acro.as_dict().cloned())
                .ok_or_else(|| unreadable("its form cannot be read"))?;
            add_field(&mut writer, &mut acro)?;
            writer.update.object(number, generation, &Object::Dict(acro));
        }
        Some(Object::Dict(mut acro)) => {
            add_field(&mut writer, &mut acro)?;
            set(&mut catalog, b"AcroForm", Object::Dict(acro));
            writer.update.object(root_number, root_generation, &Object::Dict(catalog));
        }
        _ => {
            let number = writer.take_number()?;
            let mut acro = Dict::default();
            add_field(&mut writer, &mut acro)?;
            writer.update.object(number, 0, &Object::Dict(acro));
            set(&mut catalog, b"AcroForm", Object::Ref(number, 0));
            writer.update.object(root_number, root_generation, &Object::Dict(catalog));
        }
    }

    let update = writer.update;
    let tail = update.finish(form, file.trailer.clone(), section, next_number);
    let mut bytes = Vec::with_capacity(base.len() + tail.len());
    bytes.extend_from_slice(base);
    bytes.extend_from_slice(&tail);
    let contents_start = body_at + contents_at;
    let contents_end = contents_start + 2 * reserved + 2;
    debug_assert_eq!(bytes.get(contents_start), Some(&b'<'));
    debug_assert_eq!(bytes.get(contents_end - 1), Some(&b'>'));
    Ok(Prepared { bytes, contents: (contents_start, contents_end), byte_range: body_at + byte_range_at })
}

// ------------------------------------------------------------ reading

/// A signature dictionary found in a file.
#[derive(Debug, Clone)]
pub(crate) struct Found {
    /// The object the dictionary is (or is in).
    pub(crate) object: (u32, u16),
    /// Where that object starts in the file.
    pub(crate) offset: Option<usize>,
    pub(crate) dict: Dict,
    /// The field's fully qualified name, when its field was found.
    pub(crate) field: Option<String>,
    /// The page its widget is on, and the widget's box in page space.
    pub(crate) page: Option<usize>,
    pub(crate) rect: Option<[f32; 4]>,
}

impl Found {
    /// A text entry, decrypted.
    pub(crate) fn text(&self, key: &[u8], crypt: Option<&Crypt>) -> Option<String> {
        let Some(Object::Str(bytes)) = self.dict.get(key) else { return None };
        let bytes = match crypt {
            Some(crypt) => crypt.decrypt_string(self.object.0, self.object.1, bytes)?,
            None => bytes.clone(),
        };
        let text = crate::forms::decode_text(&bytes);
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_string())
    }

    pub(crate) fn name(&self, key: &[u8]) -> Option<String> {
        self.dict.get(key).and_then(Object::as_name).map(|name| String::from_utf8_lossy(name).into_owned())
    }

    /// `/ByteRange`, when it is four non-negative numbers.
    pub(crate) fn byte_range(&self) -> Option<[usize; 4]> {
        let items = self.dict.get(b"ByteRange")?.as_array()?;
        let numbers: Vec<usize> = items
            .iter()
            .map(|item| item.as_int().and_then(|n| usize::try_from(n).ok()))
            .collect::<Option<_>>()?;
        numbers.try_into().ok()
    }

    /// `/Contents` as read (never encrypted).
    pub(crate) fn contents(&self) -> Option<&[u8]> {
        match self.dict.get(b"Contents") {
            Some(Object::Str(bytes)) => Some(bytes),
            _ => None,
        }
    }
}

/// Every signature in the newest revision of `bytes`: through the form's
/// fields where they can be read, and by looking for signature dictionaries
/// in the file for any the fields did not lead to (forms kept in encrypted
/// object streams, which this reader cannot open).
pub(crate) fn find_signatures(bytes: &[u8]) -> Vec<Found> {
    let Some(file) = File::new(bytes) else { return Vec::new() };
    let mut found: Vec<Found> = Vec::new();
    let pages = page_of_annotations(&file);

    // Through the fields.
    let mut seen = HashSet::new();
    if let Some(acro) = catalog_entry(&file, b"AcroForm").and_then(|acro| file.resolve(&acro))
        && let Some(fields) =
            acro.as_dict().and_then(|acro| acro.get(b"Fields")).and_then(|f| file.resolve(f))
        && let Some(fields) = fields.as_array()
    {
        let mut stack: Vec<(Object, String, Option<Vec<u8>>, usize)> =
            fields.iter().rev().map(|field| (field.clone(), String::new(), None, 0)).collect();
        let mut visited = 0usize;
        while let Some((reference, parent_name, inherited_type, depth)) = stack.pop() {
            visited += 1;
            if visited > 20_000 || depth > 32 || found.len() >= MAX_SIGNATURES {
                break;
            }
            let Object::Ref(number, generation) = reference else { continue };
            if !seen.insert(number) {
                continue;
            }
            let Some(Object::Dict(node)) = file.resolve(&reference) else { continue };
            let partial = match node.get(b"T") {
                Some(Object::Str(bytes)) => Some(crate::forms::decode_text(bytes)),
                _ => None,
            };
            let name = match (&partial, parent_name.is_empty()) {
                (Some(partial), true) => partial.clone(),
                (Some(partial), false) => format!("{parent_name}.{partial}"),
                (None, _) => parent_name.clone(),
            };
            let field_type = node.get(b"FT").and_then(Object::as_name).map(<[u8]>::to_vec).or(inherited_type);
            if let Some(Object::Array(kids)) = node.get(b"Kids").and_then(|k| file.resolve(k)) {
                for kid in kids.iter().rev() {
                    stack.push((kid.clone(), name.clone(), field_type.clone(), depth + 1));
                }
            }
            if field_type.as_deref() != Some(b"Sig") {
                continue;
            }
            let Some(value) = node.get(b"V") else { continue };
            let (object, offset, dict) = match value {
                Object::Ref(n, g) => match (file.resolve(value), file.entries.get(n)) {
                    (Some(Object::Dict(dict)), Some(Entry::Offset(at))) => ((*n, *g), Some(*at), dict),
                    (Some(Object::Dict(dict)), _) => ((*n, *g), None, dict),
                    _ => continue,
                },
                Object::Dict(dict) => ((number, generation), None, dict.clone()),
                _ => continue,
            };
            if found.iter().any(|f| f.object == object) {
                continue;
            }
            // The widget: this node, or its first kid.
            let widget = if node.get(b"Subtype").and_then(Object::as_name) == Some(b"Widget") {
                Some((number, node.clone()))
            } else {
                match node.get(b"Kids").and_then(|k| file.resolve(k)) {
                    Some(Object::Array(kids)) => kids.iter().find_map(|kid| match (kid, file.resolve(kid)) {
                        (Object::Ref(n, _), Some(Object::Dict(dict))) => Some((*n, dict)),
                        _ => None,
                    }),
                    _ => None,
                }
            };
            let (page, rect) = match widget {
                Some((widget_number, dict)) => (
                    pages.get(&widget_number).copied(),
                    crate::annotate::rect_of(&file, &dict).filter(|r| r[2] - r[0] > 0.5 && r[3] - r[1] > 0.5),
                ),
                None => (None, None),
            };
            found.push(Found { object, offset, dict, field: Some(name), page, rect });
        }
    }

    // Signature dictionaries no field led to.
    let mut at = 0;
    while let Some(position) = find(&bytes[at..], b"/ByteRange").map(|p| p + at) {
        at = position + 1;
        if found.len() >= MAX_SIGNATURES {
            break;
        }
        let Some((number, generation, offset)) = enclosing_object(bytes, position) else { continue };
        // Only an object the cross-reference table points at: not an old
        // revision's, and not text inside some stream.
        if !matches!(file.entries.get(&number), Some(Entry::Offset(current)) if *current == offset)
            || found.iter().any(|f| f.object.0 == number)
        {
            continue;
        }
        let Some((Object::Dict(dict), None)) = file.indirect_object_at(offset) else { continue };
        let kind = dict.get(b"Type").and_then(Object::as_name);
        if !matches!(kind, Some(b"Sig" | b"DocTimeStamp")) && dict.get(b"Filter").is_none() {
            continue;
        }
        found.push(Found {
            object: (number, generation),
            offset: Some(offset),
            dict,
            field: None,
            page: None,
            rect: None,
        });
    }
    found.sort_by_key(|f| f.byte_range().map_or(usize::MAX, |range| range[2] + range[3]));
    found
}

/// The newest `n g obj` before `position`, if `position` is inside it.
fn enclosing_object(bytes: &[u8], position: usize) -> Option<(u32, u16, usize)> {
    let window = &bytes[position.saturating_sub(64 * 1024)..position];
    let base = position - window.len();
    let mut end = window.len();
    while let Some(obj) = crate::pdf::rfind(&window[..end], b"obj") {
        end = obj;
        // `obj` must stand alone, after "n g".
        let after = window.get(obj + 3).copied();
        if after.is_some_and(|b| !crate::pdf::is_white(b) && !crate::pdf::is_delim(b)) {
            continue;
        }
        let mut start = obj;
        let mut fields = 0;
        while start > 0 && fields < 2 {
            while start > 0 && crate::pdf::is_white(window[start - 1]) {
                start -= 1;
            }
            let digits_end = start;
            while start > 0 && window[start - 1].is_ascii_digit() {
                start -= 1;
            }
            if start == digits_end {
                break;
            }
            fields += 1;
        }
        if fields != 2 {
            continue;
        }
        let mut parser = Parser::new(bytes, base + start);
        let number = u32::try_from(parser.unsigned()?).ok()?;
        let generation = u16::try_from(parser.unsigned()?).ok()?;
        // Not past an `endobj` between the object's start and the position.
        if find(&bytes[base + obj..position], b"endobj").is_some() {
            return None;
        }
        return Some((number, generation, base + start));
    }
    None
}

fn catalog_entry(file: &File<'_>, key: &[u8]) -> Option<Object> {
    let catalog = file.resolve(file.trailer.get(b"Root")?)?;
    catalog.as_dict()?.get(key).cloned()
}

/// Which page each annotation object is on, by the pages' `/Annots`.
fn page_of_annotations(file: &File<'_>) -> HashMap<u32, usize> {
    let mut pages = HashMap::new();
    let Ok(tree) = PageTree::read(file) else { return pages };
    for (index, leaf) in tree.leaves.iter().enumerate() {
        if let Some(Object::Array(items)) = leaf.dict.get(b"Annots").and_then(|a| file.resolve(a)) {
            for item in items {
                if let Object::Ref(number, _) = item {
                    pages.entry(number).or_insert(index);
                }
            }
        }
    }
    pages
}

// ------------------------------------------------------------ revisions

/// What later revisions changed after a signature, by kind.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Changes {
    /// Incremental updates appended after the signed revision.
    pub revisions: usize,
    /// Pages whose content or properties changed (0-based, in the newest
    /// revision's order).
    pub pages: Vec<usize>,
    /// Pages were added, removed or reordered.
    pub page_order: bool,
    /// Annotations (comments, markup, Fill & Sign items) added, changed or removed.
    pub annotations: usize,
    /// Form fields filled in or changed.
    pub form_fields: usize,
    /// Signatures or document time-stamps added.
    pub signatures: usize,
    /// Anything else (the catalog, document information, unreadable objects).
    pub other: usize,
}

/// How many revisions `bytes` has up to and including the one ending at `end`.
pub(crate) fn revision_at(bytes: &[u8], end: usize) -> usize {
    let mut count = 0;
    let mut at = 0;
    while let Some(position) = find(&bytes[at..end.min(bytes.len())], b"%%EOF") {
        count += 1;
        at += position + 5;
    }
    count.max(1)
}

/// What changed in `bytes` after its first `signed_end` bytes (a signed
/// revision): the objects the later revisions wrote, by what they are.
pub(crate) fn changes_after(bytes: &[u8], signed_end: usize) -> Changes {
    let mut changes = Changes::default();
    let (Some(signed), Some(newest)) = (File::new(&bytes[..signed_end]), File::new(bytes)) else {
        changes.other = 1;
        return changes;
    };
    // Sections after the signed revision.
    let mut next = startxref(bytes);
    let mut sections = HashSet::new();
    let mut walker = File { bytes, entries: HashMap::new(), trailer: Dict::default() };
    while let Some(offset) = next.take() {
        if offset < signed_end || !sections.insert(offset) || sections.len() > 256 {
            break;
        }
        changes.revisions += 1;
        next = walker
            .read_section(offset)
            .and_then(|trailer| trailer.get(b"Prev").and_then(Object::as_int))
            .and_then(|prev| usize::try_from(prev).ok());
    }
    let changed: Vec<u32> = newest
        .entries
        .iter()
        .filter(|(number, entry)| match (signed.entries.get(number), entry) {
            (Some(Entry::Offset(a)), Entry::Offset(b)) => a != b,
            (Some(Entry::InStream { stream: a, index: i }), Entry::InStream { stream: b, index: j }) => {
                a != b || i != j
            }
            _ => true,
        })
        .map(|(number, _)| *number)
        .collect();
    if changed.is_empty() {
        return changes;
    }
    let new_tree = PageTree::read(&newest).ok();
    let old_tree = PageTree::read(&signed).ok();
    // Page content streams, by page.
    let mut content_pages: HashMap<u32, usize> = HashMap::new();
    if let Some(tree) = &new_tree {
        for (index, leaf) in tree.leaves.iter().enumerate() {
            let contents = match leaf.dict.get(b"Contents") {
                Some(Object::Array(items)) => items.clone(),
                Some(other) => match newest.resolve_shallow(other) {
                    Some(Object::Array(items)) => items,
                    _ => vec![other.clone()],
                },
                None => Vec::new(),
            };
            for item in contents {
                if let Object::Ref(number, _) = item {
                    content_pages.insert(number, index);
                }
            }
        }
    }
    let page_numbers = |tree: &Option<PageTree>| -> Vec<u32> {
        tree.as_ref().map(|t| t.leaves.iter().map(|leaf| leaf.number).collect()).unwrap_or_default()
    };
    let (old_pages, new_pages) = (page_numbers(&old_tree), page_numbers(&new_tree));
    changes.page_order = old_pages != new_pages;
    let mut pages = HashSet::new();
    let mut fields = HashSet::new();
    for number in changed {
        if let Some(&page) = content_pages.get(&number) {
            pages.insert(page);
            continue;
        }
        let reference = Object::Ref(number, 0);
        let Some(object) = newest.resolve(&reference) else {
            // In an encrypted object stream (unreadable here), or freed.
            changes.other += 1;
            continue;
        };
        let Some(dict) = object.as_dict() else { continue };
        let kind = dict.get(b"Type").and_then(Object::as_name);
        let subtype = dict.get(b"Subtype").and_then(Object::as_name);
        match (kind, subtype) {
            (Some(b"Sig" | b"DocTimeStamp"), _) => changes.signatures += 1,
            (_, Some(b"Widget")) => {
                // A signature's widget is counted by its signature
                // dictionary; a field's by the field (its parent, if any).
                if dict.get(b"FT").and_then(Object::as_name) != Some(b"Sig") {
                    let field = match dict.get(b"Parent") {
                        Some(Object::Ref(parent, _)) => *parent,
                        _ => number,
                    };
                    fields.insert(field);
                }
            }
            (_, Some(b"Popup")) => {}
            (Some(b"Annot"), _) => changes.annotations += 1,
            (Some(b"Page"), _) => {
                // A page rewritten only for its /Annots list is counted by
                // its annotations.
                let old = signed.resolve(&reference);
                let strip =
                    |dict: &Dict| Dict(dict.0.iter().filter(|(k, _)| k != b"Annots").cloned().collect());
                let same =
                    old.as_ref().and_then(Object::as_dict).is_some_and(|old| strip(old) == strip(dict));
                if !same && let Some(index) = new_pages.iter().position(|n| *n == number) {
                    pages.insert(index);
                }
            }
            (Some(b"Pages"), _) => {}
            (Some(b"XObject" | b"Font" | b"FontDescriptor" | b"ExtGState" | b"XRef" | b"ObjStm"), _) => {}
            (Some(b"Catalog"), _) => {}
            _ if dict.get(b"FT").is_some() || dict.get(b"Kids").is_some() && dict.get(b"T").is_some() => {
                fields.insert(number);
            }
            // Form dictionaries, appearance streams without a /Type, arrays
            // and other support objects of the above.
            _ => {}
        }
    }
    changes.form_fields = fields.len();
    let mut pages: Vec<usize> = pages.into_iter().collect();
    pages.sort_unstable();
    changes.pages = pages;
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_object_around_a_position_is_found() {
        let bytes =
            b"%PDF-1.7\n1 0 obj\n<</A 1>>\nendobj\n12 0 obj\n<</Type/Sig/ByteRange [0 1 2 3]>>\nendobj\n";
        let at = find(bytes, b"/ByteRange").expect("present");
        assert_eq!(enclosing_object(bytes, at), Some((12, 0, 33)));
        // Not inside an object.
        let loose = b"1 0 obj\n<<>>\nendobj\n/ByteRange";
        assert_eq!(enclosing_object(loose, loose.len() - 10), None);
    }

    #[test]
    fn revisions_are_counted_by_their_end_markers() {
        let bytes = b"%PDF\n%%EOF\nupdate\n%%EOF\nmore\n%%EOF\n";
        assert_eq!(revision_at(bytes, 11), 1);
        assert_eq!(revision_at(bytes, 24), 2);
        assert_eq!(revision_at(bytes, bytes.len()), 3);
    }
}
