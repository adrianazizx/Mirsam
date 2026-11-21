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
