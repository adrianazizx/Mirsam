# 0072 — Releases: GitHub Releases from a version tag, macOS signed and notarized in CI

Status: Accepted. Date: 2026-11-24. Milestone 7's distribution (spec §12; PROGRESS item 17).

## Context

`scripts/bundle.mjs` (ADR 0060) builds and checks every installer, and the Bundles workflow
self-tests them, but nothing put an installer where people could download it. The repository is
public; the owner has an Apple Developer account and no Windows certificate yet.

## Decision

- **GitHub Releases** host the downloads: no server to run, free for a public repository, and the
  same place Tauri's updater can read later.
- **A version tag on `main`** (`v0.1.0`) runs `.github/workflows/release.yml`: one universal macOS
  disk image, the Windows setup program and MSI, and the Linux deb, rpm and AppImage, each built by
  `bundle.mjs` without the self-test harness. The run refuses a tag that is not the app's version
  or not on `main`. It leaves a **draft**; the owner publishes it.
- **One version, four places** (Tauri config, `package.json`, `Cargo.toml`, `Cargo.lock`), set and
  checked by `scripts/set-version.mjs`. Pre-releases are `X.Y.Z-N`: Windows Installer accepts only
  a number there.
- **Download names without the version** (`Mirsam-macOS.dmg`, …), so
  `releases/latest/download/<name>` is a permanent link. `SHA256SUMS.txt` beside them.
- **macOS signing in CI when the secrets exist**: the certificate goes into a keychain of the
  run's own; `tauri build` signs (hardened runtime) and notarizes and staples the app; the workflow
  checks the app, its executables and PDFium carry the Developer ID, then signs, notarizes and
  staples the disk image too. Without a certificate the build is unsigned with a warning; with a
  certificate but no notarization credentials it fails, because a signed, un-notarized app is
  still blocked. Notarization by App Store Connect API key (preferred, no personal password) or by
  Apple ID and app-specific password.
- **PDFium re-signed is still the pinned build**: signing rewrites the dylib's bytes, so
  `bundle.mjs` accepts a macOS copy whose Mach-O UUIDs equal the pinned library's and whose
  signature verifies (the ELF build-ID rule for AppImages, ADR 0060, is the same idea).
- **Release notes** from `.github/release-notes.md`, English and Arabic, with the Mac
  instructions chosen by whether the disk image was signed.

## Alternatives

- **`tauri-apps/tauri-action`**: builds and uploads in one step, but calls `tauri build` directly,
  which ADR 0060's guard refuses (no PDFium overlay), and names files with their versions.
- **A website or bucket of our own**: a domain and hosting to maintain for no gain yet; a site can
  link to the `latest/download` URLs later.
- **Separate arm64 and Intel disk images**: smaller (about 11 MiB each against 22), but people
  have to know which Mac they have. The universal image is within the 60 MB budget.

## Consequences

- Windows installers stay unsigned (SmartScreen warns) until a certificate is bought; the notes
  say how to get past the warning.
- The release installers are not launched in CI (no harness); the Bundles workflow's run on the
  same commit is the test.
- The bundle identifier should be settled before the first public release (PROGRESS item 6).
