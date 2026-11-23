//! Crash reports, kept on this computer until the user sends or deletes them
//! (ADR 0061).
//!
//! The shell captures three kinds of failure and turns each into a
//! [`CrashReport`]:
//!
//! - a Rust panic anywhere in the process (the shell, this crate, a render or
//!   OCR thread), from the panic hook: its message, source location and stack;
//! - a native crash (PDFium, the webview, an abort), which no Rust code sees:
//!   the run's marker file is still there at the next start, and the
//!   operating system's own crash log for that process supplies the exception
//!   and the crashed thread's frames ([`os_logs`]);
//! - an uncaught error in the UI, which the UI hands to the shell.
//!
//! A report holds the app's version, the operating system, the stack, and the
//! names of the last commands run. It never holds document contents, file
//! names or paths: messages and frames go through [`redact`] when the report is
//! made, and what is stored is exactly what [`CrashReport::text`] shows the
//! user and what any [`Destination`] sends.

pub mod os_logs;
pub mod redact;

use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use os_logs::NativeCrash;
pub use redact::Scrubber;

/// Reports kept at most; the oldest go first.
pub const MAX_REPORTS: usize = 20;
/// Commands remembered for a report.
pub const MAX_COMMANDS: usize = 30;
/// Stack frames kept in a report.
pub const MAX_FRAMES: usize = 48;

/// What failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CrashKind {
    /// A Rust panic: the app stopped, or one of its threads did.
    Panic,
    /// The process was ended by the operating system (a crash in native code).
    Native,
    /// An error the UI did not handle.
    Interface,
}

impl CrashKind {
    fn describe(self) -> &'static str {
        match self {
            Self::Panic => "the app stopped (Rust panic)",
            Self::Native => "the app was ended by the system (native crash)",
            Self::Interface => "an error in the window (interface)",
        }
    }
}

/// The build and the machine, the same for every report of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct About {
    /// The app's version, e.g. `0.0.1`.
    pub version: String,
    /// `release` or `debug`, the architecture, and the commit when known.
    pub build: String,
    /// The operating system and its version, e.g. `macOS 26.1`.
    pub os: String,
}

/// One crash, as stored and as shown. Built only through the constructors,
/// which redact what they are given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashReport {
    /// File name in the store; digits, letters and dashes only.
    pub id: String,
    pub kind: CrashKind,
    /// Unix seconds.
    pub created: i64,
    pub about: About,
    /// The redacted message (a panic's, an exception's, an error's).
    pub message: String,
    /// Source location (`crates/safha-core/src/x.rs:12:5`), for panics.
    pub location: Option<String>,
    /// The thread's name, when it has one.
    pub thread: Option<String>,
    /// Stack frames, innermost first.
    pub stack: Vec<String>,
    /// The last commands run, oldest first (names only, never arguments).
    pub commands: Vec<String>,
    /// The user has been asked about it once (the prompt after a crash).
    #[serde(default)]
    pub offered: bool,
}

/// A panic as the hook sees it.
#[derive(Debug, Clone, Copy)]
pub struct PanicDetails<'a> {
    pub message: &'a str,
    /// File, line, column.
    pub location: Option<(&'a str, u32, u32)>,
    pub thread: Option<&'a str>,
    /// `std::backtrace::Backtrace` printed with `{:#}` (frames with addresses).
    pub backtrace: &'a str,
}

impl CrashReport {
    fn new(kind: CrashKind, about: &About, created: i64) -> Self {
        Self {
            id: new_id(created),
            kind,
            created,
            about: about.clone(),
            message: String::new(),
            location: None,
            thread: None,
            stack: Vec::new(),
            commands: Vec::new(),
            offered: false,
        }
    }

    /// A report of a Rust panic.
    pub fn from_panic(
        about: &About,
        created: i64,
        panic: PanicDetails<'_>,
        commands: Vec<String>,
        scrubber: &Scrubber,
    ) -> Self {
        let mut report = Self::new(CrashKind::Panic, about, created);
        report.message = redact::redact_message(panic.message, scrubber);
        report.location = panic
            .location
            .map(|(file, line, column)| format!("{}:{line}:{column}", redact::source_path(file)));
        report.thread = panic.thread.map(|name| redact::redact_message(name, scrubber));
        report.stack = rust_frames(panic.backtrace);
        report.commands = commands;
        report
    }

