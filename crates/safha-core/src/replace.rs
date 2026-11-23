//! Find and replace (M6, ADR 0052): the matches of a search, replaced
//! through the in-place editing engine (ADR 0023).
//!
//! - **What is replaced** is the editor's own reading of each paragraph
//!   ([`crate::block_edit`]): its text in logical order, every character
//!   placed on the page. The search's options apply as they do in the search
//!   panel (case, whole words, regular expressions with groups, Arabic
//!   normalisation); a regular expression's replacement may name its groups
//!   ([`Query::expand`]).
//! - **What search shows** is PDFium's reading of the page (the index). A hit
//!   there that no paragraph's match covers is listed as well, skipped, with
//!   why: it runs from one paragraph into another, its text cannot be edited
//!   (and why), or the editor reads that text differently.
//! - **Each paragraph is set again once**, with all its replacements: in its
//!   own fonts when they can set the new text (their codes, then their
//!   embedded programs, ADR 0026), otherwise in a bundled substitute for the
//!   whole paragraph, which the report names.
//! - **Nothing inexact is kept:** after each paragraph is set, the page is
//!   read again; unless the paragraph reads back as the new text, that edit
//!   is dropped and its matches are reported as skipped.
//! - **All the edits are one incremental update:** one undo step.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::block_edit::{
    BlockEdit, PageParagraphs, Paragraph, Refusal, Substitution, edit_block_update, encrypted,
    page_paragraphs,
};
use crate::document::Document;
use crate::error::{Error, Result};
use crate::search::{DocumentIndex, Found, HitContext, Query};
use crate::text::{Direction, PageText};
use crate::view::{ViewRect, ViewTransform};

/// Characters of context on each side of a match in the preview.
pub const CONTEXT_CHARS: usize = 32;

/// Why a match is not replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SkipReason {
    /// It runs from one paragraph into another (or into text that is not one).
    CrossesParagraphs,
    /// Its paragraph is drawn together with other text.
    Shared,
    /// Its text needs OCR to be read exactly.
    Unreadable,
    /// Its font cannot be read (Type 3, no way to read its codes).
    Font,
    /// It is not upright on the page as displayed.
    Rotated,
    /// The file is encrypted in a way this editor cannot write.
    Encrypted,
    /// It is drawn in a way the editor does not follow yet.
    Unsupported,
    /// The editor reads this text differently from search, or the paragraph
    /// did not read back as the replaced text: it would not be replaced exactly.
    Inexact,
    /// The page's text changed since the matches were listed.
    Changed,
    /// The paragraph would be longer than the editor sets at once.
    TooLong,
}

impl From<Refusal> for SkipReason {
    fn from(refusal: Refusal) -> Self {
        match refusal {
            Refusal::Shared => Self::Shared,
            Refusal::Unreadable => Self::Unreadable,
            Refusal::Font => Self::Font,
            Refusal::Rotated => Self::Rotated,
            Refusal::Encrypted => Self::Encrypted,
            Refusal::Unsupported => Self::Unsupported,
        }
    }
}

/// A match that can be replaced: its paragraph as the editor read it, and
/// the match's byte range in that paragraph's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub page: usize,
    pub block: usize,
    pub fingerprint: u32,
    pub start: usize,
    pub end: usize,
}

/// One match, as the preview lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Occurrence {
    pub page: usize,
    /// Where it is replaced; `None` when it is skipped.
    pub target: Option<Target>,
    /// Its text's boxes on the displayed page.
    pub rects: Vec<ViewRect>,
    /// Its paragraph (or line) reads right to left.
    pub rtl: bool,
    pub context: HitContext,
    /// What it would become.
    pub replacement: String,
    pub skip: Option<SkipReason>,
}

/// Every match of a query, in page order, replaceable or not.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub occurrences: Vec<Occurrence>,
    /// More matches exist than were listed.
    pub truncated: bool,
}

impl Plan {
    /// The matches that will be replaced.
    pub fn targets(&self) -> Vec<Target> {
        self.occurrences.iter().filter_map(|o| o.target).collect()
    }
}

/// A match that was to be replaced and was not, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    pub target: Target,
    pub reason: SkipReason,
}

