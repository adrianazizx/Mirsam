//! Certificate signatures (PAdES, ADR 0047): signing the fixture corpus as
//! incremental updates, invisible and visible, B-B and B-T, plain and
//! encrypted, classic and stream cross-references; and checking signatures:
//! intact, changed bytes, untrusted and expired signers, later revisions.
//! Every signed file passes `qpdf --check` and opens in PDFium as signed.
//! External validators (Poppler's `pdfsig`, OpenSSL) are run on the same
//! kind of output by `scripts/check-signatures.sh`.

mod common;
#[path = "common/pki.rs"]
mod pki;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use common::manifest;
use pki::{DownAuthority, PASSWORD, Pki};
use safha_core::annotate::{AnnotationEdit, Note, Signature, TypedSignature};
use safha_core::fonts::FontFamily;
use safha_core::sign::{
    Identity, Integrity, SignOptions, Status, Trust, TrustStore, VisibleSignature, sign_bytes, verify,
    verify_at,
};
use safha_core::{Document, Engine, Error, PdfiumSource};

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
    let dir = std::env::temp_dir().join(format!("safha-sign-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

fn qpdf_check(bytes: &[u8], password: Option<&str>, name: &str) {
    let path = scratch(name);
    std::fs::write(&path, bytes).expect("write");
    let password = format!("--password={}", password.unwrap_or(""));
    let output = Command::new("qpdf")
        .args([password.as_str(), "--check", &path.to_string_lossy()])
        .output()
        .expect("qpdf runs (install qpdf)");
    assert!(
        matches!(output.status.code(), Some(0 | 3)),
        "qpdf --check {name}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn open(bytes: &[u8], password: Option<&str>) -> Document {
    engine().open_bytes(bytes.to_vec(), password).expect("the signed file opens")
}

fn invisible() -> SignOptions {
    SignOptions {
        reason: Some("Approved — موافق".into()),
        location: Some("مسقط".into()),
        contact: None,
        visible: None,
    }
}

#[test]
fn an_unsigned_file_is_signed_invisibly_and_checks_out() {
    let pki = Pki::new();
    let (original, _) = fixture("chrome/en-paragraphs");
    let signed = sign_bytes(&original, None, &pki.identity(&pki.signer), &invisible(), None).expect("signs");

    // An incremental update: the original bytes are untouched.
    assert_eq!(&signed[..original.len()], original.as_slice());
    qpdf_check(&signed, None, "invisible.pdf");
    let document = open(&signed, None);
    assert!(document.is_signed());
    let before = open(&original, None);
    assert_eq!(document.page_text_raw(0).expect("text"), before.page_text_raw(0).expect("text"));

    let reports = verify(&signed, None, &pki.trust());
    assert_eq!(reports.len(), 1);
    let report = &reports[0];
    assert_eq!(report.status, Status::Valid, "{report:?}");
    assert_eq!((report.integrity, report.trust), (Integrity::Intact, Trust::Trusted));
    assert!(report.covers_whole_file && report.changes.is_none());
    assert_eq!(report.sub_filter.as_deref(), Some("ETSI.CAdES.detached"));
    assert_eq!(report.signer.as_ref().map(|s| s.name.as_str()), Some("Test Signer"));
    assert_eq!(report.name.as_deref(), Some("Test Signer"));
    assert_eq!(report.reason.as_deref(), Some("Approved — موافق"));
    assert_eq!(report.location.as_deref(), Some("مسقط"));
    assert_eq!(report.field.as_deref(), Some("Signature1"));
    assert_eq!((report.revision, report.revisions), (2, 2));
    assert!(report.page.is_some() && report.rect.is_none(), "invisible: {report:?}");
    let claimed = report.claimed_time.expect("/M");
    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).expect("time").as_secs() as i64;
    assert!((now - claimed).abs() < 120, "{claimed} vs {now}");
    assert!(report.timestamp.is_none());

    // Without the root in the trust store, the signer is unknown, not invalid.
    let untrusted = &verify(&signed, None, &TrustStore::default())[0];
    assert_eq!((untrusted.status, untrusted.trust), (Status::Unknown, Trust::Untrusted));
    assert!(untrusted.problems.contains(&"signer_untrusted"));
}

#[test]
fn a_changed_byte_in_the_signed_range_is_caught() {
    let pki = Pki::new();
    let (original, _) = fixture("chrome/en-paragraphs");
    let mut signed =
        sign_bytes(&original, None, &pki.identity(&pki.signer), &invisible(), None).expect("signs");
    // A byte of page content, well inside the original file.
    let at = original.len() / 2;
    signed[at] = if signed[at] == b'0' { b'1' } else { b'0' };
    let report = &verify(&signed, None, &pki.trust())[0];
    assert_eq!((report.status, report.integrity), (Status::Invalid, Integrity::Altered), "{report:?}");
    assert!(report.problems.contains(&"digest_mismatch"));
}

#[test]
fn rsa_keys_and_arabic_names_sign_on_every_cross_reference_form() {
    let pki = Pki::new();
    for (fixture_name, issued, expected) in [
        ("qpdf/rtl-book-objstm", &pki.rsa, "Test Signer (RSA)"),
        ("coretext/ar-plain", &pki.arabic, "الموقّع التجريبي"),
        ("qpdf/linearized", &pki.signer, "Test Signer"),
    ] {
        let (original, _) = fixture(fixture_name);
        let signed = sign_bytes(&original, None, &pki.identity(issued), &invisible(), None).expect("signs");
        qpdf_check(&signed, None, &format!("{}.pdf", fixture_name.replace('/', "-")));
        assert!(open(&signed, None).is_signed(), "{fixture_name}");
        let report = &verify(&signed, None, &pki.trust())[0];
        assert_eq!(report.status, Status::Valid, "{fixture_name}: {report:?}");
        assert_eq!(report.signer.as_ref().map(|s| s.name.as_str()), Some(expected));
        assert_eq!(report.name.as_deref(), Some(expected));
    }
}

#[test]
fn a_visible_signature_shows_the_kept_signature_and_an_arabic_caption() {
    let pki = Pki::new();
    let (original, _) = fixture("qpdf/rotated");
    let before = open(&original, None);
    for page in [0, 1] {
        let view = before.view_transform(page).expect("view");
        // The bottom corner, clear of the page's own text.
        let size = before.page_size(page).expect("size");
        let (x, y) = ((size.width - 252.0).floor(), (size.height - 120.0).floor());
        let graphic = Signature::Typed(TypedSignature { text: "سارة".into(), family: FontFamily::Naskh });
        let visible = VisibleSignature::at(
            page,
            (x, y),
            (180.0, 72.0),
            &view,
            Some(graphic),
            "وقّعه الموقّع التجريبي\n٢٠٢٦/١١/٢٣".into(),
            Signature::DEFAULT_COLOR,
        )
        .expect("fits");
        let options = SignOptions { visible: Some(visible), ..invisible() };
        let signed = sign_bytes(&original, None, &pki.identity(&pki.arabic), &options, None).expect("signs");
        qpdf_check(&signed, None, &format!("visible-{page}.pdf"));
        let document = open(&signed, None);
        let reports = document.signatures(&pki.trust());
        let report = &reports[0];
        assert_eq!(report.status, Status::Valid, "{report:?}");
        assert_eq!(report.page, Some(page));
        let box_ = report.view.expect("a box on the displayed page");
        assert!(
            (box_.x - x).abs() < 0.5 && (box_.y - y).abs() < 0.5 && (box_.width - 180.0).abs() < 0.5,
            "page {page}: {box_:?}"
        );
        // Ink inside the box, as displayed.
        let rendered = document.render_page(page, 1.0).expect("renders");
        let plain = before.render_page(page, 1.0).expect("renders");
        let mut inked = 0;
        for py in y as u32..y as u32 + 72 {
            for px in x as u32..x as u32 + 180 {
                if rendered.pixel(px, py) != plain.pixel(px, py) {
                    inked += 1;
                }
            }
        }
        assert!(inked > 200, "page {page}: {inked} pixels drawn in the box");
    }
}

#[test]
fn a_visible_caption_reads_back_in_logical_order() {
    let pki = Pki::new();
    let (original, _) = fixture("chrome/en-paragraphs");
    let before = open(&original, None);
    let size = before.page_size(0).expect("size");
    let view = before.view_transform(0).expect("view");
    let visible = VisibleSignature::at(
        0,
        (size.width - 252.0, size.height - 120.0),
        (180.0, 72.0),
        &view,
        None,
        "وقّعه الموقّع التجريبي".into(),
        Signature::DEFAULT_COLOR,
    )
    .expect("fits");
    let options = SignOptions { visible: Some(visible), ..invisible() };
    let signed = sign_bytes(&original, None, &pki.identity(&pki.arabic), &options, None).expect("signs");
    // PDFium does not extract annotations: flatten the widget into the page.
    let (input, output) = (scratch("caption-in.pdf"), scratch("caption-flat.pdf"));
    std::fs::write(&input, &signed).expect("write");
    let result = Command::new("qpdf")
        .args(["--flatten-annotations=all", &input.to_string_lossy(), &output.to_string_lossy()])
        .output()
        .expect("qpdf runs (install qpdf)");
    assert!(matches!(result.status.code(), Some(0 | 3)), "{}", String::from_utf8_lossy(&result.stderr));
    let flat = engine().open_file(&output, None).expect("opens");
    let text = flat.page_text(0).expect("text").text();
    assert!(text.contains("وقّعه الموقّع التجريبي"), "{text}");
}

#[test]
fn a_time_stamp_makes_it_b_t_and_an_unreachable_authority_fails_the_signing() {
    let pki = Pki::new();
    let (original, _) = fixture("chrome/ar-plain");
    let authority = pki.authority();
    let signed = sign_bytes(&original, None, &pki.identity(&pki.signer), &invisible(), Some(&authority))
        .expect("signs");
    let report = &verify(&signed, None, &pki.trust())[0];
    assert_eq!(report.status, Status::Valid, "{report:?}");
    let timestamp = report.timestamp.as_ref().expect("time-stamped");
    assert!(timestamp.intact && timestamp.trusted, "{timestamp:?}");
    assert_eq!(timestamp.authority.as_deref(), Some("Mirsam Test Time-Stamping"));

    // Ten years on the signer's certificate has expired, but the time-stamp
    // proves it was in force when the document was signed.
    let later = SystemTime::now() + Duration::from_secs(6 * 365 * 86_400);
    let report = &verify_at(&signed, None, &pki.trust(), later)[0];
    assert_eq!(report.trust, Trust::Trusted, "{report:?}");

    // Without a time-stamp, the same certificate is expired by then.
    let plain = sign_bytes(&original, None, &pki.identity(&pki.signer), &invisible(), None).expect("signs");
    let report = &verify_at(&plain, None, &pki.trust(), later)[0];
    assert_eq!((report.status, report.trust), (Status::Unknown, Trust::Expired), "{report:?}");

    let down = sign_bytes(&original, None, &pki.identity(&pki.signer), &invisible(), Some(&DownAuthority));
    assert!(matches!(down, Err(Error::Signing("timestamp_failed"))), "{down:?}");
}

#[test]
fn a_second_signature_and_later_edits_keep_the_first_valid_and_are_listed() {
    let pki = Pki::new();
    let (original, _) = fixture("chrome/contract-two-column");
    let first = sign_bytes(&original, None, &pki.identity(&pki.signer), &invisible(), None).expect("signs");
    let second = sign_bytes(&first, None, &pki.identity(&pki.arabic), &invisible(), None).expect("signs");
    qpdf_check(&second, None, "two-signatures.pdf");
    let reports = verify(&second, None, &pki.trust());
    assert_eq!(reports.len(), 2);
    assert!(reports.iter().all(|r| r.status == Status::Valid), "{reports:?}");
    assert_eq!(reports[1].field.as_deref(), Some("Signature2"));
    let changes = reports[0].changes.as_ref().expect("the first does not cover the second");
    assert_eq!((changes.revisions, changes.signatures, changes.annotations), (1, 1, 0), "{changes:?}");
    assert!(changes.pages.is_empty() && !changes.page_order && changes.form_fields == 0, "{changes:?}");
    assert!(reports[1].covers_whole_file);

    // A comment added after signing: still valid, and the change is listed.
    let document = open(&second, None);
    let view = document.view_transform(0).expect("view");
    let note = Note {
        rect: Note::rect_at(300.0, 300.0, &view),
        contents: "after".into(),
        color: Note::DEFAULT_COLOR,
    };
    let (commented, _) =
        document.with_annotation_edit(&AnnotationEdit::AddNote { page: 0, note }).expect("comments");
    let reports = commented.signatures(&pki.trust());
    assert!(reports.iter().all(|r| r.status == Status::Valid), "{reports:?}");
    let changes = reports[1].changes.as_ref().expect("changed after the second");
    assert_eq!((changes.revisions, changes.annotations), (1, 1), "{changes:?}");
    assert!(changes.pages.is_empty(), "a comment is not a page change: {changes:?}");
    let changes = reports[0].changes.as_ref().expect("changed after the first");
    assert_eq!((changes.revisions, changes.signatures, changes.annotations), (2, 1, 1), "{changes:?}");

    // A page turned after signing changes the page.
    let (turned, _) = document
        .with_page_edit(&safha_core::edit::PageEdit::Rotate { pages: vec![0], quarter_turns: 1 })
        .expect("turns");
    let changes = turned.signatures(&pki.trust())[1].changes.clone().expect("changed");
    assert_eq!(changes.pages, vec![0], "{changes:?}");
}

#[test]
fn an_encrypted_file_is_signed_with_its_strings_encrypted_and_contents_left_clear() {
    let pki = Pki::new();
    let (original, password) = fixture("qpdf/encrypted");
    let password = password.expect("the fixture has a password");
    let document = open(&original, Some(&password));
    let signed = document
        .sign(&pki.identity(&pki.signer), &invisible(), None, false)
        .expect("signs an encrypted file");
    assert_eq!(&signed[..original.len()], original.as_slice());
    qpdf_check(&signed, Some(&password), "encrypted.pdf");
    let reopened = open(&signed, Some(&password));
    assert!(reopened.is_signed());
    let report = &reopened.signatures(&pki.trust())[0];
    assert_eq!(report.status, Status::Valid, "{report:?}");
    assert_eq!(report.name.as_deref(), Some("Test Signer"));
    // The signer's name is in the update only encrypted.
    let update = &signed[original.len()..];
    assert!(!update.windows(11).any(|w| w == b"Test Signer"), "the name is written in the clear");
}

#[test]
fn an_edited_unsigned_document_is_rewritten_before_it_is_signed() {
    let pki = Pki::new();
    let (original, _) = fixture("chrome/en-paragraphs");
    let document = open(&original, None);
    let view = document.view_transform(0).expect("view");
    let note = Note {
        rect: Note::rect_at(100.0, 100.0, &view),
        contents: "draft".into(),
        color: Note::DEFAULT_COLOR,
    };
    let (edited, _) =
        document.with_annotation_edit(&AnnotationEdit::AddNote { page: 0, note }).expect("edits");
    let signed = edited.sign(&pki.identity(&pki.signer), &invisible(), None, true).expect("signs");
    // One revision before the signature: the rewrite, not the edit history.
    let report = &verify(&signed, None, &pki.trust())[0];
    assert_eq!((report.status, report.revision, report.revisions), (Status::Valid, 2, 2), "{report:?}");
    assert_eq!(open(&signed, None).page_annotations(0).expect("annotations").0.len(), 1);
}

#[test]
fn pkcs12_files_open_with_their_password_only() {
    let pki = Pki::new();
    for legacy in [false, true] {
        let file = pki.pkcs12(&pki.arabic, PASSWORD, legacy);
        let identity = Identity::from_pkcs12(&file, PASSWORD).expect("opens");
        let summary = identity.summary();
        assert_eq!(summary.name, "الموقّع التجريبي");
        assert_eq!(summary.organization.as_deref(), Some("مرسام"));
        assert_eq!(summary.issuer, "Mirsam Test Root CA");
        assert!(!summary.self_signed && summary.fingerprint.len() == 64);
        let wrong = Identity::from_pkcs12(&file, "wrong");
        assert!(matches!(wrong, Err(Error::Signing("certificate_password"))), "legacy={legacy}: {wrong:?}");
    }
}

#[test]
fn a_key_that_signs_elsewhere_signs_through_the_same_path() {
    use safha_core::sign::KeyOperation;
    struct Remote(p256::ecdsa::SigningKey);
    impl KeyOperation for Remote {
        fn sign(&self, message: &[u8]) -> Result<Vec<u8>, String> {
            use signature::{SignatureEncoding, Signer};
            let signature: p256::ecdsa::DerSignature = self.0.try_sign(message).map_err(|e| e.to_string())?;
            Ok(signature.to_vec())
        }
    }
    let pki = Pki::new();
    use pkcs8::DecodePrivateKey;
    let key = p256::ecdsa::SigningKey::from_pkcs8_der(&pki.signer.key).expect("key");
    let identity =
        Identity::external(&pki.signer.der(), &[pki.ca.der()], Arc::new(Remote(key))).expect("identity");
    let (original, _) = fixture("chrome/en-paragraphs");
    let signed = sign_bytes(&original, None, &identity, &invisible(), None).expect("signs");
    assert_eq!(verify(&signed, None, &pki.trust())[0].status, Status::Valid);

    // A key that refuses (the user said no to the keychain) fails the signing.
    struct Refusing;
    impl KeyOperation for Refusing {
        fn sign(&self, _: &[u8]) -> Result<Vec<u8>, String> {
            Err("user cancelled".into())
        }
    }
    let identity = Identity::external(&pki.signer.der(), &[], Arc::new(Refusing)).expect("identity");
    let refused = sign_bytes(&original, None, &identity, &invisible(), None);
    assert!(matches!(refused, Err(Error::Signing("signing_failed"))), "{refused:?}");
}
