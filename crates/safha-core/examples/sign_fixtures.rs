//! Signs fixtures with a throwaway PKI and writes everything an external
//! validator needs (ADR 0047): `scripts/check-signatures.sh` runs Poppler's
//! `pdfsig`, OpenSSL and qpdf over the output.
//!
//! ```text
//! cargo run -p safha-core --example sign_fixtures -- <out-dir>
//! ```
//!
//! Writes `ca.pem` (the root to trust), `tsa.pem`, and signed PDFs:
//! `invisible.pdf` (B-B, P-256), `rsa.pdf` (RSA, cross-reference streams),
//! `visible.pdf` (Arabic signer, visible on a turned page), `timestamped.pdf`
//! (B-T), `encrypted.pdf` (AES, password in `encrypted.password`), and
//! `two-signatures.pdf`; each with `<name>.expect` naming its signers.

#[path = "../tests/common/mod.rs"]
mod common;
#[path = "../tests/common/pki.rs"]
mod pki;

use std::path::Path;

use safha_core::annotate::{Signature, TypedSignature};
use safha_core::fonts::FontFamily;
use safha_core::sign::{SignOptions, VisibleSignature, sign_bytes, verify};
use safha_core::{Engine, PdfiumSource};

fn fixture(name: &str) -> (Vec<u8>, Option<String>) {
    let manifest = common::manifest();
    let fixture =
        manifest.fixtures.iter().find(|f| f.name() == name).expect("fixture: run node fixtures/generate.mjs");
    (std::fs::read(fixture.file()).expect("readable"), fixture.password.clone())
}

fn main() {
    let out = std::env::args().nth(1).expect("usage: sign_fixtures <out-dir>");
    let out = Path::new(&out);
    std::fs::create_dir_all(out).expect("out dir");
    let pki = pki::Pki::new();
    std::fs::write(out.join("ca.pem"), pki.ca.pem()).expect("write");
    std::fs::write(out.join("tsa.pem"), pki.tsa.pem()).expect("write");
    let engine = Engine::global(&PdfiumSource::Auto).expect("PDFium");
    let options = SignOptions {
        reason: Some("Approved — موافق".into()),
        location: Some("Muscat — مسقط".into()),
        contact: None,
        visible: None,
    };
    let write = |name: &str, bytes: &[u8], password: Option<&str>| {
        let reports = verify(bytes, password, &pki.trust());
        for report in &reports {
            println!(
                "{name}: {:?} {:?} {:?} signer={:?} timestamp={:?}",
                report.status,
                report.integrity,
                report.trust,
                report.signer.as_ref().map(|s| &s.name),
                report.timestamp
            );
        }
        std::fs::write(out.join(name), bytes).expect("write");
        let signers: Vec<String> =
            reports.iter().filter_map(|r| r.signer.as_ref().map(|s| s.name.clone())).collect();
        std::fs::write(out.join(format!("{name}.expect")), signers.join("\n")).expect("write");
    };

    let (plain, _) = fixture("chrome/en-paragraphs");
    let signed = sign_bytes(&plain, None, &pki.identity(&pki.signer), &options, None).expect("signs");
    write("invisible.pdf", &signed, None);

    let (objstm, _) = fixture("qpdf/rtl-book-objstm");
    let signed = sign_bytes(&objstm, None, &pki.identity(&pki.rsa), &options, None).expect("signs");
    write("rsa.pdf", &signed, None);

    let (rotated, _) = fixture("qpdf/rotated");
    let document = engine.open_bytes(rotated.clone(), None).expect("opens");
    let view = document.view_transform(1).expect("view");
    let graphic =
        Signature::Typed(TypedSignature { text: "سارة Sarah".into(), family: FontFamily::Naskh });
    let visible = VisibleSignature::at(
        1,
        (72.0, 72.0),
        (200.0, 80.0),
        &view,
        Some(graphic),
        "وقّعه الموقّع التجريبي\n٢٠٢٦/١١/٢٣".into(),
        Signature::DEFAULT_COLOR,
    )
    .expect("fits");
    let signed = sign_bytes(
        &rotated,
        None,
        &pki.identity(&pki.arabic),
        &SignOptions { visible: Some(visible), ..options.clone() },
        None,
    )
    .expect("signs");
    write("visible.pdf", &signed, None);

    let (arabic, _) = fixture("chrome/ar-plain");
    let signed = sign_bytes(&arabic, None, &pki.identity(&pki.signer), &options, Some(&pki.authority()))
        .expect("signs");
    write("timestamped.pdf", &signed, None);

    let (encrypted, password) = fixture("qpdf/encrypted");
    let password = password.expect("password");
    let signed =
        sign_bytes(&encrypted, Some(&password), &pki.identity(&pki.signer), &options, None).expect("signs");
    std::fs::write(out.join("encrypted.password"), &password).expect("write");
    write("encrypted.pdf", &signed, Some(&password));

    let (contract, _) = fixture("chrome/contract-two-column");
    let first = sign_bytes(&contract, None, &pki.identity(&pki.signer), &options, None).expect("signs");
    let second = sign_bytes(&first, None, &pki.identity(&pki.arabic), &options, None).expect("signs");
    write("two-signatures.pdf", &second, None);
}
