//! Prints a page's text as `Document::page_text` reconstructs it.
//!
//!   cargo run -p safha-core --example page_text -- <file.pdf> [page=0] [password]

use std::path::Path;

use safha_core::{Engine, PdfiumSource};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        return Err("usage: page_text <file.pdf> [page=0] [password]".into());
    };
    let page: usize = args.get(1).map(|s| s.parse()).transpose()?.unwrap_or(0);
    let document =
        Engine::global(&PdfiumSource::Auto)?.open_file(Path::new(path), args.get(2).map(String::as_str))?;
    let text = document.page_text(page)?;
    println!("direction: {:?}, quality: {:?}\n", text.direction, text.quality);
    for block in &text.blocks {
        println!("[{:?}]", block.direction);
        for line in &block.lines {
            println!("{}", line.text);
        }
        println!();
    }
    Ok(())
}
