# 0061 — Opt-in crash reports, kept on the computer

Status: Accepted. Date: 2026-11-23. Whether to run a crash server is the owner's decision
(`PROGRESS.md`, owner item 25); nothing here needs one.

## Context

Spec §12 puts "crash reporting (opt-in)" in M7. Spec §1 promises "no telemetry without opt-in",
and D4 rules out accounts and a backend in v1. Three things can fail:

- **A Rust panic**, in the shell, in `safha-core`, or on one of its threads (rendering, OCR,
  downloads). Rust code runs when it happens: the panic hook.
- **A native crash.** PDFium (C++), the webview, an `abort()`: the process is killed by a
  signal or an exception and no Rust code runs afterwards.
- **An error in the UI** that nothing handled: a render error unmounts the whole React tree and
  leaves a blank window; an error in an event handler is lost.

A report must never carry document contents, file names or paths. Rust's own panic messages
quote strings (slicing a string inside a character quotes the character, and older toolchains
the whole string), our errors name files, and system crash logs carry the executable's path,
which can contain the user's name.

## Options

- **Sentry's Rust SDK** (`sentry`, MIT) with a Sentry-compatible server. Rejected for now: it
  exists to send reports automatically to an endpoint, and choosing an endpoint is the open
  owner decision. Its native-crash support (`sentry-rust-minidump`) is built on the next option.
- **`crash-handler` + `minidumper`** (Embark, MIT OR Apache-2.0): an in-process signal/exception
  handler and an out-of-process minidump writer. Rejected:
  - a minidump holds stack memory, which holds whatever the crashed code was reading — document
    text included — so it cannot be shown to the user as "exactly what is sent";
  - installing the handler is an `unsafe` call whose closure must stay async-signal-safe, and the
    workspace denies `unsafe`;
  - it needs a second process for the whole run.
- **Our own, about 1,100 lines with tests, no new crates** (chosen). The operating systems
  already write a crash log for every crashed process; we read the part we need at the next
  start. `windows-version` (MIT OR Apache-2.0, already in the tree through Tauri) names the
  Windows version.

## Decision

### What is captured (`safha_core::crash`, `apps/desktop/src-tauri/src/crash.rs`)

