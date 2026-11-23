//! Second experiment: put one shaped line on the first page of PDFs we did not
//! write, through both routes, and measure what happens to what was already there.
//!
//! Sources are the repo's generated fixtures (`fixtures/out`, git-ignored); a case
//! is skipped when its file is missing.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use pdfium_render::prelude::Pdfium;

use crate::fontinfo::FontInfo;
use crate::pdfparts::{ActualText, ToUnicodePolicy};
use crate::route_b::{Options, Placement, Positioning};
use crate::shape::ShapedLine;
use crate::{route_a, route_b, verify};

/// (key, path under fixtures/out, password)
const CASES: [(&str, &str, Option<&str>); 7] = [
    ("chrome-vocalized", "chrome/ar-vocalized.pdf", None),
    ("chrome-table", "chrome/table-bilingual.pdf", None),
    ("coretext-vocalized", "coretext/ar-vocalized.pdf", None),
    ("linearized", "derived/linearized.pdf", None),
    // Test data from fixtures/generate.mjs, not a secret.
    ("encrypted", "derived/encrypted.pdf", Some("mirsam-test")),
    ("truncated", "derived/truncated.pdf", None),
    ("1000-pages", "derived/bilingual-1000-pages.pdf", None),
];

const FONT_SIZE: f32 = 14.0;
const BASELINE: f64 = 30.0;
/// Rows of the page (in points, from the bottom) the new line may touch.
const BAND: f32 = 70.0;

pub struct Outcome {
    pub case: &'static str,
    pub route: &'static str,
    pub stem: String,
    pub password: Option<&'static str>,
    pub source_bytes: usize,
    pub page_size: Option<(f32, f32)>,
    /// Output size, or why the route could not produce one.
    pub result: Result<usize, String>,
    pub millis: u128,
    pub objects: usize,
    pub qpdf: String,
    /// The source page's decoded content is still in the output page, byte for byte.
    pub content_kept: Option<bool>,
    pub content_streams: Option<usize>,
    /// Share of pixels above the new line that changed on page 1.
    pub pixels_changed: Option<f64>,
    /// Pixels in the band of the new line that changed (it should have appeared).
    pub band_pixels_changed: Option<usize>,
    /// What became of the source page's extracted text in the output's.
    pub pdfium_text_kept: Option<TextKept>,
    pub pdfkit_text_kept: Option<TextKept>,
    /// `N 0 obj` definitions in the output, and how many of them come after a
    /// verbatim copy of the source (only when the output starts with one).
    pub objects_written: Option<usize>,
    pub objects_after_source: Option<usize>,
    /// `qpdf --requires-password` says the output needs one.
    pub output_encrypted: Option<bool>,
    /// PDFium (through mirsam-core) could open the output.
    pub pdfium_opens_output: Option<bool>,
    /// PDFium could open the source.
    pub pdfium_opens_source: Option<bool>,
    /// Start of the output's page 1 text when there is no source text to compare with.
    pub pdfium_excerpt: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextKept {
    /// Still there as one contiguous piece.
    Verbatim,
    /// Every line is still there, but the extractor now returns them in another order.
    Reordered,
    Changed,
}

impl TextKept {
    pub fn label(self) -> &'static str {
        match self {
            TextKept::Verbatim => "yes",
            TextKept::Reordered => "lines reordered",
            TextKept::Changed => "NO",
        }
    }
}

/// One source and the paths written for it.
pub struct Case {
    pub key: &'static str,
    pub source_stem: String,
}

