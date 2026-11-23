use std::sync::Arc;

use pdfium_render::prelude::{
    PdfBitmap, PdfBitmapFormat, PdfColor, PdfDocument, PdfPage, PdfPageAnnotationCommon,
    PdfPageAnnotationType, PdfPageIndex, PdfRenderConfig,
};
use serde::{Deserialize, Serialize};

use crate::bitmap::Bitmap;
use crate::error::{Error, Result};
use crate::view::ViewTransform;

/// Upper bound on either side of a single rendered bitmap. Larger views are
/// assembled from tiles; this guards against a hostile or buggy request
/// allocating gigabytes.
pub const MAX_BITMAP_SIDE: u32 = 8192;

/// Page dimensions in PDF points (1/72 inch), after the page's own `/Rotate`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
}

/// A rectangle of a page rendered at a given zoom.
///
/// `scale` is device pixels per PDF point, so the whole page is
/// `ceil(width * scale) x ceil(height * scale)` pixels and the tile is the
/// `width x height` pixel window whose top-left corner sits at (`x`, `y`) in
/// that space.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TileRequest {
    pub scale: f32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// An open PDF document.
pub struct Document {
    inner: PdfDocument<'static>,
    hints: crate::catalog::CatalogHints,
    /// The file's bytes as PDFium reads them; an edit appends to a copy.
    bytes: Arc<[u8]>,
    /// Kept in memory while the document is open, so an edited revision of an
    /// encrypted file can be opened too (ADR 0014). Never written anywhere.
    password: Option<String>,
    engine: &'static crate::Engine,
}

/// The kind of an annotation the app knows, by PDFium's reading of it.
fn annotation_kind(
    annotation: &pdfium_render::prelude::PdfPageAnnotation<'_>,
) -> Option<crate::annotate::AnnotationKind> {
    use crate::annotate::AnnotationKind;
    Some(match annotation.annotation_type() {
        PdfPageAnnotationType::Highlight => AnnotationKind::Highlight,
        PdfPageAnnotationType::Underline => AnnotationKind::Underline,
        PdfPageAnnotationType::Strikeout => AnnotationKind::StrikeOut,
        PdfPageAnnotationType::Squiggly => AnnotationKind::Squiggly,
        PdfPageAnnotationType::Ink => AnnotationKind::Ink,
        PdfPageAnnotationType::Text => AnnotationKind::Note,
        PdfPageAnnotationType::FreeText => AnnotationKind::TextBox,
        _ => return None,
    })
}

fn is_markup(kind: crate::annotate::AnnotationKind) -> bool {
    use crate::annotate::AnnotationKind;
    matches!(
        kind,
        AnnotationKind::Highlight
            | AnnotationKind::Underline
            | AnnotationKind::StrikeOut
            | AnnotationKind::Squiggly
    )
}

/// Page-space rectangles an annotation covers: one per quad for markup,
/// else its bounds.
fn annotation_rects(annotation: &pdfium_render::prelude::PdfPageAnnotation<'_>) -> Vec<crate::text::Rect> {
    let rect = |left: f32, bottom: f32, right: f32, top: f32| crate::text::Rect { left, bottom, right, top };
    let points = annotation.attachment_points();
    let mut rects: Vec<_> = (0..points.len())
        .filter_map(|q| points.get(q).ok())
        .map(|quad| rect(quad.left().value, quad.bottom().value, quad.right().value, quad.top().value))
        .filter(|r| r.width() > 0.0 && r.height() > 0.0)
        .collect();
    if rects.is_empty()
        && let Ok(bounds) = annotation.bounds()
    {
        rects.push(rect(
            bounds.left().value,
            bounds.bottom().value,
            bounds.right().value,
            bounds.top().value,
        ));
    }
    rects
}

/// `text` cut to `limit` characters, with an ellipsis when cut.
fn excerpt(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((at, _)) => format!("{}…", text[..at].trim_end()),
        None => text.to_string(),
    }
}

/// How a revision follows the one before it.
#[derive(Debug, Clone)]
pub enum RevisionStep {
    /// An incremental update to append.
    Update(Vec<u8>),
    /// The whole new file.
    Snapshot(Arc<[u8]>),
}

/// A redaction's result.
#[derive(Debug)]
pub struct Redacted {
    pub document: Document,
    pub step: RevisionStep,
    pub report: crate::redact::Report,
}

impl Document {
    pub(crate) fn new(
        inner: PdfDocument<'static>,
        hints: crate::catalog::CatalogHints,
        bytes: Arc<[u8]>,
        password: Option<String>,
        engine: &'static crate::Engine,
    ) -> Self {
        Self { inner, hints, bytes, password, engine }
    }

    /// The document's bytes: the file as opened plus any edits.
    pub fn bytes(&self) -> Arc<[u8]> {
        Arc::clone(&self.bytes)
    }

    /// Another revision of this document (after an edit, an undo or a redo),
    /// opened with the same password.
    pub fn revision(&self, bytes: Arc<[u8]>) -> Result<Document> {
        self.engine.open_shared(bytes, self.password.as_deref())
    }

    /// This document with `edit` applied, and the update that applies it.
    pub fn with_page_edit(&self, edit: &crate::edit::PageEdit) -> Result<(Document, Vec<u8>)> {
        let update = crate::edit::page_edit_update(&self.bytes, edit)?;
        let mut bytes = Vec::with_capacity(self.bytes.len() + update.len());
        bytes.extend_from_slice(&self.bytes);
        bytes.extend_from_slice(&update);
        Ok((self.revision(Arc::from(bytes))?, update))
    }

