//! The objects of a file, separated by `endobj`, written out with a correct
//! cross-reference table: the fuzzer reaches page trees, inherited
//! attributes, resources, streams and their filters without first having to
//! guess byte offsets. Seeds are the fixtures' objects (fuzz/seed.mjs).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let pdf = mirsam_fuzz::pdf_from_objects(&mirsam_fuzz::split_bodies(data));
    mirsam_fuzz::exercise(&pdf);
});
