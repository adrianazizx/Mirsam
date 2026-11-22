//! M6 output commands (ADR 0050): headers, footers, page numbers, and
//! watermarks (one edit each); page images, text, and Markdown written where
//! the user chooses; compression with a size preview before saving.
//!
//! The UI never touches the filesystem: a watermark image is picked and read
//! here, and every exported file is written here after a save dialog.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use safha_core::compress::{CompressOptions, CompressReport};
use safha_core::edit::Change;
use safha_core::export::{ImageFormat, TextFormat};
use safha_core::stamp::{Band, Mark, Stamp, StampKind, Watermark};
use serde::Deserialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::editing::EditOutcome;
use crate::error::{CommandError, CommandResult};
use crate::navigation::blocking;
use crate::state::{AppState, lock};

/// The largest watermark image read, in bytes.
const MAX_IMAGE_FILE: u64 = 50 * 1024 * 1024;

/// A compressed copy: the bytes it was made from, and its own.
type Pending = (Arc<[u8]>, Vec<u8>);

/// What the output commands keep between calls.
#[derive(Default)]
pub struct OutputState {
    /// The watermark image last picked: its file name and bytes.
    image: Mutex<Option<(String, Vec<u8>)>>,
    /// Compressed bytes waiting to be saved, per document, with the bytes
    /// they were made from (stale once the document changes).
    compressed: Mutex<HashMap<u32, Pending>>,
}

/// A stamp as the UI sends it.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StampArg {
    Band { band: Band },
    Watermark { watermark: Watermark },
}

