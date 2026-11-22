//! Indexes the installed fonts, and optionally finds the best one for a PDF font.
//!
//!   cargo run -p safha-core --example system_fonts                      # count and time
//!   cargo run -p safha-core --example system_fonts -- "Arial,Bold" "نص عربي"
//!   SAFHA_FONT_LIST=1 cargo run -p safha-core --example system_fonts    # every face

use std::time::Instant;

use safha_core::typeset::{FontLibrary, FontRequest, system_font_dirs};

fn main() {
    let dirs = system_font_dirs();
    let start = Instant::now();
    let library = FontLibrary::scan(&dirs);
    println!(
        "{} faces indexed, {} files skipped, in {:.0} ms, from {:?}",
        library.entries().len(),
        library.skipped(),
        start.elapsed().as_secs_f64() * 1000.0,
        dirs
    );
    if std::env::var_os("SAFHA_FONT_LIST").is_some() {
        for e in library.entries() {
            println!("{:?} #{} {:?} {:?} {:?}", e.path, e.index, e.traits, e.coverage, e.weight_range);
        }
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [name, text, ..] = args.as_slice() {
        let start = Instant::now();
        let request = FontRequest::from_pdf(name, None, None, None, None);
        match library.find(&request, text) {
            Some((_, entry, found)) => println!(
                "{name} -> {} ({}) #{} {:?}, covers text: {}, {:.0} ms",
                entry.traits.family,
                entry.path.display(),
                entry.index,
                found.kind,
                found.covers_text,
                start.elapsed().as_secs_f64() * 1000.0
            ),
            None => println!("{name}: no fonts"),
        }
    }
}
