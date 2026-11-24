//! Sanitizing (ADR 0042): what a file carries besides what its pages show.
//!
//! - **Metadata**: the document information dictionary, XMP packets, and
//!   private application data (`/PieceInfo`), wherever they hang.
//! - **Scripts**: JavaScript actions (document, page, field, link, and
//!   outline), additional-action triggers, and XFA forms.
//! - **Attachments**: embedded files, associated files, and file-attachment
//!   annotations.
//! - **Comments**: every annotation but links and form widgets.
//! - **Hidden content**: invisible text (render mode 3), text lying wholly off
//!   the page, and content in optional-content layers switched off.
//!
//! This writes an update that unlinks them; the caller then rewrites the
//! file in full (`Document::with_sanitized`), which is what drops the
//! objects nothing refers to any more.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::interp::{Area, IDENTITY, Rewriter, Settings};
use super::overlay::Overlay;
use super::{Fate, filter_annotations, flate_dict, page_content, refuse_encrypted, resources_of};
use crate::edit::{PageTree, Start};
use crate::error::{Error, Result};
use crate::pdf::write::Update;
use crate::pdf::{Dict, File, Object};

/// What to take out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SanitizeOptions {
    pub metadata: bool,
    pub scripts: bool,
    pub attachments: bool,
    pub comments: bool,
    pub hidden_content: bool,
}

impl Default for SanitizeOptions {
    fn default() -> Self {
        Self { metadata: true, scripts: true, attachments: true, comments: true, hidden_content: true }
    }
}

/// How much of each kind was taken out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SanitizeReport {
    pub metadata: usize,
    pub scripts: usize,
    pub attachments: usize,
    pub comments: usize,
    pub hidden: usize,
}