    /// A report of a native crash, from the operating system's crash log.
    pub fn from_native(about: &About, created: i64, crash: NativeCrash, commands: Vec<String>) -> Self {
        let mut report = Self::new(CrashKind::Native, about, created);
        report.message = crash.exception;
        report.thread = crash.thread;
        report.stack = crash.frames.into_iter().take(MAX_FRAMES).collect();
        report.commands = commands;
        report
    }

    /// A report of an error the UI did not handle.
    pub fn from_interface(
        about: &About,
        created: i64,
        message: &str,
        stack: &str,
        commands: Vec<String>,
        scrubber: &Scrubber,
    ) -> Self {
        let mut report = Self::new(CrashKind::Interface, about, created);
        report.message = redact::redact_message(message, scrubber);
        report.stack = stack
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .take(MAX_FRAMES)
            .map(|line| redact::ui_frame(line, scrubber))
            .collect();
        report.commands = commands.iter().take(MAX_COMMANDS).map(|name| redact::command_name(name)).collect();
        report
    }

    /// The report exactly as it is shown to the user and sent: plain text, in
    /// English (it is read by the developers; the dialog around it is in the
    /// user's language).
    pub fn text(&self) -> String {
        let mut text = String::from("Mirsam crash report\n");
        text += &format!("Kind: {}\n", self.kind.describe());
        text += &format!("Report: {}\n", self.id);
        text += &format!("Time: {}\n", utc_minute(self.created));
        text += &format!("Version: {} ({})\n", self.about.version, self.about.build);
        text += &format!("System: {}\n", self.about.os);
        text += "\nWhat happened:\n";
        text += &format!("  {}\n", if self.message.is_empty() { "(no message)" } else { &self.message });
        match (&self.location, &self.thread) {
            (Some(location), Some(thread)) => text += &format!("  at {location}, thread {thread}\n"),
            (Some(location), None) => text += &format!("  at {location}\n"),
            (None, Some(thread)) => text += &format!("  thread {thread}\n"),
            (None, None) => {}
        }
        if !self.stack.is_empty() {
            text += "\nStack (innermost first):\n";
            for frame in &self.stack {
                text += &format!("  {frame}\n");
            }
        }
        text += "\nLast commands (oldest first):\n";
        if self.commands.is_empty() {
            text += "  (none recorded)\n";
        }
        for command in &self.commands {
            text += &format!("  {command}\n");
        }
        text += "\nNo document contents, file names or paths are included.\n";
        text
    }

    /// One line for a list: the kind and the start of the message.
    pub fn summary(&self) -> String {
        let first = self.message.lines().next().unwrap_or_default();
        let mut summary: String = first.chars().take(100).collect();
        if first.chars().count() > 100 {
            summary.push('…');
        }
        summary
    }
}

/// Whether a panic only follows from an earlier one in the same run: a panic
/// that reaches native code (the webview's callbacks on Windows and Linux)
/// becomes a second, "cannot unwind" panic before the process aborts. One
/// crash makes one report, so these are not reported after a first.
pub fn is_follow_on_panic(message: &str) -> bool {
    ["panic in a function that cannot unwind", "panic in a destructor during cleanup"]
        .iter()
        .any(|follow_on| message.starts_with(follow_on))
}

/// A report id: the time and a random suffix.
fn new_id(created: i64) -> String {
    let random = getrandom::u32().unwrap_or_else(|_| std::process::id());
    format!("{created}-{random:08x}")
}

/// Ids name files in the store, so only these characters are accepted.
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 40
        && id.chars().all(|c| c.is_ascii_digit() || c.is_ascii_lowercase() || c == '-')
}

