#!/usr/bin/env bash
# End-to-end self-test of the real desktop shell (see apps/desktop/src-tauri/src/diagnostics.rs).
#
# Launches the debug app against the Vite dev server with a PDF, lets the UI
# render page 1 to its canvas and report what is on screen, then exits. Writes
# report.json and canvas.png (the canvas pixels as the webview holds them).
#
# Usage: scripts/selftest.sh <file.pdf> <out-dir> [en|ar] [light|dark]
# Needs: cargo build -p safha-desktop --features devtools
# Env:   SAFHA_SELFTEST_APP=<binary> runs another devtools build, e.g. the release
#        bundle (target/release/bundle/macos/Safha.app/Contents/MacOS/safha-desktop);
#        SAFHA_SELFTEST_SPIKE=<name> runs a benchmark (scroll, tile-transport);
#        SAFHA_SELFTEST_TIMEOUT=<seconds> (default 60).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PDF="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
mkdir -p "$2"
OUT="$(cd "$2" && pwd)"
LOCALE="${3:-en}"
THEME="${4:-light}"
APP="${SAFHA_SELFTEST_APP:-$ROOT/target/debug/safha-desktop}"
[[ -f "$APP.exe" ]] && APP="$APP.exe"
TIMEOUT_SECONDS="${SAFHA_SELFTEST_TIMEOUT:-60}"

[[ -x "$APP" ]] || { echo "missing $APP: run cargo build -p safha-desktop --features devtools" >&2; exit 2; }
# Git Bash on Windows: the app is a native program and needs Windows paths.
native() { if command -v cygpath >/dev/null; then cygpath -w "$1"; else printf '%s' "$1"; fi; }
rm -f "$OUT/report.json" "$OUT/canvas.png"
# A fresh state store per run: no recents or session from the user's own use,
# and nothing written back to them.
rm -rf "$OUT/state"

cd "$ROOT/apps/desktop"
pnpm exec vite --port 1420 --strictPort >"$OUT/vite.log" 2>&1 &
VITE_PID=$!
cleanup() { kill "$VITE_PID" 2>/dev/null || true; }
trap cleanup EXIT

for _ in $(seq 1 150); do
  curl --silent --fail --output /dev/null http://localhost:1420/ && break
  sleep 0.2
done

SAFHA_OPEN="$(native "$PDF")" SAFHA_STATE_DIR="$(native "$OUT/state")" SAFHA_SELFTEST_OUT="$(native "$OUT")" \
  SAFHA_SELFTEST_LOCALE="$LOCALE" SAFHA_SELFTEST_THEME="$THEME" \
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
