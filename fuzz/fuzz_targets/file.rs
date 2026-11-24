//! A whole file, as bytes: every PDFium-free reader in mirsam-core
//! (`mirsam_fuzz::exercise`). Seeds are the fixture corpus.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| mirsam_fuzz::exercise(data));
