//! True redaction and sanitizing (ADR 0042).
//!
//! Redaction removes what is under the marked areas from the file, rather
//! than drawing over it: glyphs are cut out of text strings, image pixels are
//! overwritten, paths and annotations under the areas are dropped, and the
//! redacted strings are scrubbed out of metadata. A black box is then drawn
//! over each area, so what was there is visibly gone. It is written as an
//! incremental update like every other edit (ADR 0014), so undo works; the
//! full rewrite that saving does (ADR 0015) leaves the old content behind in
//! no object. A signed file is rewritten in full at once (`Document`).
//!
//! Sanitizing ([`sanitize`]) strips what a file carries besides its pages.

mod content;
mod find;
mod fonts;
mod image;
mod interp;
mod overlay;
mod sanitize;
mod scrub;

use std::collections::HashSet;

pub use find::{Hit, Pattern, Preset, find_matches};
pub use interp::Counts;
pub use sanitize::{SanitizeOptions, SanitizeReport, sanitize_update};

use crate::edit::{PageTree, Start};
use crate::error::{Error, Result};
use crate::pdf::write::Update;
use crate::pdf::{Dict, Entry, File, Object, decode_stream};
use crate::text::Rect;
use interp::{Area, IDENTITY, Rewriter, Settings, with_xobjects};
use overlay::Overlay;

/// An area to redact, on one page.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    /// Zero-based page index.
    pub page: usize,
    /// In page space (points, y up, before the page's rotation).
    pub rect: Rect,
    /// The text under it, when the mark came from a search: it is scrubbed
    /// from the rest of the file too.
    pub text: Option<String>,
}

/// What a redaction removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// Pages changed.
    pub pages: usize,
    pub counts: Counts,
    /// Annotations removed or emptied.
    pub annotations: usize,
    /// Strings of metadata and the like that held redacted text.
    pub strings: usize,
}

/// The most marks one redaction may carry.
const MAX_MARKS: usize = 100_000;

/// The update that applies `marks` to `bytes`.
pub fn redaction_update(bytes: &[u8], marks: &[Mark]) -> Result<(Vec<u8>, Report)> {
    if marks.is_empty() || marks.len() > MAX_MARKS {
        return Err(Error::InvalidEdit("no areas to redact"));
    }
    let Start { file, section, form, next_number } = Start::read(bytes)?;
    refuse_encrypted(&file)?;
    let tree = PageTree::read(&file)?;
    let count = tree.leaves.len();
    let mut areas: std::collections::BTreeMap<usize, Vec<Area>> = std::collections::BTreeMap::new();
    for mark in marks {
        if mark.page >= count {
            return Err(Error::PageOutOfRange { index: mark.page, count });
        }
        let Rect { left, bottom, right, top } = mark.rect;
        if ![left, bottom, right, top].iter().all(|v| v.is_finite() && v.abs() < 1.0e7) {
            return Err(Error::InvalidEdit("an area to redact is not a rectangle"));
        }
        let area = [left.min(right), bottom.min(top), left.max(right), bottom.max(top)].map(f64::from);
        if area[2] - area[0] <= 0.0 || area[3] - area[1] <= 0.0 {
            return Err(Error::InvalidEdit("an area to redact is empty"));
        }
        areas.entry(mark.page).or_default().push(area);
    }

    let mut overlay = Overlay::new(&file);
    let mut report = Report::default();
    let mut next = next_number;
    for (&page, page_areas) in &areas {
        let leaf = &tree.leaves[page];
        let resources = resources_of(&overlay, &tree, leaf);
        let data = page_content(&file, &leaf.dict)?;
        let settings = Settings { areas: page_areas.clone(), ..Settings::default() };
        let mut rewriter = Rewriter::new(&file, settings, next);
        let rewritten = rewriter
            .rewrite(&data, &resources, IDENTITY, 0)
            .ok_or(Error::EditUnsupported("a page's content could not be read to redact it"))?;

        // The rewritten content, then a black box over each area.
        let mut content = b"q\n".to_vec();
        content.extend_from_slice(&rewritten.bytes);
        content.extend_from_slice(b"\nQ\nq\n0 0 0 rg\n");
        for [l, b, r, t] in page_areas {
            content.extend_from_slice(format!("{l:.3} {b:.3} {:.3} {:.3} re\n", r - l, t - b).as_bytes());
        }
        content.extend_from_slice(b"f\nQ\n");
        let Some(content_number) = rewriter.alloc() else {
            return Err(Error::EditUnsupported("it has too many objects"));
        };
        overlay.put_stream(
            content_number,
            0,
            flate_dict(),
            miniz_oxide::deflate::compress_to_vec_zlib(&content, 6),
        );

        let mut page_dict = leaf.dict.clone();
        crate::edit::set(&mut page_dict, b"Contents", Object::Ref(content_number, 0));
        if !rewritten.xobjects.is_empty() {
            let patched = with_xobjects(&file, &resources, &rewritten);
            crate::edit::set(&mut page_dict, b"Resources", Object::Dict(patched));
        }
        // A thumbnail shows the page as it was.
        page_dict.0.retain(|(k, _)| k != b"Thumb");
        report.annotations += redact_annotations(&mut overlay, &mut page_dict, page_areas);
        overlay.put(leaf.number, leaf.generation, Object::Dict(page_dict));

        report.counts.add(rewriter.counts);
        report.pages += 1;
        next = rewriter.next_number;
        for (number, dict, data) in rewriter.writes {
            overlay.put_stream(number, 0, dict, data);
        }
    }

    let texts: Vec<String> = marks.iter().filter_map(|m| m.text.clone()).collect();
    report.strings = scrub::scrub_all(&mut overlay, &texts);

    let mut update = Update::new(bytes);
    overlay.write(&mut update);
    Ok((update.finish(form, file.trailer.clone(), section, next), report))
}

