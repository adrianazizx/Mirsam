//! Recorded engine answers, so the OCR pipeline can be scored without its models
//! (the benchmark gate in CI, ADR 0036).
//!
//! [`Recorder`] wraps a real engine during a full run on the evaluation set and
//! keeps every answer with the image it answered: its size and a coarse
//! fingerprint (the mean brightness of an 8×8 grid). [`Replay`] answers the same
//! requests from those recordings. Everything the pipeline does around the
//! engines (assessment, line strips, which words to re-read, reconciling, order
//! fixes, word boxes, the text layer) runs for real, so a change there shows in
//! the scores; only the models' answers are canned.
//!
//! Images are matched by size first, then fingerprint, with a little slack, so a
//! strip one pixel taller on another platform's floating point still finds its
//! answer. A request with no close recording is an error that names the image:
//! the pipeline changed what it asks, and the recordings must be made again.

use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};

use super::{OcrEngine, OcrError, OcrImage, Recognition, Timings, WordConfidence};

/// One engine answer and the image it answered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordedRead {
    pub engine: String,
    pub width: u32,
    pub height: u32,
    /// [`fingerprint`] of the image.
    pub print: Vec<u8>,
    pub text: String,
    /// Each word with its probability.
    pub words: Vec<(String, f32)>,
    #[serde(default)]
    pub truncated: bool,
}

/// The answers recorded for one sample, in the order they were asked, and the
/// draft reader's lines (the word boxes' guide).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Recording {
    pub reads: Vec<RecordedRead>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<super::draft::DraftPage>,
}

/// The mean brightness of an 8×8 grid over the image.
pub fn fingerprint(image: &OcrImage) -> Vec<u8> {
    let (w, h) = (image.width() as usize, image.height() as usize);
    let rgb = image.rgb();
    let mut out = Vec::with_capacity(64);
    for gy in 0..8 {
        for gx in 0..8 {
            let (x0, x1) = (gx * w / 8, ((gx + 1) * w / 8).max(gx * w / 8 + 1).min(w));
            let (y0, y1) = (gy * h / 8, ((gy + 1) * h / 8).max(gy * h / 8 + 1).min(h));
            let (mut sum, mut count) = (0u64, 0u64);
            for y in y0..y1 {
                for x in x0..x1 {
                    let at = (y * w + x) * 3;
                    sum += u64::from(rgb[at]) + u64::from(rgb[at + 1]) + u64::from(rgb[at + 2]);
                    count += 3;
                }
            }
            out.push(sum.checked_div(count).map_or(255, |mean| mean as u8));
        }
    }
    out
}

/// A real engine whose answers are kept in a shared [`Recording`].
pub struct Recorder<'a> {
    pub inner: &'a dyn OcrEngine,
    pub recording: Arc<Mutex<Recording>>,
}

impl OcrEngine for Recorder<'_> {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn recognize(&self, image: &OcrImage) -> Result<Recognition, OcrError> {
        let answer = self.inner.recognize(image)?;
        let read = RecordedRead {
            engine: self.inner.name().to_owned(),
            width: image.width(),
            height: image.height(),
            print: fingerprint(image),
            text: answer.text.clone(),
            words: answer.words.iter().map(|w| (w.word.clone(), w.probability)).collect(),
            truncated: answer.truncated,
        };
        self.recording.lock().unwrap_or_else(PoisonError::into_inner).reads.push(read);
        Ok(answer)
    }
}

/// An engine that answers from a recording.
pub struct Replay {
    name: String,
    reads: Vec<RecordedRead>,
}

impl Replay {
    /// The answers `engine` gave in `recording`.
    pub fn new(engine: &str, recording: &Recording) -> Self {
        Self {
            name: engine.to_owned(),
            reads: recording.reads.iter().filter(|r| r.engine == engine).cloned().collect(),
        }
    }
}

impl OcrEngine for Replay {
    fn name(&self) -> &str {
        &self.name
    }

    fn recognize(&self, image: &OcrImage) -> Result<Recognition, OcrError> {
        let print = fingerprint(image);
        let (w, h) = (image.width() as f32, image.height() as f32);
        let best = self
            .reads
            .iter()
            .filter_map(|read| {
                let (dw, dh) = ((read.width as f32 - w).abs(), (read.height as f32 - h).abs());
                // A few pixels either way: a strip boundary that moved by rounding.
                if dw > 4.0 + 0.01 * w || dh > 4.0 + 0.03 * h {
                    return None;
                }
                let distance: u32 =
                    read.print.iter().zip(&print).map(|(a, b)| u32::from(a.abs_diff(*b))).sum();
                Some((dw + dh, distance, read))
            })
            .filter(|(_, distance, _)| *distance <= 64 * 12)
            .min_by(|a, b| (a.0, a.1).partial_cmp(&(b.0, b.1)).unwrap_or(std::cmp::Ordering::Equal));
        let Some((_, _, read)) = best else {
            return Err(OcrError::BadResponse(format!(
                "{}: no recorded answer for a {}x{} image; record again (ADR 0036)",
                self.name,
                image.width(),
                image.height()
            )));
        };
        Ok(Recognition {
            text: read.text.clone(),
            raw: read.text.clone(),
            words: read
                .words
                .iter()
                .map(|(word, p)| WordConfidence { word: word.clone(), probability: *p })
                .collect(),
            truncated: read.truncated,
            timings: Timings::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;

    impl OcrEngine for Echo {
        fn name(&self) -> &str {
            "echo"
        }

        fn recognize(&self, image: &OcrImage) -> Result<Recognition, OcrError> {
            let text = format!("{}x{}", image.width(), image.height());
            Ok(Recognition {
                words: vec![WordConfidence { word: text.clone(), probability: 0.5 }],
                raw: text.clone(),
                text,
                truncated: false,
                timings: Timings::default(),
            })
        }
    }

    fn image(w: u32, h: u32, shade: u8) -> OcrImage {
        OcrImage::from_rgb(w, h, vec![shade; (w * h * 3) as usize]).expect("valid")
    }

    #[test]
    fn answers_are_recorded_and_replayed_with_slack() {
        let recording = Arc::new(Mutex::new(Recording::default()));
        let recorder = Recorder { inner: &Echo, recording: Arc::clone(&recording) };
        recorder.recognize(&image(100, 40, 200)).expect("reads");
        recorder.recognize(&image(100, 40, 20)).expect("reads");
        recorder.recognize(&image(300, 400, 255)).expect("reads");
        let recording = recording.lock().expect("lock").clone();
        let json = serde_json::to_string(&recording).expect("json");
        let recording: Recording = serde_json::from_str(&json).expect("parses");

        let replay = Replay::new("echo", &recording);
        // Same size: the closer fingerprint wins.
        assert_eq!(replay.recognize(&image(100, 40, 30)).expect("found").text, "100x40");
        assert_eq!(replay.recognize(&image(100, 41, 190)).expect("found").words[0].probability, 0.5);
        assert_eq!(replay.recognize(&image(301, 400, 250)).expect("found").text, "300x400");
        assert!(replay.recognize(&image(200, 40, 200)).is_err(), "nothing that size");
        assert!(Replay::new("other", &recording).recognize(&image(100, 40, 200)).is_err());
    }
}
