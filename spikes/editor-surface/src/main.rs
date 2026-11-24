//! Dumps the engine's layout of the spike's test paragraphs as JSON
//! (`out/layout.json`), per character in UTF-16 units, so `index.html` can
//! compare it with what the webview's own text layout does with the same
//! fonts at the same width.
//!
//!   cargo run -p spike-editor-surface
//!   (cd <repo root> && python3 -m http.server 8765)  # then open /spikes/editor-surface/index.html

use std::path::PathBuf;

use mirsam_core::typeset::{Align, BaseDirection, FontFace, ParagraphStyle, Span, layout};
use serde_json::json;

const CASES: &[(&str, &str, f32, f32, bool)] = &[
    // (name, text, width pt, size pt, justify)
    (
        "english",
        "Payment is due within thirty days of the invoice date. Late payments carry interest at the rate set out in clause 7, and the supplier may suspend work until the balance is cleared. Fix a typo here and nothing else may move.",
        300.0,
        12.0,
        false,
    ),
    (
        "arabic",
        "يستحق السداد خلال ثلاثين يوما من تاريخ الفاتورة، وتترتب على المتأخرات فائدة بالنسبة المحددة في البند السابع، ويحق للمورد إيقاف العمل إلى حين تسوية الرصيد كاملا.",
        300.0,
        12.0,
        false,
    ),
    (
        "mixed",
        "اشتريت 3 أجهزة iPhone 15 Pro بسعر 4,500 ريال للجهاز الواحد، ثم أضفت Apple Care+ مقابل 250 ريالا، فبلغ المجموع 13,750 ريالا شاملا الضريبة.",
        300.0,
        12.0,
        false,
    ),
    (
        "arabic-diacritics",
        "بِسْمِ اللَّهِ الرَّحْمَٰنِ الرَّحِيمِ، الْحَمْدُ لِلَّهِ رَبِّ الْعَالَمِينَ، الرَّحْمَٰنِ الرَّحِيمِ، مَالِكِ يَوْمِ الدِّينِ.",
        300.0,
        14.0,
        false,
    ),
    (
        "arabic-justified",
        "يستحق السداد خلال ثلاثين يوما من تاريخ الفاتورة، وتترتب على المتأخرات فائدة بالنسبة المحددة في البند السابع، ويحق للمورد إيقاف العمل إلى حين تسوية الرصيد كاملا.",
        300.0,
        12.0,
        true,
    ),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts");
    let load = |name: &str, wght: bool| -> Result<FontFace, Box<dyn std::error::Error>> {
        let face = FontFace::from_bytes(std::fs::read(dir.join(name))?, 0)?;
        Ok(if wght { face.with_variations(&[(*b"wght", 400.0), (*b"wdth", 100.0)])? } else { face })
    };
    let plex = load("IBMPlexSans-VF.ttf", true)?;
    let arabic = load("IBMPlexSansArabic-Regular.ttf", false)?;
    let mut cases = Vec::new();
    for &(name, text, width, size, justify) in CASES {
        let rtl = text.chars().next().is_some_and(|c| ('\u{0600}'..='\u{06FF}').contains(&c));
        let faces = if rtl { vec![arabic.clone(), plex.clone()] } else { vec![plex.clone(), arabic.clone()] };
        let style = ParagraphStyle {
            direction: BaseDirection::Auto,
            align: if justify { Align::Justify } else { Align::Start },
            width: Some(width),
            ..ParagraphStyle::default()
        };
        let laid = layout(text, &[Span::new(0..text.len(), 0, size)], &faces, &style)?;
        // UTF-16 offset of each byte offset.
        let utf16 = |byte: usize| text[..byte].encode_utf16().count();
        let mut lines = Vec::new();
        for line in &laid.lines {
            let mut clusters = Vec::new();
            for run in &line.runs {
                for cluster in &run.clusters {
                    let glyphs = &run.glyphs[cluster.glyphs.clone()];
                    let x0 = glyphs.iter().map(|g| g.x).fold(f32::MAX, f32::min);
                    let x1 = glyphs.iter().map(|g| g.x + g.advance).fold(f32::MIN, f32::max);
                    clusters.push(json!({"s": utf16(cluster.text.start), "e": utf16(cluster.text.end), "x0": x0, "x1": x1}));
                }
            }
            lines.push(json!({
                "start": utf16(line.text_range.start), "end": utf16(line.text_range.end),
                "rtl": line.rtl, "baseline": line.baseline, "top": line.top, "height": line.height, "clusters": clusters,
            }));
        }
        cases.push(json!({"name": name, "text": text, "width": width, "size": size, "rtl": rtl, "justify": justify, "height": laid.height, "lines": lines}));
    }
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("out/layout.json");
    std::fs::write(&out, serde_json::to_string_pretty(&cases)?)?;
    println!("wrote {}", out.display());
    Ok(())
}
