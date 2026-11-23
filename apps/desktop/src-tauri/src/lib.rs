//! Mirsam desktop shell.
//!
//! This crate is deliberately thin: it owns the window, exposes `safha-core`
//! to the UI through typed commands, and nothing else. Engine logic belongs in
//! `safha-core` so the CLI and tests can reach it without a webview.

mod commands;
mod crash;
#[cfg(feature = "devtools")]
mod diagnostics;
mod editing;
mod error;
mod form_design;
mod forms;
mod instance;
mod models;
mod navigation;
mod objects;
mod ocr;
mod output;
#[cfg(target_os = "linux")]
mod print_linux;
mod recovery;
mod redaction;
mod security;
mod signing;
#[cfg(feature = "devtools")]
mod spike_tiles;
mod state;
mod viewer;

use std::path::{Path, PathBuf};
use std::time::Instant;

use state::AppState;
use tauri::{Emitter as _, Manager};

/// Every product command, in one list so release and devtools builds agree.
macro_rules! product_commands {
    ($($extra:path),* $(,)?) => {
        tauri::generate_handler![
            commands::app_info,
            crash::crash_reports,
            crash::new_crash_reports,
            crash::delete_crash_report,
            crash::save_crash_report,
            crash::open_crash_issue,
            crash::record_ui_error,
            commands::open_document_dialog,
            commands::startup,
            commands::new_window,
            commands::move_to_new_window,
            commands::print_document,
            editing::edit_pages,
            redaction::find_redactions,
            redaction::apply_redactions,
            redaction::sanitize_document,
            security::secure_document,
            signing::signing_identities,
            signing::pick_certificate_file,
            signing::unlock_certificate,
            signing::cancel_certificate,
            signing::forget_identity,
            signing::sign_document,
            signing::document_signatures,
            editing::add_markup,
            editing::add_ink,
            editing::add_note,
            editing::add_text_box,
            form_design::design_form,
            form_design::detect_form_fields,
            forms::form_fields,
            forms::set_form_field,
            forms::add_fill_text,
            forms::add_fill_mark,
            forms::place_signature,
            forms::set_annotation_rect,
            forms::set_annotation_color,
            forms::move_annotation_to_page,
            forms::signatures,
            forms::save_drawn_signature,
            forms::save_typed_signature,
            forms::import_signature_image,
            forms::delete_signature,
            editing::add_text,
            editing::page_text_blocks,
            editing::edit_text_block,
            editing::set_annotation_contents,
            editing::remove_annotations,
            editing::page_annotations,
            editing::document_comments,
            editing::reply_to_comment,
            editing::set_comment_resolved,
            editing::undo_edit,
            editing::redo_edit,
            editing::save_document,
            editing::save_document_as,
            editing::close_window_discarding,
            editing::insert_pages_from_files,
            editing::unlock_insert,
            editing::extract_pages,
            editing::split_document,
            editing::bookmark_parts,
            editing::quit_discarding,
            objects::page_objects,
            objects::edit_object,
            objects::replace_image,
            objects::insert_image,
            objects::page_links,
            objects::named_destinations,
            objects::edit_link,
            objects::open_link,
            output::stamp_pages,
            output::pick_watermark_image,
            output::export_page_images,
            output::export_text,
            output::compress_document,
            output::save_compressed,
            recovery::recoveries,
            recovery::restore_recovery,
            recovery::unlock_recovery,
            recovery::discard_recovery,
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
            models::ocr_models,
            models::download_ocr_model,
            models::cancel_ocr_model_download,
            models::delete_ocr_model,
            ocr::start_ocr,
            ocr::ocr_jobs,
            ocr::control_ocr_job,
            ocr::remove_ocr_job,
            ocr::ocr_review,
            ocr::start_ocr_region,
            ocr::check_text_layers,
            ocr::ocr_layer_checks,
            ocr::ocr_decide,
            ocr::ocr_convert,
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
    let context = tauri::generate_context!();
    let documents = launch_documents();

    // One running copy per state directory (ADR 0046): a later copy hands
    // its documents over and exits before it shows anything.
    let mut state_dir = state_dir(&context.config().identifier);
    let mut instance = None;
    if let Some(dir) = &state_dir {
        match instance::claim(dir, &documents) {
            Ok(instance::Claim::HandedOver) => return,
            Ok(instance::Claim::Primary(claimed)) => instance = Some(claimed),
            // Another copy holds the directory but does not answer (hung, or
            // still quitting): run, but write nothing there.
            Err(err @ instance::InstanceError::NoAnswer(_)) => {
                eprintln!("safha: {err}; running without recents, session or recovery");
                state_dir = None;
            }
            Err(err) => eprintln!("safha: {err}; other copies of the app cannot be detected"),
        }
    }

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new(started, documents))
        .manage(output::OutputState::default())
        .manage(signing::SigningState::default())
        // Tiles: BMP images fetched by the webview (ADR 0011).
        .register_asynchronous_uri_scheme_protocol("safha", viewer::tile_protocol)
        .setup(move |app| {
            app.state::<AppState>().set_app(app.handle().clone());
            if let Some(dir) = &state_dir {
                open_store(app, dir);
            }
            // Crash reports (ADR 0061) follow the stored setting, so they start
            // once the store is open, before any window asks for anything.
            crash::start(app.handle(), state_dir.as_deref(), app.state::<AppState>().store());
            if let Some(instance) = instance.take() {
                let (quitting, handle) = (app.handle().clone(), app.handle().clone());
                instance.serve(
                    move || !quitting.state::<AppState>().exiting(),
                    move |documents| commands::open_arrivals(&handle, documents),
                );
            }
            // OCR models, downloaded on demand into the same directory (ADR 0034).
            app.manage(models::ModelsState::new(models::models_root(state_dir.as_deref())));
            // Background OCR jobs, kept in the state store (ADR 0035).
            app.manage(ocr::OcrState::start(
                app.handle().clone(),
                state_dir.as_deref(),
                models::models_root(state_dir.as_deref()),
            ));
            #[cfg(feature = "devtools")]
            diagnostics::prepare_window(app);
            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::Focused(true) => window.state::<AppState>().focused(window.label()),
            // A window holding unsaved changes asks its UI first (ADR 0015);
            // the UI closes it again once the user confirms.
            tauri::WindowEvent::CloseRequested { api, .. } => {
                let state = window.state::<AppState>();
                if !state.take_close_confirmed(window.label())
                    && !state.quit_confirmed()
                    && !state.unsaved(Some(window.label())).is_empty()
                {
                    api.prevent_close();
                    if let Err(err) =
                        window.app_handle().emit_to(window.label(), editing::CLOSE_REQUESTED, ())
                    {
                        eprintln!("safha: could not ask about unsaved changes: {err}");
                    }
                }
            }
            // A window's documents close with it. Its tabs leave the saved
            // session too, unless the app is quitting or it was the last window.
            tauri::WindowEvent::Destroyed => {
                let state = window.state::<AppState>();
                if state.close_window(window.label())
                    && let Err(err) = commands::write_session(&state)
                {
                    eprintln!("safha: could not save the session: {}", err.detail);
                }
            }
            _ => {}
        });

    // Release bundles expose only the product commands. The self-test harness and
    // spike transports exist solely in `devtools` builds.
    #[cfg(not(feature = "devtools"))]
    let builder = builder.invoke_handler(with_breadcrumbs(product_commands![]));
    #[cfg(feature = "devtools")]
    let builder = builder
        .register_asynchronous_uri_scheme_protocol("safha-tile", spike_tiles::protocol)
        .invoke_handler(with_breadcrumbs(product_commands![
            diagnostics::render_stats,
            diagnostics::window_labels,
            diagnostics::selftest_request_close,
            diagnostics::selftest_has_recovery_file,
            diagnostics::webview_rss_kb,
            diagnostics::selftest_log,
            diagnostics::selftest_config,
            diagnostics::selftest_report,
            spike_tiles::spike_tile_raw,
            spike_tiles::spike_tile_channel,
            spike_tiles::spike_tile_base64,
            spike_tiles::spike_pdf_bytes,
            spike_tiles::spike_rss_kb,
            diagnostics::selftest_mark,
            diagnostics::selftest_crash,
            diagnostics::selftest_crash_state,
        ]));

    let app = builder.build(context).expect("failed to start the Mirsam desktop shell");
    app.run(|handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            // The OCR servers hold gigabytes: stop them before the process goes.
            if let Some(ocr) = handle.try_state::<ocr::OcrState>() {
                ocr.shutdown();
            }
            // A clean exit: the next start finds no unfinished run (ADR 0061).
            crash::end_run();
        }
        // macOS asks the running app to open files (Finder, "Open with",
        // `open -a`) rather than starting another copy.
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Opened { urls } = &event {
            let documents = urls.iter().filter_map(|url| url.to_file_path().ok()).collect();
            commands::open_arrivals(handle, documents);
        }
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            let state = handle.state::<AppState>();
            // Unsaved changes: ask in the window of the first such document
            // (ADR 0015); the UI quits again once the user confirms.
            if !state.quit_confirmed()
                && let Some((_, window)) = state.unsaved(None).into_iter().next()
            {
                api.prevent_exit();
                if let Err(err) = handle.emit_to(window.as_str(), editing::QUIT_REQUESTED, ()) {
                    eprintln!("safha: could not ask about unsaved changes: {err}");
                }
                return;
            }
            // Quitting closes every window; their tabs are the session to restore.
            state.set_exiting();
        }
    });
}

