#!/usr/bin/env bash
# Renders the social preview cards (public/og-en.png, public/og-ar.png) and the
# touch icon (public/apple-touch-icon.png) with headless Chrome. Needs
# `pnpm install` in website/ for the font files.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PUBLIC="$HERE/../public"
CHROME="${CHROME_BIN:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
shot() { "$CHROME" --headless --disable-gpu --hide-scrollbars --allow-file-access-from-files \
  --force-device-scale-factor=1 --virtual-time-budget=3000 --window-size="$2" --screenshot="$1" "$3" 2>/dev/null; }
for lang in en ar; do shot "$PUBLIC/og-$lang.png" 1200,630 "file://$HERE/og.html?lang=$lang"; done
TMP="$(mktemp -d)"
cat > "$TMP/icon.html" <<HTML
<html><body style="margin:0;width:180px;height:180px;display:grid;place-items:center;background:#f7f6ff">
<img src="file://$HERE/../brand/mirsam-envelope.svg" style="height:136px"></body></html>
HTML
shot "$PUBLIC/apple-touch-icon.png" 180,180 "file://$TMP/icon.html"
echo "wrote og-en.png, og-ar.png, apple-touch-icon.png"
