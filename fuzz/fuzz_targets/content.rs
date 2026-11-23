//! One page's content stream, on a page with real resources (two kinds of
//! font, a form, an image, a hidden layer): the content tokenizers and
//! interpreters behind text blocks, redaction and sanitizing. Seeds are the
//! fixtures' page contents (fuzz/seed.mjs).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    mirsam_fuzz::exercise(&mirsam_fuzz::page_pdf(data, mirsam_fuzz::TO_UNICODE));
});
