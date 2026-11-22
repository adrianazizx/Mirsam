//! End to end: the layout engine's glyph runs, and its font subsets, are
//! enough to write a PDF page that PDFium draws and that `safha-core`'s own
//! extractor reads back in logical order — the property the M3 writer will be
//! held to (spec §7).
//!
//! The writer here is a deliberately small test fixture: Type 0 /
//! CIDFontType2 fonts with Identity-H and an identity CIDToGIDMap; either the
//! whole font or its subset (`typeset::subset_fonts`) embedded; a ToUnicode
//! CMap for one-glyph clusters; `/ActualText` around clusters of several
//! glyphs; one `Tm … Tj` per glyph. It is not the product's writer (ADR 0005
//! amendment).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::Command;

use safha_core::typeset::{Align, FontFace, ParagraphStyle, Span, TextLayout, layout, subset_fonts};
use safha_core::{Engine, PdfiumSource};

const MARGIN: f32 = 36.0;
const SIZE: f32 = 16.0;

fn font(file: &str) -> FontFace {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts").join(file);
    let data = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    FontFace::from_bytes(data, 0).unwrap_or_else(|e| panic!("{file}: {e}"))
}

fn amiri() -> FontFace {
    font("Amiri-Regular.ttf")
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Embed {
    /// The whole font file, glyph ids as CIDs.
    Full,
    /// `typeset::subset_fonts`: the glyphs used, renumbered; variable fonts instanced.
    Subset,
}

/// What a font dictionary needs: the program, a CID per glyph id, widths, ToUnicode.
#[derive(Default)]
struct Embedded {
    program: Vec<u8>,
    cid: BTreeMap<u32, u16>,
    widths: BTreeMap<u16, i32>,
    text: BTreeMap<u16, String>,
}

fn embedded_fonts(text: &str, out: &TextLayout, fonts: &[FontFace], embed: Embed) -> Vec<Embedded> {
    let mut all: Vec<Embedded> = fonts.iter().map(|_| Embedded::default()).collect();
    match embed {
        Embed::Full => {
            for run in out.runs() {
                let e = &mut all[run.font];
                e.program = fonts[run.font].data().to_vec();
                for g in &run.glyphs {
                    let cid = u16::try_from(g.id).unwrap_or(0);
                    e.cid.insert(g.id, cid);
                    e.widths.entry(cid).or_insert((g.advance / run.size * 1000.0).round() as i32);
                    let cluster = &run.clusters[g.cluster];
                    if cluster.glyphs.len() == 1 {
                        e.text.entry(cid).or_insert_with(|| text[cluster.text.clone()].to_string());
                    }
                }
            }
        }
        Embed::Subset => {
            for subset in subset_fonts(text, out, fonts).unwrap_or_else(|e| panic!("subset: {e}")) {
                let e = &mut all[subset.font];
                e.program = subset.data;
                for g in subset.glyphs {
                    e.cid.insert(g.original, g.cid);
                    e.widths.insert(g.cid, g.width.round() as i32);
                    if let Some(t) = g.text {
                        e.text.insert(g.cid, t);
                    }
                }
            }
        }
    }
    all
}

fn write_pdf(text: &str, out: &TextLayout, fonts: &[FontFace], embed: Embed) -> Vec<u8> {
    let page_w = out.width + 2.0 * MARGIN;
    let page_h = out.height + 2.0 * MARGIN;
    let embedded = embedded_fonts(text, out, fonts, embed);

    let mut content = String::new();
    for line in &out.lines {
        let baseline = page_h - MARGIN - line.baseline;
        for run in &line.runs {
            let _ = writeln!(content, "BT /F{} {} Tf", run.font, run.size);
            for (i, g) in run.glyphs.iter().enumerate() {
                let cluster = &run.clusters[g.cluster];
                // A one-glyph cluster maps through ToUnicode. A cluster of several
                // glyphs (Noto Naskh draws a letter's dots as glyphs of their own)
                // is wrapped in /ActualText, as Chrome does.
                let single = cluster.glyphs.len() == 1;
                if !single && i == cluster.glyphs.start {
                    let utf16: String =
                        text[cluster.text.clone()].encode_utf16().map(|u| format!("{u:04X}")).collect();
                    let _ = writeln!(content, "/Span << /ActualText <FEFF{utf16}> >> BDC");
                }
                let cid = embedded[run.font].cid.get(&g.id).copied().unwrap_or(0);
                let _ = writeln!(
                    content,
                    "1 0 0 1 {:.3} {:.3} Tm <{cid:04X}> Tj",
                    MARGIN + g.x,
                    baseline + g.y_offset
                );
                if !single && i + 1 == cluster.glyphs.end {
                    content.push_str("EMC\n");
                }
            }
            content.push_str("ET\n");
        }
    }

    let mut objects: Vec<Vec<u8>> = Vec::new();
    let mut add = |body: Vec<u8>| {
        objects.push(body);
        objects.len()
    };
    let catalog = add(Vec::new());
    let pages = add(Vec::new());
    let page = add(Vec::new());
    let contents = add(stream("", content.as_bytes()));
    let mut font_refs = String::new();
    for (index, (face, e)) in fonts.iter().zip(&embedded).enumerate() {
        if e.program.is_empty() {
            continue;
        }
        let file = add(stream(&format!("/Length1 {}", e.program.len()), &e.program));
        let upem = f32::from(face.units_per_em());
        let m = face.line_metrics();
        let (ascent, descent) = ((m.ascent * 1000.0 / upem) as i32, (-m.descent * 1000.0 / upem) as i32);
        let descriptor = add(
            format!(
                "<< /Type /FontDescriptor /FontName /F{index}Face /Flags 4 /FontBBox [0 {descent} 1000 {ascent}] \
                 /ItalicAngle 0 /Ascent {ascent} /Descent {descent} /CapHeight {} /StemV 80 /FontFile2 {file} 0 R >>",
                ascent * 7 / 10,
            )
            .into_bytes(),
        );
        let widths: String = e.widths.iter().map(|(cid, w)| format!("{cid} [{w}] ")).collect();
        let cid_font = add(format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /F{index}Face \
                 /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
                 /FontDescriptor {descriptor} 0 R /CIDToGIDMap /Identity /DW 0 /W [{widths}] >>"
        )
        .into_bytes());
        let to_unicode = add(stream("", to_unicode_cmap(&e.text).as_bytes()));
        let type0 = add(format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /F{index}Face /Encoding /Identity-H \
                 /DescendantFonts [{cid_font} 0 R] /ToUnicode {to_unicode} 0 R >>"
        )
        .into_bytes());
        let _ = write!(font_refs, "/F{index} {type0} 0 R ");
    }
    objects[catalog - 1] = format!("<< /Type /Catalog /Pages {pages} 0 R >>").into_bytes();
    objects[pages - 1] = format!("<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>").into_bytes();
    objects[page - 1] = format!(
        "<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 {page_w:.2} {page_h:.2}] \
         /Resources << /Font << {font_refs}>> >> /Contents {contents} 0 R >>"
    )
    .into_bytes();

    let mut pdf = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size {} /Root {catalog} 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1)
            .as_bytes(),
    );
    pdf
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

