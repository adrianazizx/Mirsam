//! Page edits, written as incremental updates (ADR 0014).
//!
//! An edit reads the document's page tree with the crate's own parser and
//! appends an update that changes only what it must:
//! - rotating rewrites the turned pages' dictionaries (`/Rotate`);
//! - deleting, moving, inserting, and duplicating rewrite the root `/Pages`
//!   node with every page as a direct kid, in the new order.
//!
//! Page objects keep their object numbers, so everything that points at a
//! page (outline entries, links, form fields, annotations' `/P`) still does.
//! A page that hung below an intermediate node takes the attributes it
//! inherited from there (`/Resources`, `/MediaBox`, `/CropBox`, `/Rotate`)
//! along when it moves under the root. Nodes left without a parent, and
//! deleted pages, stay in the file's earlier revision; a full save drops them.

use std::collections::HashSet;

use crate::error::{Error, Result};
use crate::pdf::write::{Update, XrefForm};
use crate::pdf::{Dict, File, MAX_DEPTH, Object, Parser, startxref};

/// One change to a document's pages. Page numbers are zero-based indices into
/// the document as it is before the edit.
#[derive(Debug, Clone, PartialEq)]
pub enum PageEdit {
    /// Turns pages clockwise by `quarter_turns` × 90° (negative: anticlockwise).
    Rotate { pages: Vec<usize>, quarter_turns: i32 },
    /// Removes pages. A document keeps at least one.
    Delete { pages: Vec<usize> },
    /// Moves pages, keeping their order among themselves, so that the first
    /// of them ends up at index `to` of the resulting document.
    Move { pages: Vec<usize>, to: usize },
    /// Inserts an empty page of `width` × `height` points before index `at`
    /// (`at` = page count appends).
    InsertBlank { at: usize, width: f32, height: f32 },
    /// Inserts a copy of each page right after it. Copies share the original's
    /// content and resources, and leave out its annotations, which belong to
    /// one page only.
    Duplicate { pages: Vec<usize> },
}

/// Attributes a page inherits from its ancestors (ISO 32000-1, table 30).
const INHERITABLE: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];
/// Bounds for hostile page trees.
const MAX_PAGES: usize = 1_000_000;
/// The largest blank page, in points (PDF's limit is 14,400).
const MAX_PAGE_SIDE: f32 = 14_400.0;