    /// This document with an annotation added or removed, and the update that
    /// does it (ADR 0017).
    pub fn with_annotation_edit(
        &self,
        edit: &crate::annotate::AnnotationEdit,
    ) -> Result<(Document, Vec<u8>)> {
        self.with_annotation_edits(std::slice::from_ref(edit))
    }

    /// This document with `edits` applied in order (marking a selection that
    /// spans pages adds one annotation per page), and the one update that
    /// does them all, so a single undo takes them back.
    pub fn with_annotation_edits(
        &self,
        edits: &[crate::annotate::AnnotationEdit],
    ) -> Result<(Document, Vec<u8>)> {
        self.with_annotation_edits_by(edits, None)
    }

    /// [`Document::with_annotation_edits`], with new annotations signed by
    /// `author` (`/T`, ADR 0041).
    pub fn with_annotation_edits_by(
        &self,
        edits: &[crate::annotate::AnnotationEdit],
        author: Option<&str>,
    ) -> Result<(Document, Vec<u8>)> {
        if edits.is_empty() {
            return Err(Error::InvalidEdit("no annotation edits given"));
        }
        let mut bytes = self.bytes.to_vec();
        let start = bytes.len();
        for edit in edits {
            // Each update builds on the last: appended together they are one.
            let update = crate::annotate::annotation_update(&bytes, self.password.as_deref(), author, edit)?;
            bytes.extend_from_slice(&update);
        }
        let update = bytes[start..].to_vec();
        Ok((self.revision(Arc::from(bytes))?, update))
    }

    /// The document's form fields (ADR 0045), each widget with its box on
    /// the displayed page.
    pub fn form(&self) -> Result<crate::forms::FormInfo> {
        let mut info = crate::forms::read(&self.bytes, self.password.as_deref())?;
        let mut views = std::collections::HashMap::new();
        for widget in info.fields.iter_mut().flat_map(|field| field.widgets.iter_mut()) {
            let Some(page) = widget.page else { continue };
            let view = views.entry(page).or_insert_with(|| self.view_transform(page).ok());
            let [left, bottom, right, top] = widget.rect;
            widget.view = view.map(|view| view.rect(crate::text::Rect { left, bottom, right, top }));
        }
        Ok(info)
    }

    /// This document with form fields filled in (ADR 0045), in order, and
    /// the one update that does it.
    pub fn with_form_edits(&self, edits: &[crate::forms::FormEdit]) -> Result<(Document, Vec<u8>)> {
        if edits.is_empty() {
            return Err(Error::InvalidEdit("no form edits given"));
        }
        let mut bytes = self.bytes.to_vec();
        let start = bytes.len();
        for edit in edits {
            let update = crate::forms::form_update(&bytes, self.password.as_deref(), edit)?;
            bytes.extend_from_slice(&update);
        }
        let update = bytes[start..].to_vec();
        Ok((self.revision(Arc::from(bytes))?, update))
    }

    /// This document with `marks` redacted (ADR 0042). The content under
    /// each mark is removed from the file, not covered, and the result is
    /// checked: the text PDFium reads from the new revision must hold nothing
    /// inside a mark, or the redaction is refused and nothing changes.
    ///
    /// A signed file is rewritten in full at once, since an update would
    /// leave the redacted content in the bytes the signature covers; the
    /// signature no longer validates, which no redaction of a signed file can
    /// avoid.
    pub fn with_redactions(&self, marks: &[crate::redact::Mark]) -> Result<Redacted> {
        let (update, report) = crate::redact::redaction_update(&self.bytes, marks)?;
        let mut bytes = Vec::with_capacity(self.bytes.len() + update.len());
        bytes.extend_from_slice(&self.bytes);
        bytes.extend_from_slice(&update);
        let mut revision = self.revision(Arc::from(bytes))?;
        let mut step = RevisionStep::Update(update);
        if self.is_signed() {
            let rewritten: Arc<[u8]> = Arc::from(revision.to_pdf_bytes()?);
            revision = self.revision(Arc::clone(&rewritten))?;
            step = RevisionStep::Snapshot(rewritten);
        }
        revision.check_nothing_under(marks)?;
        Ok(Redacted { document: revision, step, report })
    }

    /// Whether any text PDFium reads lies under a mark.
    fn check_nothing_under(&self, marks: &[crate::redact::Mark]) -> Result<()> {
        let mut pages: Vec<usize> = marks.iter().map(|m| m.page).collect();
        pages.sort_unstable();
        pages.dedup();
        for page in pages {
            let text = self.page_text(page)?;
            for line in text.blocks.iter().flat_map(|block| &block.lines) {
                for span in &line.spans {
                    let Some(rect) = span.rect else { continue };
                    let shown = line.text.get(span.start..span.end).unwrap_or("");
                    if shown.trim().is_empty() {
                        continue;
                    }
                    let (x, y) = ((rect.left + rect.right) / 2.0, (rect.bottom + rect.top) / 2.0);
                    let under = marks.iter().any(|m| {
                        m.page == page
                            && m.rect.left <= x
                            && x <= m.rect.right
                            && m.rect.bottom <= y
                            && y <= m.rect.top
                    });
                    if under {
                        return Err(Error::Redaction("text is still readable under a marked area"));
                    }
                }
            }
        }
        Ok(())
    }