/// Records each IPC command's name (never its arguments) for crash reports
/// (ADR 0061), then runs it.
fn with_breadcrumbs<R: tauri::Runtime>(
    handler: impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static,
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    move |invoke| {
        crash::breadcrumb(invoke.message.command());
        handler(invoke)
    }
}

/// Where the app keeps its state: the app data directory (as Tauri resolves
/// it: the platform's data directory and the bundle identifier), or
/// `SAFHA_STATE_DIR` (self-tests and benchmarks, which must not read or touch
/// the user's recents and session).
fn state_dir(identifier: &str) -> Option<PathBuf> {
    let dir = std::env::var_os("SAFHA_STATE_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::data_dir().map(|data| data.join(identifier)));
    if dir.is_none() {
        eprintln!("safha: no app data directory; recents and positions will not be kept");
    }
    dir
}

/// Opens the local state store in `dir`. Without a store the app still
/// runs; it just forgets everything at exit.
fn open_store(app: &tauri::App, dir: &Path) {
    match safha_core::store::Store::open(&dir.join("state.sqlite3")) {
        Ok(store) => app.state::<AppState>().set_store(store),
        Err(err) => eprintln!("safha: could not open the state store in {}: {err}", dir.display()),
    }
    // Recovery files of documents with unsaved changes, beside the store (ADR 0040).
    recovery::open_files(&app.state::<AppState>(), dir);
}

/// The PDFs handed to us at launch, as absolute paths: the command-line
/// arguments that are not flags ("Open with", file associations, a shell),
/// or `SAFHA_OPEN` during development.
fn launch_documents() -> Vec<PathBuf> {
    let mut documents: Vec<PathBuf> = std::env::args_os()
        .skip(1)
        .filter(|arg| !arg.to_string_lossy().starts_with('-'))
        .map(PathBuf::from)
        .collect();
    if documents.is_empty()
        && let Some(path) = std::env::var_os("SAFHA_OPEN")
    {
        documents.push(PathBuf::from(path));
    }
    // A copy that hands them over runs in another directory than the one
    // receiving them.
    documents.into_iter().map(|path| std::path::absolute(&path).unwrap_or(path)).collect()
}
