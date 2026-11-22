//! Command-line scorer for the OCR evaluation set.
//!
//!   safha-ocr-eval --manifest ocr-eval/out/manifest.json --hyp <dir> --engine <name>
//!                  [--targets ocr-eval/targets.json] [--baseline <report.json>] [--out <report.json>]
//!
//! `<dir>` holds one `<sample id>.txt` per sample: the engine's plain-text
//! output for that page, in reading order. Exit status is 1 when a gate in
//! `--targets` fails or any category regressed against `--baseline`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use safha_ocr_eval::{Manifest, Report, Targets, evaluate, score_sample};

struct Args {
    manifest: PathBuf,
    hyp: PathBuf,
    engine: String,
    targets: Option<PathBuf>,
    baseline: Option<PathBuf>,
    out: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut values = std::collections::HashMap::new();
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let name = flag.strip_prefix("--").ok_or_else(|| format!("unexpected argument {flag}"))?;
        let value = args.next().ok_or_else(|| format!("--{name} needs a value"))?;
        values.insert(name.to_string(), value);
    }
    let mut required = |name: &str| values.remove(name).ok_or_else(|| format!("missing --{name}"));
    let args = Args {
        manifest: required("manifest")?.into(),
        hyp: required("hyp")?.into(),
        engine: required("engine")?,
        targets: values.remove("targets").map(PathBuf::from),
        baseline: values.remove("baseline").map(PathBuf::from),
        out: values.remove("out").map(PathBuf::from),
    };
    match values.keys().next() {
        Some(unknown) => Err(format!("unknown option --{unknown}")),
        None => Ok(args),
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text = std::fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    serde_json::from_str(&text).map_err(|err| format!("{}: {err}", path.display()))
}

fn percent(rate: f64) -> String {
    format!("{:.2}%", rate * 100.0)
}

fn run() -> Result<bool, String> {
    let args = parse_args()?;
    let manifest: Manifest = read_json(&args.manifest)?;
    let set_dir = args.manifest.parent().unwrap_or(Path::new("."));

    let mut scores = Vec::with_capacity(manifest.samples.len());
    let mut missing = Vec::new();
    for sample in &manifest.samples {
        let truth_path = set_dir.join(&sample.truth);
        let truth =
            std::fs::read_to_string(&truth_path).map_err(|err| format!("{}: {err}", truth_path.display()))?;
        let hypothesis =
            std::fs::read_to_string(args.hyp.join(format!("{}.txt", sample.id))).unwrap_or_else(|_| {
                missing.push(sample.id.clone());
                String::new()
            });
        scores.push(score_sample(&truth, &hypothesis, &sample.fields, &sample.cells));
    }
    let report = evaluate(&args.engine, &manifest, scores, missing);

    println!(
        "engine: {}   evaluation set v{}   samples: {}",
        report.engine,
        report.set_version,
        manifest.samples.len()
    );
    println!(
        "{:<26} {:>4} {:>8} {:>8} {:>9} {:>8} {:>6} {:>8} {:>7}",
        "category", "n", "CER", "WER", "CER-base", "fields", "order", "cells", "c-order"
    );
    for (name, score) in &report.categories {
        let fields = if score.fields_total > 0 { percent(score.field_accuracy) } else { "-".into() };
        let (cells, cell_order) = if score.cells_total > 0 {
            (percent(score.cell_accuracy), score.cell_order_errors.to_string())
        } else {
            ("-".into(), "-".into())
        };
        println!(
            "{:<26} {:>4} {:>8} {:>8} {:>9} {:>8} {:>6} {:>8} {:>7}",
            name,
            score.samples,
            percent(score.cer),
            percent(score.wer),
            percent(score.cer_base),
            fields,
            score.order_errors,
            cells,
            cell_order
        );
    }
    if !report.missing.is_empty() {
        println!(
            "\nno output for {} sample(s), scored as empty: {}",
            report.missing.len(),
            report.missing.join(", ")
        );
    }

    let mut passed = true;
    if let Some(path) = &args.targets {
        let targets: Targets = read_json(path)?;
        let failures = report.gate_failures(&targets);
        for failure in &failures {
            println!(
                "GATE FAILED  {:<26} {:<15} {:.4} (limit {:.4})",
                failure.category, failure.metric, failure.actual, failure.limit
            );
        }
        if failures.is_empty() {
            println!("\nall {} gates passed", targets.len());
        }
        passed &= failures.is_empty();
    }
    if let Some(path) = &args.baseline {
        let baseline: Report = read_json(path)?;
        let regressions = report.regressions(&baseline)?;
        for regression in &regressions {
            println!(
                "REGRESSION   {:<26} {:<15} {:.4} -> {:.4}",
                regression.category, regression.metric, regression.before, regression.after
            );
        }
        passed &= regressions.is_empty();
    }
    if let Some(path) = &args.out {
        let json = serde_json::to_string_pretty(&report).map_err(|err| err.to_string())?;
        std::fs::write(path, json + "\n").map_err(|err| format!("{}: {err}", path.display()))?;
    }
    Ok(passed)
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(2)
        }
    }
}
