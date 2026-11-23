//! A page's text as PDFium itself reports it (what Chrome copies), beside this
//! app's reading order (`page_text`):
//!
//!     cargo run -p mirsam-core --example page_text_raw -- file.pdf [page]

use std::path::Path;

use mirsam_core::{Engine, PdfiumSource};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().ok_or("usage: page_text_raw <file.pdf> [page]")?;
    let page: usize = args.get(1).map(|s| s.parse()).transpose()?.unwrap_or(0);
    let document = Engine::global(&PdfiumSource::Auto)?.open_file(Path::new(path), None)?;
    println!("{}", document.page_text_raw(page)?);
    Ok(())
}
