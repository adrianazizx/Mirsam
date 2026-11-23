//! Runs a Nuqta model over the OCR evaluation set through `llama-server` (ADR 0032),
//! writing one text file per sample for the `mirsam-ocr-eval` scorer.
//!
//!   cargo run --release -p mirsam-core --example ocr_eval -- --out <dir>
//!       [--model nuqta-4b-q8_0] [--model-dir <dir>] [--server <llama-server>]
//!       [--cpu] [--threads N] [--only <id part>] [--stride N] [--manifest <path>]
//!       [--pipeline [--vision] [--reread-all-strips]]
//!
//! `--pipeline` runs the quality pipeline (ADR 0033): the model above as the primary,
//! Nuqta 0.8B Q8_0 as the partner, and with `--vision` Apple Vision (macOS, via
//! `ocr-eval/engines/apple-vision.swift --image`) as the third reader for numbers.
//! It also writes `<sample>.review.json` (the words to review) and the page review
//! list in `run.json`.
//!
//! `--model-dir` (or `MIRSAM_OCR_MODEL_DIR`) holds the GGUF files under their
//! repository names (`nuqta-ocr-4b-Q8_0.gguf`, `nuqta-ocr-4b-mmproj-f16.gguf`, …).
//! `--server` (or `MIRSAM_LLAMA_SERVER`) defaults to `llama-server` on `PATH`.
//! Then score:
//!
//!   cargo run -p mirsam-ocr-eval -- --manifest ocr-eval/out/manifest.json --hyp <dir>
//!       --engine <name> --targets ocr-eval/targets.json

use std::path::{Path, PathBuf};
use std::time::Instant;

use mirsam_core::ocr::{
    CommandEngine, Device, NuqtaEngine, NuqtaModel, OcrEngine, OcrImage, PageResult, QualityReader,
    ServerConfig, Settings, WordStatus, review_list,
};
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
    pipeline: bool,
    vision: bool,
    reread_all_strips: bool,
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = None;
    let mut model = NuqtaModel::DEFAULT;
    let mut model_dir = std::env::var_os("MIRSAM_OCR_MODEL_DIR").map(PathBuf::from);
    let mut server =
        std::env::var_os("MIRSAM_LLAMA_SERVER").map_or_else(|| PathBuf::from("llama-server"), PathBuf::from);
    let mut device = Device::Gpu;
    let mut threads = None;
    let mut only = None;
    let mut stride = 1;
    let mut manifest = repo.join("ocr-eval/out/manifest.json");
    let (mut pipeline, mut vision, mut reread_all_strips) = (false, false, false);
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
            "--pipeline" => pipeline = true,
            "--vision" => vision = true,
            "--reread-all-strips" => reread_all_strips = true,
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    Ok(Args {
        out: out.ok_or("--out <dir> is required")?,
        model,
        model_dir: model_dir.ok_or("--model-dir <dir> (or MIRSAM_OCR_MODEL_DIR) is required")?,
        server,
        device,
        threads,
        only,
        stride,
        manifest,
        pipeline,
        vision,
        reread_all_strips,
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
    if args.pipeline {
        return run_pipeline(&args, &engine, &samples, &config, load_seconds);
    }

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
        "runtime": "mirsam_core::ocr::NuqtaEngine via llama-server",
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

fn run_pipeline(
    args: &Args,
    primary: &NuqtaEngine,
    samples: &[(String, PathBuf)],
    primary_config: &ServerConfig,
    load_seconds: f64,
) -> Result<(), Box<dyn std::error::Error>> {
    let partner_model = NuqtaModel::PARTNER;
    let mut config = ServerConfig::for_model(&args.server, &args.model_dir, partner_model);
    config.device = primary_config.device;
    config.threads = primary_config.threads;
    config.log = Some(args.out.join("llama-server-partner.log"));
    let partner = NuqtaEngine::start(&config, partner_model)?;
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let vision = CommandEngine {
        name: "apple-vision".into(),
        program: "swift".into(),
        args: vec![repo.join("ocr-eval/engines/apple-vision.swift").display().to_string(), "--image".into()],
    };
    let reader = QualityReader {
        primary,
        partner: Some(&partner),
        third: if args.vision { Some(&vision) } else { None },
        settings: Settings { reread_all_strips: args.reread_all_strips, ..Settings::default() },
    };

    let mut results: Vec<(usize, String, PageResult)> = Vec::new();
    let mut per_sample = serde_json::Map::new();
    let mut total = 0.0;
    for (index, (id, image)) in samples.iter().enumerate() {
        let image = OcrImage::decode_png(&std::fs::read(image)?)?;
        let result = reader.read_page(&image)?;
        let seconds = result.elapsed.as_secs_f64();
        total += seconds;
        std::fs::write(args.out.join(format!("{id}.txt")), format!("{}\n", result.text().trim()))?;
        let review: Vec<Value> = result
            .review()
            .map(|w| {
                let (status, detail) = match &w.status {
                    WordStatus::Corrected { read } => ("corrected", json!(read)),
                    WordStatus::Uncertain { alternatives } => ("uncertain", json!(alternatives)),
                    WordStatus::Reordered { read } => ("reordered", json!(read)),
                    other => ("other", json!(format!("{other:?}"))),
                };
                json!({ "word": w.text, "line": w.line, "probability": w.probability, "status": status, "detail": detail })
            })
            .collect();
        std::fs::write(args.out.join(format!("{id}.review.json")), serde_json::to_string_pretty(&review)?)?;
        per_sample.insert(
            id.clone(),
            json!({
                "seconds": (seconds * 10.0).round() / 10.0,
                "reads": result.reads,
                "skew_degrees": result.layout.skew_degrees,
                "strips": result.layout.lines.len(),
                "needs_review": result.needs_review(),
            }),
        );
        println!("{seconds:7.1} s  {:2} reads  {:2} to review  {id}", result.reads, result.review().count());
        results.push((index, id.clone(), result));
    }
    let list: Vec<Value> = review_list(results.iter().map(|(i, _, r)| (*i, r)))
        .into_iter()
        .map(|p| {
            json!({
                "page": results[p.page].1,
                "uncertain": p.uncertain,
                "corrected": p.corrected,
                "reordered": p.reordered,
                "truncated": p.truncated,
                "lowest_probability": p.lowest_probability,
            })
        })
        .collect();
    let record = json!({
        "engine": format!("pipeline:{}+{}{}", primary.name(), partner.name(), if args.vision { "+apple-vision" } else { "" }),
        "runtime": "mirsam_core::ocr::QualityReader via llama-server",
        "device": format!("{:?}", args.device),
        "reread_all_strips": args.reread_all_strips,
        "samples": samples.len(),
        "load_seconds": (load_seconds * 10.0).round() / 10.0,
        "seconds_per_page": if samples.is_empty() { 0.0 } else { (total / samples.len() as f64 * 100.0).round() / 100.0 },
        "pages_needing_review": list,
        "per_sample": per_sample,
    });
    std::fs::write(args.out.join("run.json"), serde_json::to_string_pretty(&record)? + "\n")?;
    println!("{:.2} s per page", record["seconds_per_page"].as_f64().unwrap_or(0.0));
    Ok(())
}
