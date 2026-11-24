//! Links (ADR 0051): `/Link` annotations to web addresses, pages, and named
//! destinations — read with where they go, added over selected text or a
//! drawn rectangle, pointed elsewhere, moved, and removed, as incremental
//! updates. Encrypted files: strings are decrypted to read them and new ones
//! encrypted under their object's number.
//!
//! Only `http`, `https` and `mailto` addresses are written. Other actions
//! (JavaScript, launching files, other documents) are read and shown for
//! what they are, never followed.

use std::collections::HashSet;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::annotate::Quad;
use crate::annotate::{pdf_date, rect_of, unique_name};
use crate::edit::{Leaf, PageTree, Start, set};
use crate::error::{Error, Result};
use crate::pdf::crypt::Crypt;
use crate::pdf::write::Update;
use crate::pdf::{Dict, File, Object};
use crate::view::{ViewRect, ViewTransform};

/// The longest web address written.
pub const MAX_URL: usize = 2048;
/// The most rectangles one link covers.
const MAX_RECTS: usize = 1000;
/// The most named destinations read.
const MAX_NAMES: usize = 100_000;
const MAX_TREE_DEPTH: usize = 32;

/// Where a link goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LinkTarget {
    /// A web or mail address.
    Url { url: String },
    /// A page of this document (from 0).
    Page { page: usize },
    /// A named destination of this document, and the page it is on.
    Named {
        name: String,
        #[serde(default)]
        page: Option<usize>,
    },
    /// An action this app shows but does not follow or write: `javascript`,
    /// `launch`, `remote` (another file), `named` (a viewer command), or the
    /// action's own name.
    Other { action: String },
}

/// One link of a page.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkInfo {
    /// Its position in the page's `/Annots` (what changing or removing it takes).
    pub index: usize,
    /// The areas it covers on the displayed page (one per quad, else its box).
    pub rects: Vec<ViewRect>,
    pub target: LinkTarget,
}

/// A named destination of the document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedDestination {
    pub name: String,
    pub page: Option<usize>,
}

/// A change to a page's links.
#[derive(Debug, Clone)]
pub enum LinkEdit {
    /// A new link over `rects` (displayed page): one rectangle drawn, or a
    /// text selection's runs.
    Add {
        page: usize,
        rects: Vec<ViewRect>,
        target: LinkTarget,
    },
    /// Points link `index` somewhere else.
    SetTarget {
        page: usize,
        index: usize,
        target: LinkTarget,
    },
    /// Moves or resizes link `index` to one rectangle (displayed page).
    SetRect {
        page: usize,
        index: usize,
        rect: ViewRect,
    },
    Remove {
        page: usize,
        index: usize,
    },
}

/// A web address as written: `https://` added to a bare domain, `mailto:` to
/// a bare e-mail address; anything but `http`, `https` and `mailto` refused;
/// characters outside ASCII percent-encoded.
pub fn normalise_url(input: &str) -> Result<String> {
    let text = input.trim();
    if text.is_empty() || text.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(Error::InvalidEdit("a web address has no spaces"));
    }
    let lower = text.to_ascii_lowercase();
    let with_scheme =
        if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:") {
            text.to_string()
        } else if text.contains(':') {
            return Err(Error::InvalidEdit("only web and e-mail addresses can be linked"));
        } else if text.contains('@') && !text.contains('/') {
            format!("mailto:{text}")
        } else if text.contains('.') {
            format!("https://{text}")
        } else {
            return Err(Error::InvalidEdit("this is not a web address"));
        };
    let mut out = String::with_capacity(with_scheme.len());
    for c in with_scheme.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            let mut buffer = [0u8; 4];
            for b in c.encode_utf8(&mut buffer).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    let rest = out.split_once(':').map_or("", |(_, rest)| rest).trim_start_matches('/');
    if rest.is_empty() || out.len() > MAX_URL {
        return Err(Error::InvalidEdit("a web address is at most 2,048 characters"));
    }
    Ok(out)
}

