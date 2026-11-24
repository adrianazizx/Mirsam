#!/usr/bin/env bash
# Prints showcase.html (the fictional sample document in the website's
# screenshots) to a PDF with headless Chrome.
# Usage: website/screenshots/make-showcase.sh <out.pdf>
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="${1:?usage: make-showcase.sh <out.pdf>}"
CHROME="${CHROME_BIN:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
"$CHROME" --headless --disable-gpu --no-pdf-header-footer --allow-file-access-from-files \
  --print-to-pdf="$OUT" "file://$HERE/showcase.html" 2>/dev/null
echo "wrote $OUT"
