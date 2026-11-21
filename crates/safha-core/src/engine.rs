use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use pdfium_render::prelude::Pdfium;

use crate::document::Document;
use crate::error::{Error, Result};

/// Where to look for the PDFium shared library.
#[derive(Debug, Clone, Default)]
pub enum PdfiumSource {
    /// Search, in order: the `SAFHA_PDFIUM_DIR` environment variable, locations
    /// relative to the running executable (app bundle layouts), and — in debug
    /// builds only — the repository's `third_party/pdfium` download.
    #[default]
    Auto,
    /// Load from exactly this directory.
    Dir(PathBuf),
}

/// Process-wide handle to PDFium.
///
/// PDFium keeps global state and is not re-entrant, so there is exactly one
/// `Engine` per process; `pdfium-render`'s `thread_safe` feature serializes
/// every call behind a mutex. That makes `Engine` and [`Document`] `Send + Sync`,
/// at the price of no parallelism inside one process. The M1 render scheduler
/// has to be designed around that constraint.
pub struct Engine {
    pdfium: Pdfium,
    library_path: PathBuf,
}

static ENGINE: OnceLock<Engine> = OnceLock::new();

impl Engine {
    /// Returns the process-wide engine, loading PDFium on first use.
    ///
    /// `source` is only consulted by the call that performs the initialization.
    pub fn global(source: &PdfiumSource) -> Result<&'static Engine> {
        if let Some(engine) = ENGINE.get() {
            return Ok(engine);
        }
        // Two threads can race to this point; `pdfium-render` rejects the second
        // bind, so the loser simply picks up the winner's engine.
        match Self::load(source) {
            Ok(engine) => Ok(ENGINE.get_or_init(|| engine)),
            Err(err) => ENGINE.get().ok_or(err),
        }
    }

    fn load(source: &PdfiumSource) -> Result<Engine> {
        let candidates = candidate_dirs(source);
        let library_path = candidates
            .iter()
            .map(Pdfium::pdfium_platform_library_name_at_path)
            .find(|path| path.is_file())
            .ok_or_else(|| Error::PdfiumNotFound { searched: candidates.clone() })?;

        let bindings = Pdfium::bind_to_library(&library_path)
            .map_err(|err| Error::PdfiumLoad { path: library_path.clone(), reason: format!("{err:?}") })?;

        Ok(Engine { pdfium: Pdfium::new(bindings), library_path })
    }

    /// The PDFium shared library this engine loaded.
    pub fn library_path(&self) -> &Path {
        &self.library_path
    }

    /// Opens a document from an in-memory copy of the file.
    ///
    /// The engine never holds the source file open, so the original on disk
    /// stays untouched and replaceable (spec section 9, "Document model and saving").
    pub fn open_bytes(&'static self, bytes: Vec<u8>, password: Option<&str>) -> Result<Document> {
        let inner = self.pdfium.load_pdf_from_byte_vec(bytes, password)?;
        Ok(Document::new(inner))
    }

    /// Reads `path` fully into memory and opens it.
    pub fn open_file(&'static self, path: &Path, password: Option<&str>) -> Result<Document> {
        self.open_bytes(std::fs::read(path)?, password)
    }
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine").field("library_path", &self.library_path).finish()
    }
}

fn candidate_dirs(source: &PdfiumSource) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    match source {
        PdfiumSource::Dir(dir) => dirs.push(dir.clone()),
        PdfiumSource::Auto => {
            if let Some(dir) = std::env::var_os("SAFHA_PDFIUM_DIR") {
                dirs.push(PathBuf::from(dir));
            }
            if let Some(exe_dir) =
                std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf))
            {
                // Windows/Linux installs keep the library beside the executable;
                // a macOS .app keeps it in Contents/Frameworks or Contents/Resources.
                dirs.push(exe_dir.clone());
                dirs.push(exe_dir.join("../Frameworks"));
                dirs.push(exe_dir.join("../Resources"));
                dirs.push(exe_dir.join("../lib"));
            }
            #[cfg(debug_assertions)]
            dirs.push(dev_pdfium_dir());
        }
    }
    dirs
}

/// The `scripts/fetch-pdfium.sh` download location for the host platform.
/// Debug builds only: release builds must ship PDFium inside the bundle.
#[cfg(debug_assertions)]
fn dev_pdfium_dir() -> PathBuf {
    let os = match std::env::consts::OS {
        "macos" => "mac",
        "windows" => "win",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        other => other,
    };
    // The Windows package ships the DLL in bin/, the others in lib/.
    let subdir = if cfg!(windows) { "bin" } else { "lib" };
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../third_party/pdfium")
        .join(format!("{os}-{arch}"))
        .join(subdir)
}
