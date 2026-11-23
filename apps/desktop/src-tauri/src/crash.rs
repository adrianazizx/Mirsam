//! Crash reports (ADR 0061): the panic hook, this run's marker, the names of
//! the last commands run, and the commands the UI reviews and sends reports
//! with. The reports themselves (what they hold, redaction, the store, the
//! system crash logs) are `safha_core::crash`.
//!
//! Nothing here sends anything. A report leaves the computer only when the
//! user copies it, saves it, or opens a pre-filled issue in their browser.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use safha_core::crash::{
    self, About, Breadcrumbs, CrashKind, CrashReport, CrashStore, Destination, PanicDetails, RunRecord,
    Scrubber,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter as _};
use tauri_plugin_dialog::DialogExt;

use crate::error::{CommandError, CommandResult};

/// The setting: `ask` (the default: keep reports and ask after a crash) or
/// `off` (keep none).
pub const SETTING: &str = "crashReports";

/// Where reports can be sent, chosen at build time:
/// `SAFHA_CRASH_DESTINATION=github:owner/name` or `manual`.
fn destination() -> Destination {
    let spec = option_env!("SAFHA_CRASH_DESTINATION").unwrap_or(crash::DEFAULT_DESTINATION);
    Destination::parse(spec).unwrap_or_else(|err| {
        eprintln!("safha: crash reports: {err}; reports can be copied and saved only");
        Destination::Manual { github: None }
    })
}

/// Commands too frequent to say anything (scrolling fetches page data) or
/// part of the reporting itself.
const QUIET: &[&str] = &[
    "update_viewport",
    "page_text",
    "page_annotations",
    "page_links",
    "page_objects",
    "page_text_blocks",
    "record_ui_error",
    "selftest_log",
    "webview_rss_kb",
    "render_stats",
];
/// Panic reports per run: later panics are usually the first one's
/// consequences (a poisoned lock).
const MAX_PANICS: usize = 3;
/// Interface error reports per run.
const MAX_INTERFACE: usize = 5;
/// How often the run marker takes the latest commands.
const MARKER_INTERVAL: Duration = Duration::from_secs(2);
/// How long after the start the system's crash logs of earlier runs are
/// looked for again, and how often: macOS writes one from seconds to well over
/// a minute after the crash, and the user may have started the app again at once.
const WATCH_FOR: Duration = Duration::from_secs(180);
const WATCH_EVERY: Duration = Duration::from_secs(5);
/// Event to every window: reports were added while the app runs (a crash
/// log found late); the UI asks for new reports again.
pub const REPORTS_ADDED: &str = "crash-reports-added";

static BREADCRUMBS: Mutex<Breadcrumbs> = Mutex::new(Breadcrumbs::new());
/// Paths of documents opened in this run: their names are scrubbed from
/// messages. Kept after a document closes; a message may still name it.
static PRIVATE: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
static CONTEXT: OnceLock<Context> = OnceLock::new();

struct Context {
    store: CrashStore,
    about: About,
    run: Mutex<RunRecord>,
    enabled: AtomicBool,
    panics: AtomicUsize,
    interface: AtomicUsize,
    destination: Destination,
}

/// Records a backend command's name (never its arguments).
pub fn breadcrumb(command: &str) {
    if QUIET.contains(&command) {
        return;
    }
    if let Ok(mut crumbs) = BREADCRUMBS.lock() {
        crumbs.push(command);
    }
}

/// Remembers a document's path, so its name never appears in a report.
pub fn remember_path(path: &Path) {
    if let Ok(mut private) = PRIVATE.lock() {
        private.extend(Scrubber::path_secrets(path));
    }
}

fn scrubber() -> Scrubber {
    let mut secrets = Scrubber::user_secrets();
    // The hook may run while another thread holds the lock: then only the
    // user's own strings are scrubbed, and the word rules do the rest.
    if let Ok(private) = PRIVATE.try_lock() {
        secrets.extend(private.iter().cloned());
    }
    Scrubber::new(secrets)
}

