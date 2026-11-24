#!/usr/bin/env bash
# End-to-end self-test of the real desktop shell (see apps/desktop/src-tauri/src/diagnostics.rs).
#
# Launches the debug app against the Vite dev server with a PDF, lets the UI
# render page 1 to its canvas and report what is on screen, then exits. Writes
# report.json and canvas.png (the canvas pixels as the webview holds them).
#
# Usage: scripts/selftest.sh <file.pdf> <out-dir> [en|ar] [light|dark]
# Needs: cargo build -p mirsam-desktop --features devtools
# Env:   MIRSAM_SELFTEST_APP=<binary> runs another devtools build, e.g. the release
#        bundle (target/release/bundle/macos/Mirsam.app/Contents/MacOS/mirsam-desktop);
#        MIRSAM_SELFTEST_SPIKE=<name> runs a benchmark (scroll, tile-transport);
#        MIRSAM_SELFTEST_TIMEOUT=<seconds> (default 60);
#        MIRSAM_SELFTEST_UI=bundled: the app carries its own UI (a release build or
#        an installed bundle, scripts/bundle-selftest.sh), so no dev server is started;
#        MIRSAM_SELFTEST_STATE=<dir>: keep the app's state there, across runs
#        (scripts/crash-selftest.sh), instead of a fresh <out-dir>/state.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# A missing PDF must stop here: the path below would come out as /<name>, and
# the app would report only an `io_error` (a Windows bundle run, 2026-11-24,
# whose derived fixtures were skipped for want of qpdf).
[[ -f "$1" ]] || {
  echo "no PDF at $1 (a fixture? run node fixtures/generate.mjs; derived/ ones need qpdf)" >&2
  exit 2
}
PDF="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
mkdir -p "$2"
OUT="$(cd "$2" && pwd)"
LOCALE="${3:-en}"
THEME="${4:-light}"
APP="${MIRSAM_SELFTEST_APP:-$ROOT/target/debug/mirsam-desktop}"
[[ -f "$APP.exe" ]] && APP="$APP.exe"
TIMEOUT_SECONDS="${MIRSAM_SELFTEST_TIMEOUT:-60}"

[[ -x "$APP" ]] || { echo "missing $APP: run cargo build -p mirsam-desktop --features devtools" >&2; exit 2; }
# Git Bash on Windows: the app is a native program and needs Windows paths.
native() { if command -v cygpath >/dev/null; then cygpath -w "$1"; else printf '%s' "$1"; fi; }
rm -f "$OUT/report.json" "$OUT/canvas.png"
# A fresh state store per run: no recents or session from the user's own use,
# and nothing written back to them.
if [[ -n "${MIRSAM_SELFTEST_STATE:-}" ]]; then
  mkdir -p "$MIRSAM_SELFTEST_STATE"
  STATE="$(cd "$MIRSAM_SELFTEST_STATE" && pwd)"
else
  STATE="$OUT/state"
  rm -rf "$STATE"
fi

VITE_PID=""
cleanup() { [[ -z "$VITE_PID" ]] || kill "$VITE_PID" 2>/dev/null || true; }
trap cleanup EXIT

if [[ "${MIRSAM_SELFTEST_UI:-dev}" != "bundled" ]]; then
  # The debug app loads its UI from http://localhost:1420 (tauri.conf.json). A
  # server already there (another checkout's, another session's) would be
  # tested instead of this checkout's UI, so refuse rather than test the wrong code.
  if curl --silent --output /dev/null http://localhost:1420/; then
    echo "port 1420 is already serving a UI (another dev server?); stop it so this checkout's UI is tested" >&2
    exit 2
  fi

  cd "$ROOT/apps/desktop"
  pnpm exec vite --port 1420 --strictPort >"$OUT/vite.log" 2>&1 &
  VITE_PID=$!

  for _ in $(seq 1 150); do
    curl --silent --fail --output /dev/null http://localhost:1420/ && break
    sleep 0.2
  done
fi

# The launch time, in Unix milliseconds, for the cold-start budget (spec
# section 11): the app reports how long after it its window was usable.
launched_ms() { perl -MTime::HiRes=time -e 'printf("%.0f", time() * 1000)' 2>/dev/null || echo ""; }
MIRSAM_OPEN="$(native "$PDF")" MIRSAM_STATE_DIR="$(native "$STATE")" MIRSAM_SELFTEST_OUT="$(native "$OUT")" \
  MIRSAM_SELFTEST_LOCALE="$LOCALE" MIRSAM_SELFTEST_THEME="$THEME" MIRSAM_SELFTEST_LAUNCHED_MS="$(launched_ms)" \
  "$APP" >"$OUT/app.log" 2>&1 &
APP_PID=$!
( sleep "$TIMEOUT_SECONDS"; kill "$APP_PID" 2>/dev/null || true ) &
WATCHDOG_PID=$!
wait "$APP_PID" || true
kill "$WATCHDOG_PID" 2>/dev/null || true

if [[ -f "$OUT/report.json" ]]; then
  echo "self-test report: $OUT/report.json"
else
  echo "self-test produced no report within ${TIMEOUT_SECONDS}s (see $OUT/app.log)" >&2
  exit 1
fi