pub fn run(
    pdfium: &Pdfium,
    root: &Path,
    out_dir: &Path,
    font: &FontInfo,
    line: &ShapedLine,
    verify_args: &mut Vec<String>,
) -> Result<(Vec<Case>, Vec<Outcome>), Box<dyn std::error::Error>> {
    let mut cases = Vec::new();
    let mut outcomes = Vec::new();
    let arg = |path: &PathBuf, password: Option<&str>| match password {
        Some(password) => format!("{}|{password}", path.display()),
        None => path.display().to_string(),
    };

    for (key, relative, password) in CASES {
        let source_path = root.join("fixtures/out").join(relative);
        let Ok(source) = std::fs::read(&source_path) else { continue };
        let source_stem = format!("insert-src-{key}");
        let copy = out_dir.join(format!("{source_stem}.pdf"));
        std::fs::write(&copy, &source)?;
        verify_args.push(arg(&copy, password));
        cases.push(Case { key, source_stem });

        let page_size = route_a::first_page_size(pdfium, &source, password).ok();
        // Without a page size (PDFium could not open the file) assume A4 for route B.
        let (page_w, _) = page_size.unwrap_or((595.0, 842.0));
        let width = line.width as f64 / font.upem as f64 * FONT_SIZE as f64;
        let x = page_w as f64 - 50.0 - width;

        type Attempt<'a> = Box<dyn Fn() -> Result<(Vec<u8>, usize), Box<dyn std::error::Error>> + 'a>;
        let source_ref = &source;
        let attempts: [(&'static str, &str, Attempt); 3] = [
            (
                "A4, full save",
                "a",
                Box::new(move || {
                    route_a::insert_into(
                        pdfium,
                        source_ref,
                        password,
                        font,
                        line,
                        FONT_SIZE,
                        (x, BASELINE),
                        false,
                    )
                    .map(|(bytes, stats)| (bytes, stats.objects.iter().sum()))
                }),
            ),
            (
                "A4, incremental save",
                "a-inc",
                Box::new(move || {
                    route_a::insert_into(
                        pdfium,
                        source_ref,
                        password,
                        font,
                        line,
                        FONT_SIZE,
                        (x, BASELINE),
                        true,
                    )
                    .map(|(bytes, stats)| (bytes, stats.objects.iter().sum()))
                }),
            ),
            (
                "B, lopdf load + save",
                "b",
                Box::new(move || {
                    let options = Options {
                        to_unicode: ToUnicodePolicy::Source,
                        actual_text: ActualText::None,
                        positioning: Positioning::Tj,
                        reversed_chars: false,
                        probe: None,
                    };
                    let place =
                        Placement { size: FONT_SIZE as f64, x, baseline: BASELINE, resource: "MirsamF1" };
                    route_b::insert_into(source_ref, password, font, line, options, &place)
                        .map(|bytes| (bytes, 1))
                }),
            ),
        ];

        for (route, suffix, attempt) in attempts {
            let stem = format!("insert-{suffix}-{key}");
            let started = Instant::now();
            let attempt = attempt();
            let millis = started.elapsed().as_millis();
            let mut outcome = Outcome {
                case: key,
                route,
                stem: stem.clone(),
                password,
                source_bytes: source.len(),
                page_size,
                result: Err(String::new()),
                millis,
                objects: 0,
                qpdf: "n/a".into(),
                content_kept: None,
                content_streams: None,
                pixels_changed: None,
                band_pixels_changed: None,
                pdfium_text_kept: None,
                pdfkit_text_kept: None,
                objects_written: None,
                objects_after_source: None,
                output_encrypted: None,
                pdfium_opens_output: None,
                pdfium_opens_source: None,
                pdfium_excerpt: None,
            };
            match attempt {
                Ok((bytes, objects)) => {
                    let path = out_dir.join(format!("{stem}.pdf"));
                    std::fs::write(&path, &bytes)?;
                    outcome.result = Ok(bytes.len());
                    outcome.objects = objects;
                    outcome.qpdf = qpdf_check(&path, password)?;
                    let definitions = |data: &[u8]| data.windows(6).filter(|w| w == b" 0 obj").count();
                    outcome.objects_written = Some(definitions(&bytes));
                    outcome.objects_after_source =
                        bytes.starts_with(&source).then(|| definitions(&bytes[source.len()..]));
                    // Exit 0: a password is needed. Exit 2: not encrypted.
                    let needs = Command::new("qpdf").arg("--requires-password").arg(&path).status()?;
                    outcome.output_encrypted = needs.code().map(|code| code != 2);
                    let (kept, streams) = content_kept(&source, &bytes, password);
                    outcome.content_kept = kept;
                    outcome.content_streams = streams;
                    verify_args.push(arg(&path, password));
                }
                Err(error) => outcome.result = Err(error.to_string()),
            }
            outcomes.push(outcome);
        }
    }
    Ok((cases, outcomes))
}

fn qpdf_check(path: &Path, password: Option<&str>) -> Result<String, Box<dyn std::error::Error>> {
    let mut command = Command::new("qpdf");
    if let Some(password) = password {
        command.arg(format!("--password={password}"));
    }
    let check = command.arg("--check").arg(path).output()?;
    Ok(match check.status.code() {
        Some(0) => "pass".into(),
        code => {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&check.stdout),
                String::from_utf8_lossy(&check.stderr)
            );
            let first = text.lines().find(|l| l.contains("WARNING") || l.to_lowercase().contains("error"));
            // qpdf prefixes messages with the full path; keep the file name only.
            let message =
                first.unwrap_or("").replace(&format!("{}", path.display()), "").replace("WARNING: ", "");
            format!("warning (exit {}): {}", code.unwrap_or(-1), message.trim())
        }
    })
}

