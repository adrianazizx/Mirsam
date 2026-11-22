#!/usr/bin/env bash
# Scroll and zoom benchmark of the real desktop shell on the 1,000-page fixture
# (spec section 11: 60 fps, no page blank for more than 100 ms, memory < 500 MB).
# The UI side is apps/desktop/src/benchmarks/scroll.ts; budgets are asserted by
# scripts/check-scroll-benchmark.mjs.
#
# Usage: scripts/scroll-benchmark.sh <out-dir> [debug|release]
#   debug:   needs cargo build -p safha-desktop --features devtools
#   release: needs (cd apps/desktop && pnpm tauri build --features devtools --bundles app)
#
# The window must stay visible and the screen unlocked for the whole run: an
# occluded WKWebView gets no animation frames, and the checker then rejects the
# frame and blank-time numbers as invalid.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:?usage: scripts/scroll-benchmark.sh <out-dir> [debug|release]}"
BUILD="${2:-debug}"
PDF="$ROOT/fixtures/out/derived/bilingual-1000-pages.pdf"

[[ -f "$PDF" ]] || { echo "missing $PDF: run node fixtures/generate.mjs" >&2; exit 2; }
case "$BUILD" in
  debug) ;;
  release) export SAFHA_SELFTEST_APP="$ROOT/target/release/bundle/macos/Safha.app/Contents/MacOS/safha-desktop" ;;
  *) echo "build must be debug or release" >&2; exit 2 ;;
esac

SAFHA_SELFTEST_SPIKE=scroll SAFHA_SELFTEST_TIMEOUT="${SAFHA_SELFTEST_TIMEOUT:-240}" \
  "$ROOT/scripts/selftest.sh" "$PDF" "$OUT" en light
node "$ROOT/scripts/check-scroll-benchmark.mjs" "$OUT"
