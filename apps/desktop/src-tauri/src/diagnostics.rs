//! Developer self-test harness.
//!
//! macOS has no WebDriver for WKWebView, so end-to-end checks of the real shell
//! work the other way round: launched with `SAFHA_SELFTEST_OUT=<dir>`, the UI
//! drives itself (open the startup document, render, read the canvas back) and
//! reports here; this module writes the evidence to `<dir>` and exits.
//!
//! This module is compiled only with the `devtools` feature, so release bundles
//! expose no file-writing command to the webview.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelftestConfig {
    /// UI locale to switch to before running ("en" or "ar").
    locale: String,
    /// "light" or "dark".
    theme: String,
    /// Name of a spike benchmark to run instead of the plain self-test.
    spike: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelftestReport {
    /// Free-form results from the UI; written verbatim into report.json.
    results: serde_json::Value,
    /// Optional PNG data URL of the page canvas.
    canvas_png: Option<String>,
}

/// During a self-test run, keeps the window above others so it is not
/// occluded: an occluded WKWebView gets no animation frames, which would
/// invalidate every frame-time measurement (ADR 0007).
pub fn prepare_window(app: &tauri::App) {
    use tauri::Manager as _;
    if std::env::var_os("SAFHA_SELFTEST_OUT").is_none() {
        return;
    }
    if let Some(window) = app.get_webview_window("main") {
        // With several displays, a restored window can open on one the user
        // is not looking at (or that is off), where it counts as hidden. Put
        // it in the middle of the display under the pointer.
        if let Ok(cursor) = window.cursor_position()
            && let Ok(Some(monitor)) = window.monitor_from_point(cursor.x, cursor.y)
            && let Ok(size) = window.outer_size()
        {
            let area = monitor.position();
            let bounds = monitor.size();
            let x = area.x
                + (i32::try_from(bounds.width).unwrap_or(0) - i32::try_from(size.width).unwrap_or(0)) / 2;
            let y = area.y
                + (i32::try_from(bounds.height).unwrap_or(0) - i32::try_from(size.height).unwrap_or(0)) / 2;
            let _ = window.set_position(tauri::PhysicalPosition::new(x.max(area.x), y.max(area.y)));
        }
        let _ = window.set_always_on_top(true);
        let _ = window.set_focus();
    }
}

/// A progress line from the UI half of a self-test, written to the shell's
/// stderr (the harness's `app.log`): the webview's console is not captured.
#[tauri::command]
pub fn selftest_log(message: String) {
    eprintln!("[selftest] {message}");
}

/// Asks to close the calling window, as its close button would: with unsaved
/// changes the shell stops and asks the UI (ADR 0015). The webview itself
/// has no permission to close windows.
#[tauri::command]
pub fn selftest_request_close(window: tauri::WebviewWindow) -> CommandResult<()> {
    window.close().map_err(|err| CommandError::new("window_error", err.to_string()))
}

/// Whether a document's unsaved changes are kept in a recovery file that the
/// store lists, for the self-test's autosave check (ADR 0040).
#[tauri::command]
pub fn selftest_has_recovery_file(state: tauri::State<'_, crate::state::AppState>, id: u32) -> bool {
    let Some(key) = state.autosave_target(id).map(|target| target.key) else { return false };
    let listed = state
        .store()
        .and_then(|store| store.recoveries().ok())
        .is_some_and(|entries| entries.iter().any(|entry| entry.key == key));
    listed && state.recovery().is_some_and(|files| files.exists(&key))
}

/// Labels of the open windows, for the self-test's multi-window check.
#[tauri::command]
pub fn window_labels(app: AppHandle) -> Vec<String> {
    use tauri::Manager as _;
    let mut labels: Vec<String> = app.webview_windows().into_keys().collect();
    labels.sort();
    labels
}

/// Render-queue and tile-cache counters, for the scroll benchmark.
#[tauri::command]
pub fn render_stats(state: State<'_, AppState>) -> safha_core::render::RenderStats {
    state.render.stats()
}

