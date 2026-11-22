//! A composite font's ToUnicode CMap, read while the page's text is
//! extracted, redacted and scrubbed. Seeds are the fixtures' CMaps
//! (fuzz/seed.mjs).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    safha_fuzz::exercise(&safha_fuzz::page_pdf(safha_fuzz::CONTENT, data));
});
