//! Mirsam desktop shell.
//!
//! This crate is deliberately thin: it owns the window, exposes `safha-core`
//! to the UI through typed commands, and nothing else. Engine logic belongs in
//! `safha-core` so the CLI and tests can reach it without a webview.

mod commands;
#[cfg(feature = "devtools")]
mod diagnostics;
mod error;
mod navigation;
#[cfg(target_os = "linux")]
mod print_linux;
#[cfg(feature = "devtools")]
mod spike_tiles;
mod state;
mod viewer;

use std::path::PathBuf;
use std::time::Instant;

use state::AppState;
use tauri::Manager;

/// Every product command, in one list so release and devtools builds agree.
macro_rules! product_commands {
    ($($extra:path),* $(,)?) => {
        tauri::generate_handler![
            commands::app_info,
            commands::open_document_dialog,
            commands::startup,
            commands::new_window,
            commands::move_to_new_window,
            commands::print_document,
            commands::unlock_document,
            commands::cancel_unlock,
            commands::close_document,
            commands::recent_files,
            commands::open_recent,
            commands::forget_recent,
            commands::clear_recents,
            commands::save_view_state,
            commands::save_session,
            commands::settings,
            commands::set_setting,
            navigation::document_outline,
            navigation::page_labels,
            navigation::page_text,
            navigation::search_document,
            viewer::page_sizes,
            viewer::update_viewport,
            $($extra),*
        ]
    };
}

pub fn run() {
    let started = Instant::now();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new(started, startup_document_path()))
        // Tiles: BMP images fetched by the webview (ADR 0011).
        .register_asynchronous_uri_scheme_protocol("safha", viewer::tile_protocol)
        .setup(|app| {
            open_store(app);
            #[cfg(feature = "devtools")]
            diagnostics::prepare_window(app);
            Ok(())
        })
        // A window's documents close with it. Its tabs leave the saved
        // session too, unless the app is quitting or it was the last window.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                let state = window.state::<AppState>();
                if state.close_window(window.label())
                    && let Err(err) = commands::write_session(&state)
                {
                    eprintln!("safha: could not save the session: {}", err.detail);
                }
            }
        });

    // Release bundles expose only the product commands. The self-test harness and
    // spike transports exist solely in `devtools` builds.
    #[cfg(not(feature = "devtools"))]
    let builder = builder.invoke_handler(product_commands![]);
    #[cfg(feature = "devtools")]
    let builder = builder
        .register_asynchronous_uri_scheme_protocol("safha-tile", spike_tiles::protocol)
        .invoke_handler(product_commands![
            diagnostics::render_stats,
            diagnostics::window_labels,
            diagnostics::webview_rss_kb,
            diagnostics::selftest_log,
            diagnostics::selftest_config,
            diagnostics::selftest_report,
            spike_tiles::spike_tile_raw,
            spike_tiles::spike_tile_channel,
            spike_tiles::spike_tile_base64,
            spike_tiles::spike_pdf_bytes,
            spike_tiles::spike_rss_kb,
        ]);

    let app = builder.build(tauri::generate_context!()).expect("failed to start the Mirsam desktop shell");
    app.run(|handle, event| {
        // Quitting closes every window; their tabs are the session to restore.
        if let tauri::RunEvent::ExitRequested { .. } = event {
            handle.state::<AppState>().set_exiting();
        }
    });
}

/// Opens the local state store in the app data directory, or in
/// `SAFHA_STATE_DIR` (self-tests and benchmarks, which must not read or touch
/// the user's recents and session). Without a store the app still runs; it
/// just forgets everything at exit.
fn open_store(app: &tauri::App) {
    let dir =
        std::env::var_os("SAFHA_STATE_DIR").map(PathBuf::from).or_else(|| app.path().app_data_dir().ok());
    let Some(dir) = dir else {
        eprintln!("safha: no app data directory; recents and positions will not be kept");
        return;
    };
    match safha_core::store::Store::open(&dir.join("state.sqlite3")) {
        Ok(store) => app.state::<AppState>().set_store(store),
        Err(err) => eprintln!("safha: could not open the state store in {}: {err}", dir.display()),
    }
}

/// A PDF handed to us at launch: the first command-line argument that is not a
/// flag ("Open with", file associations), or `SAFHA_OPEN` during development.
fn startup_document_path() -> Option<PathBuf> {
    std::env::args_os()
        .skip(1)
        .find(|arg| !arg.to_string_lossy().starts_with('-'))
        .or_else(|| std::env::var_os("SAFHA_OPEN"))
        .map(PathBuf::from)
}