/// The address the app opens for a link, if it may open it: web and mail
/// only; characters outside ASCII percent-encoded (an IRI as a URI); none a
/// shell or the platform's launcher reads specially.
pub fn openable(url: &str) -> Option<String> {
    let mut out = String::with_capacity(url.len());
    for c in url.chars() {
        if c.is_ascii() {
            out.push(c);
        } else if c.is_control() || c.is_whitespace() {
            return None;
        } else {
            let mut buffer = [0u8; 4];
            for b in c.encode_utf8(&mut buffer).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    let lower = out.to_ascii_lowercase();
    let scheme =
        lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:");
    (scheme
        && out.len() <= MAX_URL
        && out.bytes().all(|b| b.is_ascii_graphic() && !b"\"<>\\^`{|}".contains(&b)))
    .then_some(out)
}

/// Whether the app may open an address (see [`openable`]).
pub fn may_open(url: &str) -> bool {
    openable(url).is_some()
}

/// Reads objects with their strings decrypted.
struct Reader<'a, 'f> {
    file: &'a File<'f>,
    crypt: Option<Crypt>,
}

impl Reader<'_, '_> {
    /// `object`, followed if it is a reference, with the strings of the
    /// object it reached decrypted. A direct object is returned as it is: it
    /// was decrypted with the object it sits in.
    fn plain(&self, object: &Object) -> Option<Object> {
        let Object::Ref(number, generation) = object else { return Some(object.clone()) };
        let mut value = self.file.resolve(object)?;
        if let Some(crypt) = &self.crypt {
            crypt.decrypt_strings(*number, *generation, &mut value)?;
        }
        Some(value)
    }

    /// A direct annotation read from an array of `owner`'s, decrypted.
    fn plain_in(&self, object: &Object, (number, generation): (u32, u16)) -> Option<Object> {
        if matches!(object, Object::Ref(..)) {
            return self.plain(object);
        }
        let mut value = object.clone();
        if let Some(crypt) = &self.crypt {
            crypt.decrypt_strings(number, generation, &mut value)?;
        }
        Some(value)
    }
}

/// A page's `/Annots`, where it lives, and its items' owner.
struct Annots {
    own: Option<(u32, u16)>,
    items: Vec<Object>,
}

impl Annots {
    fn read(file: &File<'_>, leaf: &Leaf) -> Result<Self> {
        let unreadable = || Error::EditUnsupported("the page's annotations cannot be read");
        match leaf.dict.get(b"Annots") {
            None | Some(Object::Null) => Ok(Self { own: None, items: Vec::new() }),
            Some(Object::Array(items)) => Ok(Self { own: None, items: items.clone() }),
            Some(reference @ Object::Ref(number, generation)) => match file.resolve(reference) {
                Some(Object::Array(items)) => Ok(Self { own: Some((*number, *generation)), items }),
                Some(Object::Null) => Ok(Self { own: Some((*number, *generation)), items: Vec::new() }),
                _ => Err(unreadable()),
            },
            Some(_) => Err(unreadable()),
        }
    }

    /// The object a direct annotation in the array belongs to.
    fn owner(&self, leaf: &Leaf) -> (u32, u16) {
        self.own.unwrap_or((leaf.number, leaf.generation))
    }

    fn write(self, update: &mut Update, leaf: &Leaf) {
        match self.own {
            Some((number, generation)) => update.object(number, generation, &Object::Array(self.items)),
            None => {
                let mut dict = leaf.dict.clone();
                set(&mut dict, b"Annots", Object::Array(self.items));
                update.object(leaf.number, leaf.generation, &Object::Dict(dict));
            }
        }
    }
}

fn number(object: &Object) -> Option<f32> {
    match object {
        Object::Int(i) => Some(*i as f32),
        Object::Real(r) => Some(*r as f32),
        _ => None,
    }
}

fn text_of(object: &Object) -> Option<String> {
    match object {
        Object::Str(bytes) => Some(crate::forms::decode_text(bytes)),
        Object::Name(name) => Some(String::from_utf8_lossy(name).into_owned()),
        _ => None,
    }
}

/// The document's named destinations, by name: the catalog's `/Dests`
/// dictionary and its `/Names` `/Dests` tree. Each with how it is named
/// (`true`: a string of the tree; `false`: a name of the dictionary), its
/// bytes, and its destination.
fn named(reader: &Reader<'_, '_>) -> Vec<(bool, Vec<u8>, Object)> {
    let file = reader.file;
    let mut out = Vec::new();
    let Some(catalog) = file.trailer.get(b"Root").and_then(|r| reader.plain(r)) else { return out };
    let Some(catalog) = catalog.as_dict() else { return out };
    if let Some(dests) = catalog.get(b"Dests").and_then(|d| reader.plain(d))
        && let Some(dests) = dests.as_dict()
    {
        for (key, value) in dests.0.iter().take(MAX_NAMES) {
            out.push((false, key.clone(), value.clone()));
        }
    }
    let tree = catalog
        .get(b"Names")
        .and_then(|n| reader.plain(n))
        .and_then(|n| n.as_dict().and_then(|n| n.get(b"Dests").cloned()));
    if let Some(tree) = tree {
        let mut seen = HashSet::new();
        walk_names(reader, &tree, 0, &mut seen, &mut out);
    }
    out
}

fn walk_names(
    reader: &Reader<'_, '_>,
    node: &Object,
    depth: usize,
    seen: &mut HashSet<u32>,
    out: &mut Vec<(bool, Vec<u8>, Object)>,
) {
    if depth > MAX_TREE_DEPTH || out.len() >= MAX_NAMES {
        return;
    }
    if let Object::Ref(n, _) = node
        && !seen.insert(*n)
    {
        return;
    }
    let Some(dict) = reader.plain(node).and_then(|d| d.as_dict().cloned()) else { return };
    if let Some(Object::Array(names)) = dict.get(b"Names").and_then(|n| reader.plain(n)) {
        for pair in names.chunks(2) {
            if let [Object::Str(key), value] = pair {
                out.push((true, key.clone(), value.clone()));
            }
        }
    }
    if let Some(Object::Array(kids)) = dict.get(b"Kids").and_then(|k| reader.plain(k)) {
        for kid in kids {
            walk_names(reader, &kid, depth + 1, seen, out);
        }
    }
}

/// The page a destination (`[page /XYZ …]`, or `<< /D […] >>`) goes to.
fn dest_page(reader: &Reader<'_, '_>, tree: &PageTree, dest: &Object) -> Option<usize> {
    let dest = reader.plain(dest)?;
    let array = match &dest {
        Object::Array(items) => items.clone(),
        Object::Dict(dict) => reader.plain(dict.get(b"D")?)?.as_array()?.to_vec(),
        _ => return None,
    };
    match array.first()? {
        Object::Ref(number, _) => tree.leaves.iter().position(|leaf| leaf.number == *number),
        Object::Int(index) => usize::try_from(*index).ok().filter(|i| *i < tree.leaves.len()),
        _ => None,
    }
}

fn target_of(
    reader: &Reader<'_, '_>,
    tree: &PageTree,
    names: &[(bool, Vec<u8>, Object)],
    dict: &Dict,
) -> LinkTarget {
    let named_target = |dest: &Object| -> LinkTarget {
        match dest {
            Object::Str(bytes) | Object::Name(bytes) => {
                let page = names
                    .iter()
                    .find(|(_, key, _)| key == bytes)
                    .and_then(|(_, _, d)| dest_page(reader, tree, d));
                LinkTarget::Named { name: text_of(dest).unwrap_or_default(), page }
            }
            other => match dest_page(reader, tree, other) {
                Some(page) => LinkTarget::Page { page },
                None => LinkTarget::Other { action: "destination".into() },
            },
        }
    };
    if let Some(action) = dict.get(b"A").and_then(|a| reader.plain(a)) {
        let Some(action) = action.as_dict() else { return LinkTarget::Other { action: "unknown".into() } };
        let kind = action.get(b"S").and_then(Object::as_name).unwrap_or_default();
        return match kind {
            b"URI" => match action.get(b"URI").and_then(|u| reader.plain(u)) {
                Some(Object::Str(bytes)) => {
                    LinkTarget::Url { url: String::from_utf8_lossy(&bytes).into_owned() }
                }
                _ => LinkTarget::Other { action: "uri".into() },
            },
            b"GoTo" => match action.get(b"D").and_then(|d| reader.plain(d)) {
                Some(dest) => named_target(&dest),
                None => LinkTarget::Other { action: "destination".into() },
            },
            b"GoToR" | b"GoToE" => LinkTarget::Other { action: "remote".into() },
            b"Launch" => LinkTarget::Other { action: "launch".into() },
            b"JavaScript" => LinkTarget::Other { action: "javascript".into() },
            b"Named" => LinkTarget::Other { action: "named".into() },
            other => LinkTarget::Other { action: String::from_utf8_lossy(other).to_lowercase() },
        };
    }
    match dict.get(b"Dest").and_then(|d| reader.plain(d)) {
        Some(dest) => named_target(&dest),
        None => LinkTarget::Other { action: "none".into() },
    }
}

/// The areas a link covers on the displayed page.
fn rects_of(file: &File<'_>, dict: &Dict, view: &ViewTransform) -> Vec<ViewRect> {
    let corners = |points: &[(f32, f32)]| {
        let shown: Vec<(f32, f32)> = points.iter().map(|&(x, y)| view.point(x, y)).collect();
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (x, y) in shown {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        ViewRect { x: x0, y: y0, width: x1 - x0, height: y1 - y0 }
    };
    let mut rects = Vec::new();
    if let Some(Object::Array(points)) = dict.get(b"QuadPoints").and_then(|q| file.resolve(q)) {
        let values: Vec<f32> = points.iter().filter_map(number).collect();
        for quad in values.as_chunks::<8>().0.iter().take(MAX_RECTS) {
            let pts: Vec<(f32, f32)> = quad.as_chunks::<2>().0.iter().map(|p| (p[0], p[1])).collect();
            let rect = corners(&pts);
            if rect.width > 0.0 && rect.height > 0.0 {
                rects.push(rect);
            }
        }
    }
    if rects.is_empty()
        && let Some([l, b, r, t]) = rect_of(file, dict)
    {
        let rect = corners(&[(l, b), (r, t)]);
        if rect.width > 0.0 && rect.height > 0.0 {
            rects.push(rect);
        }
    }
    rects
}

/// A page's links, with where each goes.
pub fn page_links(
    bytes: &[u8],
    password: Option<&str>,
    view: &ViewTransform,
    page: usize,
) -> Result<Vec<LinkInfo>> {
    let Start { file, .. } = Start::read(bytes)?;
    let tree = PageTree::read(&file)?;
    let leaf =
        tree.leaves.get(page).ok_or(Error::PageOutOfRange { index: page, count: tree.leaves.len() })?;
    let reader = Reader { file: &file, crypt: Crypt::of(&file, password)? };
    let annots = Annots::read(&file, leaf)?;
    let owner = annots.owner(leaf);
    let mut names = None;
    let mut out = Vec::new();
    for (index, item) in annots.items.iter().enumerate() {
        let Some(Object::Dict(dict)) = reader.plain_in(item, owner) else { continue };
        if dict.get(b"Subtype").and_then(Object::as_name) != Some(b"Link") {
            continue;
        }
        let rects = rects_of(&file, &dict, view);
        if rects.is_empty() {
            continue;
        }
        let names = names.get_or_insert_with(|| named(&reader));
        let target = target_of(&reader, &tree, names, &dict);
        out.push(LinkInfo { index, rects, target });
    }
    Ok(out)
}

/// The document's named destinations, sorted by name.
pub fn named_destinations(bytes: &[u8], password: Option<&str>) -> Result<Vec<NamedDestination>> {
    let Start { file, .. } = Start::read(bytes)?;
    let tree = PageTree::read(&file)?;
    let reader = Reader { file: &file, crypt: Crypt::of(&file, password)? };
    let mut out: Vec<NamedDestination> = named(&reader)
        .iter()
        .map(|(tree_name, key, dest)| NamedDestination {
            name: if *tree_name {
                crate::forms::decode_text(key)
            } else {
                String::from_utf8_lossy(key).into_owned()
            },
            page: dest_page(&reader, &tree, dest),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.name == b.name);
    Ok(out)
}

/// The entries that send a link to `target`, as plain objects (encrypted by the caller).
fn target_entries(
    reader: &Reader<'_, '_>,
    tree: &PageTree,
    view_of: &dyn Fn(usize) -> Result<ViewTransform>,
    target: &LinkTarget,
) -> Result<(Vec<u8>, Object)> {
    match target {
        LinkTarget::Url { url } => {
            let url = normalise_url(url)?;
            let action = Dict(vec![
                (b"S".to_vec(), Object::Name(b"URI".to_vec())),
                (b"URI".to_vec(), Object::Str(url.into_bytes())),
            ]);
            Ok((b"A".to_vec(), Object::Dict(action)))
        }
        LinkTarget::Page { page } => {
            let leaf = tree
                .leaves
                .get(*page)
                .ok_or(Error::PageOutOfRange { index: *page, count: tree.leaves.len() })?;
            // The page's top-left corner as displayed, at the reader's zoom.
            let (left, top) = view_of(*page)?.page_point(0.0, 0.0);
            let real = |v: f32| Object::Real(f64::from(v));
            Ok((
                b"Dest".to_vec(),
                Object::Array(vec![
                    Object::Ref(leaf.number, leaf.generation),
                    Object::Name(b"XYZ".to_vec()),
                    real(left),
                    real(top),
                    Object::Null,
                ]),
            ))
        }
        LinkTarget::Named { name, .. } => {
            let names = named(reader);
            let found = names
                .iter()
                .find(|(tree_name, key, _)| {
                    let text = if *tree_name {
                        crate::forms::decode_text(key)
                    } else {
                        String::from_utf8_lossy(key).into_owned()
                    };
                    text == *name
                })
                .ok_or(Error::InvalidEdit("the document has no destination of that name"))?;
            let value = if found.0 { Object::Str(found.1.clone()) } else { Object::Name(found.1.clone()) };
            Ok((b"Dest".to_vec(), value))
        }
        LinkTarget::Other { .. } => {
            Err(Error::InvalidEdit("a link goes to a web address, a page, or a destination"))
        }
    }
}

fn link_rect(rects: &[ViewRect], view: &ViewTransform) -> Result<([f32; 4], Option<Object>)> {
    if rects.is_empty() || rects.len() > MAX_RECTS {
        return Err(Error::InvalidEdit("a link covers between one and 1,000 areas"));
    }
    let mut quads = Vec::new();
    for rect in rects {
        if ![rect.x, rect.y, rect.width, rect.height].iter().all(|v| v.is_finite())
            || rect.width < 1.0
            || rect.height < 1.0
        {
            return Err(Error::InvalidEdit("a link needs an area"));
        }
        quads.push(Quad::from_view(*rect, view));
    }
    let points: Vec<(f32, f32)> =
        quads.iter().flat_map(|q| [q.upper_left, q.upper_right, q.lower_left, q.lower_right]).collect();
    let bounds = crate::annotate::bounds(points.iter().copied());
    if !bounds.iter().all(|v| v.abs() <= crate::annotate::MAX_COORDINATE) {
        return Err(Error::InvalidEdit("a link lies outside the page"));
    }
    let quad_points = (rects.len() > 1).then(|| {
        Object::Array(
            points
                .iter()
                .flat_map(|&(x, y)| [Object::Real(f64::from(x)), Object::Real(f64::from(y))])
                .collect(),
        )
    });
    Ok((bounds, quad_points))
}

/// The bytes to append to `bytes` to make `edit`. `view_of` gives a page's
/// map to the page as displayed.
pub fn link_update(
    bytes: &[u8],
    password: Option<&str>,
    view_of: &dyn Fn(usize) -> Result<ViewTransform>,
    edit: &LinkEdit,
) -> Result<Vec<u8>> {
    let Start { file, section, form, mut next_number } = Start::read(bytes)?;
    let tree = PageTree::read(&file)?;
    let reader = Reader { file: &file, crypt: Crypt::of(&file, password)? };
    let leaf_of = |page: usize| {
        tree.leaves.get(page).ok_or(Error::PageOutOfRange { index: page, count: tree.leaves.len() })
    };
    let mut update = Update::new(bytes);
    let real = |v: f32| Object::Real(f64::from(v));
    let encrypt = |number: u32, generation: u16, object: &mut Object| {
        if let Some(crypt) = &reader.crypt {
            crypt.encrypt_strings(number, generation, object);
        }
    };
    // An existing link, as an object of its own: its number and plain value.
    let existing = |leaf: &Leaf, index: usize| -> Result<(u32, u16, Dict)> {
        let annots = Annots::read(&file, leaf)?;
        let item = annots.items.get(index).ok_or(Error::InvalidEdit("no such link"))?;
        let &Object::Ref(number, generation) = item else {
            return Err(Error::EditUnsupported("this link is not an object of its own"));
        };
        let dict = file
            .resolve(item)
            .and_then(|d| d.as_dict().cloned())
            .ok_or(Error::EditUnsupported("this link cannot be read"))?;
        if dict.get(b"Subtype").and_then(Object::as_name) != Some(b"Link") {
            return Err(Error::InvalidEdit("this annotation is not a link"));
        }
        Ok((number, generation, dict))
    };
    match edit {
        LinkEdit::Add { page, rects, target } => {
            let leaf = leaf_of(*page)?;
            let view = view_of(*page)?;
            let (bounds, quad_points) = link_rect(rects, &view)?;
            let (key, value) = target_entries(&reader, &tree, view_of, target)?;
            let number = next_number;
            next_number = number.checked_add(1).ok_or(Error::EditUnsupported("it has too many objects"))?;
            let now = pdf_date(SystemTime::now());
            let mut dict = vec![
                (b"Type".to_vec(), Object::Name(b"Annot".to_vec())),
                (b"Subtype".to_vec(), Object::Name(b"Link".to_vec())),
                (b"Rect".to_vec(), Object::Array(bounds.iter().map(|v| real(*v)).collect())),
                (b"Border".to_vec(), Object::Array(vec![Object::Int(0), Object::Int(0), Object::Int(0)])),
                (b"F".to_vec(), Object::Int(4)),
                (b"P".to_vec(), Object::Ref(leaf.number, leaf.generation)),
                (b"NM".to_vec(), Object::Str(unique_name())),
                (b"M".to_vec(), Object::Str(now)),
                (key, value),
            ];
            if let Some(quad_points) = quad_points {
                dict.push((b"QuadPoints".to_vec(), quad_points));
            }
            let mut dict = Object::Dict(Dict(dict));
            encrypt(number, 0, &mut dict);
            update.object(number, 0, &dict);
            let mut annots = Annots::read(&file, leaf)?;
            annots.items.push(Object::Ref(number, 0));
            annots.write(&mut update, leaf);
        }
        LinkEdit::SetTarget { page, index, target } => {
            let leaf = leaf_of(*page)?;
            let (number, generation, mut dict) = existing(leaf, *index)?;
            let (key, mut value) = target_entries(&reader, &tree, view_of, target)?;
            encrypt(number, generation, &mut value);
            dict.0.retain(|(k, _)| k != b"A" && k != b"Dest");
            dict.0.push((key, value));
            let mut date = Object::Str(pdf_date(SystemTime::now()));
            encrypt(number, generation, &mut date);
            set(&mut dict, b"M", date);
            update.object(number, generation, &Object::Dict(dict));
        }
        LinkEdit::SetRect { page, index, rect } => {
            let leaf = leaf_of(*page)?;
            let view = view_of(*page)?;
            let (number, generation, mut dict) = existing(leaf, *index)?;
            let (bounds, _) = link_rect(std::slice::from_ref(rect), &view)?;
            dict.0.retain(|(k, _)| k != b"QuadPoints");
            set(&mut dict, b"Rect", Object::Array(bounds.iter().map(|v| real(*v)).collect()));
            let mut date = Object::Str(pdf_date(SystemTime::now()));
            encrypt(number, generation, &mut date);
            set(&mut dict, b"M", date);
            update.object(number, generation, &Object::Dict(dict));
        }
        LinkEdit::Remove { page, index } => {
            let leaf = leaf_of(*page)?;
            let mut annots = Annots::read(&file, leaf)?;
            let item = annots.items.get(*index).cloned().ok_or(Error::InvalidEdit("no such link"))?;
            let owner = annots.owner(leaf);
            let dict = reader.plain_in(&item, owner).and_then(|d| d.as_dict().cloned());
            if dict.as_ref().and_then(|d| d.get(b"Subtype")).and_then(Object::as_name) != Some(b"Link") {
                return Err(Error::InvalidEdit("this annotation is not a link"));
            }
            annots.items.remove(*index);
            annots.write(&mut update, leaf);
        }
    }
    Ok(update.finish(form, file.trailer.clone(), section, next_number))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_normalised_and_held_to_web_and_mail() {
        assert_eq!(normalise_url("example.com/a").expect("valid"), "https://example.com/a");
        assert_eq!(normalise_url(" https://Example.com ").expect("valid"), "https://Example.com");
        assert_eq!(normalise_url("someone@example.com").expect("valid"), "mailto:someone@example.com");
        assert_eq!(
            normalise_url("https://مثال.com/ص").expect("valid"),
            "https://%D9%85%D8%AB%D8%A7%D9%84.com/%D8%B5"
        );
        assert!(normalise_url("javascript:alert(1)").is_err());
        assert!(normalise_url("file:///etc/passwd").is_err());
        assert!(normalise_url("two words.com").is_err());
        assert!(normalise_url("https://").is_err());
        assert!(may_open("https://example.com") && may_open("mailto:a@b.c"));
        assert!(!may_open("javascript:x") && !may_open("https://a b"));
        assert_eq!(openable("https://مثال.com").as_deref(), Some("https://%D9%85%D8%AB%D8%A7%D9%84.com"));
        assert_eq!(openable("https://a.b/\"&calc"), None);
    }
}
