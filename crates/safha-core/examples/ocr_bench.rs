//! The OCR benchmark (ADR 0036): the quality pipeline over the evaluation set, and
//! the searchable PDF it makes of each page, ready for the `safha-ocr-eval` scorer.
//!
//! With the models, recording every engine answer for the CI gate:
//!
//!   cargo run --release -p safha-core --example ocr_bench -- --out <dir> \
//!       --record ocr-eval/recordings/v2 --model-dir <dir> [--server <llama-server>] [--cpu]
//!
//! Without them, from the recordings (what CI runs):
//!
//!   cargo run --release -p safha-core --example ocr_bench -- --out <dir> --replay ocr-eval/recordings/v2
//!
//! Writes `<out>/text/<sample>.txt` (the pipeline's text) and `<out>/layer/<sample>.txt`
//! (the page as a one-image PDF with the pipeline's text layer written into it,
//! read back by this app's extractor: what search and copy see), and
//! `<out>/run.json`. `--keep-pdfs` keeps the PDFs in `<out>/pdf/`. `--only <part>`
//! picks samples. The draft reader's lines (the word boxes' guide) are recorded
//! too; recording them needs Apple Vision (macOS).
//!
//! `--truth-boxes <word-boxes-truth.json>` (from `spikes/ocr-engines/word_boxes_truth.mjs`)
//! also scores the word boxes against Chrome's layout, as ADR 0031 measured them:
//! words paired in order with true words they equal; the share on the right line,
//! and the horizontal overlap (IoU-x) with the ink-tight true box.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use safha_core::ocr::jobs::{RenderedPage, final_page};
use safha_core::ocr::{
    Device, DraftReader, NuqtaEngine, NuqtaModel, OcrImage, QualityReader, Recorder, Recording, Replay,
    ServerConfig, Settings, VisionReader, vision_helper,
};
use safha_core::{Engine, PdfiumSource};
use serde_json::{Value, json};

enum Mode {
    Record { dir: PathBuf, models: PathBuf, server: PathBuf, device: Device },
    Replay(PathBuf),
}

struct Args {
    out: PathBuf,
    mode: Mode,
    manifest: PathBuf,
    only: Option<String>,
    keep_pdfs: bool,
    truth_boxes: Option<PathBuf>,
}

