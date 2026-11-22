//! OCR evaluation: scoring only, no OCR.
//!
//! Compares an engine's text output with human-verified ground truth and
//! reports, per category, the numbers the ship gates in `docs/SPEC.md`
//! section 8 are written against:
//!
//! * **CER / WER** — character and word error rate (Levenshtein distance over
//!   the reference length), micro-averaged across a category's samples.
//! * **Field accuracy** — exact-match rate for numbers, dates, and IDs.
//! * **Reading-order errors** — text that was recognized but put in the wrong
//!   place or written backwards, counted separately from recognition errors
//!   because it is the classic Arabic OCR failure and has its own gate.
//! * **Table cells** — share of cells read exactly, and cells read but placed
//!   out of reading order, so a misordered table is not mistaken for a misread one.
//!
//! Scoring is deliberately strict: tashkeel, digit style, and punctuation all
//! count, because the spec requires OCR to preserve what is on the page. A
//! second, lenient CER (`cer_base`) ignores diacritics and letter variants and
//! is informational only — it shows how much of the error is "just" marks.

mod align;
mod normalize;
mod report;
mod score;

pub use align::edit_distance;
pub use normalize::{Profile, normalize};
pub use report::{
    CategoryScore, GateFailure, Manifest, Regression, Report, Sample, Target, Targets, evaluate,
};
pub use score::{CellScore, Counts, FieldScore, OrderScore, SampleScore, score_best, score_sample};