/// Resident memory, in kilobytes, of the webview's own processes: on macOS the
/// WebKit content and GPU processes. They are children of launchd, not of the
/// shell, so they are found heuristically: WebKit processes started after this
/// one. Another app starting a WebKit view during a benchmark would be counted.
#[tauri::command]
pub fn webview_rss_kb() -> Option<u64> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let output = std::process::Command::new("ps").args(["-axo", "pid=,rss=,etime=,comm="]).output().ok()?;
    let listing = String::from_utf8_lossy(&output.stdout);
    let processes: Vec<(u32, u64, u64, &str)> = listing
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let rss = fields.next()?.parse().ok()?;
            let elapsed = elapsed_seconds(fields.next()?)?;
            let command = line.rsplit('/').next().unwrap_or(line).trim();
            Some((pid, rss, elapsed, command))
        })
        .collect();
    let own_pid = std::process::id();
    let (_, _, own_elapsed, _) = processes.iter().find(|(pid, ..)| *pid == own_pid)?;
    Some(
        processes
            .iter()
            .filter(|(pid, _, elapsed, command)| {
                (command.contains("com.apple.WebKit.WebContent") || command.contains("com.apple.WebKit.GPU"))
                    && *pid > own_pid
                    && elapsed <= own_elapsed
            })
            .map(|(_, rss, ..)| rss)
            .sum(),
    )
}

/// Parses `ps` elapsed time, `[[dd-]hh:]mm:ss`, into seconds.
fn elapsed_seconds(text: &str) -> Option<u64> {
    let (days, clock) = match text.split_once('-') {
        Some((days, clock)) => (days.parse::<u64>().ok()?, clock),
        None => (0, text),
    };
    let mut seconds = 0;
    for part in clock.split(':') {
        seconds = seconds * 60 + part.parse::<u64>().ok()?;
    }
    Some(days * 86_400 + seconds)
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_ps_elapsed_times() {
        assert_eq!(super::elapsed_seconds("00:07"), Some(7));
        assert_eq!(super::elapsed_seconds("12:34"), Some(754));
        assert_eq!(super::elapsed_seconds("01:00:00"), Some(3600));
        assert_eq!(super::elapsed_seconds("2-00:00:01"), Some(172_801));
        assert_eq!(super::elapsed_seconds("x"), None);
    }
}

#[tauri::command]
pub fn selftest_config() -> Option<SelftestConfig> {
    std::env::var_os("SAFHA_SELFTEST_OUT")?;
    Some(SelftestConfig {
        locale: std::env::var("SAFHA_SELFTEST_LOCALE").unwrap_or_else(|_| "en".into()),
        theme: std::env::var("SAFHA_SELFTEST_THEME").unwrap_or_else(|_| "light".into()),
        spike: std::env::var("SAFHA_SELFTEST_SPIKE").ok().filter(|name| !name.is_empty()),
    })
}

#[tauri::command]
pub fn selftest_report(
    app: AppHandle,
    state: State<'_, AppState>,
    report: SelftestReport,
) -> CommandResult<()> {
    let Some(out_dir) = std::env::var_os("SAFHA_SELFTEST_OUT").map(std::path::PathBuf::from) else {
        return Err(CommandError::new("devtools_disabled", "SAFHA_SELFTEST_OUT is not set"));
    };
    let io = |err: std::io::Error| CommandError::new("io_error", err.to_string());

    std::fs::create_dir_all(&out_dir).map_err(io)?;
    if let Some(png) = report.canvas_png.as_deref().and_then(|url| url.strip_prefix("data:image/png;base64,"))
    {
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(png)
            .map_err(|err| CommandError::new("invalid_report", err.to_string()))?;
        std::fs::write(out_dir.join("canvas.png"), bytes).map_err(io)?;
    }
    let report = serde_json::json!({
        "reportedAfterMs": state.started.elapsed().as_millis(),
        "results": report.results,
    });
    std::fs::write(out_dir.join("report.json"), format!("{report:#}\n")).map_err(io)?;

    app.exit(0);
    Ok(())
}