    /// This document sanitized (ADR 0042), rewritten in full so that what was
    /// unlinked is gone from the bytes too. The step is always a snapshot.
    pub fn with_sanitized(
        &self,
        options: crate::redact::SanitizeOptions,
    ) -> Result<(Document, Arc<[u8]>, crate::redact::SanitizeReport)> {
        let (update, report) = crate::redact::sanitize_update(&self.bytes, options)?;
        let mut bytes = Vec::with_capacity(self.bytes.len() + update.len());
        bytes.extend_from_slice(&self.bytes);
        bytes.extend_from_slice(&update);
        let rewritten: Arc<[u8]> = Arc::from(self.revision(Arc::from(bytes))?.to_pdf_bytes()?);
        Ok((self.revision(Arc::clone(&rewritten))?, rewritten, report))
    }

    /// This document with `text` written into its page's content (ADR 0021),
    /// the update that does it, and the text's height as displayed.
    pub fn with_new_text(&self, text: &crate::text_edit::NewText) -> Result<(Document, Vec<u8>, f32)> {
        let view = self.view_transform(text.page)?;
        let update = crate::text_edit::add_text_update(&self.bytes, self.password.as_deref(), &view, text)?;
        let height = text.height()?;
        let mut bytes = self.bytes.to_vec();
        bytes.extend_from_slice(&update);
        Ok((self.revision(Arc::from(bytes))?, update, height))
    }