pub fn sanitize_update(bytes: &[u8], options: SanitizeOptions) -> Result<(Vec<u8>, SanitizeReport)> {
    let Start { file, section, form, next_number } = Start::read(bytes)?;
    refuse_encrypted(&file)?;
    let tree = PageTree::read(&file)?;
    let mut overlay = Overlay::new(&file);
    let mut report = SanitizeReport::default();
    let mut next = next_number;

    let off_layers = if options.hidden_content { off_layers(&overlay, &file) } else { HashSet::new() };
    for leaf in &tree.leaves {
        let mut page = leaf.dict.clone();
        let mut changed = false;
        if options.hidden_content {
            let resources = resources_of(&overlay, &tree, leaf);
            let data = page_content(&file, &leaf.dict)?;
            let settings = Settings {
                invisible_text: true,
                page_box: page_box(&overlay, &tree, leaf),
                off_layers: off_layers.clone(),
                every_form: true,
                ..Settings::default()
            };
            let mut rewriter = Rewriter::new(&file, settings, next);
            let rewritten = rewriter
                .rewrite(&data, &resources, IDENTITY, 0)
                .ok_or(Error::EditUnsupported("a page's content could not be read to sanitize it"))?;
            let counts = rewriter.counts;
            if counts.glyphs + counts.hidden + counts.paths + counts.images_blanked + counts.images_dropped
                > 0
            {
                report.hidden += counts.glyphs + counts.hidden;
                let number = rewriter.alloc().ok_or(Error::EditUnsupported("it has too many objects"))?;
                overlay.put_stream(
                    number,
                    0,
                    flate_dict(),
                    miniz_oxide::deflate::compress_to_vec_zlib(&rewritten.bytes, 6),
                );
                crate::edit::set(&mut page, b"Contents", Object::Ref(number, 0));
                if !rewritten.xobjects.is_empty() {
                    let patched = super::interp::with_xobjects(&file, &resources, &rewritten);
                    crate::edit::set(&mut page, b"Resources", Object::Dict(patched));
                }
                for (n, dict, data) in std::mem::take(&mut rewriter.writes) {
                    overlay.put_stream(n, 0, dict, data);
                }
                next = rewriter.next_number;
                changed = true;
            }
        }
        if options.comments || options.attachments {
            let (comments, attachments) = (options.comments, options.attachments);
            let mut removed_comments = 0;
            let mut removed_attachments = 0;
            let decided = filter_annotations(&mut overlay, &mut page, |_, dict| {
                match dict.get(b"Subtype").and_then(Object::as_name) {
                    Some(b"FileAttachment") if attachments => Fate::Remove,
                    Some(b"Link" | b"Widget" | b"FileAttachment") | None => Fate::Keep,
                    Some(_) if comments => Fate::Remove,
                    Some(_) => Fate::Keep,
                }
            });
            // The count includes pop-ups and replies; split it by kind below.
            if decided > 0 {
                changed = true;
                removed_comments += if comments { decided } else { 0 };
                removed_attachments += if attachments && !comments { decided } else { 0 };
            }
            report.comments += removed_comments;
            report.attachments += removed_attachments;
        }
        if changed {
            overlay.put(leaf.number, leaf.generation, Object::Dict(page));
        }
    }

    // Every object: the keys that carry metadata, scripts, and attachments.
    for number in overlay.numbers() {
        let Some((object, data)) = overlay.get(number) else { continue };
        let generation = overlay.generation(number);
        let (mut object, was_stream) = match (object, &data) {
            (Object::Dict(dict), Some(_)) => (Object::Dict(dict), true),
            (other, _) => (other, false),
        };
        let mut counts = SanitizeReport::default();
        let changed = match &mut object {
            Object::Dict(dict) => strip(&overlay, dict, options, &mut counts),
            _ => false,
        };
        if changed {
            match (object, data) {
                (Object::Dict(dict), Some(data)) if was_stream => {
                    overlay.put_stream(number, generation, dict, data)
                }
                (object, _) => overlay.put(number, generation, object),
            }
            report.metadata += counts.metadata;
            report.scripts += counts.scripts;
            report.attachments += counts.attachments;
        }
    }

    // A reader merges trailers across revisions, so a trailer without `/Info`
    // would leave the old one visible: the object itself is emptied instead.
    let mut trailer = file.trailer.clone();
    if options.metadata {
        match trailer.get(b"Info").cloned() {
            Some(Object::Ref(number, generation)) => {
                overlay.put(number, generation, Object::Dict(Dict::default()));
                report.metadata += 1;
            }
            Some(_) => {
                crate::edit::set(&mut trailer, b"Info", Object::Dict(Dict::default()));
                report.metadata += 1;
            }
            None => {}
        }
    }
    let mut update = Update::new(bytes);
    overlay.write(&mut update);
    Ok((update.finish(form, trailer, section, next), report))
}

/// Removes from `dict`, and from the dictionaries direct inside it, the keys
/// `options` asks for. Returns whether anything went.
fn strip(
    overlay: &Overlay<'_, '_>,
    dict: &mut Dict,
    options: SanitizeOptions,
    counts: &mut SanitizeReport,
) -> bool {
    let before = dict.0.len();
    let drop = |dict: &mut Dict, keys: &[&[u8]]| -> usize {
        let n = dict.0.len();
        dict.0.retain(|(k, _)| !keys.contains(&k.as_slice()));
        n - dict.0.len()
    };
    if options.metadata {
        counts.metadata += drop(dict, &[b"Metadata", b"PieceInfo"]);
    }
    if options.attachments {
        counts.attachments += drop(dict, &[b"AF", b"EmbeddedFiles"]);
    }
    if options.scripts {
        counts.scripts += drop(dict, &[b"AA", b"XFA"]);
        // `/JavaScript` as a name tree of a Names dictionary.
        let tree = dict.get(b"JavaScript").and_then(|t| overlay.resolve(t));
        if tree.is_some_and(|t| {
            t.as_dict().is_some_and(|t| t.get(b"Names").is_some() || t.get(b"Kids").is_some())
        }) {
            counts.scripts += drop(dict, &[b"JavaScript"]);
        }
        for key in [&b"A"[..], b"OpenAction"] {
            if let Some(value) = dict.get(key)
                && is_script(overlay, value)
            {
                counts.scripts += drop(dict, &[key]);
            }
        }
    }
    let mut changed = dict.0.len() != before;
    for (_, value) in &mut dict.0 {
        changed |= strip_nested(overlay, value, options, counts);
    }
    changed
}

