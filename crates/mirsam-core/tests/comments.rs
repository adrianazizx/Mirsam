//! Comments, replies and review states (ADR 0041), on real files: what is
//! written reads back through `Document::comments`, draws nothing on the
//! page, passes `qpdf --check`, and threads written by other viewers (replies
//! to replies, older and newer states, groups) are followed.

mod common;

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use common::manifest;
use mirsam_core::annotate::{AnnotationEdit, AnnotationKind, Comment, Markup, MarkupKind, Note, Quad};
use mirsam_core::view::ViewRect;
use mirsam_core::{Bitmap, Document, Engine, Error, PdfiumSource};

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

fn qpdf_check(bytes: &[u8], password: Option<&str>, name: &str) {
    let dir = std::env::temp_dir().join(format!("mirsam-comments-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write");
    let mut command = Command::new("qpdf");
    if let Some(password) = password {
        command.arg(format!("--password={password}"));
    }
    let output = command.arg("--check").arg(&path).output().expect("qpdf runs (install qpdf)");
    assert!(
        matches!(output.status.code(), Some(0 | 3)),
        "qpdf --check failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn render(document: &Document) -> Bitmap {
    document.render_page(0, 1.5).expect("renders")
}

fn same_pixels(a: &Bitmap, b: &Bitmap) -> bool {
    a.width() == b.width()
        && a.height() == b.height()
        && (0..a.height()).all(|y| (0..a.width()).all(|x| a.pixel(x, y) == b.pixel(x, y)))
}

/// The first text line of page 0: its text and its displayed rectangle.
fn first_line(document: &Document) -> (String, ViewRect) {
    let (text, view) = document.page_text_view(0).expect("text");
    let line = text.blocks.first().and_then(|block| block.lines.first()).expect("a line");
    (line.text.split_whitespace().collect::<Vec<_>>().join(" "), view.rect(line.bounds))
}

fn highlight(document: &Document, rect: ViewRect) -> AnnotationEdit {
    let view = document.view_transform(0).expect("view");
    AnnotationEdit::AddMarkup {
        page: 0,
        markup: Markup {
            kind: MarkupKind::Highlight,
            quads: vec![Quad::from_view(rect, &view)],
            color: MarkupKind::Highlight.default_color(),
        },
    }
}

fn note(document: &Document, text: &str) -> AnnotationEdit {
    let view = document.view_transform(0).expect("view");
    AnnotationEdit::AddNote {
        page: 0,
        note: Note {
            rect: Note::rect_at(8.0, 8.0, &view),
            contents: text.into(),
            color: Note::DEFAULT_COLOR,
        },
    }
}

fn by(document: &Document, author: &str, edit: AnnotationEdit) -> Document {
    document.with_annotation_edits_by(&[edit], Some(author)).unwrap_or_else(|e| panic!("{e}")).0
}

fn now() -> i64 {
    i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).expect("after 1970").as_secs()).expect("fits")
}

fn replies(comment: &Comment) -> Vec<(Option<&str>, &str)> {
    comment.replies.iter().map(|reply| (reply.author.as_deref(), reply.contents.as_str())).collect()
}

