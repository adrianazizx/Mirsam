#!/usr/bin/env bash
# Runs the Milestone 0 tile-transport spike in the real desktop shell and writes
# spikes/out/tile-transport/report.json. See docs/adr/0004-tile-transport.md.
# Needs: cargo build -p safha-desktop --features devtools, node fixtures/generate.mjs
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# The WebAssembly candidate is served to the webview from Vite's public folder.
scripts/fetch-pdfium.sh wasm
mkdir -p apps/desktop/public/spike-pdfium
cp third_party/pdfium/wasm/lib/pdfium.js third_party/pdfium/wasm/lib/pdfium.wasm apps/desktop/public/spike-pdfium/
trap 'rm -rf "$ROOT/apps/desktop/public/spike-pdfium"' EXIT

SAFHA_SELFTEST_SPIKE=tile-transport scripts/selftest.sh \
  "${1:-fixtures/out/chrome/contract-two-column.pdf}" spikes/out/tile-transport