#[derive(serde::Deserialize)]
struct TruthWord {
    text: String,
    #[serde(rename = "box")]
    bounds: [f32; 4],
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (mut out, mut record, mut replay, mut only, mut keep_pdfs) = (None, None, None, None, false);
    let mut truth_boxes = None;
    let mut models = std::env::var_os("SAFHA_OCR_MODEL_DIR").map(PathBuf::from);
    let mut server =
        std::env::var_os("SAFHA_LLAMA_SERVER").map_or_else(|| PathBuf::from("llama-server"), PathBuf::from);
    let mut device = Device::Gpu;
    let mut manifest = repo.join("ocr-eval/out/manifest.json");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--out" => out = Some(PathBuf::from(value()?)),
            "--record" => record = Some(PathBuf::from(value()?)),
            "--replay" => replay = Some(PathBuf::from(value()?)),
            "--model-dir" => models = Some(PathBuf::from(value()?)),
            "--server" => server = PathBuf::from(value()?),
            "--cpu" => device = Device::Cpu,
            "--manifest" => manifest = PathBuf::from(value()?),
            "--only" => only = Some(value()?),
            "--keep-pdfs" => keep_pdfs = true,
            "--truth-boxes" => truth_boxes = Some(PathBuf::from(value()?)),
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    let mode = match (record, replay) {
        (Some(dir), None) => Mode::Record {
            dir,
            models: models.ok_or("--model-dir <dir> (or SAFHA_OCR_MODEL_DIR) is needed to record")?,
            server,
            device,
        },
        (None, Some(dir)) => Mode::Replay(dir),
        _ => return Err("give one of --record <dir> or --replay <dir>".into()),
    };
    Ok(Args { out: out.ok_or("--out <dir> is required")?, mode, manifest, only, keep_pdfs, truth_boxes })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(&args.manifest).map_err(|err| {
            format!("{}: {err} (run `node ocr-eval/generate.mjs`)", args.manifest.display())
        })?)?;
    let base = args.manifest.parent().unwrap_or(Path::new("."));
    let samples: Vec<(String, PathBuf, f32)> = manifest["samples"]
        .as_array()
        .ok_or("manifest has no samples")?
        .iter()
        .filter_map(|s| {
            Some((s["id"].as_str()?.to_owned(), base.join(s["image"].as_str()?), s["dpi"].as_f64()? as f32))
        })
        .filter(|(id, ..)| args.only.as_ref().is_none_or(|only| id.contains(only.as_str())))
        .collect();
    for dir in ["text", "layer"] {
        std::fs::create_dir_all(args.out.join(dir))?;
    }
    if args.keep_pdfs {
        std::fs::create_dir_all(args.out.join("pdf"))?;
    }
    let pdfium = Engine::global(&PdfiumSource::Auto)?;

    // The real engines, when recording.
    let (primary_model, partner_model) = (NuqtaModel::DEFAULT, NuqtaModel::PARTNER);
    let mut real: Option<(NuqtaEngine, NuqtaEngine, VisionReader)> = None;
    if let Mode::Record { dir, models, server, device } = &args.mode {
        std::fs::create_dir_all(dir)?;
        let start = |model: NuqtaModel, log: &str| -> Result<NuqtaEngine, Box<dyn std::error::Error>> {
            let mut config = ServerConfig::for_model(server, models, model);
            config.device = *device;
            config.log = Some(args.out.join(log));
            Ok(NuqtaEngine::start(&config, model)?)
        };
        let helper = vision_helper(&std::env::temp_dir().join("mirsam-helpers"))?;
        real = Some((
            start(primary_model, "llama-server.log")?,
            start(partner_model, "llama-server-partner.log")?,
            VisionReader::start(&helper)?,
        ));
        println!("engines ready; {} samples", samples.len());
    }

    let truth: std::collections::HashMap<String, Vec<TruthWord>> = match &args.truth_boxes {
        Some(path) => serde_json::from_str(&std::fs::read_to_string(path)?)?,
        None => Default::default(),
    };
    let mut boxes = BoxScore::default();
    let mut per_sample = serde_json::Map::new();
    let mut total = Duration::ZERO;
    for (id, image_path, dpi) in &samples {
        let image = OcrImage::decode_png(&std::fs::read(image_path)?)?;
        let started = Instant::now();
        let (result, draft) = match (&args.mode, real.as_mut()) {
            (Mode::Record { dir, .. }, Some((primary, partner, vision))) => {
                let recording = Arc::new(Mutex::new(Recording::default()));
                let primary = Recorder { inner: primary, recording: Arc::clone(&recording) };
                let partner = Recorder { inner: partner, recording: Arc::clone(&recording) };
                let draft = vision.read(&image)?;
                let result = QualityReader {
                    primary: &primary,
                    partner: Some(&partner),
                    third: None,
                    settings: Settings::default(),
                }
                .read_page(&image)?;
                let mut recording = recording.lock().map_err(|_| "recording lock")?.clone();
                recording.draft = Some(draft.clone());
                std::fs::write(dir.join(format!("{id}.json")), serde_json::to_string(&recording)? + "\n")?;
                (result, Some(draft))
            }
            (Mode::Replay(dir), _) => {
                let path = dir.join(format!("{id}.json"));
                let recording: Recording = serde_json::from_str(
                    &std::fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?,
                )?;
                let primary = Replay::new(&primary_model.name(), &recording);
                let partner = Replay::new(&partner_model.name(), &recording);
                let result = QualityReader {
                    primary: &primary,
                    partner: Some(&partner),
                    third: None,
                    settings: Settings::default(),
                }
                .read_page(&image)
                .map_err(|err| format!("{id}: {err}"))?;
                (result, recording.draft)
            }
            _ => return Err("no engines".into()),
        };
        let read = started.elapsed();
        total += read;
        std::fs::write(
            args.out.join("text").join(format!("{id}.txt")),
            format!("{}\n", result.text().trim()),
        )?;

        // The searchable PDF: the image as a page, the text layer written into it, read back.
        let pdf = image_pdf(&image, *dpi);
        let document = pdfium.open_bytes(pdf, None)?;
        let rendered = RenderedPage { image, scale: dpi / 72.0, placement: document.ocr_placement(0)? };
        let truth_words = truth.get(id);
        let page = final_page(0, &result, &rendered, draft.as_ref(), "bench", read);
        let (with_layer, _) = document.with_text_layers(std::slice::from_ref(&page.layer))?;
        let text = with_layer.page_text(0)?.text();
        std::fs::write(args.out.join("layer").join(format!("{id}.txt")), format!("{}\n", text.trim()))?;
        if args.keep_pdfs {
            std::fs::write(args.out.join("pdf").join(format!("{id}.pdf")), &*with_layer.bytes())?;
        }
        let by_width = page.words.iter().filter(|w| w.placed_by_width).count();
        let mut sample_boxes = BoxScore::default();
        if let Some(truth_words) = truth_words {
            sample_boxes.add(&page.words, truth_words, &rendered.image, rendered.scale);
            boxes.merge(&sample_boxes);
        }
        per_sample.insert(
            id.clone(),
            json!({
                "seconds": (read.as_secs_f64() * 10.0).round() / 10.0,
                "reads": result.reads,
                "words": page.words.len(),
                "placed_by_width": by_width,
                "needs_review": result.needs_review(),
                "boxes": if truth_words.is_some() { sample_boxes.json() } else { Value::Null },
            }),
        );
        println!(
            "{:7.1} s  {:2} reads  {:3} words  {by_width:3} by width  {id}",
            read.as_secs_f64(),
            result.reads,
            page.words.len()
        );
    }
    let record = json!({
        "engine": format!("pipeline:{}+{}", primary_model.name(), partner_model.name()),
        "mode": match args.mode { Mode::Record { .. } => "record", Mode::Replay(_) => "replay" },
        "samples": samples.len(),
        "seconds_per_page": if samples.is_empty() { 0.0 } else { (total.as_secs_f64() / samples.len() as f64 * 100.0).round() / 100.0 },
        "word_boxes": if truth.is_empty() { Value::Null } else { boxes.json() },
        "per_sample": per_sample,
    });
    if !truth.is_empty() {
        println!("word boxes: {}", boxes.json());
    }
    std::fs::write(args.out.join("run.json"), serde_json::to_string_pretty(&record)? + "\n")?;
    Ok(())
}