    /// This document with `stamp` (a header or footer, page numbers, or a
    /// watermark) drawn on its pages (ADR 0050), and the update that does it.
    pub fn with_stamp(&self, stamp: &crate::stamp::Stamp) -> Result<(Document, Vec<u8>)> {
        let mut pages = stamp.pages.clone();
        pages.sort_unstable();
        pages.dedup();
        let targets = pages
            .iter()
            .map(|&index| {
                let size = self.page_size(index)?;
                Ok(crate::stamp::Target {
                    index,
                    view: self.view_transform(index)?,
                    width: size.width,
                    height: size.height,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let stamp = crate::stamp::Stamp { pages, kind: stamp.kind.clone() };
        let update = crate::stamp::stamp_update(&self.bytes, self.password.as_deref(), &targets, &stamp)?;
        let mut bytes = self.bytes.to_vec();
        bytes.extend_from_slice(&update);
        Ok((self.revision(Arc::from(bytes))?, update))
    }

    /// The document written again, smaller (ADR 0050), and what that did.
    /// The result is checked by opening it: same page count, or it is
    /// refused. A signed document is refused, since a rewrite breaks its
    /// signatures; so is an encrypted one.
    pub fn compressed(
        &self,
        options: crate::compress::CompressOptions,
    ) -> Result<(Vec<u8>, crate::compress::CompressReport)> {
        if self.is_signed() {
            return Err(Error::EditUnsupported("it is signed, and compressing would break its signatures"));
        }
        let (bytes, report) = crate::compress::compress(&self.bytes, options)?;
        let reopened = self
            .revision(Arc::from(bytes.as_slice()))
            .map_err(|_| Error::SaveRejected("the compressed file would not open"))?;
        if reopened.page_count() != self.page_count() {
            return Err(Error::SaveRejected("the compressed file would have a different number of pages"));
        }
        Ok((bytes, report))
    }

    /// The paragraphs of a page's text, rebuilt from its glyphs (ADR 0023),
    /// then where its text cannot be edited and why. When the page's content
    /// cannot be read at all (an encrypted file, a filter the editor does
    /// not follow), every block of its text as PDFium reads it is refused.
    pub fn text_blocks(&self, page: usize) -> Result<Vec<crate::block_edit::TextBlock>> {
        use crate::block_edit::{Refusal, refused_page, text_blocks};
        let view = self.view_transform(page)?;
        match text_blocks(&self.bytes, &view, page) {
            Err(Error::EditUnsupported(_)) => {
                let reason = if crate::block_edit::encrypted(&self.bytes)? {
                    Refusal::Encrypted
                } else {
                    Refusal::Unsupported
                };
                let text = self.page_text(page)?;
                Ok(refused_page(
                    text.blocks.iter().map(|block| {
                        let shown = view.rect(block.bounds);
                        [shown.x, shown.x + shown.width, shown.y, shown.y + shown.height]
                    }),
                    reason,
                ))
            }
            other => other,
        }
    }

    /// This document with one paragraph of its page's text edited in place
    /// (ADR 0023), the update that does it, and what the edit did.
    pub fn with_block_edit(
        &self,
        edit: &crate::block_edit::BlockEdit,
    ) -> Result<(Document, Vec<u8>, crate::block_edit::EditedBlock)> {
        let view = self.view_transform(edit.page)?;
        let width = self.page_size(edit.page)?.width;
        let (update, outcome) = crate::block_edit::edit_block_update(&self.bytes, &view, width, edit)?;
        let mut bytes = self.bytes.to_vec();
        bytes.extend_from_slice(&update);
        Ok((self.revision(Arc::from(bytes))?, update, outcome))
    }

    /// The objects of a page (ADR 0051): images, paths, text blocks and
    /// forms, in drawing order, with their bounds on the displayed page.
    pub fn page_objects(&self, page: usize) -> Result<Vec<crate::objects::PageObject>> {
        let view = self.view_transform(page)?;
        let size = self.page_size(page)?;
        crate::objects::page_objects(
            &self.bytes,
            self.password.as_deref(),
            &view,
            page,
            f64::from(size.width) * f64::from(size.height),
        )
    }

    /// This document with one object of a page changed (ADR 0051), the
    /// update that does it, and the object's index afterwards (none once removed).
    pub fn with_object_edit(
        &self,
        change: &crate::objects::ObjectChange,
    ) -> Result<(Document, Vec<u8>, Option<usize>)> {
        let view = self.view_transform(change.page)?;
        let size = self.page_size(change.page)?;
        let area = f64::from(size.width) * f64::from(size.height);
        let (update, kept) =
            crate::objects::object_update(&self.bytes, self.password.as_deref(), &view, area, change)?;
        let mut bytes = self.bytes.to_vec();
        bytes.extend_from_slice(&update);
        let index = match kept {
            crate::objects::Kept::Group(group) => {
                crate::objects::object_with_group(&bytes, self.password.as_deref(), change.page, area, group)?
            }
            crate::objects::Kept::Index(index) => Some(index),
            crate::objects::Kept::Gone => None,
        };
        Ok((self.revision(Arc::from(bytes))?, update, index))
    }

    /// This document with a new image on a page (ADR 0051), the update, and
    /// the image's index among the page's objects.
    pub fn with_new_image(
        &self,
        image: &crate::objects::NewImage,
    ) -> Result<(Document, Vec<u8>, Option<usize>)> {
        let view = self.view_transform(image.page)?;
        let size = self.page_size(image.page)?;
        let area = f64::from(size.width) * f64::from(size.height);
        let (update, group) =
            crate::objects::insert_image_update(&self.bytes, self.password.as_deref(), &view, area, image)?;
        let mut bytes = self.bytes.to_vec();
        bytes.extend_from_slice(&update);
        let index =
            crate::objects::object_with_group(&bytes, self.password.as_deref(), image.page, area, group)?;
        Ok((self.revision(Arc::from(bytes))?, update, index))
    }

    /// A page's links and where they go (ADR 0051).
    pub fn page_links(&self, page: usize) -> Result<Vec<crate::links::LinkInfo>> {
        let view = self.view_transform(page)?;
        crate::links::page_links(&self.bytes, self.password.as_deref(), &view, page)
    }

    /// The document's named destinations, for links to them.
    pub fn named_destinations(&self) -> Result<Vec<crate::links::NamedDestination>> {
        crate::links::named_destinations(&self.bytes, self.password.as_deref())
    }

    /// This document with a link added, pointed elsewhere, moved, or
    /// removed (ADR 0051), and the update that does it.
    pub fn with_link_edit(&self, edit: &crate::links::LinkEdit) -> Result<(Document, Vec<u8>)> {
        let view_of = |page: usize| self.view_transform(page);
        let update = crate::links::link_update(&self.bytes, self.password.as_deref(), &view_of, edit)?;
        let mut bytes = self.bytes.to_vec();
        bytes.extend_from_slice(&update);
        Ok((self.revision(Arc::from(bytes))?, update))
    }

    /// The page's annotations of the kinds this app writes (markup, ink,
    /// notes, text boxes), and the map from page space to the page as
    /// displayed. Replies and review states are left out: they belong to
    /// another annotation's thread and are not on the page (ADR 0041).
    pub fn page_annotations(
        &self,
        index: usize,
    ) -> Result<(Vec<crate::annotate::AnnotationInfo>, ViewTransform)> {
        use crate::annotate::{AnnotationInfo, AnnotationKind};
        let page = self.page(index)?;
        let annotations = page.annotations();
        let mut out = Vec::new();
        let mut fills = None;
        // By index, not by iterator: the iterator stops at the first entry
        // PDFium cannot read, and the index is the annotation's position in
        // `/Annots`, which changing or removing it needs.
        for position in 0..annotations.len() {
            let Ok(annotation) = annotations.get(position) else { continue };
            let kind = match annotation_kind(&annotation) {
                Some(kind) => kind,
                // Ours are told apart by their `/Name`, which PDFium does not read.
                None if annotation.annotation_type() == PdfPageAnnotationType::Stamp => {
                    let fills = fills.get_or_insert_with(|| crate::annotate::fill_kinds(&self.bytes, index));
                    let Some(kind) = fills.get(&position) else { continue };
                    *kind
                }
                None => continue,
            };
            let contents =
                matches!(kind, AnnotationKind::Note | AnnotationKind::TextBox | AnnotationKind::FillText)
                    .then(|| annotation.contents().unwrap_or_default());
            out.push(AnnotationInfo {
                index: position,
                kind,
                rects: annotation_rects(&annotation),
                contents,
                strokes: Vec::new(),
            });
        }
        if out.iter().any(|info| info.kind == AnnotationKind::Note) {
            let threads = crate::annotate::threads(&self.bytes, index);
            out.retain(|info| !threads.answers(info.index));
        }
        if out.iter().any(|info| info.kind == AnnotationKind::Ink) {
            let mut strokes = crate::annotate::ink_strokes(&self.bytes, index);
            for info in &mut out {
                if let Some(found) = strokes.remove(&info.index) {
                    info.strokes = found;
                }
            }
        }
        Ok((out, ViewTransform::of_page(&page)?))
    }

    /// Every comment in the document, page by page in `/Annots` order: the
    /// annotations of the kinds this app knows that answer nothing, each with
    /// its author, date, text, replies, and whether it is resolved (ADR 0041).
    /// Markup carries the page's text under it.
    pub fn comments(&self) -> Result<Vec<crate::annotate::Comment>> {
        use std::collections::HashMap;

        use crate::annotate::{AnnotationKind, Comment, Reply};

        /// What PDFium reads of one annotation (decrypted).
        struct Read {
            position: usize,
            kind: AnnotationKind,
            author: Option<String>,
            modified: Option<i64>,
            contents: Option<String>,
            rects: Vec<crate::text::Rect>,
        }
        /// The longest excerpt of marked text, in characters.
        const EXCERPT: usize = 500;

        // Loading a page costs; our parser knows which have annotations.
        let annotated = crate::annotate::annotated_pages(&self.bytes);
        let mut out = Vec::new();
        for index in 0..self.page_count() {
            if annotated.as_ref().is_some_and(|pages| !pages.contains(&index)) {
                continue;
            }
            let page = self.page(index)?;
            let annotations = page.annotations();
            let mut read = Vec::new();
            for position in 0..annotations.len() {
                let Ok(annotation) = annotations.get(position) else { continue };
                let Some(kind) = annotation_kind(&annotation) else { continue };
                read.push(Read {
                    position,
                    kind,
                    author: annotation.creator().filter(|author| !author.trim().is_empty()),
                    modified: annotation
                        .modification_date()
                        .or_else(|| annotation.creation_date())
                        .and_then(|date| crate::annotate::parse_pdf_date(&date)),
                    contents: annotation.contents().filter(|contents| !contents.trim().is_empty()),
                    rects: annotation_rects(&annotation),
                });
            }
            if read.is_empty() {
                continue;
            }
            let threads = crate::annotate::threads(&self.bytes, index);
            let text = read
                .iter()
                .any(|entry| is_markup(entry.kind) && !threads.answers(entry.position))
                .then(|| self.page_text(index).ok())
                .flatten();

            let mut comments: Vec<Comment> = Vec::new();
            let mut at = HashMap::new();
            for entry in &read {
                if threads.answers(entry.position) {
                    continue;
                }
                let Some(bounds) = entry.rects.iter().copied().reduce(crate::text::Rect::union) else {
                    continue;
                };
                let marked_text = text
                    .as_ref()
                    .filter(|_| is_markup(entry.kind))
                    .map(|text| excerpt(&text.text_in(&entry.rects), EXCERPT))
                    .filter(|marked| !marked.is_empty());
                at.insert(entry.position, comments.len());
                comments.push(Comment {
                    page: index,
                    index: entry.position,
                    kind: entry.kind,
                    author: entry.author.clone(),
                    modified: entry.modified,
                    contents: entry.contents.clone(),
                    marked_text,
                    bounds,
                    resolved: false,
                    replies: Vec::new(),
                });
            }
            // Replies, oldest first (undated ones by their place in `/Annots`).
            let order = |entry: &Read| (entry.modified.unwrap_or(i64::MIN), entry.position);
            let mut replies: Vec<(usize, &Read)> = read
                .iter()
                .filter_map(|entry| threads.replies.get(&entry.position).map(|&root| (root, entry)))
                .collect();
            replies.sort_by_key(|(_, entry)| order(entry));
            for (root, entry) in replies {
                if let Some(&i) = at.get(&root) {
                    comments[i].replies.push(Reply {
                        index: entry.position,
                        author: entry.author.clone(),
                        modified: entry.modified,
                        contents: entry.contents.clone().unwrap_or_default(),
                    });
                }
            }
            // The latest review state counts.
            let mut latest: HashMap<usize, ((i64, usize), bool)> = HashMap::new();
            for entry in &read {
                let Some(&(root, Some(resolved))) = threads.states.get(&entry.position) else { continue };
                let key = order(entry);
                let state = latest.entry(root).or_insert((key, resolved));
                if key >= state.0 {
                    *state = (key, resolved);
                }
            }
            for (root, (_, resolved)) in latest {
                if let Some(&i) = at.get(&root) {
                    comments[i].resolved = resolved;
                }
            }
            out.extend(comments);
        }
        Ok(out)
    }

    /// The map from page space to the page as displayed.
    pub fn view_transform(&self, index: usize) -> Result<ViewTransform> {
        ViewTransform::of_page(&self.page(index)?)
    }

    /// `pages` (in that order) as a PDF of their own, copied by PDFium with
    /// everything they use (fonts, images, annotations). The new file has no
    /// outline and no encryption: it is a fresh document (ADR 0016).
    pub fn extract_pages(&self, pages: &[usize]) -> Result<Vec<u8>> {
        if pages.is_empty() {
            return Err(Error::InvalidEdit("no pages given"));
        }
        let mut out = self.engine.new_pdf()?;
        for (position, &page) in pages.iter().enumerate() {
            let index = self.page_index(page)?;
            let at = PdfPageIndex::try_from(position).map_err(|_| Error::InvalidEdit("too many pages"))?;
            out.pages_mut().copy_page_range_from_document(&self.inner, index..=index, at)?;
        }
        Ok(out.save_to_bytes()?)
    }

    /// The document in parts of at most `size` pages each, in order.
    pub fn split_every(&self, size: usize) -> Result<Vec<Vec<u8>>> {
        if size == 0 {
            return Err(Error::InvalidEdit("parts need at least one page"));
        }
        let pages: Vec<usize> = (0..self.page_count()).collect();
        pages.chunks(size).map(|part| self.extract_pages(part)).collect()
    }

    /// Where a split at each top-level bookmark falls (ADR 0016): one part
    /// per top-level outline entry that points into this document, from its
    /// page to the next one's, in page order, named by the entry's title.
    /// Pages before the first such entry go with it; entries on the same page
    /// as an earlier one add nothing. Fewer than two parts means the outline
    /// gives no split.
    pub fn bookmark_parts(&self) -> Vec<(String, std::ops::Range<usize>)> {
        let count = self.page_count();
        let mut starts: Vec<(usize, String)> = self
            .outline()
            .into_iter()
            .filter_map(|item| Some((item.page.filter(|page| *page < count)?, item.title)))
            .collect();
        // Stable: of entries on one page, the first listed names the part.
        starts.sort_by_key(|(page, _)| *page);
        starts.dedup_by_key(|(page, _)| *page);
        let mut parts = Vec::new();
        for (i, (page, title)) in starts.iter().enumerate() {
            let start = if i == 0 { 0 } else { *page };
            let end = starts.get(i + 1).map_or(count, |(next, _)| *next);
            parts.push((title.clone(), start..end));
        }
        parts
    }

    /// The pages of each range, in order, each as a PDF of its own (as
    /// [`Document::extract_pages`] makes them).
    pub fn split_ranges(&self, ranges: &[std::ops::Range<usize>]) -> Result<Vec<Vec<u8>>> {
        ranges.iter().map(|range| self.extract_pages(&range.clone().collect::<Vec<_>>())).collect()
    }

    /// The document split, in page order, into files of at most `limit`
    /// bytes each: every part holds as many pages as fit (ADR 0016). A page
    /// that alone is larger than `limit` makes a part of its own. Each part's
    /// size is measured by writing it, so shared fonts and images count in
    /// every part that uses them.
    pub fn split_by_size(&self, limit: usize) -> Result<Vec<(std::ops::Range<usize>, Vec<u8>)>> {
        let count = self.page_count();
        let part = |range: std::ops::Range<usize>| self.extract_pages(&range.collect::<Vec<_>>());
        let mut parts = Vec::new();
        let mut start = 0;
        while start < count {
            // One page always goes; then the longest run that still fits,
            // found by doubling, then halving the gap (sizes grow with pages).
            let mut fits = start + 1;
            let mut bytes = part(start..fits)?;
            if bytes.len() <= limit {
                let mut too_long = None;
                let mut length = 2;
                while fits < count {
                    let end = (start + length).min(count);
                    let candidate = part(start..end)?;
                    if candidate.len() > limit {
                        too_long = Some(end);
                        break;
                    }
                    (fits, bytes) = (end, candidate);
                    length *= 2;
                }
                if let Some(mut over) = too_long {
                    while over - fits > 1 {
                        let middle = fits + (over - fits) / 2;
                        let candidate = part(start..middle)?;
                        if candidate.len() > limit {
                            over = middle;
                        } else {
                            (fits, bytes) = (middle, candidate);
                        }
                    }
                }
            }
            parts.push((start..fits, bytes));
            start = fits;
        }
        Ok(parts)
    }

    /// This document with every page of `others` inserted before page `at`
    /// (`at` = page count appends), in order; and the new revision's bytes.
    ///
    /// PDFium copies the pages with everything they use, then rewrites the
    /// whole file: copying objects across files is beyond the incremental
    /// writer (ADR 0016). A signed document is refused, since a rewrite would
    /// invalidate its signatures.
    pub fn with_pages_from(&self, others: &[&Document], at: usize) -> Result<(Document, Arc<[u8]>)> {
        if self.is_signed() {
            return Err(Error::EditUnsupported(
                "it is signed, and inserting pages from another file would invalidate its signatures",
            ));
        }
        let count = self.page_count();
        if at > count {
            return Err(Error::PageOutOfRange { index: at, count });
        }
        let mut working = self.revision(self.bytes())?;
        let mut position = PdfPageIndex::try_from(at).map_err(|_| Error::InvalidEdit("too many pages"))?;
        for other in others {
            let pages = PdfPageIndex::try_from(other.page_count())
                .map_err(|_| Error::InvalidEdit("too many pages"))?;
            if pages == 0 {
                continue;
            }
            working.inner.pages_mut().copy_page_range_from_document(&other.inner, 0..=pages - 1, position)?;
            position += pages;
        }
        let bytes: Arc<[u8]> = Arc::from(working.inner.save_to_bytes()?);
        Ok((self.revision(Arc::clone(&bytes))?, bytes))
    }

    /// Whether the document carries digital signatures, which only an
    /// incremental save keeps valid.
    pub fn is_signed(&self) -> bool {
        !self.inner.signatures().is_empty()
    }

    /// The document's own reading preferences: `/ViewerPreferences /Direction`
    /// and `/PageLayout`, where it states them (see [`crate::catalog`]).
    pub fn catalog_hints(&self) -> crate::catalog::CatalogHints {
        self.hints
    }

    pub fn page_count(&self) -> usize {
        usize::try_from(self.inner.pages().len()).unwrap_or(0)
    }

    /// Size of one page, read without loading the page's content.
    pub fn page_size(&self, index: usize) -> Result<PageSize> {
        let rect = self.inner.pages().page_size(self.page_index(index)?)?;
        Ok(PageSize { width: rect.width().value, height: rect.height().value })
    }

    /// Sizes of every page, in order, read without loading any page's content,
    /// so a viewer can lay out a long document before rendering anything.
    pub fn page_sizes(&self) -> Result<Vec<PageSize>> {
        Ok(self
            .inner
            .pages()
            .page_sizes()?
            .iter()
            .map(|rect| PageSize { width: rect.width().value, height: rect.height().value })
            .collect())
    }

    /// Parses one page and keeps it open, so several tiles can be rendered from
    /// it without re-parsing its content each time (parsing costs more than
    /// rasterizing a 512 px tile of a typical text page). The page holds a
    /// reference to the document, which therefore stays open at least as long.
    pub fn load_page(self: &Arc<Self>, index: usize) -> Result<LoadedPage> {
        let page = self.page(index)?;
        let size = PageSize { width: page.width().value, height: page.height().value };
        Ok(LoadedPage { page, size, index, _document: Arc::clone(self) })
    }

    /// Renders the whole page at `scale` device pixels per point.
    pub fn render_page(&self, index: usize, scale: f32) -> Result<Bitmap> {
        let size = self.page_size(index)?;
        let (width, height) = scaled_page_pixels(size, scale)?;
        self.render_tile(index, TileRequest { scale, x: 0, y: 0, width, height })
    }

    /// Renders one tile of a page. The tile must lie entirely inside the scaled
    /// page; callers clamp edge tiles to the page bounds.
    pub fn render_tile(&self, index: usize, tile: TileRequest) -> Result<Bitmap> {
        validate_tile_size(&tile)?;
        let page = self.page(index)?;
        let size = PageSize { width: page.width().value, height: page.height().value };
        render_tile_of(&page, size, tile)
    }

    /// The page's text exactly as PDFium reports it.
    ///
    /// This is *not* reading order: PDFs frequently store Arabic in visual order.
    /// The bidi-aware extraction required by spec section 6.1 is Milestone 1 and
    /// will be a separate API; this one exists for diagnostics and fixtures.
    pub fn page_text_raw(&self, index: usize) -> Result<String> {
        Ok(self.page(index)?.text()?.all())
    }

    /// The document as PDF bytes, as PDFium would save it (with any edits).
    pub fn to_pdf_bytes(&self) -> Result<Vec<u8>> {
        Ok(self.inner.save_to_bytes()?)
    }

    /// How the document is protected, and what the password it was opened
    /// with allows.
    pub fn security(&self) -> crate::security::SecurityInfo {
        use crate::pdf::{File, crypt::Crypt};
        let Some(file) = File::new(&self.bytes) else { return crate::security::SecurityInfo::open() };
        if file.trailer.get(b"Encrypt").is_none() {
            return crate::security::SecurityInfo::open();
        }
        match Crypt::of(&file, self.password.as_deref()) {
            Ok(Some(crypt)) => crate::security::SecurityInfo::of(&crypt),
            // Opened by PDFium, but with a handler this editor cannot write.
            _ => crate::security::SecurityInfo {
                encrypted: true,
                cipher: None,
                revision: 0,
                permissions: crate::security::Permissions::all(),
                owner: false,
            },
        }
    }

    /// The document as PDF bytes with its protection changed (ADR 0043).
    ///
    /// Changing a signed file would break its signatures, and changing a
    /// file's protection needs the owner password when it restricts anything.
    pub fn to_pdf_bytes_secured(&self, change: &crate::security::SecurityChange) -> Result<Vec<u8>> {
        use crate::security::SecurityChange;
        let base = self.to_pdf_bytes()?;
        let target = match change {
            SecurityChange::Keep => return Ok(base),
            SecurityChange::Remove => None,
            SecurityChange::Set(protection) => Some(protection),
        };
        let info = self.security();
        if info.encrypted && !info.owner && info.permissions != crate::security::Permissions::all() {
            return Err(Error::NotPermitted);
        }
        if !info.encrypted && target.is_none() {
            return Ok(base);
        }
        if self.is_signed() {
            return Err(Error::EditUnsupported(
                "it is signed, and changing its protection would break the signatures",
            ));
        }
        crate::pdf::protect::rewrite(&base, self.password.as_deref(), target)
    }

    /// The password the document was opened with, if any.
    pub(crate) fn password_in_use(&self) -> Option<String> {
        self.password.clone()
    }

    /// The same file's bytes opened as a new document with another password
    /// (after its protection changed).
    pub fn reopen(&self, bytes: Arc<[u8]>, password: Option<&str>) -> Result<Document> {
        self.engine.open_shared(bytes, password)
    }

    /// The document outline (bookmarks), as a tree.
    pub fn outline(&self) -> Vec<crate::outline::OutlineItem> {
        crate::outline::read(&self.inner)
    }

    /// The page's label (e.g. "iv", "A-3") when the document defines labels.
    /// Loads the page, so ask for visible pages, not all of them.
    pub fn page_label(&self, index: usize) -> Result<Option<String>> {
        Ok(self.page(index)?.label().map(str::to_string))
    }

    /// Every page's label, or `None` when the document defines no labels.
    ///
    /// A `/PageLabels` tree must cover page 0, so a document whose first page
    /// has no label has none at all and costs one page load. Otherwise every
    /// page is loaded (PDFium reads labels only through a loaded page): call
    /// this off the UI thread.
    pub fn page_labels(&self) -> Result<Option<Vec<Option<String>>>> {
        if self.page_count() == 0 || self.page_label(0)?.is_none() {
            return Ok(None);
        }
        (0..self.page_count()).map(|index| self.page_label(index)).collect::<Result<Vec<_>>>().map(Some)
    }

    /// The page's text in logical reading order, with per-character geometry
    /// for selection (spec section 6.1). See [`crate::text`].
    pub fn page_text(&self, index: usize) -> Result<crate::text::PageText> {
        analyze_page(&self.page(index)?)
    }

    /// [`page_text`](Self::page_text) plus the mapping from its page-space
    /// geometry to the page as displayed (rotation, crop-box origin), from
    /// one page load.
    pub fn page_text_view(&self, index: usize) -> Result<(crate::text::PageText, ViewTransform)> {
        let page = self.page(index)?;
        Ok((analyze_page(&page)?, ViewTransform::of_page(&page)?))
    }

    fn page(&self, index: usize) -> Result<PdfPage<'static>> {
        Ok(self.inner.pages().get(self.page_index(index)?)?)
    }

    fn page_index(&self, index: usize) -> Result<PdfPageIndex> {
        let count = self.page_count();
        if index >= count {
            return Err(Error::PageOutOfRange { index, count });
        }
        PdfPageIndex::try_from(index).map_err(|_| Error::PageOutOfRange { index, count })
    }
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document").field("page_count", &self.page_count()).finish()
    }
}

/// One page of a [`Document`], parsed once and kept open for rendering
/// (see [`Document::load_page`]).
pub struct LoadedPage {
    // Declared before `_document` so the page is closed before the document.
    page: PdfPage<'static>,
    size: PageSize,
    index: usize,
    _document: Arc<Document>,
}

impl LoadedPage {
    pub fn index(&self) -> usize {
        self.index
    }

    pub fn size(&self) -> PageSize {
        self.size
    }

    /// Same contract as [`Document::render_tile`].
    pub fn render_tile(&self, tile: TileRequest) -> Result<Bitmap> {
        validate_tile_size(&tile)?;
        render_tile_of(&self.page, self.size, tile)
    }
}

impl std::fmt::Debug for LoadedPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedPage").field("index", &self.index).field("size", &self.size).finish()
    }
}

