//! M0 spike (throwaway): can PDFium's edit API write pre-shaped Arabic, or do we
//! need our own content-stream writer? See README.md for the findings.
//!
//!   cargo run -p spike-pdfium-edit            # regenerate everything
//!   cargo run -p spike-pdfium-edit -- shape   # dump the shaper's clusters

mod fontinfo;
mod insert;
mod inspect;
mod pdfparts;
mod report;
mod route_a;
mod route_b;
mod shape;
mod verify;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use pdfium_render::prelude::Pdfium;

use fontinfo::FontInfo;
use pdfparts::{ActualText, Embed, ToUnicodePolicy};
use route_a::{ActualTextEncoding, Mode};
use route_b::{Options, Positioning, Probe};
use shape::ShapedLine;

pub const INPUTS: [(&str, &str); 4] = [
    ("lam-alef", "لا بأس؛ لأن الأولاد الآن في الملعب، فلا داعي للقلق."),
    ("vocalized", "قَالَ الْمُعَلِّمُ: إِنَّ الْعِلْمَ نُورٌ، وَإِنَّ الْقِرَاءَةَ مِفْتَاحُ الْمَعْرِفَةِ."),
    ("mixed", "اشتريت 3 أجهزة iPhone 15 Pro بسعر 4,500 ريال (شاملة الضريبة)."),
    ("latin", "The office staff finished the final draft efficiently."),
];
pub const FONTS: [(&str, &str); 2] =
    [("amiri", "Amiri-Regular.ttf"), ("plex", "IBMPlexSansArabic-Regular.ttf")];

/// One page per input line.
pub struct Layout {
    pub page_w: f32,
    pub page_h: f32,
    pub font_size: f32,
    pub margin: f32,
    pub baseline: f32,
}

pub const LAYOUT: Layout =
    Layout { page_w: 520.0, page_h: 70.0, font_size: 18.0, margin: 20.0, baseline: 27.0 };

impl Layout {
    /// x of the line origin: RTL paragraphs are right-aligned.
    pub fn line_x(&self, font: &FontInfo, line: &ShapedLine) -> f64 {
        let width = line.width as f64 / font.upem as f64 * self.font_size as f64;
        if line.base_rtl { (self.page_w - self.margin) as f64 - width } else { self.margin as f64 }
    }
}

#[derive(Clone, Copy)]
pub enum Variant {
    A(Mode),
    /// `Mode::CidType2` without /ActualText, through pdfium-render's safe API.
    ASafe,
    B(Options),
}

pub struct Spec {
    pub key: &'static str,
    pub what: &'static str,
    pub variant: Variant,
    /// Shown in the per-input comparison sheets (variants that render differently).
    pub in_sheet: bool,
    /// An /ActualText marker experiment: reported apart from the round-trip tables.
    pub probe: bool,
}

