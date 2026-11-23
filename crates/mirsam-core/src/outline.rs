//! The document outline (bookmarks) — spec section 5.1.

use pdfium_render::prelude::{PdfBookmark, PdfDestination, PdfDestinationViewSettings, PdfDocument};
use serde::Serialize;

/// Outlines come from untrusted files: a cyclic or absurdly deep tree must not
/// hang or exhaust the process (spec section 9, "Security").
const MAX_DEPTH: usize = 32;
const MAX_ITEMS: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlineItem {
    /// As written; Arabic titles are already logical-order strings.
    pub title: String,
    /// Zero-based target page, when the item points into this document.
    pub page: Option<usize>,
    /// Target position on the page (PDF points, y up), when the destination gives one.
    pub top: Option<f32>,
    pub children: Vec<OutlineItem>,
}

pub(crate) fn read(document: &PdfDocument<'_>) -> Vec<OutlineItem> {
    let mut budget = MAX_ITEMS;
    siblings(document.bookmarks().root(), 0, &mut budget)
}

fn siblings(first: Option<PdfBookmark<'_>>, depth: usize, budget: &mut usize) -> Vec<OutlineItem> {
    let mut items = Vec::new();
    let mut next = first;
    while let Some(bookmark) = next {
        if *budget == 0 || depth >= MAX_DEPTH {
            break;
        }
        *budget -= 1;
        let (page, top) = target(&bookmark);
        let children = siblings(bookmark.first_child(), depth + 1, budget);
        items.push(OutlineItem { title: bookmark.title().unwrap_or_default(), page, top, children });
        next = bookmark.next_sibling();
    }
    items
}

fn target(bookmark: &PdfBookmark<'_>) -> (Option<usize>, Option<f32>) {
    if let Some(destination) = bookmark.destination() {
        return describe(&destination);
    }
    if let Some(action) = bookmark.action()
        && let Some(local) = action.as_local_destination_action()
        && let Ok(destination) = local.destination()
    {
        return describe(&destination);
    }
    (None, None)
}

fn describe(destination: &PdfDestination<'_>) -> (Option<usize>, Option<f32>) {
    let page = destination.page_index().ok().and_then(|index| usize::try_from(index).ok());
    let top = match destination.view_settings() {
        Ok(PdfDestinationViewSettings::SpecificCoordinatesAndZoom(_, y, _))
        | Ok(PdfDestinationViewSettings::FitPageHorizontallyToWindow(y))
        | Ok(PdfDestinationViewSettings::FitBoundsHorizontallyToWindow(y)) => y.map(|y| y.value),
        _ => None,
    };
    (page, top)
}
