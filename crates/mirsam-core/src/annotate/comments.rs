//! Comments (ADR 0041): the annotations a review is made of, with their
//! replies and review state, as the comments panel lists them; and the two
//! edits that answer one, a reply and marking it resolved.
//!
//! A reply is a `/Text` annotation whose `/IRT` ("in reply to") names the
//! comment it answers. A review state is one with `/IRT`, `/StateModel
//! /Review` and `/State` (ISO 32000-2, 12.5.6.10); the latest one counts, and
//! `/Completed` means resolved. PDFium's safe API exposes neither `/IRT` nor
//! the state entries, so this module reads them with our own parser (names and
//! references are never encrypted). Text, authors and dates come from PDFium,
//! which decrypts them.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use super::{AnnotationKind, Annots, Appearance, resolve_dict};
use crate::edit::{PageTree, Start};
use crate::pdf::{Dict, Object};
use crate::text::Rect;

/// A comment: an annotation of a kind the app knows that answers nothing.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub page: usize,
    /// Its position in the page's `/Annots`.
    pub index: usize,
    pub kind: AnnotationKind,
    /// `/T`.
    pub author: Option<String>,
    /// `/M`, else `/CreationDate`, in Unix seconds.
    pub modified: Option<i64>,
    /// `/Contents`: a note's or a text box's text, or a comment on markup.
    pub contents: Option<String>,
    /// For markup, the page's text under it, in reading order.
    pub marked_text: Option<String>,
    /// Page-space bounds.
    pub bounds: Rect,
    /// Its latest review state is `/Completed`.
    pub resolved: bool,
    /// Oldest first.
    pub replies: Vec<Reply>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reply {
    pub index: usize,
    pub author: Option<String>,
    pub modified: Option<i64>,
    pub contents: String,
}

/// A page's replies and review states, by position in its `/Annots`.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct Threads {
    /// Each reply → the comment it answers (a reply to a reply → the comment
    /// at the start of the thread).
    pub(crate) replies: HashMap<usize, usize>,
    /// Each review state → the comment it marks, and whether it is
    /// `/Completed`. States of other models (`/Marked`) are kept with
    /// `None`: they belong to a thread, but say nothing about resolving it.
    pub(crate) states: HashMap<usize, (usize, Option<bool>)>,
    /// Members of a group (`/RT /Group`), shown as one with the annotation
    /// they name: no comments of their own.
    pub(crate) members: HashSet<usize>,
}

impl Threads {
    /// Whether the annotation at `index` belongs to another one's thread
    /// (a reply, a state, a member of a group) rather than standing alone.
    pub(crate) fn answers(&self, index: usize) -> bool {
        self.replies.contains_key(&index) || self.states.contains_key(&index) || self.members.contains(&index)
    }
}

/// Where a thread entry points, before chains are followed to their start.
enum Link {
    Reply(usize),
    State(usize, Option<bool>),
    Member(usize),
}

