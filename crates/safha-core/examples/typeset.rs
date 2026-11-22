//! Lays out a paragraph and prints its lines and glyph runs.
//!
//!   cargo run -p safha-core --example typeset -- "text" [width] [font.ttf ...]
//!
//! Fonts default to Amiri then IBM Plex Sans (assets/fonts). Size 18 pt.
//! With `SAFHA_TYPESET_BENCH=<n>` set, also times `n` layouts of the text
//! (use `--release`; timings are noisy on a shared machine).

use std::path::PathBuf;

use safha_core::typeset::{FontFace, ParagraphStyle, Span, layout};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let text =
        args.first().cloned().unwrap_or_else(|| "اشتريت 3 أجهزة iPhone 15 Pro بسعر 4,500 ريال.".into());
    let width = args.get(1).and_then(|w| w.parse::<f32>().ok());
    let fonts_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts");
    let paths: Vec<PathBuf> = if args.len() > 2 {
        args[2..].iter().map(PathBuf::from).collect()
    } else {
        vec![fonts_dir.join("Amiri-Regular.ttf"), fonts_dir.join("IBMPlexSans-VF.ttf")]
    };
    let fonts = paths
        .iter()
        .map(|p| FontFace::from_bytes(std::fs::read(p)?, 0).map_err(Into::into))
        .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
    let spans = [Span::new(0..text.len(), 0, 18.0)];
    let style = ParagraphStyle { width, ..ParagraphStyle::default() };
    let out = layout(&text, &spans, &fonts, &style)?;
    if let Some(n) = std::env::var("SAFHA_TYPESET_BENCH").ok().and_then(|n| n.parse::<u32>().ok()) {
        let start = std::time::Instant::now();
        for _ in 0..n {
            std::hint::black_box(layout(&text, &spans, &fonts, &style)?);
        }
        let per = start.elapsed().as_secs_f64() * 1000.0 / f64::from(n.max(1));
        println!("{n} layouts: {per:.3} ms each ({} chars, {} lines)", text.chars().count(), out.lines.len());
    }
    println!("box {:.1} x {:.1}", out.width, out.height);
    for line in &out.lines {
        println!(
            "line {:?} {} x={:.1} w={:.1} baseline={:.1} {:?}",
            line.text_range,
            if line.rtl { "RTL" } else { "LTR" },
            line.x,
            line.width,
            line.baseline,
            &text[line.text_range.clone()]
        );
        for run in &line.runs {
            let clusters: Vec<String> =
                run.clusters.iter().map(|c| format!("{:?}", &text[c.text.clone()])).collect();
            println!(
                "  run font={} {} {} {:?} x={:.1}..{:.1} glyphs={} clusters: {}",
                run.font,
                if run.rtl { "RTL" } else { "LTR" },
                String::from_utf8_lossy(&run.script),
                &text[run.text_range.clone()],
                run.glyphs.first().map_or(0.0, |g| g.x),
                run.glyphs.last().map_or(0.0, |g| g.x + g.advance),
                run.glyphs.len(),
                clusters.join(" ")
            );
        }
    }
    Ok(())
}