fn analyze_page(page: &PdfPage<'_>) -> Result<crate::text::PageText> {
    let text = page.text()?;
    let glyphs = crate::text::collect_glyphs(&text);
    let spaces = crate::text::collect_space_marks(page, &text);
    Ok(crate::text::analyze(&glyphs, &spaces))
}

fn validate_tile_size(tile: &TileRequest) -> Result<()> {
    if tile.width == 0 || tile.height == 0 {
        return Err(Error::InvalidRenderRequest("tile has zero area"));
    }
    if tile.width > MAX_BITMAP_SIDE || tile.height > MAX_BITMAP_SIDE {
        return Err(Error::InvalidRenderRequest("tile exceeds the maximum bitmap size"));
    }
    Ok(())
}

fn render_tile_of(page: &PdfPage<'_>, size: PageSize, tile: TileRequest) -> Result<Bitmap> {
    let (page_w, page_h) = scaled_page_pixels(size, tile.scale)?;

    if tile.x.checked_add(tile.width).is_none_or(|right| right > page_w)
        || tile.y.checked_add(tile.height).is_none_or(|bottom| bottom > page_h)
    {
        return Err(Error::InvalidRenderRequest("tile extends past the page"));
    }

    // PDFium renders straight into our buffer: no engine-side allocation and no
    // copy per tile. A 4-byte format's rows need no padding, so the buffer is
    // exactly the `Bitmap` layout.
    let byte_len = (tile.width as usize)
        .checked_mul(tile.height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(Error::InvalidRenderRequest("tile exceeds the maximum bitmap size"))?;
    let mut pixels = vec![0u8; byte_len];
    {
        let mut bitmap = PdfBitmap::from_bytes(
            to_i32(tile.width)?,
            to_i32(tile.height)?,
            PdfBitmapFormat::BGRA,
            &mut pixels,
        )?;
        let config = PdfRenderConfig::new()
            .set_fixed_size(to_i32(page_w)?, to_i32(page_h)?)
            // A negative origin slides the full-size page under the tile-sized bitmap.
            // pdfium-render only honours the origin on its form-data render path
            // (FPDF_RenderPageBitmap); the matrix path ignores it, so form data
            // rendering must stay enabled here.
            .set_origin(-to_i32(tile.x)?, -to_i32(tile.y)?)
            .render_form_data(true)
            .render_annotations(true)
            .clear_before_rendering(true)
            .set_clear_color(PdfColor::WHITE)
            // PDFium's native order (BGRA): no swap, and what a BMP holds.
            .set_reverse_byte_order(false);
        page.render_into_bitmap_with_config(&mut bitmap, &config)?;
    }

    Bitmap::from_bgra(tile.width, tile.height, pixels)
        .ok_or_else(|| Error::Pdfium("PDFium returned a bitmap of unexpected size".into()))
}

fn scaled_page_pixels(size: PageSize, scale: f32) -> Result<(u32, u32)> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err(Error::InvalidRenderRequest("scale must be a positive number"));
    }
    let width = (size.width * scale).ceil();
    let height = (size.height * scale).ceil();
    // A page can legitimately be far larger than one bitmap once zoomed in (that is
    // what tiles are for), but it must still fit PDFium's signed 32-bit coordinates.
    if !(1.0..=i32::MAX as f32).contains(&width) || !(1.0..=i32::MAX as f32).contains(&height) {
        return Err(Error::InvalidRenderRequest("scaled page size is out of range"));
    }
    Ok((width as u32, height as u32))
}

fn to_i32(value: u32) -> Result<i32> {
    i32::try_from(value).map_err(|_| Error::InvalidRenderRequest("coordinate is out of range"))
}
