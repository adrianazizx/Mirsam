//! Links (ADR 0051) on real files: added over a drawn rectangle or a text
//! selection's runs, to a web address, a page, or a named destination; read
//! back with where they go; pointed elsewhere, moved, and removed. Turned
//! pages, an encrypted file, `qpdf --check`, undo and save.

mod common;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use common::manifest;
use mirsam_core::edit::EditHistory;
use mirsam_core::links::{LinkEdit, LinkTarget};
use mirsam_core::save::save;
use mirsam_core::view::ViewRect;
use mirsam_core::{Document, Engine, PdfiumSource};

fn engine() -> &'static Engine {
    Engine::global(&PdfiumSource::Auto).expect("PDFium is available (run scripts/fetch-pdfium.sh)")
}

fn fixture(name: &str) -> (Vec<u8>, Option<String>) {
    let manifest = manifest();
    let fixture = manifest
        .fixture_or_unavailable(name)
        .unwrap_or_else(|| panic!("fixture {name} missing: run `node fixtures/generate.mjs`"));
    (std::fs::read(fixture.file()).expect("fixture readable"), fixture.password.clone())
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mirsam-links-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

fn qpdf(bytes: &[u8], password: Option<&str>, name: &str, args: &[&str]) -> String {
    let path = scratch(name);
    std::fs::write(&path, bytes).expect("write");
    let mut command = Command::new("qpdf");
    if let Some(password) = password {
        command.arg(format!("--password={password}"));
    }
    let output = command.args(args).arg(&path).output().expect("qpdf runs (install qpdf)");
    assert!(
        matches!(output.status.code(), Some(0 | 3)),
        "qpdf failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn close(a: &ViewRect, b: &ViewRect) -> bool {
    (a.x - b.x).abs() < 0.05
        && (a.y - b.y).abs() < 0.05
        && (a.width - b.width).abs() < 0.05
        && (a.height - b.height).abs() < 0.05
}

fn add(document: &Document, page: usize, rects: Vec<ViewRect>, target: LinkTarget) -> (Document, Vec<u8>) {
    document.with_link_edit(&LinkEdit::Add { page, rects, target }).expect("adds")
}

/// A two-page PDF with a named destination "chapter-2" (name tree) and
/// "intro" (the catalog's old `/Dests`), both on page 2.
fn named_pdf() -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /Names << /Dests 6 0 R >> /Dests << /intro [4 0 R /Fit] >> >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 300 300] >>",
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>",
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>",
        "<< /Length 0 >>\nstream\n\nendstream",
        "<< /Names [(chapter-2) [4 0 R /XYZ 0 300 null]] >>",
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1)
            .as_bytes(),
    );
    out
}

#[test]
fn a_drawn_link_to_a_web_address_reads_back_where_it_was_drawn() {
    let (bytes, _) = fixture("chrome/en-paragraphs");
    let document = engine().open_bytes(bytes, None).expect("opens");
    let before = document.page_links(0).expect("links").len();
    let rect = ViewRect { x: 72.0, y: 100.0, width: 120.0, height: 18.0 };
    let (linked, _) = add(&document, 0, vec![rect], LinkTarget::Url { url: "example.com/عربي".into() });
    let links = linked.page_links(0).expect("links");
    assert_eq!(links.len(), before + 1);
    let link = links.last().expect("the new link");
    assert!(close(&link.rects[0], &rect), "{:?}", link.rects);
    assert_eq!(link.target, LinkTarget::Url { url: "https://example.com/%D8%B9%D8%B1%D8%A8%D9%8A".into() });
    qpdf(&linked.bytes(), None, "web.pdf", &["--check"]);
    // The page's text is untouched: a link is an annotation, not content.
    assert_eq!(linked.page_text(0).expect("text").text(), document.page_text(0).expect("text").text());
}

#[test]
fn unsafe_addresses_are_refused() {
    let document = engine().open_bytes(named_pdf(), None).expect("opens");
    let rect = ViewRect { x: 10.0, y: 10.0, width: 50.0, height: 20.0 };
    for url in ["javascript:alert(1)", "file:///etc/passwd", "no spaces allowed.com", ""] {
        let edit = LinkEdit::Add { page: 0, rects: vec![rect], target: LinkTarget::Url { url: url.into() } };
        assert!(document.with_link_edit(&edit).is_err(), "{url}");
    }
}

