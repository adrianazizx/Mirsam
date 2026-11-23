//! OCR against the real model (ADR 0032). The weights are several GB, so CI
//! cannot run this; it is ignored explicitly and run on a machine that has them:
//!
//!   MIRSAM_OCR_MODEL_DIR=<dir with the Nuqta GGUFs> [MIRSAM_LLAMA_SERVER=<path>] \
//!     cargo test --release -p mirsam-core --test ocr -- --ignored
//!
//! It also needs the evaluation set (`node ocr-eval/generate.mjs`).

use std::path::{Path, PathBuf};

use mirsam_core::ocr::{NuqtaEngine, NuqtaModel, OcrEngine, OcrImage, ServerConfig};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn engine(model: NuqtaModel) -> NuqtaEngine {
    let dir = std::env::var_os("MIRSAM_OCR_MODEL_DIR")
        .expect("set MIRSAM_OCR_MODEL_DIR to the directory holding the Nuqta GGUF files");
    let server =
        std::env::var_os("MIRSAM_LLAMA_SERVER").map_or_else(|| PathBuf::from("llama-server"), PathBuf::from);
    NuqtaEngine::start(&ServerConfig::for_model(server, Path::new(&dir), model), model)
        .expect("llama-server starts with the model (install llama.cpp, check the model directory)")
}

fn sample(id: &str) -> (OcrImage, String) {
    let out = repo().join("ocr-eval/out");
    let image = std::fs::read(out.join(format!("images/{id}.png")))
        .unwrap_or_else(|err| panic!("evaluation set missing ({err}): run `node ocr-eval/generate.mjs`"));
    let truth =
        std::fs::read_to_string(out.join(format!("truth/{id}.txt"))).expect("truth file beside the image");
    (OcrImage::decode_png(&image).expect("PNG"), truth)
}

fn words(text: &str) -> Vec<&str> {
    text.split_whitespace().collect()
}

#[test]
#[ignore = "needs llama-server and the Nuqta GGUFs (MIRSAM_OCR_MODEL_DIR); see the file's header"]
fn nuqta_4b_reads_clean_arabic_exactly() {
    let engine = engine(NuqtaModel::DEFAULT);
    for id in ["syn-ar-plain-naskh-clean-300", "syn-ar-vocalized-naskh-clean-300"] {
        let (image, truth) = sample(id);
        let result = engine.recognize(&image).expect("recognised");
        assert_eq!(words(&result.text), words(&truth), "{id}");
        assert!(!result.truncated, "{id}");
        assert_eq!(result.words.len(), words(&truth).len(), "{id}: one confidence per word");
        assert!(result.words.iter().all(|w| (0.0..=1.0).contains(&w.probability)));
        assert!(result.timings.prompt_tokens > 1000, "{id}: the page went in at full size");
    }
}