/// Draws a header or footer (page numbers are one) or a watermark on
/// `pages`: one undo step. An image watermark uses the image last picked.
#[tauri::command]
pub async fn stamp_pages(
    state: State<'_, AppState>,
    output: State<'_, OutputState>,
    document_id: u32,
    pages: Vec<usize>,
    stamp: StampArg,
) -> CommandResult<EditOutcome> {
    let kind = match stamp {
        StampArg::Band { band } => StampKind::Band(band),
        StampArg::Watermark { mut watermark } => {
            if let Mark::Image { data, .. } = &mut watermark.mark {
                let picked = lock(&output.image);
                let (_, bytes) = picked
                    .as_ref()
                    .ok_or_else(|| CommandError::new("no_image", "no watermark image picked"))?;
                data.clone_from(bytes);
            }
            StampKind::Watermark(watermark)
        }
    };
    let history = state.history(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let stamp = Stamp { pages, kind };
    // Stamping a long document sets text for every page: off the UI's way.
    let (revision, update) = blocking(move || Ok(document.with_stamp(&stamp)?)).await?;
    let mut history = lock(&history);
    history.push_content(update);
    let page_count = revision.page_count();
    state.replace_revision(document_id, revision);
    Ok(EditOutcome::new(page_count, &history, Change::Content))
}

/// Asks for a PNG or JPEG to use as a watermark and keeps it. Returns its
/// file name, or `None` when the user cancels.
#[tauri::command]
pub async fn pick_watermark_image(
    app: AppHandle,
    output: State<'_, OutputState>,
) -> CommandResult<Option<String>> {
    let picked = app.dialog().file().add_filter("PNG, JPEG", &["png", "jpg", "jpeg"]).blocking_pick_file();
    let Some(path) = picked.and_then(|file| file.into_path().ok()) else { return Ok(None) };
    let size = std::fs::metadata(&path).map_err(safha_core::Error::from)?.len();
    if size > MAX_IMAGE_FILE {
        return Err(CommandError::new("image_unreadable", "the image file is over 50 MB"));
    }
    let bytes = std::fs::read(&path).map_err(safha_core::Error::from)?;
    let png = bytes.starts_with(b"\x89PNG\r\n\x1a\n");
    if !png && safha_core::jpeg::frame(&bytes).is_none() {
        return Err(CommandError::new("image_unreadable", "not a PNG or baseline JPEG file"));
    }
    let name = crate::state::display_name(&path);
    *lock(&output.image) = Some((name.clone(), bytes));
    Ok(Some(name))
}

/// The name of the document's file without its extension, and its folder.
fn stem_and_folder(state: &AppState, document_id: u32) -> CommandResult<(String, Option<PathBuf>)> {
    let current = state.path_of(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let stem = current.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    Ok((stem, current.parent().map(Path::to_path_buf)))
}

/// `path` with `extension` when it has none of `accepted`.
fn with_extension(mut path: PathBuf, extension: &str, accepted: &[&str]) -> PathBuf {
    if !path.extension().is_some_and(|ext| accepted.iter().any(|a| ext.eq_ignore_ascii_case(a))) {
        path.as_mut_os_string().push(format!(".{extension}"));
    }
    path
}

/// `folder/name.ext`, or `folder/name 2.ext`, … whichever does not exist yet.
fn free_path(folder: &Path, name: &str, extension: &str) -> PathBuf {
    let first = folder.join(format!("{name}.{extension}"));
    if !first.exists() {
        return first;
    }
    (2..).map(|n| folder.join(format!("{name} {n}.{extension}"))).find(|path| !path.exists()).unwrap_or(first)
}

/// Writes `pages` as images at `dpi`: one page to a file the user names,
/// several into a folder the user picks (`name (page 3).png`, …). Returns how
/// many files were written, or `None` when the user cancels.
#[tauri::command]
pub async fn export_page_images(
    app: AppHandle,
    state: State<'_, AppState>,
    document_id: u32,
    pages: Vec<usize>,
    dpi: u32,
    format: ImageFormat,
) -> CommandResult<Option<usize>> {
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    if pages.is_empty() || pages.iter().any(|&page| page >= document.page_count()) {
        return Err(CommandError::new("invalid_edit", "pages out of range"));
    }
    let (stem, folder) = stem_and_folder(&state, document_id)?;
    let extension = format.extension();
    let accepted: &[&str] = if extension == "jpg" { &["jpg", "jpeg"] } else { &["png"] };
    let targets: Vec<(usize, PathBuf)> = if let [page] = pages[..] {
        let mut dialog = app
            .dialog()
            .file()
            .add_filter(extension.to_uppercase(), accepted)
            .set_file_name(format!("{stem} (page {}).{extension}", page + 1));
        if let Some(dir) = &folder {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.blocking_save_file().and_then(|file| file.into_path().ok()) else {
            return Ok(None);
        };
        vec![(page, with_extension(path, extension, accepted))]
    } else {
        let mut dialog = app.dialog().file();
        if let Some(dir) = &folder {
            dialog = dialog.set_directory(dir);
        }
        let Some(chosen) = dialog.blocking_pick_folder().and_then(|folder| folder.into_path().ok()) else {
            return Ok(None);
        };
        pages.iter().map(|&page| (page, chosen.join(format!("{stem} (page {})", page + 1)))).collect()
    };
    let several = targets.len() > 1;
    blocking(move || {
        for (page, path) in &targets {
            let bytes = document.export_page_image(*page, dpi, format)?;
            let path = if several {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                free_path(path.parent().unwrap_or(Path::new(".")), &name, extension)
            } else {
                path.clone()
            };
            safha_core::save::write_atomically(&path, &bytes)?;
        }
        Ok(Some(targets.len()))
    })
    .await
}

/// Writes the text of `pages` as plain text or Markdown to a file the user
/// names. Returns the file's name, or `None` when the user cancels.
#[tauri::command]
pub async fn export_text(
    app: AppHandle,
    state: State<'_, AppState>,
    document_id: u32,
    pages: Vec<usize>,
    format: TextFormat,
) -> CommandResult<Option<String>> {
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let (stem, folder) = stem_and_folder(&state, document_id)?;
    let extension = format.extension();
    let label = match format {
        TextFormat::Plain => "Text",
        TextFormat::Markdown => "Markdown",
    };
    let mut dialog =
        app.dialog().file().add_filter(label, &[extension]).set_file_name(format!("{stem}.{extension}"));
    if let Some(dir) = &folder {
        dialog = dialog.set_directory(dir);
    }
    let Some(path) = dialog.blocking_save_file().and_then(|file| file.into_path().ok()) else {
        return Ok(None);
    };
    let path = with_extension(path, extension, &[extension]);
    blocking(move || {
        let text = document.export_text(&pages, format)?;
        safha_core::save::write_atomically(&path, text.as_bytes())?;
        Ok(Some(crate::state::display_name(&path)))
    })
    .await
}

/// Compresses the document in memory and reports the sizes; the result
/// waits for [`save_compressed`]. `edit_unsupported` for signed or
/// encrypted files.
#[tauri::command]
pub async fn compress_document(
    state: State<'_, AppState>,
    output: State<'_, OutputState>,
    document_id: u32,
    options: CompressOptions,
) -> CommandResult<CompressReport> {
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let source = document.bytes();
    let (bytes, report) = blocking(move || Ok(document.compressed(options)?)).await?;
    lock(&output.compressed).insert(document_id, (source, bytes));
    Ok(report)
}

/// Saves the compressed copy made last for this document as a new file the
/// user names. Returns its name, or `None` when the user cancels.
/// `compress_stale` when the document changed since.
#[tauri::command]
pub async fn save_compressed(
    app: AppHandle,
    state: State<'_, AppState>,
    output: State<'_, OutputState>,
    document_id: u32,
) -> CommandResult<Option<String>> {
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let stale = || CommandError::new("compress_stale", "the document changed since it was compressed");
    {
        let pending = lock(&output.compressed);
        let (source, _) = pending.get(&document_id).ok_or_else(stale)?;
        if !Arc::ptr_eq(source, &document.bytes()) {
            return Err(stale());
        }
    }
    let (stem, folder) = stem_and_folder(&state, document_id)?;
    let mut dialog =
        app.dialog().file().add_filter("PDF", &["pdf"]).set_file_name(format!("{stem} (compressed).pdf"));
    if let Some(dir) = &folder {
        dialog = dialog.set_directory(dir);
    }
    let Some(path) = dialog.blocking_save_file().and_then(|file| file.into_path().ok()) else {
        return Ok(None);
    };
    let path = with_extension(path, "pdf", &["pdf"]);
    let (_, bytes) = lock(&output.compressed).remove(&document_id).ok_or_else(stale)?;
    // Checked when it was made (it opens, with every page); written atomically.
    safha_core::save::write_atomically(&path, &bytes)?;
    Ok(Some(crate::state::display_name(&path)))
}

/// Drops what is kept for a closed document.
pub fn forget(output: &OutputState, document_id: u32) {
    lock(&output.compressed).remove(&document_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps_arrive_in_the_shape_the_ui_sends() {
        let band: StampArg = serde_json::from_str(
            r#"{"kind":"band","band":{"edge":"bottom","start":"","center":"{page}","end":"","rtl":true,
            "family":"naskh","size":10,"color":[0,0,0],"margin":30,"digits":"arabicIndic","firstNumber":1}}"#,
        )
        .expect("a band");
        assert!(matches!(band, StampArg::Band { band } if band.rtl));
        let image: StampArg = serde_json::from_str(
            r#"{"kind":"watermark","watermark":{"mark":{"kind":"image","width":0.5},
            "opacity":0.3,"rotation":45,"behind":false}}"#,
        )
        .expect("a watermark");
        assert!(
            matches!(image, StampArg::Watermark { watermark } if matches!(watermark.mark, Mark::Image { .. }))
        );
        let format: ImageFormat = serde_json::from_str(r#"{"kind":"jpeg","quality":85}"#).expect("format");
        assert_eq!(format, ImageFormat::Jpeg { quality: 85 });
        let options: CompressOptions =
            serde_json::from_str(r#"{"imageDpi":150,"jpegQuality":null,"objectStreams":true}"#)
                .expect("options");
        assert_eq!(options, CompressOptions::default());
    }

    #[test]
    fn exported_files_never_overwrite() {
        let dir = std::env::temp_dir().join(format!("safha-output-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("a.png"), b"x").expect("write");
        assert_eq!(free_path(&dir, "a", "png"), dir.join("a 2.png"));
        assert_eq!(free_path(&dir, "b", "png"), dir.join("b.png"));
        assert_eq!(with_extension(PathBuf::from("x.JPEG"), "jpg", &["jpg", "jpeg"]), PathBuf::from("x.JPEG"));
        assert_eq!(with_extension(PathBuf::from("x"), "md", &["md"]), PathBuf::from("x.md"));
    }
}