#[test]
fn links_to_pages_and_named_destinations() {
    let document = engine().open_bytes(named_pdf(), None).expect("opens");
    let names = document.named_destinations().expect("names");
    let listed: Vec<(&str, Option<usize>)> = names.iter().map(|n| (n.name.as_str(), n.page)).collect();
    assert_eq!(listed, [("chapter-2", Some(1)), ("intro", Some(1))]);

    let rect = ViewRect { x: 20.0, y: 20.0, width: 100.0, height: 20.0 };
    let (one, _) = add(&document, 0, vec![rect], LinkTarget::Page { page: 1 });
    let (two, _) = add(&one, 0, vec![rect], LinkTarget::Named { name: "chapter-2".into(), page: None });
    let (three, _) = add(&two, 0, vec![rect], LinkTarget::Named { name: "intro".into(), page: None });
    let targets: Vec<LinkTarget> =
        three.page_links(0).expect("links").into_iter().map(|l| l.target).collect();
    assert_eq!(
        targets,
        [
            LinkTarget::Page { page: 1 },
            LinkTarget::Named { name: "chapter-2".into(), page: Some(1) },
            LinkTarget::Named { name: "intro".into(), page: Some(1) },
        ]
    );
    let unknown = LinkEdit::Add {
        page: 0,
        rects: vec![rect],
        target: LinkTarget::Named { name: "nowhere".into(), page: None },
    };
    assert!(three.with_link_edit(&unknown).is_err());
    qpdf(&three.bytes(), None, "named.pdf", &["--check"]);
}

#[test]
fn a_link_is_pointed_elsewhere_moved_and_removed_and_undone() {
    let bytes = named_pdf();
    let document = engine().open_bytes(bytes.clone(), None).expect("opens");
    let rect = ViewRect { x: 20.0, y: 20.0, width: 100.0, height: 20.0 };
    let (linked, first) =
        add(&document, 0, vec![rect], LinkTarget::Url { url: "https://example.com".into() });
    let index = linked.page_links(0).expect("links")[0].index;
    let (retargeted, second) = linked
        .with_link_edit(&LinkEdit::SetTarget { page: 0, index, target: LinkTarget::Page { page: 1 } })
        .expect("sets");
    assert_eq!(retargeted.page_links(0).expect("links")[0].target, LinkTarget::Page { page: 1 });
    let moved_to = ViewRect { x: 50.0, y: 200.0, width: 60.0, height: 30.0 };
    let (moved, third) =
        retargeted.with_link_edit(&LinkEdit::SetRect { page: 0, index, rect: moved_to }).expect("moves");
    assert!(close(&moved.page_links(0).expect("links")[0].rects[0], &moved_to));
    let (removed, fourth) = moved.with_link_edit(&LinkEdit::Remove { page: 0, index }).expect("removes");
    assert!(removed.page_links(0).expect("links").is_empty());

    let mut history = EditHistory::new(Arc::from(bytes.clone()));
    for update in [first, second, third, fourth] {
        history.push_annotations(update);
    }
    for _ in 0..4 {
        history.undo().expect("undoes");
    }
    assert_eq!(&*history.current(), bytes.as_slice());
}

#[test]
fn a_selection_link_covers_each_run_on_a_turned_page() {
    let (bytes, _) = fixture("qpdf/rotated");
    let document = engine().open_bytes(bytes, None).expect("opens");
    for page in 0..4 {
        let runs = vec![
            ViewRect { x: 40.0, y: 50.0, width: 200.0, height: 14.0 },
            ViewRect { x: 40.0, y: 66.0, width: 120.0, height: 14.0 },
        ];
        let (linked, _) =
            add(&document, page, runs.clone(), LinkTarget::Url { url: "https://example.org".into() });
        let link = linked.page_links(page).expect("links").pop().expect("a link");
        assert_eq!(link.rects.len(), 2, "page {page}");
        for (got, want) in link.rects.iter().zip(&runs) {
            assert!(close(got, want), "page {page}: {got:?} vs {want:?}");
        }
        qpdf(&linked.bytes(), None, "turned.pdf", &["--check"]);
    }
}

#[test]
fn encrypted_files_get_encrypted_links_that_read_back() {
    let (bytes, password) = fixture("qpdf/encrypted");
    let document = engine().open_bytes(bytes, password.as_deref()).expect("opens");
    let rect = ViewRect { x: 60.0, y: 60.0, width: 100.0, height: 20.0 };
    let url = "https://example.com/secret-path";
    let (linked, _) = add(&document, 0, vec![rect], LinkTarget::Url { url: url.into() });
    assert!(!String::from_utf8_lossy(&linked.bytes()).contains("secret-path"), "the address is encrypted");
    let link = linked.page_links(0).expect("links").pop().expect("a link");
    assert_eq!(link.target, LinkTarget::Url { url: url.into() });
    let shown = qpdf(&linked.bytes(), password.as_deref(), "encrypted.pdf", &["--check"]);
    assert!(!shown.is_empty());
    let target = scratch("encrypted-saved.pdf");
    save(&linked, &target).expect("saves");
    let reopened = engine().open_file(&target, password.as_deref()).expect("reopens");
    assert_eq!(
        reopened.page_links(0).expect("links").pop().map(|l| l.target),
        Some(LinkTarget::Url { url: url.into() })
    );
}