/// Starts crash reporting for this run, once the state store is open:
/// earlier runs that ended in a native crash become reports, this run's
/// marker is written, and the panic hook is installed. Without a state
/// directory nothing is kept (the panic hook still prints, as by default).
pub fn start(app: &AppHandle, state_dir: Option<&Path>, store: Option<&safha_core::store::Store>) {
    let Some(dir) = state_dir else { return };
    let crash_store = match CrashStore::open(&dir.join("crash-reports")) {
        Ok(store) => store,
        Err(err) => {
            eprintln!("safha: crash reports cannot be kept: {err}");
            return;
        }
    };
    let enabled = store.and_then(|store| store.setting(SETTING).ok().flatten()).as_deref() != Some("off");
    let about = about();
    let now = crash::now();
    let record = RunRecord {
        pid: std::process::id(),
        started: now,
        updated: now,
        about: about.clone(),
        exe: std::env::current_exe()
            .ok()
            .and_then(|exe| exe.file_name().map(|name| name.to_string_lossy().into_owned()))
            .unwrap_or_default(),
        commands: Vec::new(),
        reported: false,
    };
    let earlier = crash_store.begin_run(&record);
    let mut waiting = Vec::new();
    if enabled && !earlier.is_empty() {
        let resolved = crash::resolve_earlier_runs(earlier, now, crash::os_logs::find_crash);
        for (run, native) in resolved.crashes {
            keep_native(&crash_store, run, native);
        }
        crash_store.keep_unconfirmed(&resolved.unconfirmed);
        waiting = resolved.unconfirmed;
    }
    let context = Context {
        store: crash_store,
        about,
        run: Mutex::new(record),
        enabled: AtomicBool::new(enabled),
        panics: AtomicUsize::new(0),
        interface: AtomicUsize::new(0),
        destination: destination(),
    };
    if CONTEXT.set(context).is_err() {
        return;
    }
    install_panic_hook();
    if !waiting.is_empty() {
        watch_for_crash_logs(app.clone(), waiting);
    }
    let marker = std::thread::Builder::new().name("safha-crash-marker".into()).spawn(|| {
        loop {
            std::thread::sleep(MARKER_INTERVAL);
            write_marker();
        }
    });
    if let Err(err) = marker {
        eprintln!("safha: crash reports: no marker thread: {err}");
    }
}

fn keep_native(store: &CrashStore, run: RunRecord, native: crash::NativeCrash) {
    let report = CrashReport::from_native(&run.about, run.updated, native, run.commands);
    if let Err(err) = store.write(&report) {
        eprintln!("safha: could not keep a crash report: {err}");
    }
}

/// Looks again, for a while, for the system's crash logs of earlier runs
/// that ended without one yet; each found becomes a report, and the windows
/// are told.
fn watch_for_crash_logs(app: AppHandle, mut waiting: Vec<RunRecord>) {
    let watcher = std::thread::Builder::new().name("safha-crash-logs".into()).spawn(move || {
        let deadline = Instant::now() + WATCH_FOR;
        while !waiting.is_empty() && Instant::now() < deadline {
            std::thread::sleep(WATCH_EVERY);
            let Some(context) = CONTEXT.get() else { return };
            if !context.enabled.load(Ordering::Relaxed) {
                return;
            }
            let before = waiting.len();
            let mut found = Vec::new();
            waiting.retain(|run| match crash::os_logs::find_crash(run) {
                Some(native) => {
                    found.push((run.clone(), native));
                    false
                }
                None => true,
            });
            if waiting.len() == before {
                continue;
            }
            for (run, native) in found {
                keep_native(&context.store, run, native);
            }
            context.store.keep_unconfirmed(&waiting);
            if let Err(err) = app.emit(REPORTS_ADDED, ()) {
                eprintln!("safha: could not announce a crash report: {err}");
            }
        }
    });
    if let Err(err) = watcher {
        eprintln!("safha: crash reports: no crash log watcher: {err}");
    }
}

/// The run marker takes the latest commands, if they changed.
fn write_marker() {
    let Some(context) = CONTEXT.get() else { return };
    let commands = {
        let Ok(mut crumbs) = BREADCRUMBS.lock() else { return };
        if !crumbs.take_dirty() {
            return;
        }
        crumbs.list()
    };
    if let Ok(mut run) = context.run.lock() {
        run.commands = if context.enabled.load(Ordering::Relaxed) { commands } else { Vec::new() };
        run.updated = crash::now();
        context.store.update_run(&run);
    }
}

/// The app is quitting normally: the next start finds no unfinished run.
pub fn end_run() {
    if let Some(context) = CONTEXT.get() {
        context.store.end_run();
    }
}

