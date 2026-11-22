//! Runs a Nuqta model over the OCR evaluation set through `llama-server` (ADR 0032),
//! writing one text file per sample for the `safha-ocr-eval` scorer.
//!
//!   cargo run --release -p safha-core --example ocr_eval -- --out <dir>
//!       [--model nuqta-4b-q8_0] [--model-dir <dir>] [--server <llama-server>]
//!       [--cpu] [--threads N] [--only <id part>] [--stride N] [--manifest <path>]
//!
//! `--model-dir` (or `SAFHA_OCR_MODEL_DIR`) holds the GGUF files under their
//! repository names (`nuqta-ocr-4b-Q8_0.gguf`, `nuqta-ocr-4b-mmproj-f16.gguf`, …).
//! `--server` (or `SAFHA_LLAMA_SERVER`) defaults to `llama-server` on `PATH`.
//! Then score:
//!
//!   cargo run -p safha-ocr-eval -- --manifest ocr-eval/out/manifest.json --hyp <dir>
//!       --engine <name> --targets ocr-eval/targets.json

use std::path::{Path, PathBuf};
use std::time::Instant;

use safha_core::ocr::{Device, NuqtaEngine, NuqtaModel, OcrEngine, OcrImage, ServerConfig};
use serde_json::{Value, json};

struct Args {
    out: PathBuf,
    model: NuqtaModel,
    model_dir: PathBuf,
    server: PathBuf,
    device: Device,
    threads: Option<u32>,
    only: Option<String>,
    stride: usize,
    manifest: PathBuf,
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = None;
    let mut model = NuqtaModel::DEFAULT;
    let mut model_dir = std::env::var_os("SAFHA_OCR_MODEL_DIR").map(PathBuf::from);
    let mut server =
        std::env::var_os("SAFHA_LLAMA_SERVER").map_or_else(|| PathBuf::from("llama-server"), PathBuf::from);
    let mut device = Device::Gpu;
    let mut threads = None;
    let mut only = None;
    let mut stride = 1;
    let mut manifest = repo.join("ocr-eval/out/manifest.json");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--out" => out = Some(PathBuf::from(value()?)),
            "--model" => {
                let name = value()?;
                model = NuqtaModel::from_name(&name).ok_or_else(|| format!("unknown model {name}"))?;
            }
            "--model-dir" => model_dir = Some(PathBuf::from(value()?)),
            "--server" => server = PathBuf::from(value()?),
            "--cpu" => device = Device::Cpu,
            "--threads" => threads = Some(value()?.parse()?),
            "--only" => only = Some(value()?),
            "--stride" => stride = value()?.parse::<usize>()?.max(1),
            "--manifest" => manifest = PathBuf::from(value()?),
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    Ok(Args {
        out: out.ok_or("--out <dir> is required")?,
        model,
        model_dir: model_dir.ok_or("--model-dir <dir> (or SAFHA_OCR_MODEL_DIR) is required")?,
        server,
        device,
        threads,
        only,
        stride,
        manifest,
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    let manifest_text = std::fs::read_to_string(&args.manifest)
        .map_err(|err| format!("{}: {err} (run `node ocr-eval/generate.mjs`)", args.manifest.display()))?;
    let manifest: Value = serde_json::from_str(&manifest_text)?;
    let base = args.manifest.parent().unwrap_or(Path::new("."));
    let samples: Vec<(String, PathBuf)> = manifest["samples"]
        .as_array()
        .ok_or("manifest has no samples")?
        .iter()
        .filter_map(|s| Some((s["id"].as_str()?.to_owned(), base.join(s["image"].as_str()?))))
        .filter(|(id, _)| args.only.as_ref().is_none_or(|only| id.contains(only.as_str())))
        .step_by(args.stride)
        .collect();

    std::fs::create_dir_all(&args.out)?;
    let mut config = ServerConfig::for_model(&args.server, &args.model_dir, args.model);
    config.device = args.device;
    config.threads = args.threads;
    config.log = Some(args.out.join("llama-server.log"));
    let started = Instant::now();
    let engine = NuqtaEngine::start(&config, args.model)?;
    let load_seconds = started.elapsed().as_secs_f64();
    println!("{} ready in {load_seconds:.1} s; {} samples", engine.name(), samples.len());

    let mut per_sample = serde_json::Map::new();
    let mut total = 0.0;
    for (id, image) in &samples {
        let image = OcrImage::decode_png(&std::fs::read(image)?)?;
        let result = engine.recognize(&image)?;
        let seconds = result.timings.total.as_secs_f64();
        total += seconds;
        std::fs::write(args.out.join(format!("{id}.txt")), format!("{}\n", result.text.trim()))?;
        std::fs::write(args.out.join(format!("{id}.raw.txt")), &result.raw)?;
        let words: Vec<Value> = result.words.iter().map(|w| json!([w.word, w.probability])).collect();
        std::fs::write(args.out.join(format!("{id}.words.json")), serde_json::to_string(&words)?)?;
        per_sample.insert(
            id.clone(),
            json!({
                "seconds": (seconds * 1000.0).round() / 1000.0,
                "prompt_tokens": result.timings.prompt_tokens,
                "prompt_ms": result.timings.prompt.as_millis() as u64,
                "generated_tokens": result.timings.generated_tokens,
                "generation_ms": result.timings.generation.as_millis() as u64,
                "truncated": result.truncated,
            }),
        );
        println!("{seconds:7.1} s  {id}{}", if result.truncated { "  (cut off)" } else { "" });
    }
    let record = json!({
        "engine": engine.name(),
        "runtime": "safha_core::ocr::NuqtaEngine via llama-server",
        "device": format!("{:?}", args.device),
        "samples": samples.len(),
        "load_seconds": (load_seconds * 10.0).round() / 10.0,
        "seconds_per_page": if samples.is_empty() { 0.0 } else { (total / samples.len() as f64 * 100.0).round() / 100.0 },
        "per_sample": per_sample,
    });
    std::fs::write(args.out.join("run.json"), serde_json::to_string_pretty(&record)? + "\n")?;
    println!("{:.2} s per page", record["seconds_per_page"].as_f64().unwrap_or(0.0));
    Ok(())
}
