//! Safha desktop shell.
//!
//! This crate is deliberately thin: it owns the window, exposes `safha-core`
//! to the UI through typed commands, and nothing else. Engine logic belongs in
//! `safha-core` so the CLI and tests can reach it without a webview.

mod commands;
#[cfg(feature = "devtools")]
mod diagnostics;
mod error;
#[cfg(feature = "devtools")]
mod spike_tiles;
mod state;

use std::time::Instant;

use state::AppState;

pub fn run() {
    let started = Instant::now();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new(started, startup_document_path()));

    // Release bundles expose only the product commands. The self-test harness and
    // spike transports exist solely in `devtools` builds.
    #[cfg(not(feature = "devtools"))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        commands::app_info,
        commands::open_document_dialog,
        commands::take_startup_document,
        commands::page_size,
        commands::render_page,
        commands::close_document,
    ]);
    #[cfg(feature = "devtools")]
    let builder = builder
        .register_asynchronous_uri_scheme_protocol("safha-tile", spike_tiles::protocol)
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::open_document_dialog,
            commands::take_startup_document,
            commands::page_size,
            commands::render_page,
            commands::close_document,
            diagnostics::selftest_config,
            diagnostics::selftest_report,
            spike_tiles::spike_tile_raw,
            spike_tiles::spike_tile_channel,
            spike_tiles::spike_tile_base64,
            spike_tiles::spike_pdf_bytes,
            spike_tiles::spike_rss_kb,
        ]);

    builder.run(tauri::generate_context!()).expect("failed to start the Safha desktop shell");
}

/// A PDF handed to us at launch: the first command-line argument that is not a
/// flag ("Open with", file associations), or `SAFHA_OPEN` during development.
fn startup_document_path() -> Option<std::path::PathBuf> {
    std::env::args_os()
        .skip(1)
        .find(|arg| !arg.to_string_lossy().starts_with('-'))
        .or_else(|| std::env::var_os("SAFHA_OPEN"))
        .map(std::path::PathBuf::from)
}