/// The threads of `page`'s annotations. Empty where our parser cannot read
/// the page (an encrypted object stream): every annotation then stands alone.
pub(crate) fn threads(bytes: &[u8], page: usize) -> Threads {
    let mut out = Threads::default();
    let Ok(Start { file, .. }) = Start::read(bytes) else { return out };
    let Ok(tree) = PageTree::read(&file) else { return out };
    let Some(leaf) = tree.leaves.get(page) else { return out };
    let Ok(annots) = Annots::read(&file, leaf) else { return out };
    let by_reference: HashMap<(u32, u16), usize> = annots
        .items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| match item {
            Object::Ref(number, generation) => Some(((*number, *generation), index)),
            _ => None,
        })
        .collect();
    let mut links = HashMap::new();
    for (index, item) in annots.items.iter().enumerate() {
        let Some(dict) = resolve_dict(&file, item) else { continue };
        let Some(&Object::Ref(number, generation)) = dict.get(b"IRT") else { continue };
        let Some(&target) = by_reference.get(&(number, generation)) else { continue };
        if target == index {
            continue;
        }
        let name = |key: &[u8]| dict.get(key).and_then(Object::as_name).map(<[u8]>::to_vec);
        let state = name(b"State");
        let link = if state.is_some() || dict.get(b"StateModel").is_some() {
            let review = name(b"StateModel").as_deref() == Some(b"Review");
            Link::State(target, review.then(|| state.as_deref() == Some(b"Completed")))
        } else if name(b"RT").as_deref() == Some(b"Group") {
            Link::Member(target)
        } else {
            Link::Reply(target)
        };
        links.insert(index, link);
    }
    // The start of each thread: follow replies to replies (bounded: a cycle
    // in a damaged file ends at its first repeat).
    let root = |mut index: usize| {
        for _ in 0..=links.len() {
            match links.get(&index) {
                Some(Link::Reply(target) | Link::State(target, _) | Link::Member(target)) => index = *target,
                None => return Some(index),
            }
        }
        None
    };
    for (&index, link) in &links {
        match link {
            Link::Reply(target) => {
                if let Some(start) = root(*target) {
                    out.replies.insert(index, start);
                }
            }
            Link::State(target, resolved) => {
                if let Some(start) = root(*target) {
                    out.states.insert(index, (start, *resolved));
                }
            }
            Link::Member(_) => {
                out.members.insert(index);
            }
        }
    }
    out
}

/// A reply's or a state's entries after `/Rect`: it answers `target`.
pub(crate) fn thread_entries(
    target: &Object,
    contents: Option<&str>,
    state: Option<bool>,
) -> Vec<(Vec<u8>, Object)> {
    let mut entries = vec![(b"IRT".to_vec(), target.clone())];
    match state {
        None => entries.push((b"RT".to_vec(), Object::Name(b"R".to_vec()))),
        Some(resolved) => {
            entries.push((b"StateModel".to_vec(), Object::Name(b"Review".to_vec())));
            let state: &[u8] = if resolved { b"Completed" } else { b"None" };
            entries.push((b"State".to_vec(), Object::Name(state.to_vec())));
        }
    }
    if let Some(contents) = contents {
        entries.push((b"Contents".to_vec(), Object::Str(super::text_string(contents))));
    }
    entries.extend([
        (b"Name".to_vec(), Object::Name(b"Comment".to_vec())),
        (b"Open".to_vec(), Object::Bool(false)),
    ]);
    entries
}

/// Replies and states draw nothing on the page: an empty appearance over
/// the comment's own `/Rect` (without one, viewers draw a note's icon).
pub(crate) fn empty_appearance(rect: [f32; 4]) -> Appearance {
    Appearance::in_page_space(rect, Dict::default(), Vec::new())
}

/// Annotation flags (ISO 32000-2, 12.5.3): a reply prints and keeps its
/// size and orientation, as other viewers write them; a review state is
/// also hidden.
pub(crate) const REPLY_FLAGS: i64 = 4 | 8 | 16;
pub(crate) const STATE_FLAGS: i64 = 2 | REPLY_FLAGS;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_entries_reply_or_mark() {
        let target = Object::Ref(12, 0);
        let reply = thread_entries(&target, Some("نعم"), None);
        assert_eq!(reply[0], (b"IRT".to_vec(), target.clone()));
        assert!(reply.contains(&(b"RT".to_vec(), Object::Name(b"R".to_vec()))));
        assert!(reply.iter().any(|(key, _)| key == b"Contents"));
        let done = thread_entries(&target, None, Some(true));
        assert!(done.contains(&(b"State".to_vec(), Object::Name(b"Completed".to_vec()))));
        assert!(done.contains(&(b"StateModel".to_vec(), Object::Name(b"Review".to_vec()))));
        assert!(!done.iter().any(|(key, _)| key == b"RT" || key == b"Contents"));
        let open = thread_entries(&target, None, Some(false));
        assert!(open.contains(&(b"State".to_vec(), Object::Name(b"None".to_vec()))));
        // Hidden only for states.
        assert_eq!(STATE_FLAGS & 2, 2);
        assert_eq!(REPLY_FLAGS & 2, 0);
    }
}