/// Follows the setting when the user changes it.
pub fn set_enabled(value: &str) {
    if let Some(context) = CONTEXT.get() {
        context.enabled.store(value != "off", Ordering::Relaxed);
    }
}

fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        record_panic(info);
    }));
}

fn record_panic(info: &std::panic::PanicHookInfo<'_>) {
    let Some(context) = CONTEXT.get() else { return };
    let payload = info.payload();
    let message = payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(a panic without a message)".to_owned());
    // The first panic's report says what happened; the "cannot unwind" panic
    // that follows it out of native code only repeats it.
    if crash::is_follow_on_panic(&message) && context.panics.load(Ordering::Relaxed) > 0 {
        return;
    }
    if !context.enabled.load(Ordering::Relaxed)
        || context.panics.fetch_add(1, Ordering::Relaxed) >= MAX_PANICS
    {
        return;
    }
    let thread = std::thread::current();
    let backtrace = format!("{:#}", std::backtrace::Backtrace::force_capture());
    // `try_lock`: the panicking thread may hold the lock.
    let commands = BREADCRUMBS.try_lock().map(|crumbs| crumbs.list()).unwrap_or_default();
    let details = PanicDetails {
        message: &message,
        location: info.location().map(|at| (at.file(), at.line(), at.column())),
        thread: thread.name(),
        backtrace: &backtrace,
    };
    let report = CrashReport::from_panic(&context.about, crash::now(), details, commands, &scrubber());
    if let Err(err) = context.store.write(&report) {
        eprintln!("safha: could not keep a crash report: {err}");
        return;
    }
    if let Ok(mut run) = context.run.try_lock() {
        run.reported = true;
        run.updated = crash::now();
        context.store.update_run(&run);
    }
}

/// The app's version, build and operating system.
fn about() -> About {
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    let commit = option_env!("SAFHA_BUILD_COMMIT")
        .filter(|commit| !commit.is_empty())
        .map(|commit| format!(", commit {commit}"))
        .unwrap_or_default();
    About {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        build: format!("{profile}, {}{commit}", std::env::consts::ARCH),
        os: os_description(),
    }
}

#[cfg(target_os = "macos")]
fn os_description() -> String {
    let plist =
        std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist").unwrap_or_default();
    let value = |key: &str| {
        let after = plist.split(&format!("<key>{key}</key>")).nth(1)?;
        let start = after.find("<string>")? + "<string>".len();
        let end = after[start..].find("</string>")?;
        Some(after[start..start + end].trim().to_owned())
    };
    match (value("ProductVersion"), value("ProductBuildVersion")) {
        (Some(version), Some(build)) => format!("macOS {version} ({build})"),
        (Some(version), None) => format!("macOS {version}"),
        _ => "macOS".to_owned(),
    }
}