pub(crate) fn refuse_encrypted(file: &File<'_>) -> Result<()> {
    if file.trailer.get(b"Encrypt").is_some() {
        return Err(Error::EditUnsupported(
            "it is encrypted; redacting and sanitizing need it decrypted first",
        ));
    }
    Ok(())
}

pub(crate) fn flate_dict() -> Dict {
    Dict(vec![(b"Filter".to_vec(), Object::Name(b"FlateDecode".to_vec()))])
}

/// A page's resources, own or inherited.
pub(crate) fn resources_of(overlay: &Overlay<'_, '_>, tree: &PageTree, leaf: &crate::edit::Leaf) -> Dict {
    let found = leaf
        .dict
        .get(b"Resources")
        .cloned()
        .or_else(|| leaf.inherited_below_root.iter().find(|(k, _)| k == b"Resources").map(|(_, v)| v.clone()))
        .or_else(|| tree.root.get(b"Resources").cloned());
    found.and_then(|r| overlay.resolve(&r)).and_then(|r| r.as_dict().cloned()).unwrap_or_default()
}

/// A page's content streams, decoded and joined.
pub(crate) fn page_content(file: &File<'_>, page: &Dict) -> Result<Vec<u8>> {
    let unreadable = || Error::EditUnsupported("a page's content uses an encoding this editor cannot read");
    let Some(contents) = page.get(b"Contents") else { return Ok(Vec::new()) };
    let items = match file.resolve(contents) {
        Some(Object::Array(items)) => items,
        Some(Object::Dict(_)) => vec![contents.clone()],
        Some(Object::Null) => Vec::new(),
        _ => return Err(unreadable()),
    };
    let mut joined = Vec::new();
    for item in items {
        let Object::Ref(number, _) = item else { return Err(unreadable()) };
        let Some(Entry::Offset(offset)) = file.entries.get(&number).copied() else {
            return Err(unreadable());
        };
        let (dict, raw) = file.indirect_at(offset).ok_or_else(unreadable)?;
        let data = decode_stream(&dict, raw.ok_or_else(unreadable)?, file).ok_or_else(unreadable)?;
        joined.extend_from_slice(&data);
        joined.push(b'\n');
    }
    Ok(joined)
}

/// What happens to an annotation.
pub(crate) enum Fate {
    Keep,
    Remove,
    /// A widget: it stays, but its field's value and look go.
    Clear,
}