pub fn variants() -> Vec<Spec> {
    use ActualText as At;
    let (blob, utf8) = (ActualTextEncoding::Utf16Blob, ActualTextEncoding::Utf8String);
    let (tj, td) = (Positioning::Tj, Positioning::Td);
    let (src, cmap) = (ToUnicodePolicy::Source, ToUnicodePolicy::Cmap);
    let a4 = |actual_text, encoding| Variant::A(Mode::CidType2 { actual_text, encoding });
    let b = |to_unicode, positioning, actual_text, reversed_chars| {
        Variant::B(Options { to_unicode, actual_text, positioning, reversed_chars, probe: None })
    };
    let probe = |actual_text, probe| {
        Variant::B(Options {
            to_unicode: src,
            actual_text,
            positioning: tj,
            reversed_chars: false,
            probe: Some(probe),
        })
    };
    let spec = |key, what, variant, in_sheet| Spec { key, what, variant, in_sheet, probe: false };
    vec![
        // Route A: PDFium edit API.
        spec(
            "a1-settext",
            "FPDFText_LoadFont(cid) + FPDFText_SetText(logical string)",
            Variant::A(Mode::SetText),
            true,
        ),
        spec(
            "a2-charcodes",
            "FPDFText_LoadFont(cid) + FPDFText_SetCharcodes(shaped glyph IDs), no positions",
            Variant::A(Mode::Charcodes),
            true,
        ),
        spec(
            "a3-positions",
            "A2 + FPDFText_SetPositions (x) + one text object per y offset",
            Variant::A(Mode::CharcodesPositioned),
            true,
        ),
        spec(
            "a4-cid2",
            "FPDFText_LoadCidType2Font(our subset, our ToUnicode, CIDToGIDMap) + positions as A3",
            a4(At::None, blob),
            true,
        ),
        spec(
            "a4-cid2-at",
            "A4 + /Span /ActualText per base+marks group (FPDFPageObjMark_SetBlobParam, UTF-16BE)",
            a4(At::Grapheme, blob),
            false,
        ),
        spec(
            "a4-cid2-at8",
            "A4 + the same spans via FPDFPageObjMark_SetStringParam (UTF-8 with BOM)",
            a4(At::Grapheme, utf8),
            false,
        ),
        spec(
            "a4-safe",
            "a4-cid2 through pdfium-render's safe wrappers only (load_cid_type2_from_bytes, set_charcodes, set_positions)",
            Variant::ASafe,
            false,
        ),
        // Route B: own writer. ToUnicode = source text unless "cmap".
        spec(
            "b-src",
            "own writer: TJ numbers + Ts for offsets, ToUnicode only",
            b(src, tj, At::None, false),
            true,
        ),
        spec("b-src-rc", "b-src + /ReversedChars around RTL runs", b(src, tj, At::None, true), false),
        spec(
            "b-src-td",
            "b-src but Td before every repositioned glyph (Skia style)",
            b(src, td, At::None, false),
            true,
        ),
        spec(
            "b-src-cluster",
            "b-src + /ActualText on clusters ToUnicode cannot reproduce",
            b(src, tj, At::Cluster, false),
            false,
        ),
        spec(
            "b-src-grapheme",
            "b-src + /ActualText per base+marks group",
            b(src, tj, At::Grapheme, false),
            false,
        ),
        spec("b-src-word", "b-src + /ActualText per word", b(src, tj, At::Word, false), false),
        spec("b-src-line", "b-src + one /ActualText span per line", b(src, tj, At::Line, false), false),
        spec(
            "b-src-tuned",
            "b-src + /ActualText bent to PDFium's extractor (pre-reversed ligatures, pre-mirrored brackets): not spec-conformant",
            b(src, tj, At::PdfiumTuned, false),
            false,
        ),
        spec(
            "b-cmap",
            "own writer, ToUnicode from the font's cmap where the glyph is encoded (PDFium/Skia convention)",
            b(cmap, tj, At::None, false),
            false,
        ),
        // Markers instead of real text in /ActualText: who reads it, and when?
        Spec {
            probe: true,
            ..spec(
                "probe-line-short",
                "one span per line, /ActualText (PROBE)",
                probe(At::Line, Probe::Short),
                false,
            )
        },
        Spec {
            probe: true,
            ..spec(
                "probe-line-perglyph",
                "one span per line, /ActualText = one x per glyph",
                probe(At::Line, Probe::OnePerGlyph),
                false,
            )
        },
        Spec {
            probe: true,
            ..spec(
                "probe-word-short",
                "one span per word, /ActualText (PROBE)",
                probe(At::Word, Probe::Short),
                false,
            )
        },
        Spec {
            probe: true,
            ..spec(
                "probe-cluster-short",
                "one span per cluster, /ActualText (PROBE)",
                probe(At::EveryCluster, Probe::Short),
                false,
            )
        },
        Spec {
            probe: true,
            ..spec(
                "probe-cluster-perglyph",
                "one span per cluster, /ActualText = one x per glyph",
                probe(At::EveryCluster, Probe::OnePerGlyph),
                false,
            )
        },
        Spec {
            probe: true,
            ..spec(
                "probe-word-perglyph",
                "one span per word, /ActualText = one x per glyph",
                probe(At::Word, Probe::OnePerGlyph),
                false,
            )
        },
    ]
}

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repo root")
}

