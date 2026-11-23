//! M3 check (throwaway), the open item in ADR 0005: when page objects are
//! deleted or replaced and PDFium regenerates the page's content stream, does
//! everything else on the page survive — clipping, z-order, marked content,
//! unusual operators?
//!
//!   cargo run -p spike-pdfium-regenerate     # corpus + stress page; see README.md
//!
//! The oracle: PDFium renders a page from its in-memory object list. So the page
//! rendered *after the edit but before saving* is what PDFium believes the page
//! is; the page rendered after regenerate → save → reload is what it wrote. Any
//! difference between the two is loss in regeneration. For edits that should
//! change nothing visible (`dirty`, `reinsert`), both must also equal the
//! untouched original.

#![allow(clippy::print_stdout, clippy::print_stderr)]

mod content;
mod stress;

use std::path::{Path, PathBuf};
use std::process::Command;

use pdfium_render::prelude::*;
use serde_json::{Value, json};

const SCALE: f32 = 2.0;
/// A channel difference above this counts as a visibly different pixel (as in ADR 0005).
const PIXEL_TOLERANCE: u8 = 32;
const MAX_PAGES: usize = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Edit {
    /// Mark one text object dirty without changing it: the whole stream is regenerated.
    Dirty,
    /// Remove the middle text object.
    Delete,
    /// Remove the middle text object and insert the same object back at its index.
    Reinsert,
    /// Remove the middle text object and insert an opaque box at its index.
    Replace,
    /// Append a box at the top of the z-order (the M0 spike's case).
    Append,
}

const EDITS: [Edit; 5] = [Edit::Dirty, Edit::Delete, Edit::Reinsert, Edit::Replace, Edit::Append];

struct Input {
    name: String,
    bytes: Vec<u8>,
    password: Option<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("verify") {
        return verify(&args[2..]);
    }

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize()?;
    let out_dir = root.join("spikes/out/pdfium-regenerate");
    std::fs::create_dir_all(&out_dir)?;

    let inputs = load_inputs(&root, &out_dir)?;
    let lib_dir = root.join("third_party/pdfium/mac-arm64/lib");
    let pdfium = Pdfium::new(
        Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(&lib_dir))
            .map_err(|e| format!("bind PDFium at {}: {e:?}", lib_dir.display()))?,
    );

    let mut rows: Vec<Value> = Vec::new();
    let mut verify_args: Vec<String> = Vec::new();
    let only = std::env::var("SPIKE_ONLY").ok();
    for input in &inputs {
        if only.as_ref().is_some_and(|o| !input.name.contains(o.as_str())) {
            continue;
        }
        let password = input.password.as_deref();
        let page_count = match pdfium.load_pdf_from_byte_slice(&input.bytes, password) {
            Ok(doc) => doc.pages().len() as usize,
            Err(error) => {
                rows.push(json!({ "file": input.name, "refused": format!("{error:?}") }));
                continue;
            }
        };
        let original_path = out_dir.join(format!("{}.original.pdf", input.name));
        std::fs::write(&original_path, &input.bytes)?;
        verify_args.push(with_password(&original_path, password));

        for page_index in 0..page_count.min(MAX_PAGES) {
            for edit in EDITS {
                let out_path =
                    out_dir.join(format!("{}.p{page_index}.{edit:?}.pdf", input.name).to_lowercase());
                match run_edit(&pdfium, input, page_index, edit, &out_path) {
                    Ok(Some(mut row)) => {
                        row["qpdf"] = json!(qpdf_check(&out_path, password));
                        verify_args.push(with_password(&out_path, password));
                        rows.push(row);
                    }
                    Ok(None) => {}
                    Err(error) => rows.push(json!({
                        "file": input.name, "page": page_index, "edit": format!("{edit:?}"),
                        "error": error.to_string(),
                    })),
                }
            }
        }
    }
    drop(pdfium);

    // Logical text through mirsam-core's own extractor, in a child process:
    // pdfium-render allows one binding per process.
    let child = Command::new(std::env::current_exe()?).arg("verify").args(&verify_args).output()?;
    if !child.status.success() {
        return Err(format!("verify failed: {}", String::from_utf8_lossy(&child.stderr)).into());
    }
    let texts: Value = serde_json::from_slice(&child.stdout)?;
    for row in &mut rows {
        let (Some(file), Some(page), Some(out)) =
            (row["file"].as_str(), row["page"].as_u64(), row["out"].as_str())
        else {
            continue;
        };
        let original = out_dir.join(format!("{file}.original.pdf"));
        let before = &texts[original.to_string_lossy().as_ref()][page as usize];
        let after = &texts[out][page as usize];
        row["mirsam_text_same"] = json!(before == after);
    }

    let report = json!({ "scale": SCALE, "pixel_tolerance": PIXEL_TOLERANCE, "rows": rows });
    std::fs::write(out_dir.join("report.json"), serde_json::to_string_pretty(&report)?)?;
    print_summary(&rows);
    Ok(())
}

