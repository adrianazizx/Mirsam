//! Concurrency stress tests, in their own test binary: they hold the engine's
//! open lock for seconds, which would distort the timing assertions of tests
//! running beside them (`tests/fixtures.rs`).

mod common;

use common::manifest;
use mirsam_core::{Engine, Error, PdfiumSource};

fn engine() -> &'static Engine {
    Engine::global(&PdfiumSource::Auto).expect("PDFium is available")
}

/// PDFium's "last error" is process-wide: concurrent loads must still report
/// their own failure, not each other's.
#[test]
fn concurrent_failed_opens_report_their_own_error() {
    let manifest = manifest();
    let encrypted = manifest.fixtures.iter().find(|f| f.id == "encrypted").expect("exists").file();
    let truncated = manifest.fixtures.iter().find(|f| f.id == "truncated").expect("exists").file();
    let encrypted_bytes = std::fs::read(&encrypted).expect("reads");
    let garbage = b"%PDF-1.7\nnot really a pdf".to_vec();

    std::thread::scope(|scope| {
        for worker in 0..8 {
            let encrypted_bytes = encrypted_bytes.clone();
            let garbage = garbage.clone();
            let truncated = truncated.clone();
            scope.spawn(move || {
                for _ in 0..300 {
                    if worker % 2 == 0 {
                        let result = engine().open_bytes(encrypted_bytes.clone(), None);
                        assert!(matches!(result, Err(Error::PasswordRequired)), "encrypted: {result:?}");
                    } else {
                        let result = engine().open_bytes(garbage.clone(), None);
                        assert!(matches!(result, Err(Error::InvalidPdf)), "garbage: {result:?}");
                        // The truncated file may be recovered or rejected, but never as "password".
                        let result = engine().open_file(&truncated, None);
                        assert!(!matches!(result, Err(Error::PasswordRequired)), "truncated: {result:?}");
                    }
                }
            });
        }
    });
}
