//! Measures page objects (ADR 0051) on PDFs: how many objects of each kind
//! the first pages have, how long tracing a page takes, and what moving and
//! reordering the first text block costs (time, update size).
//!
//! `cargo run --release -p safha-core --example objects_report -- a.pdf [b.pdf …]`
//! (release builds need `SAFHA_PDFIUM_DIR`).

use std::collections::BTreeMap;
use std::time::Instant;

use safha_core::objects::{Arrange, ObjectChange, ObjectEdit, ObjectKind, ObjectRef};
use safha_core::{Engine, PdfiumSource};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = Engine::global(&PdfiumSource::Auto)?;
    println!("file\tpage\tobjects\tkinds\ttrace ms\tmove ms\tmove bytes\tfront ms\tlinks");
    for path in std::env::args().skip(1) {
        let document = engine.open_bytes(std::fs::read(&path)?, None)?;
        for page in 0..document.page_count().min(3) {
            let started = Instant::now();
            let objects = match document.page_objects(page) {
                Ok(objects) => objects,
                Err(error) => {
                    println!("{path}\t{page}\t{error}");
                    continue;
                }
            };
            let traced = started.elapsed().as_secs_f64() * 1000.0;
            let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
            for object in &objects {
                *kinds.entry(format!("{:?}", object.kind)).or_default() += 1;
            }
            let text = objects.iter().find(|o| o.kind == ObjectKind::Text);
            let (mut moved_ms, mut moved_bytes, mut front_ms) = (f64::NAN, 0, f64::NAN);
            if let Some(text) = text {
                let at = ObjectRef { index: text.index, fingerprint: text.fingerprint };
                let started = Instant::now();
                let change = ObjectChange {
                    page,
                    object: at,
                    edit: ObjectEdit::Transform { matrix: [1.0, 0.0, 0.0, 1.0, 5.0, 5.0] },
                };
                if let Ok((_, update, _)) = document.with_object_edit(&change) {
                    moved_ms = started.elapsed().as_secs_f64() * 1000.0;
                    moved_bytes = update.len();
                }
                let started = Instant::now();
                let change = ObjectChange { page, object: at, edit: ObjectEdit::Arrange(Arrange::Front) };
                if document.with_object_edit(&change).is_ok() {
                    front_ms = started.elapsed().as_secs_f64() * 1000.0;
                }
            }
            let links = document.page_links(page).map(|l| l.len()).unwrap_or(0);
            println!(
                "{path}\t{page}\t{}\t{kinds:?}\t{traced:.1}\t{moved_ms:.1}\t{moved_bytes}\t{front_ms:.1}\t{links}",
                objects.len()
            );
        }
    }
    Ok(())
}