/// Frames of a Rust backtrace printed with `{:#}`. Each frame is its function
/// name and source position when the binary has them, else its address (a
/// stripped binary: symbolicated later against the build's symbols). The
/// panic machinery before the panicking frame, and the thread start after the
/// outermost frame of our own, are dropped.
pub fn rust_frames(backtrace: &str) -> Vec<String> {
    struct Frame {
        function: String,
        address: Option<String>,
        at: Option<String>,
    }
    let mut frames: Vec<Frame> = Vec::new();
    for line in backtrace.lines() {
        let trimmed = line.trim();
        if let Some(at) = trimmed.strip_prefix("at ") {
            if let Some(frame) = frames.last_mut()
                && frame.at.is_none()
            {
                frame.at = Some(source_position(at));
            }
            continue;
        }
        let Some((index, rest)) = trimmed.split_once(':') else { continue };
        if index.is_empty() || !index.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let rest = rest.trim();
        let (address, function) = match rest.split_once(" - ") {
            Some((address, function)) if address.trim().starts_with("0x") => {
                (Some(address.trim().to_owned()), function.trim())
            }
            _ => (None, rest),
        };
        frames.push(Frame { function: redact::function_name(function), address, at: None });
    }

    let machinery = |function: &str| {
        [
            "std::backtrace",
            "std::panicking",
            "core::panicking",
            "std::sys::backtrace::__rust_end_short_backtrace",
            "rust_begin_unwind",
            "__rustc::",
            "::crash::",
        ]
        .iter()
        .any(|marker| function.contains(marker))
    };
    // The panic starts where the panicking code called into the panic
    // machinery (`panic_fmt`, `rust_begin_unwind`); frames above that are the
    // hook's own. The same machinery further down (a thread's
    // `catch_unwind`) is the thread start, cut below.
    let entry = frames.iter().position(|frame| {
        ["rust_begin_unwind", "panicking::panic_fmt", "panicking::begin_panic"]
            .iter()
            .any(|marker| frame.function.contains(marker))
    });
    let skip_from = |from: usize| {
        frames[from..]
            .iter()
            .position(|frame| !machinery(&frame.function))
            .map_or(frames.len(), |at| from + at)
    };
    let start = skip_from(entry.unwrap_or(0));
    let end = frames[start..]
        .iter()
        .position(|frame| frame.function.contains("__rust_begin_short_backtrace"))
        .map_or(frames.len(), |at| start + at);
    frames[start..end]
        .iter()
        .take(MAX_FRAMES)
        .map(|frame| {
            let unnamed = frame.function.is_empty()
                || frame.function == "<unknown>"
                || frame.function.starts_with("__mh_execute_header");
            match (unnamed, &frame.address, &frame.at) {
                (true, Some(address), _) => address.clone(),
                (true, None, _) => "<unknown>".to_owned(),
                (false, _, Some(at)) => format!("{} ({at})", frame.function),
                (false, _, None) => frame.function.clone(),
            }
        })
        .collect()
}

/// `/path/to/file.rs:12:5` → `crates/…/file.rs:12:5`.
fn source_position(at: &str) -> String {
    // The path may contain colons (Windows drives); the position is the last
    // one or two numeric fields.
    let mut parts: Vec<&str> = at.rsplitn(3, ':').collect();
    parts.reverse();
    let numeric = |part: &&str| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit());
    match parts.as_slice() {
        [path, line, column] if numeric(line) && numeric(column) => {
            format!("{}:{line}:{column}", redact::source_path(path))
        }
        _ => match at.rsplit_once(':') {
            Some((path, line)) if numeric(&line) => format!("{}:{line}", redact::source_path(path)),
            _ => redact::source_path(at),
        },
    }
}

/// `2026-11-23 10:42 UTC`.
pub fn utc_minute(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let seconds = unix.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02} UTC", seconds / 3600, seconds % 3600 / 60)
}

/// Howard Hinnant's days-to-civil for the proleptic Gregorian calendar.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Now, in Unix seconds.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(i64::MAX))
}

/// The names of the last commands run, oldest first, with repeats folded
/// (`update_viewport ×120`). Names only: arguments may hold document text.
#[derive(Debug, Default)]
pub struct Breadcrumbs {
    entries: VecDeque<(String, u32)>,
    dirty: bool,
}

impl Breadcrumbs {
    pub const fn new() -> Self {
        Self { entries: VecDeque::new(), dirty: false }
    }

    pub fn push(&mut self, name: &str) {
        self.dirty = true;
        if let Some((last, count)) = self.entries.back_mut()
            && last == name
        {
            *count = count.saturating_add(1);
            return;
        }
        if self.entries.len() == MAX_COMMANDS {
            self.entries.pop_front();
        }
        self.entries.push_back((name.chars().take(64).collect(), 1));
    }

    pub fn list(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|(name, count)| if *count > 1 { format!("{name} ×{count}") } else { name.clone() })
            .collect()
    }

    /// True once after any change: the run marker is written again then.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }
}

/// A run of the app, kept in the store while it runs. Found at the next start
/// when the run did not end cleanly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub pid: u32,
    /// Unix seconds.
    pub started: i64,
    /// Last written, Unix seconds.
    pub updated: i64,
    pub about: About,
    /// The executable's file name, to find the system's crash log.
    pub exe: String,
    pub commands: Vec<String>,
    /// A panic report was written for this run: no native report is needed.
    pub reported: bool,
}

