//! Prints the paragraphs found on a page (ADR 0023).
//!
//!   cargo run -p safha-core --example page_blocks -- file.pdf [page]

use safha_core::{Engine, PdfiumSource};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().ok_or("usage: page_blocks file.pdf [page]")?;
    let page: usize = args.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
    let engine = Engine::global(&PdfiumSource::Auto)?;
    let document = engine.open_bytes(std::fs::read(path)?, None)?;
    for block in document.text_blocks(page)? {
        println!(
            "#{} {} {:?} {:.0}pt {} lines at ({:.0}, {:.0}) {:.0}x{:.0}{}{} font {} {:?}",
            block.index,
            if block.rtl { "RTL" } else { "LTR" },
            block.align,
            block.size,
            block.lines,
            block.x,
            block.y,
            block.width,
            block.height,
            if block.editable { "" } else { " [not editable]" },
            if block.styled { " [styled]" } else { "" },
            block.font,
            block.text
        );
    }
    Ok(())
}
