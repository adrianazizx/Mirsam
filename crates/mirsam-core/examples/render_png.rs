//! Renders one page of a PDF to a PNG with the real engine.
//!
//!   cargo run -p mirsam-core --example render_png -- <in.pdf> <out.png> [page=0] [scale=1.5] [password]

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use mirsam_core::{Engine, PdfiumSource};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output, rest @ ..] = args.as_slice() else {
        return Err("usage: render_png <in.pdf> <out.png> [page=0] [scale=1.5] [password]".into());
    };
    let page: usize = rest.first().map(|s| s.parse()).transpose()?.unwrap_or(0);
    let scale: f32 = rest.get(1).map(|s| s.parse()).transpose()?.unwrap_or(1.5);
    let password = rest.get(2).map(String::as_str);

    let document = Engine::global(&PdfiumSource::Auto)?.open_file(Path::new(input), password)?;
    let bitmap = document.render_page(page, scale)?;

    let mut encoder =
        png::Encoder::new(BufWriter::new(File::create(output)?), bitmap.width(), bitmap.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&bitmap.to_rgba())?;
    println!("{output}: {}x{} (page {page} of {})", bitmap.width(), bitmap.height(), document.page_count());
    Ok(())
}