/// How long a run that ended without a trace waits for the system's crash log
/// to appear (it is written a few seconds after the crash; the app may have
/// been started again before that).
pub const UNCONFIRMED_DAYS: i64 = 2;
/// Runs waiting for a crash log at most (force-quits leave runs that never get one).
pub const MAX_UNCONFIRMED: usize = 10;

/// Crash reports, and the marker of the current run, in a folder of the app's
/// data directory.
#[derive(Debug, Clone)]
pub struct CrashStore {
    dir: PathBuf,
}

const RUNNING: &str = "running.json";
const UNCONFIRMED: &str = "unconfirmed.json";

impl CrashStore {
    pub fn open(dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        Ok(Self { dir: dir.to_path_buf() })
    }

    fn report_path(&self, id: &str) -> Option<PathBuf> {
        is_valid_id(id).then(|| self.dir.join(format!("{id}.json")))
    }

    /// Stores a report, then drops the oldest beyond [`MAX_REPORTS`].
    pub fn write(&self, report: &CrashReport) -> io::Result<()> {
        let path = self.report_path(&report.id).ok_or_else(|| io::Error::other("invalid report id"))?;
        write_atomic(&path, &serde_json::to_vec_pretty(report).map_err(io::Error::other)?)?;
        for old in self.list().into_iter().skip(MAX_REPORTS) {
            let _ = self.delete(&old.id);
        }
        Ok(())
    }

    /// Every stored report, newest first. Unreadable files are skipped.
    pub fn list(&self) -> Vec<CrashReport> {
        let Ok(entries) = fs::read_dir(&self.dir) else { return Vec::new() };
        let mut reports: Vec<CrashReport> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().is_some_and(|ext| ext == "json")
                    && path.file_stem().and_then(|stem| stem.to_str()).is_some_and(is_valid_id)
            })
            .filter_map(|path| serde_json::from_slice::<CrashReport>(&fs::read(path).ok()?).ok())
            .collect();
        reports.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| b.id.cmp(&a.id)));
        reports
    }

    pub fn get(&self, id: &str) -> Option<CrashReport> {
        serde_json::from_slice(&fs::read(self.report_path(id)?).ok()?).ok()
    }

    /// Deletes a report. False when there was none.
    pub fn delete(&self, id: &str) -> io::Result<bool> {
        let Some(path) = self.report_path(id) else { return Ok(false) };
        match fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err),
        }
    }

    /// Reports not yet offered to the user, marked offered as they are returned.
    pub fn take_unoffered(&self) -> Vec<CrashReport> {
        let mut taken = Vec::new();
        for mut report in self.list() {
            if report.offered {
                continue;
            }
            report.offered = true;
            if self.write_quiet(&report) {
                taken.push(report);
            }
        }
        taken
    }

    fn write_quiet(&self, report: &CrashReport) -> bool {
        self.report_path(&report.id)
            .and_then(|path| write_atomic(&path, &serde_json::to_vec_pretty(report).ok()?).ok())
            .is_some()
    }

    /// Begins a run: returns the earlier runs that did not end cleanly (the
    /// last run's marker, and runs still waiting for a system crash log), and
    /// writes this run's marker.
    pub fn begin_run(&self, record: &RunRecord) -> Vec<RunRecord> {
        let mut earlier: Vec<RunRecord> = read_json(&self.dir.join(UNCONFIRMED)).unwrap_or_default();
        if let Some(last) = read_json::<RunRecord>(&self.dir.join(RUNNING))
            && last.pid != record.pid
        {
            earlier.push(last);
        }
        let _ = fs::remove_file(self.dir.join(UNCONFIRMED));
        self.update_run(record);
        earlier
    }

    /// Writes the current run's marker again (its commands, `reported`).
    pub fn update_run(&self, record: &RunRecord) {
        if let Ok(bytes) = serde_json::to_vec(record) {
            let _ = write_atomic(&self.dir.join(RUNNING), &bytes);
        }
    }

    /// The run ended cleanly.
    pub fn end_run(&self) {
        let _ = fs::remove_file(self.dir.join(RUNNING));
    }

    /// Runs to look for again at the next start (see [`UNCONFIRMED_DAYS`]);
    /// the [`MAX_UNCONFIRMED`] most recent.
    pub fn keep_unconfirmed(&self, runs: &[RunRecord]) {
        let path = self.dir.join(UNCONFIRMED);
        let mut runs = runs.to_vec();
        runs.sort_by_key(|run| std::cmp::Reverse(run.updated));
        runs.truncate(MAX_UNCONFIRMED);
        if runs.is_empty() {
            let _ = fs::remove_file(path);
        } else if let Ok(bytes) = serde_json::to_vec(&runs) {
            let _ = write_atomic(&path, &bytes);
        }
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Writes through a temporary file and a rename, so a crash mid-write leaves
/// the old file or the new one, never half of one.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

/// What became of the earlier runs that did not end cleanly.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct EarlierRuns {
    /// Native crashes found in the system's crash logs.
    pub crashes: Vec<(RunRecord, NativeCrash)>,
    /// Runs to look for again at the next start.
    pub unconfirmed: Vec<RunRecord>,
}