fn to_unicode_cmap(text: &BTreeMap<u16, String>) -> String {
    let mapped: Vec<(&u16, &String)> = text.iter().collect();
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def /CMapType 2 def\n\
         1 begincodespacerange <0000> <FFFF> endcodespacerange\n",
    );
    for chunk in mapped.chunks(100) {
        let _ = writeln!(cmap, "{} beginbfchar", chunk.len());
        for (cid, t) in chunk {
            let utf16: String = t.encode_utf16().map(|u| format!("{u:04X}")).collect();
            let _ = writeln!(cmap, "<{cid:04X}> <{utf16}>");
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap CMapName currentdict /CMap defineresource pop end end\n");
    cmap
}

fn qpdf_check(pdf: &[u8], name: &str) {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("typeset-{name}.pdf"));
    std::fs::write(&path, pdf).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let out = Command::new("qpdf").arg("--check").arg(&path).output().unwrap_or_else(|e| {
        panic!("qpdf is required for this test (brew install qpdf / apt install qpdf): {e}")
    });
    assert!(
        out.status.success(),
        "qpdf --check {}: {}",
        path.display(),
        String::from_utf8_lossy(&out.stdout)
    );
}

fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct Written {
    pdf_len: usize,
    rgba: Vec<u8>,
}

/// Writes the layout, and checks PDFium's view of the result: qpdf accepts
/// it, the page renders ink, and the logical text extracted equals the input.
fn write_and_check(name: &str, text: &str, out: &TextLayout, fonts: &[FontFace], embed: Embed) -> Written {
    let name = format!("{name}-{embed:?}").to_lowercase();
    let pdf = write_pdf(text, out, fonts, embed);
    let pdf_len = pdf.len();
    qpdf_check(&pdf, &name);

    let engine = Engine::global(&PdfiumSource::Auto).unwrap_or_else(|e| panic!("PDFium: {e}"));
    let doc =
        engine.open_bytes(pdf, None).unwrap_or_else(|e| panic!("{name}: PDFium cannot open the output: {e}"));
    let page = doc.render_page(0, 2.0).unwrap_or_else(|e| panic!("{name}: render: {e}"));
    let rgba = page.to_rgba();
    let inked = rgba.as_chunks::<4>().0.iter().filter(|p| p[0] < 128).count();
    assert!(inked > 50, "{name}: the page is blank ({inked} dark pixels)");

    let extracted = doc.page_text(0).unwrap_or_else(|e| panic!("{name}: extract: {e}")).text();
    assert_eq!(normalize(&extracted), normalize(text), "{name}: extracted text differs");
    Written { pdf_len, rgba }
}

/// Pixels with a channel differing by more than 32/255 (as in ADR 0005).
fn differing_pixels(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len(), "page sizes differ");
    let (a, b) = (a.as_chunks::<4>().0, b.as_chunks::<4>().0);
    a.iter().zip(b).filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > 32)).count()
}