#[test]
fn a_review_thread_is_listed_replied_to_and_resolved() {
    let (bytes, _) = fixture("chrome/en-paragraphs");
    let original = engine().open_bytes(bytes, None).expect("opens");
    assert!(original.comments().expect("comments").is_empty());
    let (line, rect) = first_line(&original);

    let document = by(&original, "Reviewer", highlight(&original, rect));
    let document = by(&document, "مراجِع", note(&document, "راجع البند ٤"));
    let comments = document.comments().expect("comments");
    assert_eq!(
        comments.iter().map(|c| c.kind).collect::<Vec<_>>(),
        [AnnotationKind::Highlight, AnnotationKind::Note]
    );
    let [marked, sticky] = &comments[..] else { unreachable!() };
    assert_eq!(marked.author.as_deref(), Some("Reviewer"));
    assert_eq!(marked.marked_text.as_deref(), Some(line.as_str()), "the highlighted line, as read");
    assert_eq!(marked.contents, None);
    assert!(marked.modified.is_some_and(|at| (at - now()).abs() < 300), "{:?}", marked.modified);
    assert_eq!(sticky.author.as_deref(), Some("مراجِع"));
    assert_eq!(sticky.contents.as_deref(), Some("راجع البند ٤"));
    assert_eq!(sticky.marked_text, None);
    assert!(!marked.resolved && !sticky.resolved);

    // Two replies: listed in order, drawn nowhere.
    let before = render(&document);
    let reply = |document: &Document, author: &str, text: &str| {
        by(document, author, AnnotationEdit::Reply { page: 0, index: marked.index, contents: text.into() })
    };
    let document = reply(&document, "Author", "Agreed — will fix");
    let document = reply(&document, "مراجِع", "شكرًا");
    let comments = document.comments().expect("comments");
    assert_eq!(comments.len(), 2, "replies are no comments of their own");
    assert_eq!(replies(&comments[0]), [(Some("Author"), "Agreed — will fix"), (Some("مراجِع"), "شكرًا")]);
    assert!(same_pixels(&before, &render(&document)), "a reply draws nothing");
    assert_eq!(document.page_annotations(0).expect("annotations").0.len(), 2, "nor is it on the page");

    // Resolved, then open again: the latest state counts, and states draw nothing.
    let resolve = |document: &Document, resolved: bool| {
        by(document, "Reviewer", AnnotationEdit::SetResolved { page: 0, index: sticky.index, resolved })
    };
    let resolved = resolve(&document, true);
    assert!(resolved.comments().expect("comments")[1].resolved);
    assert!(same_pixels(&before, &render(&resolved)));
    let reopened = resolve(&resolved, false);
    assert!(!reopened.comments().expect("comments")[1].resolved);
    assert_eq!(reopened.page_annotations(0).expect("annotations").0.len(), 2);
    qpdf_check(&reopened.bytes(), None, "thread.pdf");

    // A reply answers a comment, not another reply, and needs text.
    let comments = reopened.comments().expect("comments");
    let first_reply = comments[0].replies[0].index;
    for (edit, what) in [
        (AnnotationEdit::Reply { page: 0, index: first_reply, contents: "x".into() }, "a reply to a reply"),
        (AnnotationEdit::Reply { page: 0, index: marked.index, contents: "  ".into() }, "an empty reply"),
        (AnnotationEdit::SetResolved { page: 0, index: 99, resolved: true }, "no such comment"),
    ] {
        assert!(matches!(reopened.with_annotation_edit(&edit), Err(Error::InvalidEdit(_))), "{what}");
    }

    // Removing the highlight takes its replies along; the note keeps its states.
    let removed = reopened
        .with_annotation_edit(&AnnotationEdit::Remove { page: 0, index: marked.index })
        .expect("removes")
        .0;
    let comments = removed.comments().expect("comments");
    assert_eq!(comments.iter().map(|c| c.kind).collect::<Vec<_>>(), [AnnotationKind::Note]);
    assert!(!comments[0].resolved);
    qpdf_check(&removed.bytes(), None, "thread-removed.pdf");
}

#[test]
fn arabic_markup_reads_back_the_text_it_marks() {
    let (bytes, _) = fixture("chrome/ar-plain");
    let document = engine().open_bytes(bytes, None).expect("opens");
    let (line, rect) = first_line(&document);
    let edited = document.with_annotation_edit(&highlight(&document, rect)).expect("highlights").0;
    let comments = edited.comments().expect("comments");
    assert_eq!(comments[0].marked_text.as_deref(), Some(line.as_str()));
    assert_eq!(comments[0].author, None, "no name given, none written");
}