/// What one generated PDF measured out to.
pub struct Output {
    pub probe: bool,
    pub spec_key: &'static str,
    pub what: &'static str,
    pub font_key: &'static str,
    pub stem: String,
    pub font_file_size: usize,
    pub inspection: inspect::Inspection,
    pub qpdf: String,
    pub a_stats: Option<route_a::Stats>,
    /// Glyphs used by our shaper, how many PDFium's/our ToUnicode covers, and how
    /// many of those map to the text the shaper says they stand for.
    pub tounicode_used: usize,
    pub tounicode_mapped: usize,
    pub tounicode_correct: usize,
    pub tounicode_gaps: Vec<String>,
    pub pdfium_text: Vec<String>,
    pub pdfkit_text: Vec<String>,
    /// Per page: share of pixels that differ visibly from `b-src` in the same font.
    pub render_diff: Vec<f64>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("verify") => verify::run(&args[1..]),
        Some("shape") => dump_shaping(),
        _ => generate_all(),
    }
}

fn dump_shaping() -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    for (fkey, ffile) in FONTS {
        let font = FontInfo::load(&root.join("assets/fonts").join(ffile))?;
        println!("== {fkey}: upem {} glyphs {}", font.upem, font.num_glyphs);
        for (ikey, text) in INPUTS {
            let line = shape::shape_line(&font, text)?;
            println!(
                "-- {ikey}: base_rtl {} runs {} glyphs {} clusters {} width {}",
                line.base_rtl,
                line.run_count,
                line.glyphs.len(),
                line.clusters.len(),
                line.width
            );
            for c in &line.clusters {
                let glyphs: Vec<String> = c
                    .glyphs
                    .clone()
                    .map(|i| {
                        let g = &line.glyphs[i];
                        format!(
                            "{}#{} adv {} off {},{}",
                            font.glyph_name(g.gid),
                            g.gid,
                            g.x_advance,
                            g.x_offset,
                            g.y_offset
                        )
                    })
                    .collect();
                println!(
                    "   run {} {} {:?} {} <- {}",
                    c.run,
                    if c.rtl { "RTL" } else { "LTR" },
                    c.text,
                    line.cluster_text(c).escape_unicode(),
                    glyphs.join(" | ")
                );
            }
        }
    }
    Ok(())
}

