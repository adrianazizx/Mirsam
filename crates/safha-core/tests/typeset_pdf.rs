//! End to end: the layout engine's glyph runs are enough to write a PDF page
//! that PDFium draws and that `safha-core`'s own extractor reads back in
//! logical order — the property the M3 writer will be held to (spec §7).
//!
//! The writer here is a deliberately small test fixture: Type 0 /
//! CIDFontType2 fonts with Identity-H and an identity CIDToGIDMap, the whole
//! font embedded (no subsetting), a ToUnicode CMap for one-glyph clusters,
//! `/ActualText` around clusters of several glyphs, and one `Tm … Tj` per
//! glyph. It is not the product's writer (ADR 0005 amendment).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::Command;

use safha_core::typeset::{Align, FontFace, ParagraphStyle, Span, TextLayout, layout};
use safha_core::{Engine, PdfiumSource};

const MARGIN: f32 = 36.0;
const SIZE: f32 = 16.0;

fn font(file: &str) -> FontFace {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts").join(file);
    let data = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    FontFace::from_bytes(data, 0).unwrap_or_else(|e| panic!("{file}: {e}"))
}

fn write_pdf(text: &str, out: &TextLayout, fonts: &[FontFace]) -> Vec<u8> {
    let page_w = out.width + 2.0 * MARGIN;
    let page_h = out.height + 2.0 * MARGIN;

    // Per font: glyph id -> (width in 1/1000 em, text of the cluster it starts).
    let mut used: Vec<BTreeMap<u32, (i32, Option<String>)>> = vec![BTreeMap::new(); fonts.len()];
    let mut content = String::new();
    for line in &out.lines {
        let baseline = page_h - MARGIN - line.baseline;
        for run in &line.runs {
            let _ = writeln!(content, "BT /F{} {} Tf", run.font, run.size);
            for (i, g) in run.glyphs.iter().enumerate() {
                let cluster = &run.clusters[g.cluster];
                let cluster_text = &text[cluster.text.clone()];
                let entry = used[run.font]
                    .entry(g.id)
                    .or_insert(((g.advance / run.size * 1000.0).round() as i32, None));
                // A one-glyph cluster maps through ToUnicode. A cluster of several
                // glyphs (Noto Naskh draws a letter's dots as glyphs of their own)
                // is wrapped in /ActualText, as Chrome does.
                let single = cluster.glyphs.len() == 1;
                if single && entry.1.is_none() {
                    entry.1 = Some(cluster_text.to_string());
                }
                if !single && i == cluster.glyphs.start {
                    let utf16: String = cluster_text.encode_utf16().map(|u| format!("{u:04X}")).collect();
                    let _ = writeln!(content, "/Span << /ActualText <FEFF{utf16}> >> BDC");
                }
                let _ = writeln!(
                    content,
                    "1 0 0 1 {:.3} {:.3} Tm <{:04X}> Tj",
                    MARGIN + g.x,
                    baseline + g.y_offset,
                    g.id
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
    for (index, (face, glyphs)) in fonts.iter().zip(&used).enumerate() {
        let file = add(stream(&format!("/Length1 {}", face.data().len()), face.data()));
        let upem = f32::from(face.units_per_em());
        let m = face.line_metrics();
        let descriptor = add(
            format!(
                "<< /Type /FontDescriptor /FontName /F{index}Face /Flags 4 /FontBBox [0 {} 1000 {}] /ItalicAngle 0 \
                 /Ascent {} /Descent {} /CapHeight {} /StemV 80 /FontFile2 {file} 0 R >>",
                (-m.descent * 1000.0 / upem) as i32,
                (m.ascent * 1000.0 / upem) as i32,
                (m.ascent * 1000.0 / upem) as i32,
                (-m.descent * 1000.0 / upem) as i32,
                (m.ascent * 700.0 / upem) as i32,
            )
            .into_bytes(),
        );
        let widths: String = glyphs.iter().map(|(gid, (w, _))| format!("{gid} [{w}] ")).collect();
        let cid_font = add(format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /F{index}Face \
                 /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
                 /FontDescriptor {descriptor} 0 R /CIDToGIDMap /Identity /DW 0 /W [{widths}] >>"
        )
        .into_bytes());
        let to_unicode = add(stream("", to_unicode_cmap(glyphs).as_bytes()));
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

fn to_unicode_cmap(glyphs: &BTreeMap<u32, (i32, Option<String>)>) -> String {
    let mapped: Vec<(u32, &str)> =
        glyphs.iter().filter_map(|(gid, (_, t))| t.as_deref().map(|t| (*gid, t))).collect();
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def /CMapType 2 def\n\
         1 begincodespacerange <0000> <FFFF> endcodespacerange\n",
    );
    for chunk in mapped.chunks(100) {
        let _ = writeln!(cmap, "{} beginbfchar", chunk.len());
        for (gid, t) in chunk {
            let utf16: String = t.encode_utf16().map(|u| format!("{u:04X}")).collect();
            let _ = writeln!(cmap, "<{gid:04X}> <{utf16}>");
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap CMapName currentdict /CMap defineresource pop end end\n");
    cmap
}

fn qpdf_check(pdf: &[u8], name: &str) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let path = dir.join(format!("typeset-{name}.pdf"));
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

/// Lays out `text`, writes it, and checks PDFium's view of the result:
/// the page renders ink, and the logical text extracted equals the input.
fn round_trip(name: &str, text: &str, fonts: &[FontFace], style: &ParagraphStyle) {
    let spans = [Span::new(0..text.len(), 0, SIZE)];
    let out = layout(text, &spans, fonts, style).unwrap_or_else(|e| panic!("{name}: {e}"));
    let pdf = write_pdf(text, &out, fonts);
    qpdf_check(&pdf, name);

    let engine = Engine::global(&PdfiumSource::Auto).unwrap_or_else(|e| panic!("PDFium: {e}"));
    let doc =
        engine.open_bytes(pdf, None).unwrap_or_else(|e| panic!("{name}: PDFium cannot open the output: {e}"));
    let page = doc.render_page(0, 1.0).unwrap_or_else(|e| panic!("{name}: render: {e}"));
    let rgba = page.to_rgba();
    let inked = rgba.as_chunks::<4>().0.iter().filter(|p| p[0] < 128).count();
    assert!(inked > 50, "{name}: the page is blank ({inked} dark pixels)");

    let extracted = doc.page_text(0).unwrap_or_else(|e| panic!("{name}: extract: {e}")).text();
    assert_eq!(normalize(&extracted), normalize(text), "{name}: extracted text differs");
}

fn amiri() -> FontFace {
    font("Amiri-Regular.ttf")
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
        &[font("IBMPlexSans-VF.ttf"), font("NotoNaskhArabic-VF.ttf")],
        &ParagraphStyle { width: Some(220.0), align: Align::Justify, ..ParagraphStyle::default() },
    );
}