/// Reads both files with lopdf and compares the first page's decoded content.
fn content_kept(source: &[u8], output: &[u8], password: Option<&str>) -> (Option<bool>, Option<usize>) {
    let load = |bytes: &[u8]| {
        lopdf::Document::load_mem(bytes).ok().or_else(|| {
            password.and_then(|p| {
                lopdf::Document::load_mem_with_options(bytes, lopdf::LoadOptions::with_password(p)).ok()
            })
        })
    };
    let first_page = |doc: &lopdf::Document| {
        let page = *doc.get_pages().get(&1)?;
        Some((doc.get_page_content(page), doc.get_page_contents(page).len()))
    };
    let Some(after) = load(output).as_ref().and_then(first_page) else { return (None, None) };
    let Some(before) = load(source).as_ref().and_then(first_page) else { return (None, Some(after.1)) };
    let kept = !before.0.is_empty() && after.0.windows(before.0.len()).any(|w| w == before.0.as_slice());
    (Some(kept), Some(after.1))
}

/// Fills in what needs the rendered pages and the extracted text.
pub fn finish(
    out_dir: &Path,
    cases: &[Case],
    outcomes: &mut [Outcome],
    text: impl Fn(&str, &str) -> Option<String>,
) {
    let normal = |s: String| s.replace("\r\n", "\n").trim().to_string();
    for outcome in outcomes.iter_mut().filter(|o| o.result.is_ok()) {
        let Some(case) = cases.iter().find(|c| c.key == outcome.case) else { continue };
        let source_text = text("pdfium", &case.source_stem);
        let output_text = text("pdfium", &outcome.stem);
        outcome.pdfium_opens_source = Some(source_text.is_some());
        outcome.pdfium_opens_output = Some(output_text.is_some());
        if source_text.is_none() {
            outcome.pdfium_excerpt =
                output_text.map(|t| t.replace("\r\n", " / ").chars().skip(60).take(90).collect());
        }
        for (extractor, slot) in
            [("pdfium", &mut outcome.pdfium_text_kept), ("pdfkit", &mut outcome.pdfkit_text_kept)]
        {
            if let (Some(before), Some(after)) =
                (text(extractor, &case.source_stem), text(extractor, &outcome.stem))
            {
                let (before, after) = (normal(before), normal(after));
                let mut remaining: Vec<&str> = after.lines().collect();
                let all_lines_present = before.lines().all(|line| {
                    remaining.iter().position(|l| *l == line).map(|at| remaining.swap_remove(at)).is_some()
                });
                *slot = Some(if !before.is_empty() && after.contains(&before) {
                    TextKept::Verbatim
                } else if all_lines_present {
                    TextKept::Reordered
                } else {
                    TextKept::Changed
                });
            }
        }
        let (Some((page_w, page_h)), Ok(before), Ok(after)) = (
            outcome.page_size,
            std::fs::read(out_dir.join(format!("{}.p0.rgba", case.source_stem))),
            std::fs::read(out_dir.join(format!("{}.p0.rgba", outcome.stem))),
        ) else {
            continue;
        };
        let width = (page_w * verify::SCALE).ceil() as usize;
        if before.len() != after.len() || width == 0 {
            continue;
        }
        let band_start = ((page_h - BAND) * verify::SCALE) as usize * width * 4;
        let differing = |a: &[u8], b: &[u8]| {
            a.chunks(4)
                .zip(b.chunks(4))
                .filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > 32))
                .count()
        };
        let split = band_start.min(before.len());
        outcome.pixels_changed =
            Some(differing(&before[..split], &after[..split]) as f64 / (split / 4).max(1) as f64);
        outcome.band_pixels_changed = Some(differing(&before[split..], &after[split..]));
    }
}