#[test]
fn an_encrypted_files_comments_are_written_and_read_back() {
    let (bytes, password) = fixture("qpdf/encrypted");
    let document = engine().open_bytes(bytes, password.as_deref()).expect("opens");
    let (_, rect) = first_line(&document);
    let document = by(&document, "مراجِع", highlight(&document, rect));
    let index = document.comments().expect("comments")[0].index;
    let document = by(&document, "Author", AnnotationEdit::Reply { page: 0, index, contents: "تم".into() });
    let document = by(&document, "مراجِع", AnnotationEdit::SetResolved { page: 0, index, resolved: true });
    let comments = document.comments().expect("comments");
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].author.as_deref(), Some("مراجِع"), "the author decrypts");
    assert_eq!(replies(&comments[0]), [(Some("Author"), "تم")]);
    assert!(comments[0].resolved);
    qpdf_check(&document.bytes(), password.as_deref(), "encrypted-thread.pdf");
}

/// A one-page file with a thread as other viewers write one: a highlight by
/// Alice; Bob's reply and Alice's reply to it (dated out of order); two
/// review states, the newer `/Completed` listed first; a `/Marked` state; a
/// group member; and a note of its own.
fn threaded_pdf() -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Annots [4 0 R 5 0 R 6 0 R 7 0 R 8 0 R 9 0 R 10 0 R 11 0 R] >>",
        "<< /Type /Annot /Subtype /Highlight /Rect [10 10 90 22] /QuadPoints [10 22 90 22 10 10 90 10] /C [1 1 0] /T (Alice) /M (D:20261101090000Z) >>",
        "<< /Type /Annot /Subtype /Text /Rect [10 10 90 22] /IRT 6 0 R /T (Alice) /Contents (second) /M (D:20261101110000Z) >>",
        "<< /Type /Annot /Subtype /Text /Rect [10 10 90 22] /IRT 4 0 R /RT /R /T (Bob) /Contents (first) /M (D:20261101100000+01'00') >>",
        "<< /Type /Annot /Subtype /Text /Rect [10 10 90 22] /IRT 4 0 R /StateModel /Review /State /Completed /T (Bob) /M (D:20261102090000Z) /F 30 >>",
        "<< /Type /Annot /Subtype /Text /Rect [10 10 90 22] /IRT 4 0 R /StateModel /Review /State /Rejected /T (Alice) /M (D:20261101120000Z) /F 30 >>",
        "<< /Type /Annot /Subtype /Text /Rect [10 10 90 22] /IRT 4 0 R /StateModel /Marked /State /Unmarked /F 30 >>",
        "<< /Type /Annot /Subtype /Text /Rect [10 10 90 22] /IRT 4 0 R /RT /Group /Contents (grouped) >>",
        "<< /Type /Annot /Subtype /Text /Rect [150 150 170 170] /Contents (On its own) >>",
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
fn threads_written_by_other_viewers_are_followed() {
    let document = engine().open_bytes(threaded_pdf(), None).expect("opens");
    let comments = document.comments().expect("comments");
    assert_eq!(
        comments.iter().map(|c| (c.index, c.kind)).collect::<Vec<_>>(),
        [(0, AnnotationKind::Highlight), (7, AnnotationKind::Note)]
    );
    let thread = &comments[0];
    assert_eq!(thread.author.as_deref(), Some("Alice"));
    // Bob's reply was at 09:00 UTC (10:00 at +01:00), before Alice's answer to it.
    assert_eq!(replies(thread), [(Some("Bob"), "first"), (Some("Alice"), "second")]);
    assert_eq!(thread.replies[0].modified, Some(1_793_523_600));
    // The newer review state (Completed) counts, wherever it is listed.
    assert!(thread.resolved);
    assert_eq!(comments[1].contents.as_deref(), Some("On its own"));
    // Only the highlight and the note are on the page.
    let on_page: Vec<usize> =
        document.page_annotations(0).expect("annotations").0.iter().map(|a| a.index).collect();
    assert_eq!(on_page, [0, 7]);

    // Removing the highlight takes its whole thread, replies to replies too.
    let removed =
        document.with_annotation_edit(&AnnotationEdit::Remove { page: 0, index: 0 }).expect("removes").0;
    let comments = removed.comments().expect("comments");
    assert_eq!(comments.iter().map(|c| c.contents.as_deref()).collect::<Vec<_>>(), [Some("On its own")]);
    assert_eq!(removed.page_annotations(0).expect("annotations").0.len(), 1);
    qpdf_check(&removed.bytes(), None, "foreign-thread-removed.pdf");
}