/// What replacing did.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// Matches replaced.
    pub replaced: usize,
    /// Paragraphs set again.
    pub paragraphs: usize,
    pub skipped: Vec<Skipped>,
    /// Paragraphs set in a bundled substitute: the original font and the
    /// face used, once each.
    pub substituted: Vec<Substitution>,
    /// How many paragraphs were substituted.
    pub substituted_paragraphs: usize,
}

/// Every match of `query` in `document`, replaceable or not, with what
/// `template` makes of each: on page `only`, or on every page. `index` is
/// the document's search index (PDFium's reading of its pages).
pub fn plan(
    document: &Document,
    index: &DocumentIndex,
    query: &Query,
    template: &str,
    only: Option<usize>,
    limit: usize,
) -> Result<Plan> {
    let pages: Vec<usize> = match only {
        Some(page) => {
            if page >= index.page_count() {
                return Err(Error::PageOutOfRange { index: page, count: index.page_count() });
            }
            vec![page]
        }
        None => (0..index.page_count())
            .filter(|&page| index.page(page).is_some_and(|text| may_match(text, query)))
            .collect(),
    };
    let mut plan = Plan::default();
    for page in pages {
        let (Some(text), Some(view)) = (index.page(page), index.view(page)) else { continue };
        let found = page_plan(document, text, view, page, query, template)?;
        for occurrence in found {
            if plan.occurrences.len() == limit {
                plan.truncated = true;
                return Ok(plan);
            }
            plan.occurrences.push(occurrence);
        }
    }
    Ok(plan)
}

/// The occurrence that is a search hit's (its boxes on the displayed page):
/// what "replace" does to the hit shown.
pub fn occurrence_at<'a>(occurrences: &'a [Occurrence], rects: &[ViewRect]) -> Option<&'a Occurrence> {
    occurrences.iter().find(|o| covers(&o.rects, rects))
}

/// Whether the query can match on a page: a hit in one of its lines, or in
/// its lines joined (a paragraph's match can run across a line break).
fn may_match(text: &PageText, query: &Query) -> bool {
    let lines = || text.blocks.iter().flat_map(|block| &block.lines);
    lines().any(|line| !query.find_in(&line.text).is_empty())
        || !query.find_in(&lines().map(|line| line.text.as_str()).collect::<Vec<_>>().join(" ")).is_empty()
}

/// One page's matches: the paragraphs' first, then the hits PDFium finds
/// that none of them covers, each with why it is skipped; in reading order.
fn page_plan(
    document: &Document,
    text: &PageText,
    view: &ViewTransform,
    page: usize,
    query: &Query,
    template: &str,
) -> Result<Vec<Occurrence>> {
    let bytes = document.bytes();
    let password = document.password_in_use();
    let read = match page_paragraphs(&bytes, password.as_deref(), view, page) {
        Ok(read) => Some(read),
        // The page's content cannot be read for editing at all.
        Err(Error::EditUnsupported(_)) => None,
        Err(other) => return Err(other),
    };
    let unreadable =
        if read.is_none() && encrypted(&bytes)? { SkipReason::Encrypted } else { SkipReason::Unsupported };

    let mut out: Vec<Occurrence> = Vec::new();
    if let Some(read) = &read {
        for paragraph in &read.paragraphs {
            let found = query.matches_in(&paragraph.text);
            if found.is_empty() {
                continue;
            }
            let replaced = replaced_text(query, template, &paragraph.text, &found);
            let skip = paragraph.refusal.map(SkipReason::from).or_else(|| {
                (replaced.encode_utf16().count() > crate::text_edit::MAX_TEXT).then_some(SkipReason::TooLong)
            });
            for m in &found {
                out.push(Occurrence {
                    page,
                    target: skip.is_none().then_some(Target {
                        page,
                        block: paragraph.index,
                        fingerprint: paragraph.fingerprint,
                        start: m.start,
                        end: m.end,
                    }),
                    rects: rects_of(paragraph, m.start, m.end),
                    rtl: paragraph.rtl,
                    context: HitContext::of(&paragraph.text, m.start, m.end, CONTEXT_CHARS),
                    replacement: query.expand(template, m, &paragraph.text),
                    skip,
                });
            }
        }
    }

    // PDFium's hits that no paragraph's match covers.
    let covered = out.len();
    for block in &text.blocks {
        for line in &block.lines {
            for m in query.matches_in(&line.text) {
                let rects: Vec<ViewRect> = line
                    .spans
                    .iter()
                    .filter(|span| span.start < m.end && m.start < span.end)
                    .filter_map(|span| span.rect)
                    .map(|rect| view.rect(rect))
                    .filter(|rect| rect.width > 0.0 && rect.height > 0.0)
                    .collect();
                if rects.is_empty() || out[..covered].iter().any(|o| covers(&o.rects, &rects)) {
                    continue;
                }
                let reason = match &read {
                    Some(read) => why_not(read, &rects),
                    None => unreadable,
                };
                out.push(Occurrence {
                    page,
                    target: None,
                    rects,
                    rtl: block.direction == Direction::Rtl,
                    context: HitContext::of(&line.text, m.start, m.end, CONTEXT_CHARS),
                    replacement: query.expand(template, &m, &line.text),
                    skip: Some(reason),
                });
            }
        }
    }
    out.sort_by(|a, b| reading_key(a).total_cmp(&reading_key(b)).then(across(a).total_cmp(&across(b))));
    Ok(out)
}