fn generate_all() -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    let out_dir = root.join("spikes/out/pdfium-edit");
    std::fs::create_dir_all(out_dir.join("qdf"))?;

    // Route A needs the edit API, so bind PDFium directly (mirsam-core does not expose it).
    let lib_dir = root.join("third_party/pdfium/mac-arm64/lib");
    let pdfium = Pdfium::new(
        Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(&lib_dir))
            .map_err(|e| format!("bind PDFium at {}: {e:?}", lib_dir.display()))?,
    );

    let specs = variants();
    let mut outputs: Vec<Output> = Vec::new();
    let mut pdf_paths: Vec<PathBuf> = Vec::new();

    for (font_key, font_file) in FONTS {
        let font = FontInfo::load(&root.join("assets/fonts").join(font_file))?;
        let lines: Vec<ShapedLine> =
            INPUTS.iter().map(|(_, text)| shape::shape_line(&font, text)).collect::<Result<_, _>>()?;
        let embed = Embed::build(&font, &lines, ToUnicodePolicy::Source)?;

        for spec in &specs {
            let stem = format!("{}-{}", spec.key, font_key);
            let (bytes, a_stats) = match spec.variant {
                Variant::A(mode) => {
                    let (bytes, stats) = route_a::write(&pdfium, &font, &lines, mode, &LAYOUT)?;
                    (bytes, Some(stats))
                }
                Variant::ASafe => {
                    let (bytes, stats) = route_a::write_safe(&pdfium, &font, &lines, &LAYOUT)?;
                    (bytes, Some(stats))
                }
                Variant::B(options) => (route_b::write(&font, &lines, options, &LAYOUT)?, None),
            };
            let path = out_dir.join(format!("{stem}.pdf"));
            std::fs::write(&path, &bytes)?;
            let inspection = inspect::inspect(&bytes)?;

            // Charcode of a glyph in this file: the subset's CID where we supplied
            // the font program, the original glyph ID where PDFium embedded it whole.
            let subset_codes =
                matches!(spec.variant, Variant::B(_) | Variant::ASafe | Variant::A(Mode::CidType2 { .. }));
            let mut used: Vec<u16> = lines.iter().flat_map(|l| l.glyphs.iter().map(|g| g.gid)).collect();
            used.sort_unstable();
            used.dedup();
            let (mut mapped, mut correct, mut gaps) = (0, 0, Vec::new());
            for gid in &used {
                let code = if subset_codes { embed.cid(*gid) as u32 } else { *gid as u32 };
                let want = embed.unicode_of.get(gid).map(String::as_str).unwrap_or("");
                match inspection.to_unicode.get(&code) {
                    Some(got) => {
                        mapped += 1;
                        if got == want {
                            correct += 1;
                        } else {
                            gaps.push(format!(
                                "{} (gid {gid}): ToUnicode {} but stands for {}",
                                font.glyph_name(*gid),
                                report::codepoints(got),
                                report::codepoints(want)
                            ));
                        }
                    }
                    None => gaps.push(format!(
                        "{} (gid {gid}): no ToUnicode entry, stands for {}",
                        font.glyph_name(*gid),
                        report::codepoints(want)
                    )),
                }
            }

            outputs.push(Output {
                probe: spec.probe,
                spec_key: spec.key,
                what: spec.what,
                font_key,
                stem,
                font_file_size: font.data.len(),
                inspection,
                qpdf: qpdf_check(&path, &out_dir)?,
                a_stats,
                tounicode_used: used.len(),
                tounicode_mapped: mapped,
                tounicode_correct: correct,
                tounicode_gaps: gaps,
                pdfium_text: Vec::new(),
                pdfkit_text: Vec::new(),
                render_diff: Vec::new(),
            });
            pdf_paths.push(path);
        }
    }

    // Second experiment: the same line added to the first page of existing PDFs.
    let mut verify_args: Vec<String> = pdf_paths.iter().map(|p| p.display().to_string()).collect();
    let (insert_cases, mut insertions) = {
        let font = FontInfo::load(&root.join("assets/fonts").join(FONTS[0].1))?;
        let line = shape::shape_line(&font, INPUTS[1].1)?;
        insert::run(&pdfium, &root, &out_dir, &font, &line, &mut verify_args)?
    };
    drop(pdfium);

    // Calibration: Chrome's (Skia's) own PDFs of similar text, when the fixtures exist.
    let mut reference_stems = Vec::new();
    for name in ["ar-lam-alef", "ar-vocalized", "mixed-inline"] {
        let source = root.join("fixtures/out/chrome").join(format!("{name}.pdf"));
        if source.is_file() {
            let stem = format!("ref-chrome-{name}");
            let copy = out_dir.join(format!("{stem}.pdf"));
            std::fs::copy(&source, &copy)?;
            verify_args.push(copy.display().to_string());
            reference_stems.push(stem);
        }
    }

    // Render + extract through mirsam-core in a child process (one binding per process).
    eprintln!("rendering and extracting {} PDFs with PDFium (mirsam-core)...", verify_args.len());
    let child = Command::new(std::env::current_exe()?).arg("verify").args(&verify_args).output()?;
    if !child.status.success() {
        return Err(format!("verify child failed: {}", String::from_utf8_lossy(&child.stderr)).into());
    }
    let pdfium_texts: BTreeMap<String, Vec<String>> = serde_json::from_slice(&child.stdout)?;

    eprintln!("extracting with PDFKit (swift)...");
    let script = root.join("spikes/pdfium-edit/scripts/pdfkit_extract.swift");
    let swift = Command::new("swift").arg(&script).args(&verify_args).output()?;
    if !swift.status.success() {
        return Err(format!("swift failed: {}", String::from_utf8_lossy(&swift.stderr)).into());
    }
    let pdfkit_texts: BTreeMap<String, Vec<String>> = serde_json::from_slice(&swift.stdout)?;

    for output in &mut outputs {
        output.pdfium_text = pdfium_texts.get(&output.stem).cloned().unwrap_or_default();
        output.pdfkit_text = pdfkit_texts.get(&output.stem).cloned().unwrap_or_default();
    }

    for output in &mut outputs {
        for index in 0..INPUTS.len() {
            let page = std::fs::read(out_dir.join(format!("{}.p{index}.rgba", output.stem)))?;
            let reference = std::fs::read(out_dir.join(format!("b-src-{}.p{index}.rgba", output.font_key)))?;
            let differing = page
                .chunks(4)
                .zip(reference.chunks(4))
                .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 32))
                .count();
            output.render_diff.push(differing as f64 / (page.len() / 4) as f64);
        }
    }
    insert::finish(&out_dir, &insert_cases, &mut insertions, |extractor, stem| {
        let texts = if extractor == "pdfium" { &pdfium_texts } else { &pdfkit_texts };
        texts.get(stem).and_then(|pages| pages.first()).cloned()
    });
    comparison_sheets(&out_dir, &specs)?;
    let references: Vec<report::Reference> = reference_stems
        .into_iter()
        .map(|stem| report::Reference {
            pdfium: pdfium_texts.get(&stem).cloned().unwrap_or_default(),
            pdfkit: pdfkit_texts.get(&stem).cloned().unwrap_or_default(),
            stem,
        })
        .collect();
    report::write(&out_dir, &outputs, &references, &insertions)?;
    eprintln!("done: {}", out_dir.display());
    Ok(())
}

