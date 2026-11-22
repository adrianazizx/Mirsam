//! Outline, page labels, page text (for selection), and search.
//!
//! The engine works in UTF-8 byte offsets and PDF page space; the UI works in
//! UTF-16 string indices and displayed-page coordinates. This module converts
//! at the boundary and nothing else: geometry goes through the page's
//! [`ViewTransform`] (rotation, crop-box origin), offsets through
//! [`Utf16Offsets`].

use safha_core::outline::OutlineItem;
use safha_core::search::{HitContext, Query, SearchError, SearchOptions};
use safha_core::text::{Direction, PageText, TextQuality};
use safha_core::view::{ViewRect, ViewTransform};
use serde::Serialize;
use tauri::State;

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// At most this many hits are sent; the UI says the list is cut short.
const MAX_HITS: usize = 5_000;
/// Characters of context on each side of a hit in the results list.
const CONTEXT_CHARS: usize = 32;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextLayer {
    direction: Direction,
    /// "good", "suspect" (the text is probably wrong: offer OCR), or "noText".
    quality: &'static str,
    blocks: Vec<TextBlockWire>,
}

#[derive(Debug, Serialize)]
pub struct TextBlockWire {
    direction: Direction,
    lines: Vec<TextLineWire>,
}

#[derive(Debug, Serialize)]
pub struct TextLineWire {
    /// Logical (reading) order.
    text: String,
    /// Logical order; `start`/`end` are UTF-16 indices into `text`.
    spans: Vec<TextSpanWire>,
    bounds: ViewRect,
}

#[derive(Debug, Serialize)]
pub struct TextSpanWire {
    start: u32,
    end: u32,
    /// Displayed-page points; absent for inferred spaces.
    rect: Option<ViewRect>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    hits: Vec<SearchHitWire>,
    /// More hits exist than were sent.
    truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct SearchHitWire {
    page: usize,
    block: usize,
    line: usize,
    /// UTF-16 indices into the line's text (see [`TextLineWire`]).
    start: u32,
    end: u32,
    direction: Direction,
    rects: Vec<ViewRect>,
    context: HitContext,
}

/// The outline (bookmarks) as a tree; empty when the document has none.
#[tauri::command]
pub async fn document_outline(
    state: State<'_, AppState>,
    document_id: u32,
) -> CommandResult<Vec<OutlineItem>> {
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    blocking(move || Ok(document.outline())).await
}

/// Every page's label, or `None` when the document defines none.
#[tauri::command]
pub async fn page_labels(
    state: State<'_, AppState>,
    document_id: u32,
) -> CommandResult<Option<Vec<Option<String>>>> {
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    blocking(move || Ok(document.page_labels()?)).await
}

/// One page's text in reading order with each character's box, for
/// selection and copy. Served from the search index when it is built.
#[tauri::command]
pub async fn page_text(
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
) -> CommandResult<TextLayer> {
    let (document, index) =
        state.index(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    blocking(move || {
        if let Some(built) = index.ready()
            && let (Some(text), Some(view)) = (built.page(page), built.view(page))
        {
            return Ok(text_layer(text, view));
        }
        let (text, view) = document.page_text_view(page)?;
        Ok(text_layer(&text, &view))
    })
    .await
}

/// Searches the whole document. The first search builds the index (one text
/// extraction per page); later ones only scan it. Rejects with
/// `invalid_query` for an empty query or a bad regular expression, and
/// `cancelled` if the document was closed meanwhile.
#[tauri::command]
pub async fn search_document(
    state: State<'_, AppState>,
    document_id: u32,
    query: String,
    options: SearchOptions,
) -> CommandResult<SearchResults> {
    let query = Query::new(&query, options).map_err(|err| match err {
        SearchError::Empty => CommandError::new("invalid_query", "empty"),
        SearchError::InvalidRegex(detail) => CommandError::new("invalid_query", detail),
    })?;
    let (document, index) =
        state.index(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    blocking(move || {
        let index = index
            .get_or_build(&document)?
            .ok_or_else(|| CommandError::new("cancelled", "document closed"))?;
        let (found, truncated) = index.search_limited(&query, MAX_HITS);
        let hits = found
            .into_iter()
            .filter_map(|(page, hit)| {
                let text = index.page(page)?;
                let view = index.view(page)?;
                let block = text.blocks.get(hit.block)?;
                let line = block.lines.get(hit.line)?;
                let offsets = Utf16Offsets::new(&line.text);
                Some(SearchHitWire {
                    page,
                    block: hit.block,
                    line: hit.line,
                    start: offsets.at(hit.start),
                    end: offsets.at(hit.end),
                    direction: block.direction,
                    rects: hit.rects.iter().map(|rect| view.rect(*rect)).collect(),
                    context: HitContext::of(&line.text, hit.start, hit.end, CONTEXT_CHARS),
                })
            })
            .collect();
        Ok(SearchResults { hits, truncated })
    })
    .await
}

fn text_layer(text: &PageText, view: &ViewTransform) -> TextLayer {
    let quality = match text.quality {
        TextQuality::Good => "good",
        TextQuality::Suspect { .. } => "suspect",
        TextQuality::NoText => "noText",
    };
    let blocks = text
        .blocks
        .iter()
        .map(|block| TextBlockWire {
            direction: block.direction,
            lines: block
                .lines
                .iter()
                .map(|line| {
                    let offsets = Utf16Offsets::new(&line.text);
                    TextLineWire {
                        text: line.text.clone(),
                        spans: line
                            .spans
                            .iter()
                            .map(|span| TextSpanWire {
                                start: offsets.at(span.start),
                                end: offsets.at(span.end),
                                rect: span.rect.map(|rect| view.rect(rect)),
                            })
                            .collect(),
                        bounds: view.rect(line.bounds),
                    }
                })
                .collect(),
        })
        .collect();
    TextLayer { direction: text.direction, quality, blocks }
}

/// UTF-8 byte offsets of a string to UTF-16 code-unit offsets (JavaScript
/// string indices).
struct Utf16Offsets {
    /// (byte offset, UTF-16 offset) at every character boundary, plus the end.
    marks: Vec<(usize, u32)>,
}

impl Utf16Offsets {
    fn new(text: &str) -> Self {
        let mut marks = Vec::with_capacity(text.len() + 1);
        let mut units = 0u32;
        for (byte, c) in text.char_indices() {
            marks.push((byte, units));
            units += c.len_utf16() as u32;
        }
        marks.push((text.len(), units));
        Self { marks }
    }

    /// The UTF-16 offset of a byte offset on a character boundary (the
    /// engine's offsets always are); a byte inside a character maps to that
    /// character's start.
    fn at(&self, byte: usize) -> u32 {
        match self.marks.binary_search_by_key(&byte, |&(b, _)| b) {
            Ok(i) => self.marks[i].1,
            Err(i) => self.marks[i.saturating_sub(1)].1,
        }
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> CommandResult<T> + Send + 'static,
) -> CommandResult<T> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|err| CommandError::new("engine_error", err.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_offsets_become_javascript_string_indices() {
        // "ع" is 2 UTF-8 bytes and 1 UTF-16 unit; "𝐀" is 4 bytes and 2 units.
        let text = "aع𝐀b";
        let offsets = Utf16Offsets::new(text);
        assert_eq!(offsets.at(0), 0);
        assert_eq!(offsets.at(1), 1);
        assert_eq!(offsets.at(3), 2);
        assert_eq!(offsets.at(7), 4);
        assert_eq!(offsets.at(text.len()), 5);
        // Inside a character: that character's start.
        assert_eq!(offsets.at(2), 1);
    }
}