/// Top to bottom, a line at a time.
fn reading_key(occurrence: &Occurrence) -> f32 {
    occurrence.rects.first().map_or(0.0, |r| ((r.y + r.height / 2.0) / 4.0).round())
}

/// Along a line, in the line's own direction.
fn across(occurrence: &Occurrence) -> f32 {
    let Some(r) = occurrence.rects.first() else { return 0.0 };
    if occurrence.rtl { -(r.x + r.width) } else { r.x }
}

/// `text` with every match in `found` replaced from `template`.
fn replaced_text(query: &Query, template: &str, text: &str, found: &[Found]) -> String {
    let mut out = text.to_string();
    for m in found.iter().rev() {
        out.replace_range(m.start..m.end, &query.expand(template, m, text));
    }
    out
}

/// The boxes of a paragraph's text from byte `start` to `end`, on the
/// displayed page: per line, one box per stretch of glyphs that touch.
fn rects_of(paragraph: &Paragraph, start: usize, end: usize) -> Vec<ViewRect> {
    let chars: Vec<_> = paragraph
        .text
        .char_indices()
        .zip(&paragraph.placed)
        .filter(|((at, _), _)| (start..end).contains(at))
        .map(|((_, c), placed)| (c, placed))
        .collect();
    let inked: Vec<_> = chars.iter().filter(|(c, _)| !c.is_whitespace()).collect();
    let chosen: Vec<_> = if inked.is_empty() { chars.iter().collect() } else { inked };
    let mut by_line: BTreeMap<usize, Vec<[f32; 2]>> = BTreeMap::new();
    for (_, placed) in chosen {
        by_line.entry(placed.line).or_default().push(placed.x);
    }
    let mut rects = Vec::new();
    for (line, mut spans) in by_line {
        let Some(&(baseline, size)) = paragraph.lines.get(line) else { continue };
        spans.sort_by(|a, b| a[0].total_cmp(&b[0]));
        let mut merged: Vec<[f32; 2]> = Vec::new();
        for span in spans {
            match merged.last_mut() {
                Some(last) if span[0] <= last[1] + 0.3 * size => last[1] = last[1].max(span[1]),
                _ => merged.push(span),
            }
        }
        for [x0, x1] in merged {
            rects.push(ViewRect {
                x: x0,
                y: baseline - 0.95 * size,
                width: (x1 - x0).max(0.5),
                height: 1.25 * size,
            });
        }
    }
    rects
}

fn centre(rect: &ViewRect) -> (f32, f32) {
    (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)
}

fn contains(rect: &ViewRect, (x, y): (f32, f32), slack: f32) -> bool {
    x >= rect.x - slack
        && x <= rect.x + rect.width + slack
        && y >= rect.y - slack
        && y <= rect.y + rect.height + slack
}

/// Whether a paragraph's match (`ours`) is the same text as PDFium's hit
/// (`theirs`): most of the hit's glyphs lie in the match's boxes.
fn covers(ours: &[ViewRect], theirs: &[ViewRect]) -> bool {
    let inside = theirs.iter().filter(|r| ours.iter().any(|o| contains(o, centre(r), 0.5))).count();
    inside * 2 > theirs.len()
}

