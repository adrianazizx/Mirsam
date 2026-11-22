//! An OCR engine that is an external program: it gets a PNG path and prints the
//! text. Used for the platform's own OCR as the pipeline's third reader (ADR 0033).
//! On macOS that is `ocr-eval/engines/apple-vision.swift --image <png>` for now; a
//! signed helper ships with the model download manager.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use super::{OcrEngine, OcrError, OcrImage, Recognition, Timings};

/// `program args… <image.png>` → text on standard output.
#[derive(Clone, Debug)]
pub struct CommandEngine {
    pub name: String,
    pub program: PathBuf,
    pub args: Vec<String>,
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

impl OcrEngine for CommandEngine {
    fn name(&self) -> &str {
        &self.name
    }

    fn recognize(&self, image: &OcrImage) -> Result<Recognition, OcrError> {
        let path = std::env::temp_dir().join(format!(
            "safha-ocr-{}-{}.png",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, image.encode_png()?)
            .map_err(|err| OcrError::StartFailed(format!("temp image: {err}")))?;
        let started = Instant::now();
        let output = Command::new(&self.program).args(&self.args).arg(&path).stdin(Stdio::null()).output();
        let _ = std::fs::remove_file(&path);
        let output = output.map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => OcrError::EngineMissing(self.program.clone()),
            _ => OcrError::StartFailed(err.to_string()),
        })?;
        if !output.status.success() {
            let message: String = String::from_utf8_lossy(&output.stderr).chars().take(300).collect();
            return Err(OcrError::Exited(format!("{}: {message}", output.status)));
        }
        let text = String::from_utf8(output.stdout)
            .map_err(|_| OcrError::BadResponse("output is not UTF-8".into()))?;
        let text = text.trim().to_owned();
        Ok(Recognition {
            raw: text.clone(),
            text,
            words: Vec::new(),
            truncated: false,
            timings: Timings { total: started.elapsed(), ..Timings::default() },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_program_is_reported() {
        let engine =
            CommandEngine { name: "x".into(), program: "safha-no-such-ocr-program".into(), args: vec![] };
        let image = OcrImage::from_rgb(2, 2, vec![255; 12]).expect("valid");
        assert_eq!(engine.recognize(&image).expect_err("missing").code(), "ocr_engine_missing");
    }
}
