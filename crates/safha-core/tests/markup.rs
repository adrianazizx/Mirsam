//! Markup annotations as incremental updates (ADR 0017), on real files: each
//! result is reopened and rendered by PDFium, so the test sees what a reader
//! sees, and checked by `qpdf --check`. In encrypted files qpdf also decrypts
//! the new objects, which shows they were encrypted with the right keys.

mod common;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use common::manifest;
use safha_core::annotate::{AnnotationEdit, Markup, MarkupKind, Quad};
use safha_core::edit::EditHistory;
use safha_core::save::save;
use safha_core::view::ViewRect;
use safha_core::{Bitmap, Document, Engine, Error, PdfiumSource};

fn engine() -> &'static Engine {
    Engine::global(&PdfiumSource::Auto).expect("PDFium is available (run scripts/fetch-pdfium.sh)")
}

fn fixture(name: &str) -> (Vec<u8>, Option<String>) {
    let manifest = manifest();
    let fixture = manifest
        .fixtures
        .iter()
        .find(|f| f.name() == name)
        .unwrap_or_else(|| panic!("fixture {name} missing: run `node fixtures/generate.mjs`"));
    (std::fs::read(fixture.file()).expect("fixture readable"), fixture.password.clone())
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("safha-markup-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

fn qpdf(args: &[&str]) -> String {
    let output = Command::new("qpdf").args(args).output().expect("qpdf runs (install qpdf)");
    // Exit 3 is warnings only.
    assert!(
        matches!(output.status.code(), Some(0 | 3)),
        "qpdf {args:?} failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Writes `bytes` to a scratch file for qpdf.
fn on_disk(bytes: &[u8], name: &str) -> String {
    let path = scratch(name);
    std::fs::write(&path, bytes).expect("write scratch file");
    path.to_string_lossy().into_owned()
}

fn password_arg(password: Option<&str>) -> String {
    format!("--password={}", password.unwrap_or(""))
}

/// The displayed rectangles of the page's first `count` text lines.
fn lines(document: &Document, page: usize, count: usize) -> Vec<ViewRect> {
    let (text, view) = document.page_text_view(page).expect("text");
    let rects: Vec<ViewRect> = text
        .blocks
        .iter()
        .flat_map(|block| &block.lines)
        .take(count)
        .map(|line| view.rect(line.bounds))
        .collect();
    assert_eq!(rects.len(), count, "page {page} has {count} lines");
    rects
}

fn markup_on(document: &Document, page: usize, kind: MarkupKind, rects: &[ViewRect]) -> AnnotationEdit {
    let view = document.view_transform(page).expect("view");
    AnnotationEdit::AddMarkup {
        page,
        markup: Markup {
            kind,
            quads: rects.iter().map(|rect| Quad::from_view(*rect, &view)).collect(),
            color: kind.default_color(),
        },
    }
}

const SCALE: f32 = 2.0;

fn render(document: &Document, page: usize) -> Bitmap {
    document.render_page(page, SCALE).expect("renders")
}

/// A pixel that is clearly a colour, not a shade of grey (text and its
/// anti-aliasing are grey).
fn coloured([r, g, b, _]: [u8; 4]) -> bool {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    max - min > 60
}

fn inside(rect: &ViewRect, x: u32, y: u32, margin: f32) -> bool {
    let (x, y) = (x as f32 / SCALE, y as f32 / SCALE);
    x >= rect.x - margin
        && x <= rect.x + rect.width + margin
        && y >= rect.y - margin
        && y <= rect.y + rect.height + margin
}

/// Coloured pixels inside `rect`.
fn coloured_in(bitmap: &Bitmap, rect: &ViewRect) -> usize {
    let mut count = 0;
    for y in 0..bitmap.height() {
        for x in 0..bitmap.width() {
            if inside(rect, x, y, 0.0) && bitmap.pixel(x, y).is_some_and(coloured) {
                count += 1;
            }
        }
    }
    count
}

/// Pixels that changed outside every one of `rects` (with a pixel's margin
/// for anti-aliasing).
fn changed_outside(before: &Bitmap, after: &Bitmap, rects: &[ViewRect]) -> usize {
    assert_eq!((before.width(), before.height()), (after.width(), after.height()));
    let mut count = 0;
    for y in 0..before.height() {
        for x in 0..before.width() {
            if rects.iter().any(|rect| inside(rect, x, y, 1.0)) {
                continue;
            }
            if before.pixel(x, y) != after.pixel(x, y) {
                count += 1;
            }
        }
    }
    count
}

fn apply(document: &Document, edit: &AnnotationEdit) -> (Document, Vec<u8>) {
    document.with_annotation_edit(edit).expect("the markup is added")
}

#[test]
fn each_kind_draws_on_its_line_and_nowhere_else() {
    let (bytes, _) = fixture("chrome/en-paragraphs");
    let original = engine().open_bytes(bytes.clone(), None).expect("opens");
    let rects = lines(&original, 0, 4);
    let before = render(&original, 0);
    for rect in &rects {
        assert_eq!(coloured_in(&before, rect), 0, "nothing coloured before");
    }

    let kinds = [MarkupKind::Highlight, MarkupKind::Underline, MarkupKind::StrikeOut, MarkupKind::Squiggly];
    let mut document = original;
    for (kind, rect) in kinds.iter().zip(&rects) {
        document = apply(&document, &markup_on(&document, 0, *kind, std::slice::from_ref(rect))).0;
    }
    let after = render(&document, 0);
    for (kind, rect) in kinds.iter().zip(&rects) {
        let area = (rect.width * rect.height * SCALE * SCALE) as usize;
        let count = coloured_in(&after, rect);
        // A highlight covers its line; a line marks a sliver of it.
        let least = if *kind == MarkupKind::Highlight { area / 2 } else { area / 40 };
        assert!(count >= least, "{kind:?}: {count} coloured pixels of {area}");
    }
    assert_eq!(changed_outside(&before, &after, &rects), 0, "markup drew outside its lines");

    // The engine lists them back where they were put, by kind.
    let (markup, view) = document.page_markup(0).expect("markup");
    assert_eq!(markup.iter().map(|m| m.kind).collect::<Vec<_>>(), kinds);
    for (info, rect) in markup.iter().zip(&rects) {
        let shown = view.rect(info.rects[0]);
        for (a, b) in
            [(shown.x, rect.x), (shown.y, rect.y), (shown.width, rect.width), (shown.height, rect.height)]
        {
            assert!((a - b).abs() < 0.01, "{shown:?} vs {rect:?}");
        }
    }
    // Text extraction does not see them.
    let text = |document: &Document| document.page_text(0).expect("text").text();
    assert_eq!(text(&document), text(&engine().open_bytes(bytes, None).expect("opens")));

    // A valid file, and saving (a full rewrite) keeps them.
    let saved_bytes = document.bytes();
    qpdf(&["--check", &on_disk(&saved_bytes, "four-kinds.pdf")]);
    let target = scratch("four-kinds-saved.pdf");
    save(&document, &target).expect("saves");
    let reopened = engine().open_file(&target, None).expect("reopens");
    assert_eq!(reopened.page_markup(0).expect("markup").0.len(), 4);
}

#[test]
fn an_arabic_selection_is_marked_where_it_is() {
    let (bytes, _) = fixture("chrome/ar-plain");
    let document = engine().open_bytes(bytes, None).expect("opens");
    let rects = lines(&document, 0, 2);
    let before = render(&document, 0);
    let (edited, _) = apply(&document, &markup_on(&document, 0, MarkupKind::Highlight, &rects));
    let after = render(&edited, 0);
    for rect in &rects {
        let area = (rect.width * rect.height * SCALE * SCALE) as usize;
        assert!(coloured_in(&after, rect) >= area / 2);
    }
    assert_eq!(changed_outside(&before, &after, &rects), 0);
    // One annotation with a quad per line.
    let (markup, _) = edited.page_markup(0).expect("markup");
    assert_eq!(markup.len(), 1);
    assert_eq!(markup[0].rects.len(), 2);
}

#[test]
fn markup_on_a_turned_page_lands_on_its_text() {
    // Pages turned 0, 90, 180 and 270 degrees: the view rectangle maps back
    // to page space through the page's rotation.
    let (bytes, _) = fixture("qpdf/rotated");
    let mut document = engine().open_bytes(bytes, None).expect("opens");
    for page in 0..4 {
        let rects = lines(&document, page, 1);
        let before = render(&document, page);
        document = apply(&document, &markup_on(&document, page, MarkupKind::Underline, &rects)).0;
        let after = render(&document, page);
        assert!(coloured_in(&after, &rects[0]) > 0, "page {page}: the underline is on the line");
        assert_eq!(changed_outside(&before, &after, &rects), 0, "page {page}: and nowhere else");
    }
}

/// Encrypts `chrome/en-paragraphs` with qpdf: `args` go after `--encrypt`.
fn encrypted_copy(name: &str, args: &[&str]) -> Vec<u8> {
    let (source, _) = fixture("chrome/en-paragraphs");
    let input = on_disk(&source, &format!("{name}-plain.pdf"));
    let output = scratch(&format!("{name}.pdf")).to_string_lossy().into_owned();
    let mut command = vec!["--allow-weak-crypto", "--encrypt"];
    command.extend_from_slice(args);
    command.extend_from_slice(&["--", &input, &output]);
    qpdf(&command);
    std::fs::read(&output).expect("encrypted copy")
}

/// The object number of the annotation an update adds.
fn annotation_number(update: &[u8]) -> String {
    let text = String::from_utf8_lossy(update);
    let at = text.find(" 0 obj\n<</Type /Annot").expect("the update writes an annotation");
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    text[start..at].to_string()
}

#[test]
fn encrypted_files_get_encrypted_markup() {
    let (aes256, aes256_password) = fixture("qpdf/encrypted");
    let cases: Vec<(&str, Vec<u8>, Option<&str>)> = vec![
        ("AES-256, user password", aes256.clone(), aes256_password.as_deref()),
        ("AES-256, owner password", encrypted_copy("aes256-owner", &["user", "owner", "256"]), Some("owner")),
        ("AES-128", encrypted_copy("aes128", &["user", "owner", "128", "--use-aes=y"]), Some("user")),
        ("RC4-128", encrypted_copy("rc4-128", &["user", "owner", "128", "--use-aes=n"]), Some("user")),
        ("RC4-40", encrypted_copy("rc4-40", &["user", "owner", "40"]), Some("user")),
        (
            "RC4-128, owner password",
            encrypted_copy("rc4-owner", &["user", "owner", "128", "--use-aes=n"]),
            Some("owner"),
        ),
        ("permissions only", encrypted_copy("permissions", &["", "owner", "256", "--modify=none"]), None),
    ];
    for (label, bytes, password) in cases {
        let document = engine().open_bytes(bytes, password).unwrap_or_else(|e| panic!("{label}: opens: {e}"));
        let rects = lines(&document, 0, 1);
        let (edited, update) = document
            .with_annotation_edit(&markup_on(&document, 0, MarkupKind::Highlight, &rects))
            .unwrap_or_else(|e| panic!("{label}: {e}"));
        // PDFium decrypts the appearance stream and draws it.
        let after = render(&edited, 0);
        let area = (rects[0].width * rects[0].height * SCALE * SCALE) as usize;
        assert!(coloured_in(&after, &rects[0]) >= area / 2, "{label}: the highlight is drawn");

        // qpdf decrypts the new strings and the stream too.
        let file = on_disk(&edited.bytes(), "encrypted-markup.pdf");
        let password = password_arg(password);
        qpdf(&[&password, "--check", &file]);
        let number = annotation_number(&update);
        let annotation = qpdf(&[&password, &format!("--show-object={number}"), &file]);
        assert!(annotation.contains("/M (D:20"), "{label}: the date decrypts: {annotation}");
        let stream: u32 = number.parse::<u32>().expect("number") + 1;
        let content = qpdf(&[&password, &format!("--show-object={stream}"), "--filtered-stream-data", &file]);
        assert!(content.starts_with("/G0 gs "), "{label}: the appearance decrypts: {content:?}");
    }
}

/// A one-page file whose `/Annots` is an object of its own, holding a
/// highlight with a pop-up, a reply to it, and a link.
fn annotated_pdf() -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Annots 4 0 R >>",
        "[5 0 R 6 0 R 7 0 R 8 0 R]",
        "<< /Type /Annot /Subtype /Highlight /Rect [10 10 90 22] /QuadPoints [10 22 90 22 10 10 90 10] /C [1 1 0] /Popup 6 0 R /P 3 0 R >>",
        "<< /Type /Annot /Subtype /Popup /Rect [100 100 180 160] /Parent 5 0 R >>",
        "<< /Type /Annot /Subtype /Text /Rect [92 10 104 22] /IRT 5 0 R /Contents (A reply) >>",
        "<< /Type /Annot /Subtype /Link /Rect [10 150 90 170] /A << /S /URI /URI (https://example.com) >> >>",
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1)
            .as_bytes(),
    );
    pdf
}

#[test]
fn an_annots_array_of_its_own_is_rewritten_in_place() {
    let document = engine().open_bytes(annotated_pdf(), None).expect("opens");
    let (markup, _) = document.page_markup(0).expect("markup");
    assert_eq!(markup.len(), 1, "the highlight; the pop-up, reply and link are not markup");
    assert_eq!((markup[0].index, markup[0].kind), (0, MarkupKind::Highlight));

    // Adding appends to the array object; the page still points at it.
    let rect = ViewRect { x: 10.0, y: 40.0, width: 60.0, height: 12.0 };
    let (added, _) = apply(&document, &markup_on(&document, 0, MarkupKind::StrikeOut, &[rect]));
    let file = on_disk(&added.bytes(), "own-array-added.pdf");
    qpdf(&["--check", &file]);
    assert!(qpdf(&["--show-object=3", &file]).contains("/Annots 4 0 R"));
    assert_eq!(qpdf(&["--show-object=4", &file]).trim(), "[ 5 0 R 6 0 R 7 0 R 8 0 R 9 0 R ]");
    let (markup, _) = added.page_markup(0).expect("markup");
    assert_eq!(
        markup.iter().map(|m| (m.index, m.kind)).collect::<Vec<_>>(),
        [(0, MarkupKind::Highlight), (4, MarkupKind::StrikeOut)]
    );

    // Removing the highlight takes its pop-up and the reply along; the link stays.
    let (removed, _) =
        added.with_annotation_edit(&AnnotationEdit::Remove { page: 0, index: 0 }).expect("removes");
    let file = on_disk(&removed.bytes(), "own-array-removed.pdf");
    qpdf(&["--check", &file]);
    assert_eq!(qpdf(&["--show-object=4", &file]).trim(), "[ 8 0 R 9 0 R ]");
    let (markup, _) = removed.page_markup(0).expect("markup");
    assert_eq!(markup.iter().map(|m| (m.index, m.kind)).collect::<Vec<_>>(), [(1, MarkupKind::StrikeOut)]);

    // The link is not markup: not removed here.
    assert!(matches!(
        removed.with_annotation_edit(&AnnotationEdit::Remove { page: 0, index: 0 }),
        Err(Error::InvalidEdit(_))
    ));
    assert!(matches!(
        removed.with_annotation_edit(&AnnotationEdit::Remove { page: 0, index: 9 }),
        Err(Error::InvalidEdit(_))
    ));
}

#[test]
fn undo_takes_markup_away_and_redo_brings_it_back() {
    let (bytes, _) = fixture("chrome/mixed-inline");
    let document = engine().open_bytes(bytes.clone(), None).expect("opens");
    let mut history = EditHistory::new(Arc::from(bytes));
    let rects = lines(&document, 0, 1);
    let (_, update) = apply(&document, &markup_on(&document, 0, MarkupKind::Squiggly, &rects));
    history.push(update);
    let count =
        |bytes: Arc<[u8]>| document.revision(bytes).expect("opens").page_markup(0).expect("markup").0.len();
    assert_eq!(count(history.current()), 1);
    assert_eq!(count(history.undo().expect("undo")), 0);
    assert_eq!(count(history.redo().expect("redo")), 1);
}

#[test]
fn a_page_out_of_range_is_refused() {
    let (bytes, _) = fixture("chrome/en-paragraphs");
    let document = engine().open_bytes(bytes, None).expect("opens");
    let rect = ViewRect { x: 10.0, y: 10.0, width: 10.0, height: 10.0 };
    let mut edit = markup_on(&document, 0, MarkupKind::Highlight, &[rect]);
    if let AnnotationEdit::AddMarkup { page, .. } = &mut edit {
        *page = 3;
    }
    assert!(matches!(
        document.with_annotation_edit(&edit),
        Err(Error::PageOutOfRange { index: 3, count: 1 })
    ));
}

#[test]
fn a_selection_across_pages_is_one_step() {
    let (bytes, _) = fixture("qpdf/rotated");
    let document = engine().open_bytes(bytes.clone(), None).expect("opens");
    let edits: Vec<AnnotationEdit> = [1, 2]
        .into_iter()
        .map(|page| markup_on(&document, page, MarkupKind::Highlight, &lines(&document, page, 1)))
        .collect();
    let (edited, update) = document.with_annotation_edits(&edits).expect("marks both pages");
    let count = |document: &Document, page: usize| document.page_markup(page).expect("markup").0.len();
    assert_eq!((0..4).map(|page| count(&edited, page)).collect::<Vec<_>>(), [0, 1, 1, 0]);
    qpdf(&["--check", &on_disk(&edited.bytes(), "two-pages.pdf")]);

    let mut history = EditHistory::new(Arc::from(bytes));
    history.push_annotations(update);
    let undone = document.revision(history.undo().expect("undo")).expect("opens");
    assert_eq!((0..4).map(|page| count(&undone, page)).collect::<Vec<_>>(), [0, 0, 0, 0]);
}
