//! Reads page images with the draft reader (Apple Vision on macOS, else Tesseract):
//!
//!     cargo run -p safha-core --example ocr_draft -- <page.png>…
//!
//! Prints each line's box and text, and the time per page.

use std::path::PathBuf;
use std::time::Instant;

use safha_core::ocr::{DraftReader, OcrImage, TesseractReader, VisionReader, vision_helper};

fn main() {
    let images: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    let mut reader: Box<dyn DraftReader> = if cfg!(target_os = "macos") {
        let cache = std::env::temp_dir().join("mirsam-helpers");
        let started = Instant::now();
        let helper = vision_helper(&cache).expect("Vision helper (needs Xcode's command-line tools)");
        eprintln!("helper ready in {:.1} s: {}", started.elapsed().as_secs_f32(), helper.display());
        Box::new(VisionReader::start(&helper).expect("starts"))
    } else {
        Box::new(TesseractReader {
            program: "tesseract".into(),
            languages: "ara+eng".into(),
            data: None,
            dpi: 300,
        })
    };
    for path in images {
        let image = OcrImage::decode_png(&std::fs::read(&path).expect("read")).expect("png");
        let started = Instant::now();
        let page = reader.read(&image).expect("reads");
        println!(
            "== {} ({} lines, {:.2} s)",
            path.display(),
            page.lines.len(),
            started.elapsed().as_secs_f32()
        );
        for line in &page.lines {
            let b = line.bounds;
            println!(
                "[{:.0},{:.0},{:.0},{:.0}] {:.2} {} ({} words)",
                b[0],
                b[1],
                b[2],
                b[3],
                line.confidence,
                line.text,
                line.words.len()
            );
        }
    }
}
