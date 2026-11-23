//! Access to the generated fixture corpus (fixtures/generate.mjs).

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub fixtures: Vec<Fixture>,
    /// Producers that could not run where the corpus was generated, and why
    /// (CoreText exists only on macOS).
    #[serde(default)]
    pub skipped: Vec<SkippedProducer>,
}

#[derive(Debug, Deserialize)]
pub struct SkippedProducer {
    pub producer: String,
    pub reason: String,
}

impl Manifest {
    /// The fixture called `name` ("producer/id"). `None` only when the
    /// manifest records that its producer could not run on this machine,
    /// which is reported; any other missing fixture fails the test.
    pub fn fixture_or_unavailable(&self, name: &str) -> Option<&Fixture> {
        if let Some(fixture) = self.fixtures.iter().find(|f| f.name() == name) {
            return Some(fixture);
        }
        let producer = name.split('/').next().unwrap_or_default();
        match self.skipped.iter().find(|skipped| skipped.producer == producer) {
            Some(skipped) => {
                eprintln!("not checked here: {name} (producer {producer} unavailable: {})", skipped.reason);
                None
            }
            None => panic!("fixture {name} missing from the corpus: run `node fixtures/generate.mjs`"),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Fixture {
    pub id: String,
    pub producer: String,
    pub path: String,
    pub pages: Option<usize>,
    pub password: Option<String>,
    #[serde(default)]
    pub damaged: bool,
    #[serde(default)]
    pub expected: Vec<ExpectedBlock>,
}

/// One block of ground truth: `text` is in logical (reading) order.
#[derive(Debug, Deserialize)]
pub struct ExpectedBlock {
    pub dir: String,
    pub text: String,
}

impl Fixture {
    pub fn name(&self) -> String {
        format!("{}/{}", self.producer, self.id)
    }

    pub fn file(&self) -> PathBuf {
        fixtures_out().join(&self.path)
    }
}

pub fn fixtures_out() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/out")
}

/// Loads the manifest, failing with instructions when the corpus has not been
/// generated. Fixture tests never skip silently.
pub fn manifest() -> Manifest {
    let path = fixtures_out().join("manifest.json");
    let json = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!("fixture corpus missing at {}: run `node fixtures/generate.mjs`", path.display())
    });
    serde_json::from_str(&json).expect("fixtures/out/manifest.json is valid")
}

/// Collapses runs of whitespace (including the line breaks PDFium inserts) to single spaces.
pub fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The form both sides are compared in: NFC, whitespace collapsed.
pub fn comparable(text: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    normalize_whitespace(&text.nfc().collect::<String>())
}

pub fn without_spaces(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}
pub mod form_pdf;

/// The streams of an incremental update: how many it writes, and how many of
/// them inflate as they are written. In an encrypted file none should: each is
/// compressed, then encrypted. (Searching the bytes for plain text cannot tell:
/// ciphertext holds any short string by chance on some runs.)
pub fn streams_in_the_clear(update: &[u8]) -> (usize, usize) {
    let find = |from: usize, needle: &[u8]| {
        update.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|i| from + i)
    };
    let (mut streams, mut clear, mut at) = (0, 0, 0);
    while let Some(keyword) = find(at, b"stream") {
        let mut start = keyword + b"stream".len();
        if update.get(start) == Some(&b'\r') {
            start += 1;
        }
        if update.get(start) == Some(&b'\n') {
            start += 1;
        }
        let Some(end) = find(start, b"endstream") else { break };
        let mut data = &update[start..end];
        while let Some((last, rest)) = data.split_last()
            && matches!(last, b'\r' | b'\n')
        {
            data = rest;
        }
        streams += 1;
        if miniz_oxide::inflate::decompress_to_vec_zlib(data).is_ok() {
            clear += 1;
        }
        at = end + b"endstream".len();
    }
    (streams, clear)
}
