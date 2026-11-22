//! Prints what font matching sees in each font file.
//!
//!   cargo run -p safha-core --example font_traits -- font.ttf ...   (default: assets/fonts/*.ttf)
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut paths: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    if paths.is_empty() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts");
        paths = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "ttf"))
            .collect();
        paths.sort();
    }
    for path in paths {
        let face = safha_core::typeset::FontFace::from_bytes(std::fs::read(&path)?, 0)?;
        println!(
            "{}: {:?}",
            path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(),
            face.traits()
        );
    }
    Ok(())
}
