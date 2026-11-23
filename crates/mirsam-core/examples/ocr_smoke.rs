//! The OCR smoke run (ADR 0036): the real thing, end to end, on one page.
//!
//!   cargo run --release -p mirsam-core --example ocr_smoke -- --models <dir> \
//!       --image <page.png> --truth <page.txt> [--package nuqta-0.8b] [--max-cer 0.05] [--gpu]
//!
//! Installs the llama.cpp runtime and a Nuqta model through the app's own
//! download manager (pinned catalog, checksums; `--models` is the store, which a
//! CI cache keeps), starts `llama-server` on the CPU, reads the page, and fails
//! when the character error rate against the truth is above `--max-cer`. It
//! proves the parts the benchmark gate replays: the download, the runtime, the
//! request and the answer's parsing.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use mirsam_core::models::{Catalog, Downloader, ModelStore, RUNTIME};
use mirsam_core::ocr::{Device, NuqtaEngine, NuqtaModel, OcrEngine, OcrImage, ServerConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (mut models, mut image, mut truth) = (None, None, None);
    let (mut package, mut max_cer, mut device) = ("nuqta-0.8b".to_owned(), 0.05f64, Device::Cpu);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--models" => models = Some(PathBuf::from(value()?)),
            "--image" => image = Some(PathBuf::from(value()?)),
            "--truth" => truth = Some(PathBuf::from(value()?)),
            "--package" => package = value()?,
            "--max-cer" => max_cer = value()?.parse()?,
            "--gpu" => device = Device::Gpu,
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    let store = ModelStore::new(models.ok_or("--models <dir> is required")?);
    let catalog = Catalog::builtin();
    let downloader = Downloader::default();
    let cancel = AtomicBool::new(false);
    for id in [RUNTIME, package.as_str()] {
        let wanted = catalog.get(id).ok_or_else(|| format!("{id} is not in this platform's catalog"))?;
        let started = Instant::now();
        let mut last = 0u64;
        store.install(wanted, &downloader, &cancel, &mut |progress| {
            if progress.received >= last + 100_000_000 || progress.received == progress.total {
                last = progress.received;
                eprintln!("{id}: {:?} {} of {} bytes", progress.phase, progress.received, progress.total);
            }
        })?;
        println!("{id} installed in {:.0} s", started.elapsed().as_secs_f64());
    }
    let runtime = store.installed(RUNTIME).ok_or("runtime missing after install")?;
    let program = runtime.program.ok_or("runtime has no program")?;
    let installed = store.installed(&package).ok_or("model missing after install")?;
    let model = match package.as_str() {
        "nuqta-4b" => NuqtaModel::DEFAULT,
        _ => NuqtaModel::PARTNER,
    };
    let mut config = ServerConfig::for_model(program, &installed.dir, model);
    config.device = device;
    let started = Instant::now();
    let engine = NuqtaEngine::start(&config, model)?;
    println!("{} ready in {:.1} s", engine.name(), started.elapsed().as_secs_f64());

    let page = OcrImage::decode_png(&std::fs::read(image.ok_or("--image <png> is required")?)?)?;
    let truth = std::fs::read_to_string(truth.ok_or("--truth <txt> is required")?)?;
    let started = Instant::now();
    let answer = engine.recognize(&page)?;
    let cer = cer(&answer.text, &truth);
    println!("read in {:.1} s; CER {:.2}%", started.elapsed().as_secs_f64(), cer * 100.0);
    println!("{}", answer.text);
    if cer > max_cer {
        return Err(format!("CER {:.2}% is above {:.2}%", cer * 100.0, max_cer * 100.0).into());
    }
    Ok(())
}

/// Character error rate with white space collapsed: edit distance over the truth's length.
fn cer(hypothesis: &str, truth: &str) -> f64 {
    let squash = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ").chars().collect::<Vec<char>>();
    let (a, b) = (squash(hypothesis), squash(truth));
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let here = row[j + 1];
            row[j + 1] = (previous + usize::from(ca != cb)).min(row[j] + 1).min(here + 1);
            previous = here;
        }
    }
    row[b.len()] as f64 / b.len().max(1) as f64
}