#[cfg(target_os = "windows")]
fn os_description() -> String {
    let version = windows_version::OsVersion::current();
    let name = if version.major == 10 && version.build >= 22_000 { "Windows 11" } else { "Windows 10" };
    format!("{name} ({}.{}.{})", version.major, version.minor, version.build)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn os_name() -> &'static str {
    match std::env::consts::OS {
        "linux" => "Linux",
        "freebsd" => "FreeBSD",
        other => other,
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn os_description() -> String {
    let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let pretty = release.lines().find_map(|line| line.strip_prefix("PRETTY_NAME=")).map(|name| {
        name.trim_matches('"')
            .chars()
            .filter(|c| c.is_ascii() && !c.is_control())
            .take(60)
            .collect::<String>()
    });
    match pretty {
        Some(name) if !name.is_empty() => format!("{} ({name})", os_name()),
        _ => os_name().to_owned(),
    }
}

/// A kept report, for the UI.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashReportInfo {
    id: String,
    kind: CrashKind,
    /// Unix seconds.
    created: i64,
    summary: String,
    /// Exactly what is copied, saved or sent.
    text: String,
}

impl From<CrashReport> for CrashReportInfo {
    fn from(report: CrashReport) -> Self {
        Self {
            text: report.text(),
            summary: report.summary(),
            id: report.id,
            kind: report.kind,
            created: report.created,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashReports {
    reports: Vec<CrashReportInfo>,
    /// The GitHub repository a report can be filed on (`owner/name`), if any.
    github: Option<String>,
}

fn context() -> CommandResult<&'static Context> {
    CONTEXT.get().ok_or_else(|| CommandError::new("crash_reports_unavailable", "no app data folder"))
}

/// Every kept report, newest first.
#[tauri::command]
pub fn crash_reports() -> CrashReports {
    match CONTEXT.get() {
        Some(context) => CrashReports {
            reports: context.store.list().into_iter().map(CrashReportInfo::from).collect(),
            github: context.destination.github().map(str::to_owned),
        },
        None => CrashReports { reports: Vec::new(), github: None },
    }
}

/// Reports the user has not been asked about yet; asked about once, so they
/// are marked as offered now. Empty when crash reports are off.
#[tauri::command]
pub fn new_crash_reports() -> Vec<CrashReportInfo> {
    match CONTEXT.get() {
        Some(context) if context.enabled.load(Ordering::Relaxed) => {
            context.store.take_unoffered().into_iter().map(CrashReportInfo::from).collect()
        }
        _ => Vec::new(),
    }
}

#[tauri::command]
pub fn delete_crash_report(id: String) -> CommandResult<()> {
    context()?.store.delete(&id).map_err(|err| CommandError::new("io_error", err.to_string()))?;
    Ok(())
}

/// Saves a report as a text file where the user chooses. False when they cancel.
#[tauri::command]
pub async fn save_crash_report(app: AppHandle, id: String) -> CommandResult<bool> {
    let report = report(&id)?;
    let picked = app
        .dialog()
        .file()
        .add_filter("Text", &["txt"])
        .set_file_name(format!("Mirsam crash report {}.txt", report.id))
        .blocking_save_file();
    let Some(path) = picked.and_then(|file| file.into_path().ok()) else { return Ok(false) };
    std::fs::write(&path, report.text()).map_err(|err| CommandError::new("io_error", err.to_string()))?;
    Ok(true)
}

/// Opens a new issue on the project's GitHub repository in the browser, with
/// the report filled in. True when the stack had to be shortened to fit the
/// link (the UI has copied the whole report to the clipboard first).
#[tauri::command]
pub async fn open_crash_issue(id: String) -> CommandResult<bool> {
    let context = context()?;
    let report = report(&id)?;
    let Some((url, shortened)) = context.destination.issue_url(&report) else {
        return Err(CommandError::new("crash_issue_unavailable", "no issue tracker is set for this build"));
    };
    open::that_detached(&url).map_err(|err| CommandError::new("io_error", err.to_string()))?;
    Ok(shortened)
}

fn report(id: &str) -> CommandResult<CrashReport> {
    context()?
        .store
        .get(id)
        .ok_or_else(|| CommandError::new("crash_report_missing", format!("no report {id:?}")))
}

/// An error the UI did not handle (ADR 0061): kept as a report, redacted
/// here. Returns the report's id, or `None` when none was kept (reports off,
/// or this run has kept enough of them).
#[tauri::command]
pub fn record_ui_error(message: String, stack: String, commands: Vec<String>) -> Option<String> {
    let context = CONTEXT.get()?;
    if !context.enabled.load(Ordering::Relaxed)
        || context.interface.fetch_add(1, Ordering::Relaxed) >= MAX_INTERFACE
    {
        return None;
    }
    let message: String = message.chars().take(4_000).collect();
    let stack: String = stack.chars().take(16_000).collect();
    let commands: Vec<String> = commands.into_iter().rev().take(crash::MAX_COMMANDS).rev().collect();
    let report =
        CrashReport::from_interface(&context.about, crash::now(), &message, &stack, commands, &scrubber());
    match context.store.write(&report) {
        Ok(()) => Some(report.id),
        Err(err) => {
            eprintln!("safha: could not keep a crash report: {err}");
            None
        }
    }
}

/// Self-test only: what the run marker says now, and the reports kept.
#[cfg(feature = "devtools")]
pub fn selftest_snapshot() -> serde_json::Value {
    write_marker();
    let Some(context) = CONTEXT.get() else { return serde_json::Value::Null };
    let commands = context.run.lock().map(|run| run.commands.clone()).unwrap_or_default();
    serde_json::json!({
        "markerCommands": commands,
        "reports": context.store.list().len(),
    })
}