/// `qpdf --check`, plus a QDF copy for reading objects by eye.
fn qpdf_check(path: &Path, out_dir: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let check = Command::new("qpdf").arg("--check").arg(path).output()?;
    let qdf = out_dir.join("qdf").join(path.file_name().ok_or("no file name")?).with_extension("qdf.pdf");
    Command::new("qpdf").args(["--qdf", "--object-streams=disable"]).arg(path).arg(&qdf).output()?;
    Ok(match check.status.code() {
        Some(0) => "pass".into(),
        code => {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&check.stdout),
                String::from_utf8_lossy(&check.stderr)
            );
            let first =
                text.lines().find(|l| l.contains("WARNING") || l.contains("ERROR") || l.contains("error"));
            let message = first.unwrap_or("").replace(&format!("{}", path.display()), "");
            format!("FAIL (exit {code:?}): {}", message.trim())
        }
    })
}

/// One PNG per (font, input) stacking the variants that render differently, in
/// `variants()` order, so they can be compared by eye.
fn comparison_sheets(out_dir: &Path, specs: &[Spec]) -> Result<(), Box<dyn std::error::Error>> {
    let width = (LAYOUT.page_w * verify::SCALE).ceil() as u32;
    let height = (LAYOUT.page_h * verify::SCALE).ceil() as u32;
    for (font_key, _) in FONTS {
        for (index, (input_key, _)) in INPUTS.iter().enumerate() {
            let mut rows = Vec::new();
            for spec in specs.iter().filter(|s| s.in_sheet) {
                rows.push(std::fs::read(out_dir.join(format!("{}-{}.p{index}.rgba", spec.key, font_key)))?);
            }
            let rows: Vec<(u32, u32, &[u8])> = rows.iter().map(|r| (width, height, r.as_slice())).collect();
            verify::write_rows(&out_dir.join(format!("compare-{font_key}-{input_key}.png")), width, &rows)?;
        }
    }
    for entry in std::fs::read_dir(out_dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "rgba") {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}