/// Why a hit PDFium finds is not one of the paragraphs' matches.
fn why_not(read: &PageParagraphs, rects: &[ViewRect]) -> SkipReason {
    let in_box = |[x0, x1, top, bottom]: [f32; 4], (x, y): (f32, f32)| {
        x >= x0 - 0.5 && x <= x1 + 0.5 && y >= top - 0.5 && y <= bottom + 0.5
    };
    let mut owners: BTreeSet<usize> = BTreeSet::new();
    let mut areas: Vec<Refusal> = Vec::new();
    let mut nowhere = 0;
    for rect in rects {
        let point = centre(rect);
        if let Some(p) = read.paragraphs.iter().position(|p| in_box(p.bounds, point)) {
            owners.insert(p);
        } else if let Some((_, reason)) = read.areas.iter().find(|(area, _)| in_box(*area, point)) {
            areas.push(*reason);
        } else {
            nowhere += 1;
        }
    }
    match (owners.len(), areas.first()) {
        (2.., _) | (1, Some(_)) => SkipReason::CrossesParagraphs,
        (1, None) => {
            let paragraph = owners.first().and_then(|&p| read.paragraphs.get(p));
            paragraph.and_then(|p| p.refusal).map_or(SkipReason::Inexact, SkipReason::from)
        }
        (0, Some(&reason)) if nowhere == 0 => reason.into(),
        _ => SkipReason::Unsupported,
    }
}

/// A document's new revision after replacing, and the update that makes it.
pub type Replaced = (Document, Vec<u8>);