fn with_password(path: &Path, password: Option<&str>) -> String {
    match password {
        Some(p) => format!("{}|{p}", path.display()),
        None => path.display().to_string(),
    }
}

fn load_inputs(root: &Path, out_dir: &Path) -> Result<Vec<Input>, Box<dyn std::error::Error>> {
    let manifest_path = root.join("fixtures/out/manifest.json");
    let manifest: Value = serde_json::from_slice(
        &std::fs::read(&manifest_path)
            .map_err(|e| format!("{}: {e} (run `node fixtures/generate.mjs`)", manifest_path.display()))?,
    )?;
    let mut inputs = Vec::new();
    for fixture in manifest["fixtures"].as_array().ok_or("manifest: no fixtures")? {
        let rel = fixture["path"].as_str().ok_or("fixture without path")?;
        inputs.push(Input {
            name: rel.trim_end_matches(".pdf").replace('/', "-"),
            bytes: std::fs::read(root.join("fixtures/out").join(rel))?,
            password: fixture["password"].as_str().map(str::to_string),
        });
    }
    let stress = stress::build();
    std::fs::write(out_dir.join("stress.pdf"), &stress)?;
    inputs.push(Input { name: "stress".into(), bytes: stress, password: None });
    Ok(inputs)
}

#[derive(Debug, Clone, PartialEq)]
struct ObjectInfo {
    kind: String,
    bounds: [f32; 4],
    /// Per clip path: segment count and the bounding box of its points.
    clip: Vec<(usize, [f32; 4])>,
}

struct Snapshot {
    objects: Vec<ObjectInfo>,
    pixels: Vec<u8>,
    size: (i32, i32),
    text: String,
}

fn snapshot(page: &PdfPage) -> Result<Snapshot, PdfiumError> {
    let mut objects = Vec::new();
    for object in page.objects().iter() {
        let b = object.bounds().map(|q| q.to_rect()).unwrap_or(PdfRect::ZERO);
        let clip = object
            .get_clip_path()
            .map(|clip| {
                clip.iter()
                    .map(|path| {
                        let mut bbox = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
                        for segment in path.iter() {
                            let (x, y) = (segment.x().value, segment.y().value);
                            bbox = [bbox[0].min(x), bbox[1].min(y), bbox[2].max(x), bbox[3].max(y)];
                        }
                        (path.len() as usize, bbox.map(round))
                    })
                    .collect()
            })
            .unwrap_or_default();
        objects.push(ObjectInfo {
            kind: format!("{:?}", object.object_type()),
            bounds: [b.left().value, b.bottom().value, b.right().value, b.top().value].map(round),
            clip,
        });
    }
    let bitmap = page.render_with_config(&PdfRenderConfig::new().scale_page_by_factor(SCALE))?;
    Ok(Snapshot {
        objects,
        size: (bitmap.width(), bitmap.height()),
        pixels: bitmap.as_raw_bytes(),
        text: page.text()?.all(),
    })
}

