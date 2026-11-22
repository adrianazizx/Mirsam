#!/usr/bin/env bash
# Runs every engine of the M4 benchmark over ocr-eval set v2, then prints the
# comparison tables (ADR 0030). Throwaway: see README.md for the one-time setup.
#
#   spikes/ocr-engines/run_all.sh [engine ...]
#
# Engines: apple-vision tesseract ppocrv5-arabic qari-0.3 qari-0.4 paddleocr-vl-1.6
#          nuqta-0.8b nuqta-4b
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CACHE="${SAFHA_OCR_BENCH_CACHE:-$HOME/Library/Caches/safha-dev/ocr-bench}"
PY="$CACHE/venv/bin/python"
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
cd "$HERE"

engines=("$@")
if [[ ${#engines[@]} -eq 0 ]]; then
  engines=(apple-vision tesseract ppocrv5-arabic qari-0.3 qari-0.4 paddleocr-vl-1.6 nuqta-0.8b nuqta-4b)
fi

for engine in "${engines[@]}"; do
  echo "== $engine"
  case "$engine" in
    apple-vision) "$PY" run_apple_vision.py ;;
    tesseract) "$PY" run_tesseract.py ;;
    ppocrv5-arabic) "$PY" run_ppocr.py ;;
    *) "$PY" run_vlm.py "$engine" ;;
  esac
done

"$PY" summarize.py "${engines[@]}"