/// Sorts earlier runs into crashes found by `find` and runs to look for again.
/// A run with a panic report already is done; so is one older than
/// [`UNCONFIRMED_DAYS`] without a crash log (force-quit, power loss), or any
/// run where the system keeps no crash logs we can read.
pub fn resolve_earlier_runs(
    runs: Vec<RunRecord>,
    now: i64,
    find: impl Fn(&RunRecord) -> Option<NativeCrash>,
) -> EarlierRuns {
    let mut resolved = EarlierRuns::default();
    for run in runs {
        if run.reported {
            continue;
        }
        match find(&run) {
            Some(crash) => resolved.crashes.push((run, crash)),
            None if os_logs::SUPPORTED && now - run.updated < UNCONFIRMED_DAYS * 86_400 => {
                resolved.unconfirmed.push(run);
            }
            None => {}
        }
    }
    resolved
}

/// Where reports go when the user sends one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// The user sends the report themselves: copies it, saves it as a file,
    /// or opens a pre-filled issue on a GitHub repository (`owner/name`),
    /// when one is set. Nothing is sent by the app itself.
    Manual { github: Option<String> },
}

/// The destination of builds that do not choose one (`SAFHA_CRASH_DESTINATION`
/// at build time, see the shell).
pub const DEFAULT_DESTINATION: &str = "github:adrianazizx/Mirsam";

impl Destination {
    /// `github:owner/name`, or `manual` (copy and save only). A crash server
    /// (`https://…`) is not built: whether to run one is the owner's decision
    /// (ADR 0061).
    pub fn parse(spec: &str) -> Result<Self, String> {
        let spec = spec.trim();
        if spec == "manual" {
            return Ok(Self::Manual { github: None });
        }
        if let Some(repo) = spec.strip_prefix("github:") {
            let valid = repo.split('/').count() == 2
                && repo.split('/').all(|part| {
                    !part.is_empty()
                        && part != "."
                        && part != ".."
                        && part.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
                });
            return if valid {
                Ok(Self::Manual { github: Some(repo.to_owned()) })
            } else {
                Err(format!("not a GitHub repository: {repo:?}"))
            };
        }
        if spec.starts_with("https://") || spec.starts_with("http://") {
            return Err("crash servers are not supported yet (ADR 0061)".to_owned());
        }
        Err(format!("unknown crash report destination {spec:?}"))
    }

    /// The GitHub repository reports can be filed on, if any.
    pub fn github(&self) -> Option<&str> {
        match self {
            Self::Manual { github } => github.as_deref(),
        }
    }

    /// A link that opens a new issue with the report filled in. Long reports
    /// lose stack frames from the end to fit what browsers and GitHub accept;
    /// the flag says so (the UI copies the whole report first).
    pub fn issue_url(&self, report: &CrashReport) -> Option<(String, bool)> {
        let repo = self.github()?;
        let title = format!("Crash: {}", report.summary());
        let mut shortened = report.clone();
        let mut truncated = false;
        loop {
            let body = format!(
                "<!-- Mirsam crash report. Say what you were doing if you like; please don't attach documents. -->\n\n```\n{}```\n{}",
                shortened.text(),
                if truncated { "\n(The stack was shortened to fit the link.)\n" } else { "" },
            );
            let url = format!(
                "https://github.com/{repo}/issues/new?title={}&body={}",
                percent_encode(&title),
                percent_encode(&body)
            );
            if url.len() <= MAX_ISSUE_URL || shortened.stack.is_empty() {
                return Some((url, truncated));
            }
            let keep = shortened.stack.len() * 3 / 4;
            shortened.stack.truncate(keep);
            truncated = true;
        }
    }
}

/// Longest issue link built. GitHub answers 414 above about 8 KB.
const MAX_ISSUE_URL: usize = 7_500;

/// Percent-encodes everything but RFC 3986's unreserved characters.
fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests;