fn round(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

/// (pixels differing by more than the tolerance, largest channel difference).
fn diff(a: &Snapshot, b: &Snapshot) -> (usize, u8) {
    if a.size != b.size {
        return (usize::MAX, 255);
    }
    let mut count = 0;
    let mut max = 0u8;
    for (pa, pb) in a.pixels.as_chunks::<4>().0.iter().zip(b.pixels.as_chunks::<4>().0) {
        let d = pa.iter().zip(pb).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
        max = max.max(d);
        if d > PIXEL_TOLERANCE {
            count += 1;
        }
    }
    (count, max)
}

fn compare_objects(expected: &[ObjectInfo], actual: &[ObjectInfo]) -> Value {
    if expected.len() != actual.len() {
        return json!(format!("object count {} -> {}", expected.len(), actual.len()));
    }
    let mut problems = Vec::new();
    for (i, (e, a)) in expected.iter().zip(actual).enumerate() {
        if e.kind != a.kind {
            problems.push(format!("#{i} type {} -> {}", e.kind, a.kind));
        } else if e.bounds.iter().zip(&a.bounds).any(|(x, y)| (x - y).abs() > 0.2) {
            problems.push(format!("#{i} {} bounds {:?} -> {:?}", e.kind, e.bounds, a.bounds));
        } else if e.clip.len() != a.clip.len()
            || e.clip
                .iter()
                .zip(&a.clip)
                .any(|(x, y)| x.0 != y.0 || x.1.iter().zip(&y.1).any(|(p, q)| (p - q).abs() > 0.2))
        {
            problems.push(format!("#{i} {} clip {:?} -> {:?}", e.kind, e.clip, a.clip));
        }
    }
    if problems.is_empty() { json!("same") } else { json!(problems) }
}

fn run_edit(
    pdfium: &Pdfium,
    input: &Input,
    page_index: usize,
    edit: Edit,
    out_path: &Path,
) -> Result<Option<Value>, Box<dyn std::error::Error>> {
    let password = input.password.as_deref();
    let document = pdfium.load_pdf_from_byte_slice(&input.bytes, password)?;
    let mut page = document.pages().get(page_index as i32)?;
    page.set_content_regeneration_strategy(PdfPageContentRegenerationStrategy::Manual);

    let original = snapshot(&page)?;
    let text_indices: Vec<usize> = page
        .objects()
        .iter()
        .enumerate()
        .filter(|(_, o)| o.object_type() == PdfPageObjectType::Text)
        .map(|(i, _)| i)
        .collect();
    let Some(&target) = text_indices.get(text_indices.len() / 2) else {
        return Ok(None);
    };
    let target_bounds = page.objects().get(target as PdfPageObjectIndex)?.bounds()?.to_rect();
    let target_text = page
        .objects()
        .get(target as PdfPageObjectIndex)?
        .as_text_object()
        .map(|t| t.text())
        .unwrap_or_default();
    let magenta = Some(PdfColor::new(255, 0, 255, 255));
    let index = target as PdfPageObjectIndex;

    match edit {
        Edit::Dirty => {
            let mut object = page.objects().get(index)?;
            object.apply_matrix(PdfMatrix::IDENTITY)?;
        }
        Edit::Delete => {
            page.objects_mut().remove_object_at_index(index)?;
        }
        Edit::Reinsert => {
            let object = page.objects_mut().remove_object_at_index(index)?;
            page.objects_mut().insert_object_at_index(index, object)?;
        }
        Edit::Replace => {
            page.objects_mut().remove_object_at_index(index)?;
            let rect = PdfPagePathObject::new_rect(&document, target_bounds, None, None, magenta)?;
            page.objects_mut().insert_object_at_index(index, PdfPageObject::from(rect))?;
        }
        Edit::Append => {
            let rect = PdfPagePathObject::new_rect(&document, target_bounds, None, None, magenta)?;
            page.objects_mut().add_path_object(rect)?;
        }
    }
    let intended = snapshot(&page)?;
    page.regenerate_content()?;
    drop(page);
    let bytes = document.save_to_bytes()?;
    drop(document);
    std::fs::write(out_path, &bytes)?;

    let reloaded_doc = pdfium.load_pdf_from_byte_slice(&bytes, password)?;
    let reloaded = snapshot(&reloaded_doc.pages().get(page_index as i32)?)?;

    let content_before = content::inspect(&input.bytes, password, page_index);
    let content_after = content::inspect(&bytes, password, page_index);
    let content = match (&content_before, &content_after) {
        (Ok(b), Ok(a)) => {
            let lost: Vec<&String> = b.operators.keys().filter(|k| !a.operators.contains_key(*k)).collect();
            let added: Vec<&String> = a.operators.keys().filter(|k| !b.operators.contains_key(*k)).collect();
            let missing_props: Vec<&String> =
                a.property_names.iter().filter(|(_, defined)| !**defined).map(|(n, _)| n).collect();
            json!({
                "operators_lost": lost,
                "operators_added": added,
                "mark_runs_same": b.mark_runs == a.mark_runs,
                "mark_runs_before": b.mark_runs.len(),
                "mark_runs_after": a.mark_runs.len(),
                "mcids_lost": b.mcids.difference(&a.mcids).collect::<Vec<_>>(),
                "undefined_property_names": missing_props,
                "streams": [b.content_streams, a.content_streams],
                "bytes": [b.decoded_bytes, a.decoded_bytes],
                "mark_runs_diff": if b.mark_runs == a.mark_runs { Value::Null } else {
                    json!({ "before": b.mark_runs, "after": a.mark_runs })
                },
            })
        }
        (b, a) => json!({ "error": format!("{:?} / {:?}", b.as_ref().err(), a.as_ref().err()) }),
    };

    if std::env::var("SPIKE_DUMP").is_ok_and(|v| out_path.to_string_lossy().contains(&v)) {
        for (tag, shot) in [("original", &original), ("intended", &intended), ("reloaded", &reloaded)] {
            dump_ppm(&out_path.with_extension(format!("{tag}.ppm")), shot)?;
        }
    }
    let (regen_px, regen_max) = diff(&intended, &reloaded);
    let (orig_px, orig_max) = diff(&original, &reloaded);
    Ok(Some(json!({
        "file": input.name,
        "page": page_index,
        "edit": format!("{edit:?}"),
        "out": out_path.display().to_string(),
        "target": { "index": target, "of": original.objects.len(), "text": target_text },
        // Loss in regeneration: what PDFium meant vs what it wrote.
        "regen_pixels": regen_px,
        "regen_max": regen_max,
        // Against the untouched page (should be 0 for dirty and reinsert).
        "vs_original_pixels": orig_px,
        "vs_original_max": orig_max,
        "objects": compare_objects(&intended.objects, &reloaded.objects),
        "pdfium_text_same": intended.text == reloaded.text,
        "content": content,
    })))
}

fn dump_ppm(path: &Path, shot: &Snapshot) -> std::io::Result<()> {
    let mut out = format!("P6 {} {} 255\n", shot.size.0, shot.size.1).into_bytes();
    for px in shot.pixels.as_chunks::<4>().0 {
        out.extend_from_slice(&[px[2], px[1], px[0]]);
    }
    std::fs::write(path, out)
}

fn qpdf_check(path: &Path, password: Option<&str>) -> String {
    let mut cmd = Command::new("qpdf");
    if let Some(p) = password {
        cmd.arg(format!("--password={p}"));
    }
    match cmd.arg("--check").arg(path).output() {
        Ok(out) if out.status.success() => "ok".into(),
        Ok(out) => {
            let text =
                String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
            format!("exit {:?}: {}", out.status.code(), text.lines().last().unwrap_or(""))
        }
        Err(e) => format!("not run: {e}"),
    }
}

/// Child process: logical page text through `mirsam-core`.
fn verify(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    use mirsam_core::{Engine, PdfiumSource};
    let engine = Engine::global(&PdfiumSource::Auto)?;
    let mut out = serde_json::Map::new();
    for arg in args {
        let (path, password) = match arg.rsplit_once('|') {
            Some((p, pw)) => (p, Some(pw)),
            None => (arg.as_str(), None),
        };
        let doc = engine.open_file(Path::new(path), password)?;
        let mut pages = Vec::new();
        for index in 0..doc.page_count().min(MAX_PAGES) {
            pages.push(Value::String(doc.page_text(index)?.text()));
        }
        out.insert(path.to_string(), Value::Array(pages));
    }
    println!("{}", Value::Object(out));
    Ok(())
}

fn print_summary(rows: &[Value]) {
    println!(
        "{:<34} {:>2} {:<8} {:>9} {:>9} {:<8} {:<5} {:<5} {:<5} {:<24} qpdf",
        "file", "pg", "edit", "regen px", "orig px", "objects", "ptext", "stext", "marks", "ops lost"
    );
    for row in rows {
        if let Some(refused) = row.get("refused") {
            println!("{:<34} refused by PDFium: {refused}", row["file"].as_str().unwrap_or("?"));
            continue;
        }
        if let Some(error) = row.get("error") {
            println!(
                "{:<34} {:>2} {:<8} ERROR {error}",
                row["file"].as_str().unwrap_or("?"),
                row["page"],
                row["edit"]
            );
            continue;
        }
        let objects = if row["objects"] == "same" { "same".to_string() } else { "DIFF".to_string() };
        println!(
            "{:<34} {:>2} {:<8} {:>9} {:>9} {:<8} {:<5} {:<5} {:<5} {:<24} {}",
            row["file"].as_str().unwrap_or("?"),
            row["page"],
            row["edit"].as_str().unwrap_or("?"),
            row["regen_pixels"],
            row["vs_original_pixels"],
            objects,
            row["pdfium_text_same"],
            row["mirsam_text_same"],
            row["content"]["mark_runs_same"],
            row["content"]["operators_lost"].to_string(),
            row["qpdf"].as_str().unwrap_or("?"),
        );
    }
}