/// A one-page PDF showing `image` at `dpi`: a scan with no text.
fn image_pdf(image: &OcrImage, dpi: f32) -> Vec<u8> {
    let (w, h) = (image.width(), image.height());
    let (pw, ph) = (w as f32 * 72.0 / dpi, h as f32 * 72.0 / dpi);
    let data = miniz_oxide::deflate::compress_to_vec_zlib(image.rgb(), 6);
    let content = format!("q {pw:.3} 0 0 {ph:.3} 0 0 cm /Im0 Do Q\n");
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    let mut object = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    object(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
    object(&mut out, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    object(
        &mut out,
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.3} {ph:.3}] /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>"
        )
        .as_bytes(),
    );
    object(&mut out, format!("<< /Length {} >>\nstream\n{content}endstream", content.len()).as_bytes());
    let mut image_object = format!(
        "<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
        data.len()
    )
    .into_bytes();
    image_object.extend_from_slice(&data);
    image_object.extend_from_slice(b"\nendstream");
    object(&mut out, &image_object);
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f\r\n", offsets.len() + 1).as_bytes());
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n\r\n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1)
            .as_bytes(),
    );
    out
}

/// Word boxes against Chrome's layout (ADR 0031's measures).
#[derive(Default)]
struct BoxScore {
    pairs: usize,
    right_line: usize,
    iou_x: Vec<f32>,
}

