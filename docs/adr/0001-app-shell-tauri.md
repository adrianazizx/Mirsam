# 0001 — App shell: Tauri 2

Status: Accepted, on assumption D1 (the project owner has not overridden it). Date: 2026-11-22.

## Context

The product is a desktop app for Windows and macOS (Linux best-effort) with hard budgets:
cold start ≤ 1.5 s, installer ≤ 60 MB without OCR models, memory bounded on 1,000-page files
(spec §11). The engine must be UI-independent so a CLI, web, and mobile version stay possible.
The alternative named in the spec is Electron: heavier, but one rendering engine on every OS.

## Decision

Tauri 2 (verified current: `tauri` 2.12.7, `@tauri-apps/cli` 2.12.6, both MIT OR Apache-2.0,
released this month). Rust shell crate `apps/desktop/src-tauri`, deliberately thin: window,
typed commands, nothing else. All engine logic is in `crates/safha-core`.

Security posture from day one:

- Capabilities grant only `core:app`, `core:event`, `core:window`, `core:webview` defaults. No fs,
  shell, or dialog permission for the webview; the file picker is invoked from Rust.
- The UI receives document ids and file names, never paths.
- CSP is restrictive (`default-src 'self'`, IPC-only `connect-src`, no remote origins).
- Developer commands (self-test, spikes) exist only behind the `devtools` cargo feature; release
  builds register a separate, smaller handler list.

## Evidence (Milestone 0, Apple-silicon MacBook, macOS 26)

- The shell compiled first time against Tauri 2.12 and runs: window → React UI → PDFium page on
  a canvas, in both LTR and RTL (`scripts/selftest.sh`).
- Cold start, bundled **release** app, five launches: 0.50–0.62 s from process start to UI
  rendered, document opened, first page on the canvas, and the self-test report written (the
  report step itself costs time, so the real figure is lower). Budget: 1.5 s. The debug build
  served by the dev server takes 0.8–1.3 s.
- Installer: 6.7 MiB `.dmg`, 14.6 MiB `.app` (macOS arm64, unsigned). Budget: 60 MB.
- The release bundle loads PDFium from `Contents/Frameworks` and runs under the production CSP.
- PDFium accounts for 7.0 MiB of the app (3.5 MB compressed).

## Consequences

- The system webview differs per OS (WKWebView / WebView2 / WebKitGTK). Mitigation: page pixels
  are produced by the core, never by the webview; UI CSS targets Safari 16+; CI builds on all
  three OSes. Windows and Linux have not been run yet — only macOS.
- WKWebView has no WebDriver, so end-to-end tests use the in-app self-test harness (ADR 0007).
- If webview differences become unmanageable, Electron remains the fallback; `safha-core` and the
  UI would carry over unchanged, only the shell crate is replaced.