fn dark_pixels(rgba: &[u8]) -> usize {
    rgba.as_chunks::<4>().0.iter().filter(|p| p[0] < 128).count()
}

/// Lays out `text` and writes it twice, with the whole fonts and with their
/// subsets. Both must pass [`write_and_check`] and draw the same pixels, and
/// the subset file must be a fraction of the size.
fn round_trip(name: &str, text: &str, fonts: &[FontFace], style: &ParagraphStyle) {
    let spans = [Span::new(0..text.len(), 0, SIZE)];
    let out = layout(text, &spans, fonts, style).unwrap_or_else(|e| panic!("{name}: {e}"));
    let full = write_and_check(name, text, &out, fonts, Embed::Full);
    let subset = write_and_check(name, text, &out, fonts, Embed::Subset);
    assert_eq!(differing_pixels(&full.rgba, &subset.rgba), 0, "{name}: the subset draws differently");
    assert!(
        subset.pdf_len * 4 < full.pdf_len,
        "{name}: subset file {} bytes, full {}",
        subset.pdf_len,
        full.pdf_len
    );
}

#[test]
fn english_round_trips() {
    round_trip(
        "english",
        "The office staff finished the final draft efficiently; the affluent client found it difficult to fault.",
        &[amiri()],
        &ParagraphStyle { width: Some(260.0), ..ParagraphStyle::default() },
    );
}

#[test]
fn arabic_round_trips() {
    round_trip(
        "arabic",
        "لا بأس؛ لأن الأولاد الآن في الملعب، فلا داعي للقلق.",
        &[amiri()],
        &ParagraphStyle { width: Some(200.0), ..ParagraphStyle::default() },
    );
}

#[test]
fn mixed_line_round_trips() {
    round_trip(
        "mixed",
        "اشتريت 3 أجهزة iPhone 15 Pro بسعر 4,500 ريال (شاملة الضريبة).",
        &[amiri()],
        &ParagraphStyle::default(),
    );
}

#[test]
fn mixed_paragraph_with_fallback_and_justification_round_trips() {
    round_trip(
        "mixed-wrapped",
        // Not shadda + fatha here: Noto Naskh draws that pair as one two-glyph
        // cluster, which this fixture writer's /ActualText recipe extracts twice
        // through PDFium. The product writer has to solve that (ADR 0020).
        "يتضمن العقد بندًا عن Service Level Agreement لمدة 12 شهرًا، ويتجدد تلقائيًا.",
        // IBM Plex Sans has no Arabic: the Arabic falls back to Noto Naskh.
        // Both are variable fonts, embedded whole or instanced by the subsetter.
        &[font("IBMPlexSans-VF.ttf"), font("NotoNaskhArabic-VF.ttf")],
        &ParagraphStyle { width: Some(220.0), align: Align::Justify, ..ParagraphStyle::default() },
    );
}

/// A variable font at a non-default instance: the subset carries that
/// instance's outlines and widths, so it draws bolder than the default and the
/// widths agree with the layout's advances.
#[test]
fn bold_instance_of_a_variable_font_is_embedded_as_that_instance() {
    let text = "Quarterly Report 2026 — مرحبا";
    let regular = [font("IBMPlexSans-VF.ttf"), font("NotoNaskhArabic-VF.ttf")];
    let bold: Vec<FontFace> = regular
        .iter()
        .map(|f| f.with_variations(&[(*b"wght", 700.0)]).unwrap_or_else(|e| panic!("{e}")))
        .collect();
    let spans = [Span::new(0..text.len(), 0, SIZE)];
    let mut ink = Vec::new();
    for (name, fonts) in [("regular", regular.as_slice()), ("bold", bold.as_slice())] {
        let out = layout(text, &spans, fonts, &ParagraphStyle::default()).unwrap_or_else(|e| panic!("{e}"));
        let subsets = subset_fonts(text, &out, fonts).unwrap_or_else(|e| panic!("{e}"));
        // Every glyph's /W width is its unkerned advance at this instance;
        // the layout's advances can only differ by kerning.
        for run in out.runs() {
            let subset = subsets
                .iter()
                .find(|s| s.font == run.font)
                .unwrap_or_else(|| panic!("no subset for {run:?}"));
            for g in &run.glyphs {
                let cid = subset.cid(g.id).unwrap_or_else(|| panic!("glyph {} not in the subset", g.id));
                let width = subset.glyphs[usize::from(cid)].width * run.size / 1000.0;
                assert!(
                    (width - g.advance).abs() < 1.0,
                    "{name}: glyph {} width {width} vs advance {}",
                    g.id,
                    g.advance
                );
            }
        }
        ink.push(dark_pixels(
            &write_and_check(&format!("instance-{name}"), text, &out, fonts, Embed::Subset).rgba,
        ));
    }
    assert!(ink[1] as f32 > ink[0] as f32 * 1.15, "bold draws {} dark pixels, regular {}", ink[1], ink[0]);
}
