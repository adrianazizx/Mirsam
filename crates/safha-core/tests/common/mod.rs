//! Access to the generated fixture corpus (fixtures/generate.mjs).

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub fixtures: Vec<Fixture>,
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
