//! Printing on Linux (ADR 0013).
//!
//! On macOS and Windows a print window shows the PDF in the platform's own
//! viewer and prints it (ADR 0012). WebKitGTK has no PDF viewer, so here GTK's
//! print dialog drives the job and PDFium draws each page when GTK asks for
//! it, at the printer's resolution:
//! - the dialog's printer, page range, copies, and scaling all apply;
//! - only one page is in memory at a time, whatever the document's length.

use std::path::PathBuf;
use std::sync::Arc;

use gtk::prelude::*;
use gtk::{
    PageOrientation, PrintContext, PrintOperation, PrintOperationAction, PrintOperationResult, Unit, cairo,
};
use mirsam_core::Document;
use tauri::{AppHandle, Manager as _};

use crate::error::{CommandError, CommandResult};

/// Raster resolution bounds. GTK reports the printer's resolution; an A4 page
/// at 600 dpi is already 140 MB of pixels.
const MIN_DPI: f64 = 150.0;
const MAX_DPI: f64 = 600.0;
/// Most pixels one page may take (about 190 MB), for unusually large pages.
const MAX_PIXELS: f64 = 48.0e6;

/// Shows GTK's print dialog over `window_label`'s window and prints. Resolves
/// once the dialog closes: printed, queued, or cancelled.
pub async fn print(
    app: &AppHandle,
    window_label: &str,
    document: Arc<Document>,
    job_name: String,
) -> CommandResult<()> {
    let (done, mut finished) = tauri::async_runtime::channel::<Result<(), String>>(1);
    let label = window_label.to_owned();
    let handle = app.clone();
    // GTK objects live on the main thread.
    app.run_on_main_thread(move || {
        let parent = handle.get_webview_window(&label).and_then(|window| window.gtk_window().ok());
        let outcome = run(document, &job_name, parent.as_ref(), export_target());
        let _ = done.try_send(outcome);
    })
    .map_err(|err| CommandError::new("print_failed", err.to_string()))?;
    match finished.recv().await {
        Some(Ok(())) => Ok(()),
        Some(Err(message)) => Err(CommandError::new("print_failed", message)),
        None => Err(CommandError::new("print_failed", "the print job ended without a result")),
    }
}

/// Self-test builds only: `MIRSAM_PRINT_EXPORT=<file.pdf>` prints to that file
/// without a dialog, so CI can check what would reach the printer.
fn export_target() -> Option<PathBuf> {
    if cfg!(feature = "devtools") { std::env::var_os("MIRSAM_PRINT_EXPORT").map(PathBuf::from) } else { None }
}

fn run(
    document: Arc<Document>,
    job_name: &str,
    parent: Option<&gtk::ApplicationWindow>,
    export: Option<PathBuf>,
) -> Result<(), String> {
    let sizes = document.page_sizes().map_err(|err| err.to_string())?;
    let pages = i32::try_from(sizes.len()).map_err(|_| "too many pages to print".to_owned())?;

    let operation = PrintOperation::new();
    operation.set_job_name(job_name);
    operation.set_n_pages(pages);
    operation.set_unit(Unit::Points);
    // A wide page turns the paper; each page then fits the printable area,
    // centred, and the dialog's own scale applies on top.
    let wide: Vec<bool> = sizes.iter().map(|size| size.width > size.height).collect();
    operation.connect_request_page_setup(move |_, _, page, setup| {
        let wide = usize::try_from(page).ok().and_then(|page| wide.get(page)).copied().unwrap_or(false);
        setup.set_orientation(if wide { PageOrientation::Landscape } else { PageOrientation::Portrait });
    });
    operation.connect_draw_page(move |_, context, page| {
        if let Err(message) = draw_page(&document, context, page) {
            eprintln!("mirsam: printing page {page} failed: {message}");
        }
    });

    let action = match export {
        Some(path) => {
            operation.set_export_filename(path);
            PrintOperationAction::Export
        }
        None => PrintOperationAction::PrintDialog,
    };
    match operation.run(action, parent) {
        Ok(PrintOperationResult::Error) => Err("the print system reported an error".to_owned()),
        // Printed, queued (Apply), cancelled, or still spooling (InProgress).
        Ok(_) => Ok(()),
        Err(err) => Err(err.to_string()),
    }
}

/// Draws one page into GTK's print context: rendered by PDFium at the
/// printer's resolution, fitted to the printable area, centred.
fn draw_page(document: &Document, context: &PrintContext, page: i32) -> Result<(), String> {
    let index = usize::try_from(page).map_err(|_| "negative page number".to_owned())?;
    let size = document.page_size(index).map_err(|err| err.to_string())?;
    let cairo = context.cairo_context().ok_or_else(|| "no drawing context".to_owned())?;

    let (area_width, area_height) = (context.width(), context.height());
    let (page_width, page_height) = (f64::from(size.width), f64::from(size.height));
    if page_width <= 0.0 || page_height <= 0.0 {
        return Err("empty page".to_owned());
    }
    let fit = (area_width / page_width).min(area_height / page_height);
    let dpi = context.dpi_x().max(context.dpi_y()).clamp(MIN_DPI, MAX_DPI);
    // Device pixels per PDF point.
    let mut scale = fit * dpi / 72.0;
    let pixels = page_width * page_height * scale * scale;
    if pixels > MAX_PIXELS {
        scale *= (MAX_PIXELS / pixels).sqrt();
    }

    let bitmap = document.render_page(index, scale as f32).map_err(|err| err.to_string())?;
    let (width, height) = (bitmap.width(), bitmap.height());
    let stride = cairo::Format::ARgb32.stride_for_width(width).map_err(|err| err.to_string())?;
    let (pixel_width, pixel_height) = (
        i32::try_from(width).map_err(|_| "page too wide".to_owned())?,
        i32::try_from(height).map_err(|_| "page too tall".to_owned())?,
    );
    if stride != pixel_width * 4 {
        return Err(format!("unexpected row stride {stride} for {width} px"));
    }
    // Pages render opaque, so BGRA is cairo's premultiplied ARGB32 as is.
    let surface = cairo::ImageSurface::create_for_data(
        bitmap.into_bgra(),
        cairo::Format::ARgb32,
        pixel_width,
        pixel_height,
        stride,
    )
    .map_err(|err| err.to_string())?;

    let (drawn_width, drawn_height) = (page_width * fit, page_height * fit);
    cairo.save().map_err(|err| err.to_string())?;
    cairo.translate((area_width - drawn_width) / 2.0, (area_height - drawn_height) / 2.0);
    cairo.scale(drawn_width / f64::from(width), drawn_height / f64::from(height));
    cairo.set_source_surface(&surface, 0.0, 0.0).map_err(|err| err.to_string())?;
    cairo.source().set_filter(cairo::Filter::Good);
    cairo.paint().map_err(|err| err.to_string())?;
    cairo.restore().map_err(|err| err.to_string())
}