| Failure | How | What the report holds |
|---|---|---|
| Rust panic | `std::panic::set_hook`, installed once the state store is open (it holds the setting); the default hook still prints | message, source location (`crates/safha-core/src/x.rs:12:5`, `tauri-2.12.7/src/app.rs:…`), thread name, frames between the panic machinery and the thread start |
| Native crash | a run marker (`crash-reports/running.json`: pid, start, version, executable name, last commands) removed on a clean exit; found at the next start, then the system's crash log for that pid | macOS `.ips`: exception and signal (`EXC_BAD_ACCESS (SIGSEGV)`), termination reason, the crashed thread's name and frames as `image + offset symbol`. Windows `Report.wer`: exception code, faulting module, version, offset |
| UI error | `window` `error` and `unhandledrejection` listeners; a root React error boundary that shows "Reload window" instead of a blank window | `Error.name: message`, the JavaScript stack (and React's component stack) with the app's own script addresses only |

Every report also holds the app version, build (`release`/`debug`, architecture, commit when
`SAFHA_BUILD_COMMIT` is set at build time), the operating system and version, and the **names**
of the last 30 commands run, with repeats folded (`update_viewport ×120`): IPC command names in
the shell (the invoke handler is wrapped), registry command ids in the UI. Arguments are never
recorded. Per-page fetches that fire while scrolling are left out so they do not push everything
else out. At most 3 panic reports and 5 UI error reports are kept per run (later ones are
usually the first one's consequences); at most 20 reports in all. A panic that reaches native
code (the webview's callbacks on Windows and Linux) turns into a second, "panic in a function
that cannot unwind", before the process aborts; that one is not reported after a first.

**The system's crash log comes late.** On this Mac, under load, ReportCrash wrote the `.ips` 34 s
and 83 s after the crash, so a user who starts the app again at once would find nothing. A run
without a log yet is kept as unconfirmed (`unconfirmed.json`) and looked for again every 5 s for
3 minutes, and at later starts for 2 days; when the log appears the report is added and every
window is told (`crash-reports-added`), which shows the prompt. A run that never gets a log
(force-quit, power loss, Linux) produces no report.

**Linux:** native crashes are not reported (no common crash log: systemd-coredump, apport or
nothing). Panics and UI errors are.

### Redaction

Reports are redacted when they are made, in `safha-core` (`crash/redact.rs`), so what is stored
is what is shown and what is sent:

1. **Known private strings**, replaced as whole words wherever they appear: every document path
   opened in the run, its file name and its name without extension (the shell remembers them as
   documents open, are saved as, or wait for a password), the home folder, the account name.
2. **Anything quoted** (`'…'`, `"…"` with escapes, `` `…` ``, `“…”`, `‘…’`, `«…»`): dropped for
   `<quoted>`. A single quote opens a quotation only at the start of a word, so "can't" stays.
3. **Each remaining word** that contains a letter outside ASCII (an Arabic word, an accented
   name), a slash or backslash (a path; not `3/4`), looks like a file name (`lease_2026.PDF`), is
   a web or mail address, or is an opaque token over 48 characters, is replaced by `<text>`,
   `<path>`, `<address>` or `<data>`. Rust paths (`a::b::c`) and numbers stay.
4. Messages are cut at 1,000 characters; frames at 48.

Source paths in frames keep only the part inside a source tree (`crates/…`, `apps/…`,
`<crate>-<version>/src/…`, `rustc/library/…`); anything else keeps its file name. UI stack frames
keep the path inside the app's own origin (`assets/index-ab12.js:1:2`); other addresses become
`<address>`. A thrown value that is not an `Error` is never turned into text (it may be a
document's data); a backend error contributes only its stable code. From system crash logs only
the fields above are read: not the executable's path, registers, other threads, or
"application-specific information".

The end-to-end check panics with a message that names `/Users/someone/Secret Contract.pdf` and
quotes Arabic text; the report reads `self-test panic opening <path> <path>: <quoted>`.

### Asking the user

- **Nothing is sent by the app, ever, in this version.** "Off by default" means no report leaves
  the computer unless the user acts on that report. Reports are kept locally (redacted) so there
  is something to ask about.
- **The first start after a crash** (or as soon as a late crash log is found) shows "Send a crash
  report?" once: what was kept and what is never in it, then each report **exactly as it would be
  sent** (the first one open), with **Copy**, **Save as file…**, **Report on GitHub**, and
  **Delete**. "Not now" keeps the reports in Settings; "Don't keep crash reports" turns the
  feature off. Each report is offered once (`offered` in its file), so the prompt does not
  return at every start.
- **Settings → Crash reports:** "Ask me after a crash" (default) or "Don't keep crash reports",
  and every kept report with the same actions. Turning it off stops new reports and the prompt;
  kept ones stay until deleted.
- Every action is a registry command (`crash.copy`, `crash.save`, `crash.openIssue`,
  `crash.delete`, `crash.dismiss`, `crash.ask`, `crash.off`, `app.reloadWindow`). The report text
  is technical English and shown left to right in either UI; everything around it is in the UI's
  language and mirrors in Arabic.

### Destinations

`Destination` is chosen at build time (`SAFHA_CRASH_DESTINATION`):

- `github:owner/name` (the default: `github:adrianazizx/Mirsam`): the user can copy, save, or open
  a pre-filled issue in their browser. The whole report goes to the clipboard first; a long one is
  shortened in the link (frames dropped from the end) to stay under GitHub's ~8 KB URL limit, and
  the user is told to paste the full text. The issue is created by the user, in their browser,
  under their account, after they read it.
- `manual`: copy and save only.
- `https://…` is refused at parse time: a crash server is not built. Adding one is a new variant
  that posts the same text (`safha-core` already has an HTTP client, `ureq` with rustls, for model
  downloads) behind the same per-report consent; automatic sending would need its own,
  explicit, off-by-default setting.

### Release builds keep function names

Release builds were stripped completely (`strip = true`): every Rust frame of a panic then reads
`__mh_execute_header`, useless without our own symbol files. They now strip debug information
only (`strip = "debuginfo"`), keeping the symbol table, so a report names the functions. The cost
is measured in `PROGRESS.md` (installer size). macOS crash logs symbolicate our frames from the
same table (Rust's v0-mangled names, `_RNv…`, readable with `rustfilt`). Windows keeps symbols in
a `.pdb` that is not shipped: frames there print as addresses, with the address of a known
function to work out the load offset. Archiving `.pdb` files per release (for symbolication) is a
follow-up with the release pipeline (owner item 17).

## Consequences

- No network, no account, no new dependency. A report is a small JSON file in
  `<app data>/crash-reports/`, readable by the user.
- Developers see crashes only when users choose to send them, and only as much as the report
  holds: no memory, no document. A crash in PDFium on a particular file cannot be reproduced from
  a report alone; the report says where, and the user can be asked for a file that shows it.
- macOS and Windows native crashes depend on the system writing its crash log (it does by
  default; Windows Error Reporting can be disabled by policy). Linux native crashes go unreported.
- Windows: the WER reader is unit-tested on a constructed `Report.wer`, not yet on a real crash.
- Tests: `safha-core` `crash::` unit tests (redaction, frames, store, run marker, `.ips` and
  `Report.wer` readers, issue links); UI tests `commands/crash.test.tsx` (prompt, copy, delete,
  issue, off, Settings in Arabic, command trail, capture, error boundary);
  `scripts/crash-selftest.sh <out> panic|abort|thread-panic` on the real app (macOS: all three;
  CI: `panic` on macOS, Windows and Linux).
