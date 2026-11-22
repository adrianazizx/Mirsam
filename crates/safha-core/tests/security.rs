//! Encryption and permissions on save (ADR 0043): files written with a
//! password are checked by qpdf, opened again, and read back.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{comparable, manifest};
use safha_core::save::save_secured;
use safha_core::security::{Cipher, CipherName, Permissions, Protection, SecurityChange};
use safha_core::{Engine, Error, PdfiumSource};

fn engine() -> &'static Engine {
    Engine::global(&PdfiumSource::Auto).expect("PDFium is available (run scripts/fetch-pdfium.sh)")
}

fn fixture(name: &str) -> (PathBuf, Option<String>) {
    let manifest = manifest();
    let fixture = manifest
        .fixtures
        .iter()
        .find(|f| f.name() == name)
        .unwrap_or_else(|| panic!("fixture {name} missing: run `node fixtures/generate.mjs`"));
    (fixture.file(), fixture.password.clone())
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("safha-security-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

fn qpdf(args: &[&str]) -> std::process::Output {
    Command::new("qpdf").args(args).output().expect("qpdf runs (install qpdf)")
}

/// `qpdf --check` with `password`; returns what it printed.
fn qpdf_check(path: &Path, password: Option<&str>) -> String {
    let password = password.map(|p| format!("--password={p}"));
    let mut args: Vec<&str> = password.iter().map(String::as_str).collect();
    let path = path.to_string_lossy();
    args.extend(["--check", &path]);
    let output = qpdf(&args);
    let text =
        format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(matches!(output.status.code(), Some(0 | 3)), "qpdf --check failed:\n{text}");
    text
}

fn qpdf_encryption(path: &Path, password: Option<&str>) -> String {
    let password = password.map(|p| format!("--password={p}"));
    let mut args: Vec<&str> = password.iter().map(String::as_str).collect();
    let path = path.to_string_lossy();
    args.extend(["--show-encryption", &path]);
    let output = qpdf(&args);
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

fn protection(cipher: Cipher, user: &str, owner: &str, permissions: Permissions) -> SecurityChange {
    SecurityChange::Set(Protection {
        cipher,
        user_password: user.to_owned(),
        owner_password: owner.to_owned(),
        permissions,
    })
}

fn text_of(document: &safha_core::Document) -> String {
    comparable(
        &(0..document.page_count()).map(|i| document.page_text_raw(i).expect("text")).collect::<String>(),
    )
}

#[test]
fn aes_256_protects_a_file_that_qpdf_and_we_open_with_either_password() {
    let (source, _) = fixture("chrome/en-paragraphs");
    let document = engine().open_file(&source, None).expect("opens");
    let permissions = Permissions { copy: false, modify: false, ..Permissions::all() };
    let out = scratch("aes256.pdf");
    save_secured(&document, &out, &protection(Cipher::Aes256, "user-pw", "owner-pw", permissions))
        .expect("saves");

    // qpdf: the file is sound with each password, and says what it is.
    qpdf_check(&out, Some("user-pw"));
    qpdf_check(&out, Some("owner-pw"));
    let shown = qpdf_encryption(&out, Some("owner-pw"));
    assert!(shown.contains("R = 6"), "{shown}");
    assert!(shown.contains("AESv3"), "{shown}");
    assert!(shown.contains("extract for any purpose: not allowed"), "{shown}");
    assert!(
        shown.contains("modify other: not allowed") || shown.contains("modify document assembly"),
        "{shown}"
    );
    // Without a password it does not open.
    assert!(!qpdf(&["--check", &out.to_string_lossy()]).status.success());

    // We read it back: same text, the permissions as set, owner vs user.
    assert!(matches!(engine().open_file(&out, None), Err(Error::PasswordRequired)));
    assert!(matches!(engine().open_file(&out, Some("wrong")), Err(Error::PasswordRequired)));
    let as_user = engine().open_file(&out, Some("user-pw")).expect("user password opens it");
    assert_eq!(text_of(&as_user), text_of(&document));
    let info = as_user.security();
    assert!(info.encrypted && !info.owner);
    assert_eq!(info.cipher, Some(CipherName::Aes256));
    assert_eq!(info.revision, 6);
    assert_eq!(info.permissions, Permissions { print_high_quality: true, ..permissions });
    let as_owner = engine().open_file(&out, Some("owner-pw")).expect("owner password opens it");
    assert!(as_owner.security().owner);
}

#[test]
fn aes_128_is_written_for_old_readers() {
    let (source, _) = fixture("chrome/en-paragraphs");
    let document = engine().open_file(&source, None).expect("opens");
    let permissions = Permissions { print: false, ..Permissions::all() };
    let out = scratch("aes128.pdf");
    save_secured(&document, &out, &protection(Cipher::Aes128, "open", "own", permissions)).expect("saves");
    qpdf_check(&out, Some("open"));
    qpdf_check(&out, Some("own"));
    let shown = qpdf_encryption(&out, Some("own"));
    assert!(shown.contains("R = 4") && shown.contains("AESv2"), "{shown}");
    assert!(shown.contains("print low resolution: not allowed"), "{shown}");
    let reopened = engine().open_file(&out, Some("open")).expect("opens");
    assert_eq!(text_of(&reopened), text_of(&document));
    assert_eq!(reopened.security().cipher, Some(CipherName::Aes128));
    assert!(!reopened.security().permissions.print);
    // AES-128 cannot store an Arabic password.
    let err = save_secured(
        &document,
        &scratch("nope.pdf"),
        &protection(Cipher::Aes128, "كلمة", "", Permissions::all()),
    )
    .expect_err("refused");
    assert!(matches!(err, Error::PasswordNotEncodable));
}

#[test]
fn an_open_file_with_only_an_owner_password_needs_none_to_read() {
    let (source, _) = fixture("chrome/en-paragraphs");
    let document = engine().open_file(&source, None).expect("opens");
    let out = scratch("owner-only.pdf");
    let permissions = Permissions { modify: false, ..Permissions::all() };
    save_secured(&document, &out, &protection(Cipher::Aes256, "", "boss", permissions)).expect("saves");
    qpdf_check(&out, None);
    let reader = engine().open_file(&out, None).expect("opens with no password");
    assert!(!reader.security().owner);
    assert!(!reader.security().permissions.modify);
    assert_eq!(text_of(&reader), text_of(&document));
    // A reader that is not the owner may not lift the permissions.
    let err = save_secured(&reader, &scratch("lifted.pdf"), &SecurityChange::Remove).expect_err("refused");
    assert!(matches!(err, Error::NotPermitted));
    // The owner may.
    let owner = engine().open_file(&out, Some("boss")).expect("owner opens");
    let lifted = scratch("lifted-by-owner.pdf");
    save_secured(&owner, &lifted, &SecurityChange::Remove).expect("owner may remove it");
    qpdf_check(&lifted, None);
    assert!(!engine().open_file(&lifted, None).expect("opens").security().encrypted);
}

#[test]
fn an_arabic_password_opens_in_any_unicode_form() {
    let (source, _) = fixture("chrome/ar-plain");
    let document = engine().open_file(&source, None).expect("opens");
    // "كلمة السر" (the password), with Arabic-Indic digits.
    let password = "كلمة السر ١٢٣";
    let out = scratch("arabic.pdf");
    save_secured(&document, &out, &protection(Cipher::Aes256, password, "المالك", Permissions::all()))
        .expect("saves");
    qpdf_check(&out, Some(password));
    qpdf_check(&out, Some("المالك"));
    let reopened = engine().open_file(&out, Some(password)).expect("the password as typed opens it");
    assert_eq!(text_of(&reopened), text_of(&document));
    assert!(engine().open_file(&out, Some("المالك")).expect("owner").security().owner);

    // The same password typed with presentation forms (what some keyboards and
    // pasted text give): NFKC turns "ﻛﻠﻤﺔ" into "كلمة", so it opens too.
    let presentation = "ﻛﻠﻤﺔ ﺍﻟﺴﺮ ١٢٣";
    assert_ne!(presentation, password);
    let alternate = engine().open_file(&out, Some(presentation)).expect("normalised form opens it");
    assert_eq!(text_of(&alternate), text_of(&document));
    // A different password does not.
    assert!(matches!(engine().open_file(&out, Some("كلمة")), Err(Error::PasswordRequired)));

    // A password typed decomposed is stored composed: ö as o + U+0308.
    let out = scratch("accent.pdf");
    save_secured(&document, &out, &protection(Cipher::Aes256, "scho\u{308}n", "", Permissions::all()))
        .expect("saves");
    assert!(engine().open_file(&out, Some("sch\u{f6}n")).is_ok());
    assert!(engine().open_file(&out, Some("scho\u{308}n")).is_ok());
}

#[test]
fn an_encrypted_file_is_changed_or_unlocked_on_save() {
    let (source, password) = fixture("qpdf/encrypted");
    let document = engine().open_file(&source, password.as_deref()).expect("opens");
    assert!(document.security().encrypted);

    // Remove: no password afterwards, the same pages and text.
    let plain = scratch("unlocked.pdf");
    let reopened = save_secured(&document, &plain, &SecurityChange::Remove).expect("saves");
    assert!(!reopened.security().encrypted);
    qpdf_check(&plain, None);
    let again = engine().open_file(&plain, None).expect("opens with no password");
    assert_eq!(text_of(&again), text_of(&document));
    assert!(!qpdf_encryption(&plain, None).contains("R = "));

    // Change: a new password replaces the old one.
    let changed = scratch("changed.pdf");
    save_secured(&document, &changed, &protection(Cipher::Aes256, "fresh", "", Permissions::all()))
        .expect("saves");
    qpdf_check(&changed, Some("fresh"));
    assert!(matches!(engine().open_file(&changed, password.as_deref()), Err(Error::PasswordRequired)));
    assert_eq!(text_of(&engine().open_file(&changed, Some("fresh")).expect("opens")), text_of(&document));

    // Keep: stays protected as it was.
    let kept = scratch("kept.pdf");
    save_secured(&document, &kept, &SecurityChange::Keep).expect("saves");
    assert!(engine().open_file(&kept, password.as_deref()).expect("opens").security().encrypted);
}

#[test]
fn a_file_with_object_streams_survives_a_change_of_protection() {
    let (source, _) = fixture("qpdf/rtl-book-objstm");
    let document = engine().open_file(&source, None).expect("opens");
    let out = scratch("objstm-protected.pdf");
    save_secured(&document, &out, &protection(Cipher::Aes256, "p", "", Permissions::all())).expect("saves");
    qpdf_check(&out, Some("p"));
    let reopened = engine().open_file(&out, Some("p")).expect("opens");
    assert_eq!(reopened.page_count(), 4);
    assert_eq!(text_of(&reopened), text_of(&document));
    // And back to plain.
    let plain = scratch("objstm-plain.pdf");
    save_secured(&reopened, &plain, &SecurityChange::Remove).expect("saves");
    qpdf_check(&plain, None);
    assert_eq!(text_of(&engine().open_file(&plain, None).expect("opens")), text_of(&document));
}

#[test]
fn pages_render_the_same_after_encryption() {
    let (source, _) = fixture("chrome/personal-data");
    let document = engine().open_file(&source, None).expect("opens");
    let out = scratch("render.pdf");
    let secured = save_secured(&document, &out, &protection(Cipher::Aes256, "p", "o", Permissions::all()))
        .expect("saves");
    let (a, b) =
        (document.render_page(0, 1.0).expect("renders"), secured.render_page(0, 1.0).expect("renders"));
    assert_eq!((a.width(), a.height()), (b.width(), b.height()));
    assert_eq!(a.as_bgra(), b.as_bgra(), "the page paints identically");
}