fn strip_nested(
    overlay: &Overlay<'_, '_>,
    value: &mut Object,
    options: SanitizeOptions,
    counts: &mut SanitizeReport,
) -> bool {
    match value {
        Object::Dict(dict) => strip(overlay, dict, options, counts),
        Object::Array(items) => {
            items.iter_mut().fold(false, |any, item| strip_nested(overlay, item, options, counts) | any)
        }
        _ => false,
    }
}

/// Whether an action (or a chain of them) runs JavaScript.
fn is_script(overlay: &Overlay<'_, '_>, action: &Object) -> bool {
    let mut at = overlay.resolve(action);
    for _ in 0..16 {
        let Some(Object::Dict(dict)) = at else { return false };
        if matches!(dict.get(b"S").and_then(Object::as_name), Some(b"JavaScript" | b"ImportData" | b"Launch"))
            || dict.get(b"JS").is_some()
        {
            return true;
        }
        at = match dict.get(b"Next").and_then(|n| overlay.resolve(n)) {
            Some(Object::Array(items)) => {
                return items.iter().any(|item| is_script(overlay, item));
            }
            other => other,
        };
    }
    false
}

/// The page's visible box: `/CropBox`, else `/MediaBox`.
fn page_box(overlay: &Overlay<'_, '_>, tree: &PageTree, leaf: &crate::edit::Leaf) -> Option<Area> {
    let find = |key: &[u8]| {
        leaf.dict
            .get(key)
            .cloned()
            .or_else(|| leaf.inherited_below_root.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()))
            .or_else(|| tree.root.get(key).cloned())
    };
    let array = find(b"CropBox").or_else(|| find(b"MediaBox"))?;
    let Object::Array(values) = overlay.resolve(&array)? else { return None };
    let v: Vec<f64> = values
        .iter()
        .filter_map(|v| match overlay.resolve(v)? {
            Object::Int(i) => Some(i as f64),
            Object::Real(r) => Some(r),
            _ => None,
        })
        .collect();
    let [a, b, c, d] = v.as_slice() else { return None };
    Some([a.min(*c), b.min(*d), a.max(*c), b.max(*d)])
}

/// The optional-content groups switched off in the default configuration.
fn off_layers(overlay: &Overlay<'_, '_>, file: &File<'_>) -> HashSet<u32> {
    let mut off = HashSet::new();
    let Some(catalog) = file.trailer.get(b"Root").and_then(|r| overlay.resolve(r)) else { return off };
    let Some(properties) =
        catalog.as_dict().and_then(|c| c.get(b"OCProperties")).and_then(|p| overlay.resolve(p))
    else {
        return off;
    };
    let Some(properties) = properties.as_dict() else { return off };
    let Some(config) = properties.get(b"D").and_then(|d| overlay.resolve(d)) else { return off };
    let Some(config) = config.as_dict() else { return off };
    let refs = |key: &[u8], dict: &Dict| -> Vec<u32> {
        match dict.get(key).and_then(|a| overlay.resolve(a)) {
            Some(Object::Array(items)) => {
                items.iter().filter_map(|i| if let Object::Ref(n, _) = i { Some(*n) } else { None }).collect()
            }
            _ => Vec::new(),
        }
    };
    if config.get(b"BaseState").and_then(Object::as_name) == Some(b"OFF") {
        let on: HashSet<u32> = refs(b"ON", config).into_iter().collect();
        off.extend(refs(b"OCGs", properties).into_iter().filter(|n| !on.contains(n)));
    }
    off.extend(refs(b"OFF", config));
    off
}