/// `document` with `targets` replaced from `template` (matches of `query`),
/// as one update, and what was done. `None` when nothing changed.
///
/// Targets come from a [`plan`] of the same revision; one whose paragraph
/// or match is no longer there is skipped as changed.
pub fn apply(
    document: &Document,
    query: &Query,
    template: &str,
    targets: &[Target],
) -> Result<(Option<Replaced>, Report)> {
    let password = document.password_in_use();
    let password = password.as_deref();
    let mut report = Report::default();
    let mut bytes: Vec<u8> = document.bytes().to_vec();
    let base = bytes.len();

    // Page → paragraph (as planned) → its matches.
    let mut pages: BTreeMap<usize, BTreeMap<(usize, u32), Vec<Target>>> = BTreeMap::new();
    for target in targets.iter().copied().collect::<BTreeSet<_>>() {
        pages
            .entry(target.page)
            .or_default()
            .entry((target.block, target.fingerprint))
            .or_default()
            .push(target);
    }
    let skip_all = |report: &mut Report, targets: &[Target], reason: SkipReason| {
        report.skipped.extend(targets.iter().map(|&target| Skipped { target, reason }));
    };

    for (page, paragraphs) in pages {
        if page >= document.page_count() {
            for targets in paragraphs.values() {
                skip_all(&mut report, targets, SkipReason::Changed);
            }
            continue;
        }
        let view = document.view_transform(page)?;
        let width = document.page_size(page)?.width;
        let mut current = match page_paragraphs(&bytes, password, &view, page) {
            Ok(read) => read,
            Err(Error::EditUnsupported(_)) => {
                for targets in paragraphs.values() {
                    skip_all(&mut report, targets, SkipReason::Unsupported);
                }
                continue;
            }
            Err(other) => return Err(other),
        };
        // Each paragraph as planned: its text and place find it again after
        // the page's other paragraphs were set (their numbers can move).
        let mut jobs: Vec<(String, [f32; 4], Vec<Target>)> = Vec::new();
        for ((block, fingerprint), targets) in paragraphs {
            match current.paragraphs.get(block).filter(|p| p.fingerprint == fingerprint) {
                Some(p) => jobs.push((p.text.clone(), p.bounds, targets)),
                None => skip_all(&mut report, &targets, SkipReason::Changed),
            }
        }
        for (text, bounds, targets) in jobs {
            let Some(paragraph) =
                current.paragraphs.iter().find(|p| p.text == text && near(p.bounds, bounds))
            else {
                skip_all(&mut report, &targets, SkipReason::Changed);
                continue;
            };
            if let Some(refusal) = paragraph.refusal {
                skip_all(&mut report, &targets, refusal.into());
                continue;
            }
            // The matches are still there, and do not overlap.
            let found = query.matches_in(&paragraph.text);
            let mut chosen: Vec<Found> = Vec::new();
            for target in &targets {
                let still = found.iter().find(|m| m.start == target.start && m.end == target.end);
                match still {
                    Some(m) if chosen.last().is_none_or(|last| last.end <= m.start) => chosen.push(m.clone()),
                    _ => report.skipped.push(Skipped { target: *target, reason: SkipReason::Changed }),
                }
            }
            if chosen.is_empty() {
                continue;
            }
            let replaced = replaced_text(query, template, &paragraph.text, &chosen);
            let done: Vec<Target> = targets
                .iter()
                .copied()
                .filter(|t| chosen.iter().any(|m| m.start == t.start && m.end == t.end))
                .collect();
            if replaced == paragraph.text {
                report.replaced += done.len();
                continue;
            }
            if replaced.encode_utf16().count() > crate::text_edit::MAX_TEXT {
                skip_all(&mut report, &done, SkipReason::TooLong);
                continue;
            }
            let edit = BlockEdit {
                page,
                index: paragraph.index,
                fingerprint: paragraph.fingerprint,
                text: replaced.clone(),
                family: None,
                marks: None,
            };
            let (update, edited) = match edit_block_update(&bytes, password, &view, width, &edit) {
                Ok(result) => result,
                Err(Error::InvalidEdit(_) | Error::EditUnsupported(_) | Error::Typeset(_)) => {
                    skip_all(&mut report, &done, SkipReason::Unsupported);
                    continue;
                }
                Err(other) => return Err(other),
            };
            // Read back: the paragraph must say exactly the new text.
            let before = bytes.len();
            bytes.extend_from_slice(&update);
            let after = match page_paragraphs(&bytes, password, &view, page) {
                Ok(after) if reads_back(&after, &replaced, paragraph) => after,
                Ok(_) | Err(Error::EditUnsupported(_)) => {
                    bytes.truncate(before);
                    skip_all(&mut report, &done, SkipReason::Inexact);
                    continue;
                }
                Err(other) => return Err(other),
            };
            report.replaced += done.len();
            report.paragraphs += 1;
            if let Some(substitution) = edited.substituted {
                report.substituted_paragraphs += 1;
                if !report
                    .substituted
                    .iter()
                    .any(|s| s.from == substitution.from && s.font == substitution.font)
                {
                    report.substituted.push(substitution);
                }
            }
            current = after;
        }
    }
    if bytes.len() == base {
        return Ok((None, report));
    }
    let update = bytes[base..].to_vec();
    let revision = document.revision(Arc::from(bytes))?;
    Ok((Some((revision, update)), report))
}

/// Two reads of one paragraph, before and after another of the page's was set.
fn near(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1.0)
}