/// The bytes to append to `bytes` to apply `edit`: an incremental update.
pub fn page_edit_update(bytes: &[u8], edit: &PageEdit) -> Result<Vec<u8>> {
    let file = File::new(bytes).ok_or(Error::EditUnsupported("its cross-reference table cannot be read"))?;
    let tree = PageTree::read(&file)?;
    let count = tree.leaves.len();
    let check = |pages: &[usize]| -> Result<Vec<usize>> {
        let mut pages: Vec<usize> = pages.to_vec();
        pages.sort_unstable();
        pages.dedup();
        if pages.is_empty() {
            return Err(Error::InvalidEdit("no pages given"));
        }
        if let Some(&last) = pages.last()
            && last >= count
        {
            return Err(Error::PageOutOfRange { index: last, count });
        }
        Ok(pages)
    };

    let section =
        startxref(bytes).ok_or(Error::EditUnsupported("its cross-reference table cannot be read"))?;
    let form = if Parser::new(bytes, section).word() == b"xref" { XrefForm::Table } else { XrefForm::Stream };
    let mut next_number = file
        .trailer
        .get(b"Size")
        .and_then(Object::as_int)
        .and_then(|size| u32::try_from(size).ok())
        .ok_or(Error::EditUnsupported("its trailer has no /Size"))?;
    let encrypted = file.trailer.get(b"Encrypt").is_some();
    let mut update = Update::new(bytes);

    match edit {
        PageEdit::Rotate { pages, quarter_turns } => {
            for index in check(pages)? {
                let leaf = &tree.leaves[index];
                let rotate = (leaf.rotate + 90 * i64::from(*quarter_turns)).rem_euclid(360);
                let mut dict = leaf.dict.clone();
                set(&mut dict, b"Rotate", Object::Int(rotate));
                update.object(leaf.number, leaf.generation, &Object::Dict(dict));
            }
        }
        PageEdit::Delete { .. }
        | PageEdit::Move { .. }
        | PageEdit::InsertBlank { .. }
        | PageEdit::Duplicate { .. } => {
            let mut order: Vec<Slot> = (0..count).map(Slot::Existing).collect();
            match edit {
                PageEdit::Delete { pages } => {
                    let doomed: HashSet<usize> = check(pages)?.into_iter().collect();
                    if doomed.len() == count {
                        return Err(Error::InvalidEdit("a document keeps at least one page"));
                    }
                    order.retain(|slot| !matches!(slot, Slot::Existing(i) if doomed.contains(i)));
                }
                PageEdit::Move { pages, to } => {
                    let moving: HashSet<usize> = check(pages)?.into_iter().collect();
                    let moved: Vec<Slot> =
                        (0..count).filter(|i| moving.contains(i)).map(Slot::Existing).collect();
                    order.retain(|slot| !matches!(slot, Slot::Existing(i) if moving.contains(i)));
                    let at = (*to).min(order.len());
                    order.splice(at..at, moved);
                }
                PageEdit::InsertBlank { at, width, height } => {
                    let valid = |side: f32| side.is_finite() && (1.0..=MAX_PAGE_SIDE).contains(&side);
                    if !valid(*width) || !valid(*height) {
                        return Err(Error::InvalidEdit("blank page size out of range"));
                    }
                    if *at > count {
                        return Err(Error::PageOutOfRange { index: *at, count });
                    }
                    let page = Dict(vec![
                        (b"Type".to_vec(), Object::Name(b"Page".to_vec())),
                        (
                            b"MediaBox".to_vec(),
                            Object::Array(vec![
                                Object::Int(0),
                                Object::Int(0),
                                Object::Real(f64::from(*width)),
                                Object::Real(f64::from(*height)),
                            ]),
                        ),
                        (b"Resources".to_vec(), Object::Dict(Dict::default())),
                        // An explicit 0: the root may hand down another rotation.
                        (b"Rotate".to_vec(), Object::Int(0)),
                    ]);
                    order.insert(*at, Slot::New(page));
                }
                PageEdit::Duplicate { pages } => {
                    let copies: HashSet<usize> = check(pages)?.into_iter().collect();
                    let mut with_copies = Vec::with_capacity(order.len() + copies.len());
                    for slot in order {
                        let copy = match &slot {
                            Slot::Existing(i) if copies.contains(i) => Some(tree.leaves[*i].copy(encrypted)?),
                            _ => None,
                        };
                        with_copies.push(slot);
                        with_copies.extend(copy.map(Slot::New));
                    }
                    order = with_copies;
                }
                PageEdit::Rotate { .. } => {}
            }

            let root = Object::Ref(tree.root_number, tree.root_generation);
            let mut kids = Vec::with_capacity(order.len());
            for slot in &order {
                match slot {
                    Slot::Existing(i) => {
                        let leaf = &tree.leaves[*i];
                        kids.push(Object::Ref(leaf.number, leaf.generation));
                    }
                    Slot::New(dict) => {
                        let mut dict = dict.clone();
                        set(&mut dict, b"Parent", root.clone());
                        update.object(next_number, 0, &Object::Dict(dict));
                        kids.push(Object::Ref(next_number, 0));
                        next_number = next_number
                            .checked_add(1)
                            .ok_or(Error::EditUnsupported("it has too many objects"))?;
                    }
                }
            }
            // Pages that hung below an intermediate node now hang from the root.
            for slot in &order {
                if let Slot::Existing(i) = slot
                    && let leaf = &tree.leaves[*i]
                    && !leaf.under_root
                {
                    let mut dict = leaf.dict.clone();
                    for (key, value) in &leaf.inherited_below_root {
                        set(&mut dict, key, value.clone());
                    }
                    set(&mut dict, b"Parent", root.clone());
                    update.object(leaf.number, leaf.generation, &Object::Dict(dict));
                }
            }
            let mut root_dict = tree.root.clone();
            let total = i64::try_from(kids.len()).map_err(|_| Error::InvalidEdit("too many pages"))?;
            set(&mut root_dict, b"Kids", Object::Array(kids));
            set(&mut root_dict, b"Count", Object::Int(total));
            update.object(tree.root_number, tree.root_generation, &Object::Dict(root_dict));
        }
    }
    Ok(update.finish(form, file.trailer.clone(), section, next_number))
}

enum Slot {
    Existing(usize),
    New(Dict),
}

struct Leaf {
    number: u32,
    generation: u16,
    dict: Dict,
    /// Effective rotation in degrees, own or inherited.
    rotate: i64,
    /// Whether the root `/Pages` node is the page's parent.
    under_root: bool,
    /// Inheritable attributes the page takes from an ancestor other than the
    /// root (it keeps those from the root wherever it moves).
    inherited_below_root: Vec<(Vec<u8>, Object)>,
}

impl Leaf {
    /// A new page dictionary showing the same content: every attribute it
    /// shows, minus what may belong to one page only.
    fn copy(&self, encrypted: bool) -> Result<Dict> {
        let mut dict = self.dict.clone();
        for (key, value) in &self.inherited_below_root {
            set(&mut dict, key, value.clone());
        }
        dict.0.retain(|(key, _)| {
            !matches!(key.as_slice(), b"Annots" | b"StructParents" | b"B" | b"Parent" | b"PieceInfo" | b"ID")
        });
        // Strings in an encrypted file are encrypted with their object's
        // number; under a new number they would no longer decrypt.
        if encrypted && dict.0.iter().any(|(_, value)| contains_string(value)) {
            return Err(Error::EditUnsupported("this page of an encrypted file cannot be copied yet"));
        }
        Ok(dict)
    }
}

