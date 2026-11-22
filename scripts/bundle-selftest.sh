#!/usr/bin/env bash
# Runs the end-to-end self-test (scripts/selftest.sh) on an app as its installer
# delivers it, in English/LTR and Arabic/RTL, and checks that the app loaded the
# PDFium inside its own bundle, not a copy from the repository (ADR 0060).
#
# The bundle must be built with the self-test harness:
#   node scripts/bundle.mjs [--target <triple>] --features devtools
#
# Usage: scripts/bundle-selftest.sh <installer-or-app> <out-dir>
#   .app        runs it where it is
#   .dmg        mounts it read-only and runs the app inside
#   .deb, .rpm  unpacks it (dpkg-deb; rpm2cpio or bsdtar) and runs usr/bin/<app>
#   .AppImage   unpacks it (--appimage-extract) and runs its AppRun
#   -setup.exe  installs it silently (NSIS) into <out-dir>/install, then uninstalls
#   .msi        unpacks it (an administrative install) into <out-dir>/install
# Env: SAFHA_SELFTEST_ARCH=x86_64|arm64 launches a macOS app as that architecture
#      (arch -x86_64: a universal app's Intel slice, or an Intel app, under
#      Rosetta) and checks that it ran as one.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INSTALLER="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
mkdir -p "$2"
OUT="$(cd "$2" && pwd)"
ARCH="${SAFHA_SELFTEST_ARCH:-}"
INSTALL="$OUT/install"
rm -rf "$INSTALL"
mkdir -p "$INSTALL"
# Git Bash on Windows: Windows programs need Windows paths, and MSYS must not
# rewrite installer switches such as /S into paths.
native() { if command -v cygpath >/dev/null; then cygpath -w "$1"; else printf '%s' "$1"; fi; }

UNDO=()
cleanup() { for step in "${UNDO[@]+"${UNDO[@]}"}"; do eval "$step" || true; done; }
trap cleanup EXIT

# APP: the executable to launch; BUNDLE_ROOT: everything the installer delivered.
case "$INSTALLER" in
  *.app)
    BUNDLE_ROOT="$INSTALLER"
    ;;
  *.dmg)
    hdiutil attach -nobrowse -readonly -noautoopen -mountpoint "$INSTALL" "$INSTALLER" >/dev/null
    UNDO+=("hdiutil detach '$INSTALL' -quiet")
    BUNDLE_ROOT="$(find "$INSTALL" -maxdepth 1 -name '*.app' | head -n 1)"
    ;;
  *.deb)
    dpkg-deb -x "$INSTALLER" "$INSTALL"
    BUNDLE_ROOT="$INSTALL"
    ;;
  *.rpm)
    # rpm2cpio | cpio where they can read it, else libarchive.
    (cd "$INSTALL" && rpm2cpio "$INSTALLER" | cpio -idm --quiet) || bsdtar -xf "$INSTALLER" -C "$INSTALL"
    BUNDLE_ROOT="$INSTALL"
    ;;
  *.AppImage)
    chmod +x "$INSTALLER"
    (cd "$INSTALL" && "$INSTALLER" --appimage-extract >/dev/null)
    BUNDLE_ROOT="$INSTALL/squashfs-root"
    APP="$BUNDLE_ROOT/AppRun"
    ;;
  *-setup.exe)
    # /D must come last and unquoted (NSIS).
    MSYS_NO_PATHCONV=1 "$INSTALLER" /S "/D=$(native "$INSTALL")"
    UNDO+=("MSYS_NO_PATHCONV=1 '$INSTALL/uninstall.exe' /S")
    BUNDLE_ROOT="$INSTALL"
    ;;
  *.msi)
    MSYS_NO_PATHCONV=1 msiexec /a "$(native "$INSTALLER")" /qn "TARGETDIR=$(native "$INSTALL")" \
      /l*v "$(native "$OUT/msiexec.log")"
    BUNDLE_ROOT="$INSTALL"
    ;;
  *)
    echo "unsupported installer: $INSTALLER" >&2
    exit 2
    ;;
esac
[[ -n "${BUNDLE_ROOT:-}" && -d "$BUNDLE_ROOT" ]] || { echo "nothing installed from $INSTALLER" >&2; exit 1; }

if [[ -z "${APP:-}" ]]; then
  case "$INSTALLER" in
    *.app | *.dmg) APP="$(find "$BUNDLE_ROOT/Contents/MacOS" -type f -perm -u+x | head -n 1)" ;;
    *.deb | *.rpm) APP="$(find "$BUNDLE_ROOT/usr/bin" -type f | head -n 1)" ;;
    *) APP="$(find "$BUNDLE_ROOT" -iname '*.exe' ! -iname 'uninstall*' | head -n 1)" ;;
  esac
fi
[[ -n "${APP:-}" && -f "$APP" ]] || { echo "no application executable in $BUNDLE_ROOT" >&2; exit 1; }
echo "bundle self-test: $APP"

LAUNCH="$APP"
if [[ -n "$ARCH" ]]; then
  # selftest.sh launches one executable; this one re-executes the app as $ARCH.
  LAUNCH="$OUT/launch-$ARCH"
  printf '#!/bin/sh\nexec arch -%s "%s" "$@"\n' "$ARCH" "$APP" >"$LAUNCH"
  chmod +x "$LAUNCH"
fi

run() { # <pdf> <name> <locale> <theme> <direction>
  SAFHA_SELFTEST_APP="$LAUNCH" SAFHA_SELFTEST_UI=bundled \
    "$ROOT/scripts/selftest.sh" "$ROOT/fixtures/out/$1" "$OUT/$2" "$3" "$4"
  node "$ROOT/scripts/check-selftest.mjs" "$OUT/$2" "$5" --ci
  node - "$OUT/$2/report.json" "$BUNDLE_ROOT" "$ROOT" "$ARCH" <<'EOF'
const fs = require("node:fs");
const path = require("node:path");
const [reportPath, bundleRoot, repo, arch] = process.argv.slice(2);
const report = JSON.parse(fs.readFileSync(reportPath, "utf8"));
// Windows reports \\?\-prefixed paths; compare canonical, case-folded forms.
const canonical = (p) => fs.realpathSync.native(p.replace(/^\\\\\?\\/, "")).toLowerCase();
const library = report.pdfiumLibrary;
const failures = [];
if (!library) failures.push("the report names no PDFium library (was the bundle built with --features devtools?)");
else {
  const loaded = canonical(library);
  const inside = canonical(bundleRoot) + path.sep;
  if (!loaded.startsWith(inside)) failures.push(`PDFium came from ${library}, outside the bundle ${bundleRoot}`);
  if (loaded.startsWith(canonical(repo) + path.sep) && !inside.startsWith(canonical(repo) + path.sep)) {
    failures.push(`PDFium came from the repository: ${library}`);
  }
}
// arch(1) says arm64 where Rust says aarch64.
const expected = arch === "arm64" ? "aarch64" : arch;
if (expected && report.arch !== expected) failures.push(`the app ran as ${report.arch}, expected ${expected}`);
if (failures.length > 0) {
  console.error(`bundle self-test failed:\n  ${failures.join("\n  ")}`);
  process.exit(1);
}
console.log(`bundle self-test: PDFium loaded from ${library} (${report.arch})`);
EOF
}

run chrome/mixed-inline.pdf en en light ltr
# Three pages, so the self-test also drives the page organizer.
run derived/page-labels.pdf ar ar dark rtl
echo "bundle self-test passed: $INSTALLER"