/// Text compared as read: compatibility forms folded, white space as single spaces.
fn normalized(text: &str) -> String {
    text.nfkc().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether the page, read again, has `text` where `old` was: a paragraph
/// overlapping its place holds it, or (emptied) the old text is gone.
fn reads_back(page: &PageParagraphs, text: &str, old: &Paragraph) -> bool {
    let wanted = normalized(text);
    let [_, _, top, bottom] = old.bounds;
    let overlapping = |p: &&Paragraph| p.bounds[2] <= bottom && top <= p.bounds[3];
    if wanted.is_empty() {
        let gone = normalized(&old.text);
        return !page.paragraphs.iter().filter(overlapping).any(|p| normalized(&p.text) == gone);
    }
    page.paragraphs
        .iter()
        .filter(overlapping)
        .any(|p| p.refusal.is_none() && normalized(&p.text).contains(&wanted))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_edit::Placed;
    use crate::blocks::RunStyle;

    fn paragraph(text: &str, places: &[(usize, [f32; 2])]) -> Paragraph {
        let style = RunStyle { font: 0, size: 10.0, color: [0.0; 3] };
        Paragraph {
            index: 0,
            fingerprint: 1,
            text: text.into(),
            placed: text.chars().zip(places).map(|(c, &(line, x))| Placed { c, style, line, x }).collect(),
            lines: vec![(100.0, 10.0), (112.0, 10.0)],
            rtl: false,
            bounds: [0.0, 100.0, 90.0, 115.0],
            refusal: None,
        }
    }

    fn rect(x: f32, y: f32, width: f32) -> ViewRect {
        ViewRect { x, y, width, height: 10.0 }
    }

    #[test]
    fn a_match_over_a_line_break_has_a_box_on_each_line() {
        // "ab cd": "ab" ends line 0, "cd" starts line 1; the joining space has no width.
        let places =
            [(0, [80.0, 85.0]), (0, [85.0, 90.0]), (0, [90.0, 90.0]), (1, [0.0, 5.0]), (1, [5.0, 10.0])];
        let p = paragraph("ab cd", &places);
        let rects = rects_of(&p, 1, 4);
        assert_eq!(rects.len(), 2, "{rects:?}");
        assert_eq!((rects[0].x, rects[0].width), (85.0, 5.0));
        assert_eq!((rects[1].x, rects[1].width), (0.0, 5.0));
        assert!(rects[1].y > rects[0].y);
    }

    #[test]
    fn a_hit_is_skipped_for_why_its_text_is_not_a_paragraphs() {
        let mut first = paragraph("one", &[]);
        first.bounds = [0.0, 50.0, 0.0, 12.0];
        let mut second = paragraph("two", &[]);
        second.index = 1;
        second.bounds = [0.0, 50.0, 20.0, 32.0];
        let mut shared = paragraph("three", &[]);
        shared.index = 2;
        shared.bounds = [0.0, 50.0, 40.0, 52.0];
        shared.refusal = Some(Refusal::Shared);
        let read = PageParagraphs {
            paragraphs: vec![first, second, shared],
            areas: vec![([60.0, 100.0, 0.0, 12.0], Refusal::Font)],
        };
        // Glyphs in two paragraphs, or in one and in text that is not one.
        assert_eq!(
            why_not(&read, &[rect(0.0, 2.0, 5.0), rect(0.0, 21.0, 5.0)]),
            SkipReason::CrossesParagraphs
        );
        assert_eq!(
            why_not(&read, &[rect(40.0, 1.0, 5.0), rect(70.0, 1.0, 5.0)]),
            SkipReason::CrossesParagraphs
        );
        // In one paragraph the editor reads otherwise; in a refused one; in refused text.
        assert_eq!(why_not(&read, &[rect(10.0, 1.0, 5.0)]), SkipReason::Inexact);
        assert_eq!(why_not(&read, &[rect(10.0, 41.0, 5.0)]), SkipReason::Shared);
        assert_eq!(why_not(&read, &[rect(70.0, 1.0, 5.0)]), SkipReason::Font);
        assert_eq!(why_not(&read, &[rect(300.0, 300.0, 5.0)]), SkipReason::Unsupported);
    }

    #[test]
    fn a_match_covers_a_hit_whose_glyphs_lie_in_its_boxes() {
        let ours = [rect(10.0, 0.0, 30.0)];
        assert!(covers(&ours, &[rect(10.0, 0.0, 9.0), rect(20.0, 0.0, 9.0), rect(30.0, 0.0, 9.0)]));
        assert!(!covers(&ours, &[rect(50.0, 0.0, 9.0)]));
    }

    #[test]
    fn reading_back_finds_the_new_text_where_the_old_was() {
        let old = paragraph("the old text", &[]);
        let mut new = paragraph("the  new\ntext", &[]);
        new.text = "the new text and more".into();
        let page = PageParagraphs { paragraphs: vec![new.clone()], areas: Vec::new() };
        assert!(reads_back(&page, "the  new\ntext", &old));
        assert!(!reads_back(&page, "the newer text", &old));
        // Elsewhere on the page does not count.
        let mut moved = new;
        moved.bounds = [0.0, 100.0, 500.0, 520.0];
        assert!(!reads_back(
            &PageParagraphs { paragraphs: vec![moved], areas: Vec::new() },
            "the new text",
            &old
        ));
        // Emptied: the old text must be gone.
        assert!(reads_back(&PageParagraphs { paragraphs: Vec::new(), areas: Vec::new() }, "", &old));
        assert!(!reads_back(&PageParagraphs { paragraphs: vec![old.clone()], areas: Vec::new() }, " ", &old));
    }
}
