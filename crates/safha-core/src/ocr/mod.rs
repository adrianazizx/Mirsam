//! OCR (spec §8): recognising the text of page images.
//!
//! This is the first slice of the quality pipeline (ADR 0030–0032):
//!
//! - [`OcrEngine`], the interface every recogniser implements;
//! - [`NuqtaEngine`], Nuqta-Arabic-OCR through llama.cpp's `llama-server`, which
//!   runs as a child process ([`LlamaServer`], ADR 0032);
//! - [`OcrImage`] and [`prepare_nuqta`], the page image at the size the model was
//!   measured with;
//! - [`to_text`] and per-word confidence from the model's token probabilities.
//!
//! - [`QualityReader`]: assessment and line strips ([`analyze`]), number–unit order
//!   ([`resolve_unit_order`]), re-reading of numbers and unsure words, and
//!   reconciling ([`reconcile`]) into words with a status and a review list
//!   ([`review_list`]) (ADR 0033).
//!
//! Word boxes and writing the text layer come in later slices.

mod command;
mod image;
mod layout;
mod llama;
mod order;
mod pipeline;
mod reconcile;
mod text;

use std::path::PathBuf;
use std::time::Duration;

pub use command::CommandEngine;
pub use image::{NUQTA_PIXELS, OcrImage, nuqta_size, prepare_nuqta};
pub use layout::{LineBox, PageLayout, analyze, deskew};
pub use llama::{
    ChatAnswer, Device, LlamaServer, NUQTA_PROMPT, NuqtaEngine, NuqtaModel, NuqtaQuant, NuqtaSize,
    ServerConfig, parse_chat_response,
};
pub use order::{OrderFix, UNIT_WORDS, resolve_unit_order};
pub use pipeline::{PageResult, PageReview, QualityReader, Settings, review_list};
pub use reconcile::{
    OtherReadings, ReviewedWord, WordStatus, is_target, key, reconcile, words_with_confidence,
};
pub use text::{Token, strip_think, to_text, word_confidences};

/// A recogniser: page (or crop) image in, text in reading order out.
pub trait OcrEngine {
    /// A short stable name for reports (`nuqta-4b-q8_0`).
    fn name(&self) -> &str;

    /// Reads `image`. Blocks until the engine answers.
    fn recognize(&self, image: &OcrImage) -> Result<Recognition, OcrError>;
}

/// What an engine read.
#[derive(Clone, Debug, PartialEq)]
pub struct Recognition {
    /// The page's text in the engine's reading order, markup removed ([`to_text`]).
    pub text: String,
    /// The engine's answer as it came (for diagnosis).
    pub raw: String,
    /// Each word of the answer with its confidence, when the engine reports one.
    pub words: Vec<WordConfidence>,
    /// The answer was cut off at the token limit (the page may be incomplete).
    pub truncated: bool,
    pub timings: Timings,
}

/// A word of an answer and how sure the engine was of it: the lowest
/// probability of any token in it (0–1).
#[derive(Clone, Debug, PartialEq)]
pub struct WordConfidence {
    pub word: String,
    pub probability: f32,
}

/// Where the time of one recognition went.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Timings {
    /// Request sent to answer received.
    pub total: Duration,
    /// Tokens of the prompt (mostly the image) and the time to process them.
    pub prompt_tokens: u32,
    pub prompt: Duration,
    /// Tokens written and the time to write them.
    pub generated_tokens: u32,
    pub generation: Duration,
}

/// Why recognition failed. [`OcrError::code`] is the stable code the UI maps to a
/// localized message.
#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    /// The engine program (`llama-server`) is not installed where it was expected.
    #[error("OCR engine not found: {0}")]
    EngineMissing(PathBuf),

    /// A model file is missing (not downloaded yet, or moved).
    #[error("OCR model file not found: {0}")]
    ModelMissing(PathBuf),

    /// The engine could not be started.
    #[error("the OCR engine could not start: {0}")]
    StartFailed(String),

    /// The engine stopped while starting or reading (often: out of memory).
    #[error("the OCR engine stopped: {0}")]
    Exited(String),

    /// The engine did not become ready, or did not answer, in time.
    #[error("the OCR engine timed out: {0}")]
    Timeout(&'static str),

    /// The engine answered with an error or something we could not read.
    #[error("the OCR engine answered unexpectedly: {0}")]
    BadResponse(String),

    /// The image could not be prepared for recognition.
    #[error("the image could not be prepared: {0}")]
    Image(&'static str),
}

impl OcrError {
    pub fn code(&self) -> &'static str {
        match self {
            OcrError::EngineMissing(_) => "ocr_engine_missing",
            OcrError::ModelMissing(_) => "ocr_model_missing",
            OcrError::StartFailed(_) => "ocr_engine_failed",
            OcrError::Exited(_) => "ocr_engine_stopped",
            OcrError::Timeout(_) => "ocr_timeout",
            OcrError::BadResponse(_) => "ocr_bad_response",
            OcrError::Image(_) => "ocr_image_invalid",
        }
    }
}
