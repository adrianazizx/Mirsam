//! Reads one PDF page with Nuqta through `llama-server` (ADR 0032) and prints its
//! text, then the words the model was least sure of.
//!
//!   cargo run --release -p safha-core --example ocr_page -- <in.pdf> [page=0]
//!       [--model nuqta-4b-q8_0] [--cpu] [--password <pw>]
//!
//! Needs `SAFHA_OCR_MODEL_DIR` (the GGUF files) and, unless `llama-server` is on
//! `PATH`, `SAFHA_LLAMA_SERVER`.

use std::path::{Path, PathBuf};

use safha_core::ocr::{Device, NuqtaEngine, NuqtaModel, OcrEngine, OcrImage, ServerConfig};
use safha_core::{Engine, PdfiumSource};

/// Pages are rendered at 300 dpi (spec §8) and then brought to the model's size.
const RENDER_DPI: f32 = 300.0;
/// Words below this probability are listed for review (ADR 0030, finding 3).
const REVIEW_BELOW: f32 = 0.95;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut positional = Vec::new();
    let mut model = NuqtaModel::DEFAULT;
    let mut device = Device::Gpu;
    let mut password = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--model" => {
                let name = args.next().ok_or("--model needs a value")?;
                model = NuqtaModel::from_name(&name).ok_or_else(|| format!("unknown model {name}"))?;
            }
            "--cpu" => device = Device::Cpu,
            "--password" => password = Some(args.next().ok_or("--password needs a value")?),
            _ => positional.push(arg),
        }
    }
    let [input, rest @ ..] = positional.as_slice() else {
        return Err(
            "usage: ocr_page <in.pdf> [page=0] [--model nuqta-4b-q8_0] [--cpu] [--password <pw>]".into()
        );
    };
    let page: usize = rest.first().map(|s| s.parse()).transpose()?.unwrap_or(0);
    let model_dir =
        std::env::var_os("SAFHA_OCR_MODEL_DIR").ok_or("set SAFHA_OCR_MODEL_DIR to the GGUF directory")?;
    let server =
        std::env::var_os("SAFHA_LLAMA_SERVER").map_or_else(|| PathBuf::from("llama-server"), PathBuf::from);

    let document = Engine::global(&PdfiumSource::Auto)?.open_file(Path::new(input), password.as_deref())?;
    let bitmap = document.render_page(page, RENDER_DPI / 72.0)?;
    let image = OcrImage::from_bitmap(&bitmap);

    let mut config = ServerConfig::for_model(server, Path::new(&model_dir), model);
    config.device = device;
    let engine = NuqtaEngine::start(&config, model)?;
    let result = engine.recognize(&image)?;

    println!("{}", result.text);
    println!(
        "\n— {} on page {page}: {:.1} s ({} image tokens, {} written){}",
        engine.name(),
        result.timings.total.as_secs_f64(),
        result.timings.prompt_tokens,
        result.timings.generated_tokens,
        if result.truncated { ", cut off at the token limit" } else { "" },
    );
    let unsure: Vec<String> = result
        .words
        .iter()
        .filter(|w| w.probability < REVIEW_BELOW)
        .map(|w| format!("{} ({:.2})", w.word, w.probability))
        .collect();
    if !unsure.is_empty() {
        println!("Review: {}", unsure.join(", "));
    }
    Ok(())
}
