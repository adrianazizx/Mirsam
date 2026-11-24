#!/usr/bin/env bash
# Encodes window captures of Mirsam (PNG, the app's default 1280×820 window,
# taken with `screencapture -o -l <window id>`) into the site's screenshots:
# public/screens/<name>.webp at 1600×1025. Names are <shot>-<en|ar>, where
# <shot> is one of src/i18n/en.json's features.chapters[].shot, "hero" or "ocr".
# Needs cwebp (brew install webp).
# Usage: website/scripts/encode-screens.sh <dir with PNGs>
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="${1:?usage: encode-screens.sh <dir with PNGs>}"
mkdir -p "$HERE/public/screens"
for png in "$SRC"/*.png; do
  name="$(basename "$png" .png)"
  cwebp -quiet -q 84 -m 6 -resize 1600 1025 "$png" -o "$HERE/public/screens/$name.webp"
  printf '%-16s %6s KB\n' "$name" "$(( $(wc -c < "$HERE/public/screens/$name.webp") / 1024 ))"
done