struct PageTree {
    root_number: u32,
    root_generation: u16,
    root: Dict,
    leaves: Vec<Leaf>,
}

impl PageTree {
    fn read(file: &File<'_>) -> Result<Self> {
        let catalog =
            file.trailer.get(b"Root").and_then(|root| file.resolve(root)).ok_or_else(|| unreadable(file))?;
        let Some(&Object::Ref(root_number, root_generation)) =
            catalog.as_dict().and_then(|c| c.get(b"Pages"))
        else {
            return Err(unreadable(file));
        };
        let root = file
            .resolve(&Object::Ref(root_number, root_generation))
            .and_then(|root| root.as_dict().cloned())
            .ok_or_else(|| unreadable(file))?;

        let mut tree = PageTree { root_number, root_generation, root: root.clone(), leaves: Vec::new() };
        let mut visited = HashSet::from([root_number]);
        let inherited: Vec<(Vec<u8>, Object, bool)> = inheritable(&root).map(|(k, v)| (k, v, true)).collect();
        tree.walk(file, &root, &inherited, true, 0, &mut visited)?;
        if tree.leaves.is_empty() {
            return Err(unreadable(file));
        }
        Ok(tree)
    }

    /// Collects the leaves below `node` in order. `inherited` holds the
    /// inheritable attributes in force, each with whether the root set it.
    fn walk(
        &mut self,
        file: &File<'_>,
        node: &Dict,
        inherited: &[(Vec<u8>, Object, bool)],
        is_root: bool,
        depth: usize,
        visited: &mut HashSet<u32>,
    ) -> Result<()> {
        let unreadable = || unreadable(file);
        if depth > MAX_DEPTH {
            return Err(unreadable());
        }
        let kids = node.get(b"Kids").and_then(|kids| file.resolve(kids)).ok_or_else(unreadable)?;
        for kid in kids.as_array().ok_or_else(unreadable)? {
            let &Object::Ref(number, generation) = kid else { return Err(unreadable()) };
            if !visited.insert(number) || self.leaves.len() >= MAX_PAGES {
                return Err(unreadable());
            }
            let dict = file.resolve(kid).and_then(|kid| kid.as_dict().cloned()).ok_or_else(unreadable)?;
            let is_node = match dict.get(b"Type").and_then(Object::as_name) {
                Some(b"Pages") => true,
                Some(b"Page") => false,
                _ => dict.get(b"Kids").is_some(),
            };
            if is_node {
                let mut below: Vec<(Vec<u8>, Object, bool)> = inherited.to_vec();
                for (key, value) in inheritable(&dict) {
                    below.retain(|(k, ..)| *k != key);
                    below.push((key, value, false));
                }
                self.walk(file, &dict, &below, false, depth + 1, visited)?;
            } else {
                let own = |key: &[u8]| dict.get(key).is_some();
                let rotate = dict
                    .get(b"Rotate")
                    .or_else(|| inherited.iter().find(|(k, ..)| k == b"Rotate").map(|(_, v, _)| v))
                    .and_then(|value| file.resolve(value))
                    .and_then(|value| value.as_int())
                    .unwrap_or(0);
                let inherited_below_root = inherited
                    .iter()
                    .filter(|(key, _, from_root)| !from_root && !own(key))
                    .map(|(key, value, _)| (key.clone(), value.clone()))
                    .collect();
                self.leaves.push(Leaf {
                    number,
                    generation,
                    dict,
                    rotate,
                    under_root: is_root,
                    inherited_below_root,
                });
            }
        }
        Ok(())
    }
}

fn inheritable(dict: &Dict) -> impl Iterator<Item = (Vec<u8>, Object)> + '_ {
    dict.0
        .iter()
        .filter(|(key, _)| INHERITABLE.contains(&key.as_slice()))
        .map(|(k, v)| (k.clone(), v.clone()))
}

fn set(dict: &mut Dict, key: &[u8], value: Object) {
    match dict.0.iter_mut().find(|(k, _)| k == key) {
        Some((_, slot)) => *slot = value,
        None => dict.0.push((key.to_vec(), value)),
    }
}

fn contains_string(object: &Object) -> bool {
    match object {
        Object::Str(_) => true,
        Object::Array(items) => items.iter().any(contains_string),
        Object::Dict(dict) => dict.0.iter().any(|(_, value)| contains_string(value)),
        _ => false,
    }
}

/// Why the page tree could not be read: this reader does not decrypt, so in
/// an encrypted file whose objects sit in object streams it sees nothing.
fn unreadable(file: &File<'_>) -> Error {
    if file.trailer.get(b"Encrypt").is_some() {
        Error::EditUnsupported("its page tree is in an encrypted object stream, which cannot be read yet")
    } else {
        Error::EditUnsupported("its page tree cannot be read")
    }
}