/// Removes the annotations of a page that lie under `areas` (widgets lose
/// their value instead, since the field outlives the widget). Returns how
/// many were changed.
fn redact_annotations(overlay: &mut Overlay<'_, '_>, page: &mut Dict, areas: &[Area]) -> usize {
    filter_annotations(overlay, page, |overlay, dict| {
        let Some(Object::Array(values)) = dict.get(b"Rect").and_then(|r| overlay.resolve(r)) else {
            return Fate::Keep;
        };
        let v: Vec<f64> = values
            .iter()
            .filter_map(|v| match overlay.resolve(v)? {
                Object::Int(i) => Some(i as f64),
                Object::Real(r) => Some(r),
                _ => None,
            })
            .collect();
        let [a, b, c, d] = v.as_slice() else { return Fate::Keep };
        let rect = [a.min(*c), b.min(*d), a.max(*c), b.max(*d)];
        let touches =
            areas.iter().any(|a| a[0] < rect[2] && rect[0] < a[2] && a[1] < rect[3] && rect[1] < a[3]);
        match (touches, dict.get(b"Subtype").and_then(Object::as_name)) {
            (false, _) => Fate::Keep,
            (true, Some(b"Widget")) => Fate::Clear,
            (true, _) => Fate::Remove,
        }
    })
}

/// Applies `decide` to each annotation of a page: removed ones leave the
/// page's `/Annots` along with their pop-ups and replies; cleared widgets
/// (and the fields above them) lose `/V`, `/DV`, and their appearance.
pub(crate) fn filter_annotations(
    overlay: &mut Overlay<'_, '_>,
    page: &mut Dict,
    decide: impl Fn(&Overlay<'_, '_>, &Dict) -> Fate,
) -> usize {
    let (own, mut items) = match page.get(b"Annots") {
        Some(Object::Array(items)) => (None, items.clone()),
        Some(reference @ Object::Ref(number, generation)) => match overlay.resolve(reference) {
            Some(Object::Array(items)) => (Some((*number, *generation)), items),
            _ => return 0,
        },
        _ => return 0,
    };
    let dict_of =
        |overlay: &Overlay<'_, '_>, item: &Object| overlay.resolve(item).and_then(|o| o.as_dict().cloned());
    let mut doomed: HashSet<usize> = HashSet::new();
    let mut widgets: Vec<usize> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let Some(dict) = dict_of(overlay, item) else { continue };
        match decide(overlay, &dict) {
            Fate::Keep => {}
            Fate::Remove => {
                doomed.insert(i);
            }
            Fate::Clear => widgets.push(i),
        }
    }
    // Pop-ups and replies go with the annotation they belong to.
    loop {
        let before = doomed.len();
        let gone: Vec<Object> = doomed.iter().filter_map(|i| items.get(*i).cloned()).collect();
        for (i, item) in items.iter().enumerate() {
            if doomed.contains(&i) {
                continue;
            }
            let Some(dict) = dict_of(overlay, item) else { continue };
            let parent = dict.get(b"IRT").or_else(|| dict.get(b"Parent"));
            let is_popup = dict.get(b"Subtype").and_then(Object::as_name) == Some(b"Popup");
            if parent.is_some_and(|p| gone.contains(p) && (is_popup || dict.get(b"IRT").is_some())) {
                doomed.insert(i);
            }
        }
        for object in &gone {
            if let Some(dict) = dict_of(overlay, object)
                && let Some(popup) = dict.get(b"Popup")
                && let Some(i) = items.iter().position(|item| item == popup)
            {
                doomed.insert(i);
            }
        }
        if doomed.len() == before {
            break;
        }
    }
    let changed = doomed.len() + widgets.len();
    if changed == 0 {
        return 0;
    }
    for &i in &widgets {
        let item = items[i].clone();
        let Some(mut dict) = dict_of(overlay, &item) else { continue };
        let parent = dict.get(b"Parent").cloned();
        dict.0.retain(|(k, _)| !matches!(k.as_slice(), b"V" | b"DV" | b"AP" | b"RV"));
        match item {
            Object::Ref(number, generation) => overlay.put(number, generation, Object::Dict(dict)),
            _ => items[i] = Object::Dict(dict),
        }
        // A field holds its value for all of its widgets.
        let mut at = parent;
        for _ in 0..16 {
            let Some(Object::Ref(number, generation)) = at else { break };
            let Some(mut field) =
                overlay.resolve(&Object::Ref(number, generation)).and_then(|o| o.as_dict().cloned())
            else {
                break;
            };
            field.0.retain(|(k, _)| !matches!(k.as_slice(), b"V" | b"DV" | b"RV"));
            at = field.get(b"Parent").cloned();
            overlay.put(number, generation, Object::Dict(field));
        }
    }
    let mut position = 0;
    items.retain(|_| {
        let keep = !doomed.contains(&position);
        position += 1;
        keep
    });
    match own {
        Some((number, generation)) => overlay.put(number, generation, Object::Array(items)),
        None => crate::edit::set(page, b"Annots", Object::Array(items)),
    }
    changed
}