impl BoxScore {
    fn add(
        &mut self,
        words: &[safha_core::ocr::jobs::OcrWord],
        truth: &[TruthWord],
        image: &OcrImage,
        scale: f32,
    ) {
        let ours: Vec<String> = words.iter().map(|w| norm(&w.text)).collect();
        let theirs: Vec<String> = truth.iter().map(|w| norm(&w.text)).collect();
        for (a, b) in common_pairs(&ours, &theirs) {
            let Some(bounds) = words[a].bounds else { continue };
            let mine = bounds.map(|v| v * scale);
            let true_box = tighten(truth[b].bounds, image);
            self.pairs += 1;
            let centre = (true_box[1] + true_box[3]) / 2.0;
            let pad = 0.1 * (mine[3] - mine[1]);
            self.right_line += usize::from(mine[1] - pad <= centre && centre <= mine[3] + pad);
            let overlap = (mine[2].min(true_box[2]) - mine[0].max(true_box[0])).max(0.0);
            let union = (mine[2] - mine[0]) + (true_box[2] - true_box[0]) - overlap;
            self.iou_x.push(if union > 0.0 { overlap / union } else { 0.0 });
        }
    }

    fn merge(&mut self, other: &BoxScore) {
        self.pairs += other.pairs;
        self.right_line += other.right_line;
        self.iou_x.extend(&other.iou_x);
    }

    fn json(&self) -> Value {
        let n = self.iou_x.len().max(1) as f32;
        let round = |v: f32| (f64::from(v) * 1000.0).round() / 1000.0;
        json!({
            "pairs": self.pairs,
            "right_line": round(self.right_line as f32 / self.pairs.max(1) as f32),
            "iou_x_mean": round(self.iou_x.iter().sum::<f32>() / n),
            "iou_x_ge_0.5": round(self.iou_x.iter().filter(|&&v| v >= 0.5).count() as f32 / n),
            "iou_x_ge_0.7": round(self.iou_x.iter().filter(|&&v| v >= 0.7).count() as f32 / n),
        })
    }
}

/// Text for pairing words: no tashkeel or tatweel, Western digits, lower case.
fn norm(text: &str) -> String {
    use unicode_normalization::UnicodeNormalization as _;
    text.nfkd()
        .filter(|c| !matches!(c, '\u{300}'..='\u{36F}' | '\u{610}'..='\u{61A}' | '\u{64B}'..='\u{65F}' | '\u{670}' | '\u{640}'))
        .map(|c| match c {
            '\u{660}'..='\u{669}' => char::from(b'0' + (c as u32 - 0x660) as u8),
            _ => c,
        })
        .flat_map(char::to_lowercase)
        .collect()
}

/// Index pairs of a longest common subsequence of equal words.
fn common_pairs(a: &[String], b: &[String]) -> Vec<(usize, usize)> {
    let (n, m) = (a.len(), b.len());
    let mut table = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i][j] =
                if a[i] == b[j] { table[i + 1][j + 1] + 1 } else { table[i + 1][j].max(table[i][j + 1]) };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < n && j < m {
        if a[i] == b[j] {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

/// A DOM box tightened to the dark pixels inside it (the spike's truth is ink-tight).
fn tighten(b: [f32; 4], image: &OcrImage) -> [f32; 4] {
    let (w, h) = (image.width() as usize, image.height() as usize);
    let clamp = |v: f32, max: usize| (v.round().max(0.0) as usize).min(max);
    let (x0, y0, x1, y1) = (clamp(b[0], w), clamp(b[1], h), clamp(b[2], w), clamp(b[3], h));
    let rgb = image.rgb();
    let dark = |x: usize, y: usize| {
        let at = (y * w + x) * 3;
        (u32::from(rgb[at]) + u32::from(rgb[at + 1]) + u32::from(rgb[at + 2])) < 3 * 140
    };
    let (mut l, mut t, mut r, mut bo) = (usize::MAX, usize::MAX, 0, 0);
    for y in y0..y1 {
        for x in x0..x1 {
            if dark(x, y) {
                l = l.min(x);
                r = r.max(x + 1);
                t = t.min(y);
                bo = bo.max(y + 1);
            }
        }
    }
    if l < r { [l as f32, t as f32, r as f32, bo as f32] } else { b }
}
