//! Verification child process: render and extract text through `safha-core`, i.e.
//! the product's own PDFium path. It runs in its own process because
//! pdfium-render allows one library binding per process and the generator
//! already holds a direct one for the edit API.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use safha_core::{Bitmap, Engine, PdfiumSource};

pub const SCALE: f32 = 3.0;

/// Pages rendered and extracted per document (the 1,000-page fixture has more).
const MAX_PAGES: usize = 4;

/// `verify <pdf>[|password]...` -> writes `<stem>.png` (first pages stacked) next
/// to each PDF and prints `{ "<stem>": ["page text", ..] }` as JSON on stdout. A
/// file PDFium cannot open is reported on stderr and left out.
pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let engine = Engine::global(&PdfiumSource::Auto)?;
    let mut texts = serde_json::Map::new();
    for arg in args {
        let (path, password) = match arg.rsplit_once('|') {
            Some((path, password)) => (path, Some(password)),
            None => (arg.as_str(), None),
        };
        let path = Path::new(path);
        let stem = path.file_stem().and_then(|s| s.to_str()).ok_or("bad path")?.to_string();
        // A route may have written the file without encryption, so try that first.
        let doc = match engine.open_file(path, None).or_else(|_| engine.open_file(path, password)) {
            Ok(doc) => doc,
            Err(error) => {
                eprintln!("verify: cannot open {}: {error}", path.display());
                continue;
            }
        };
        let mut pages = Vec::new();
        let mut page_texts = Vec::new();
        for index in 0..doc.page_count().min(MAX_PAGES) {
            pages.push(doc.render_page(index, SCALE)?);
            page_texts.push(serde_json::Value::String(doc.page_text_raw(index)?));
        }
        write_stacked(&path.with_extension("png"), &pages)?;
        // Raw RGBA per page so the parent can assemble comparison sheets.
        for (index, page) in pages.iter().enumerate() {
            std::fs::write(path.with_extension(format!("p{index}.rgba")), page.to_rgba())?;
        }
        texts.insert(stem, serde_json::Value::Array(page_texts));
    }
    println!("{}", serde_json::Value::Object(texts));
    Ok(())
}

fn write_stacked(path: &Path, pages: &[Bitmap]) -> Result<(), Box<dyn std::error::Error>> {
    let width = pages.iter().map(Bitmap::width).max().unwrap_or(1);
    let rgba: Vec<Vec<u8>> = pages.iter().map(Bitmap::to_rgba).collect();
    let rows: Vec<(u32, u32, &[u8])> =
        pages.iter().zip(&rgba).map(|(p, pixels)| (p.width(), p.height(), pixels.as_slice())).collect();
    write_rows(path, width, &rows)
}

/// Stacks RGBA images vertically with a grey rule between them.
pub fn write_rows(
    path: &Path,
    width: u32,
    rows: &[(u32, u32, &[u8])],
) -> Result<(), Box<dyn std::error::Error>> {
    const RULE: u32 = 2;
    let height: u32 = rows.iter().map(|r| r.1 + RULE).sum();
    let mut image = vec![0xFFu8; (width * height * 4) as usize];
    let mut y0 = 0u32;
    for (w, h, rgba) in rows {
        for y in 0..*h {
            let src = (y * w * 4) as usize;
            let dst = ((y0 + y) * width * 4) as usize;
            let n = (*w.min(&width) * 4) as usize;
            image[dst..dst + n].copy_from_slice(&rgba[src..src + n]);
        }
        y0 += h;
        for y in y0..y0 + RULE {
            let dst = (y * width * 4) as usize;
            for px in image[dst..dst + (width * 4) as usize].chunks_mut(4) {
                px.copy_from_slice(&[0x99, 0x99, 0x99, 0xFF]);
            }
        }
        y0 += RULE;
    }
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&image)?;
    Ok(())
}
