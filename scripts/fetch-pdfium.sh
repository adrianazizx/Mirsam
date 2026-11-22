#!/usr/bin/env bash
# Downloads the pinned PDFium build (bblanchon/pdfium-binaries, no V8/XFA) into
# third_party/pdfium/<platform>/ and verifies its SHA-256.
#
# The build number must match the `pdfium_latest` API level of the pinned
# pdfium-render crate (see docs/adr/0002-pdf-engine-pdfium.md). Bump both together.
#
# Usage: scripts/fetch-pdfium.sh [platform]
#   platform: mac-arm64 | mac-x64 | mac-universal | linux-x64 | linux-arm64 | win-x64 |
#             win-arm64 | wasm (defaults to the host platform)
#   mac-universal is not a download: it fetches mac-arm64 and mac-x64 (each
#   checksum-verified) and joins their libraries with lipo, for universal app
#   bundles (docs/adr/0060-bundling-pdfium.md).
set -euo pipefail

PDFIUM_BUILD="8125"

checksum_for() {
  case "$1" in
    mac-arm64)   echo "4939bd935ac83ddc09a3a4f038bc43a19cea1660316a3667f26cb07a070e139d" ;;
    mac-x64)     echo "8a5193535b80c7215d8932f6647ae5097465b6dba2811602c21e120c2af112b0" ;;
    linux-x64)   echo "e1c3ce6e270cb5e0c20816b379ad9f2c1309972697c505f508de92bf5e7d6659" ;;
    linux-arm64) echo "8b8818f2ec57d1b213d952ca6d7f0ffeca9d0110487d0e20ad83ba918bc92efa" ;;
    win-x64)     echo "9a964261760cff52570aabf038ab346d3a5034dd8b30d0d5fe8861235d356b8b" ;;
    win-arm64)   echo "d394ba61991fb53f4d87f70988035f337d9e73f2bd2c3c1b966df8c43fcabb05" ;;
    wasm)        echo "77db4ef8c9ab0ecef2af659fe7d407279a0ebacff4cfcc1b3cc28160be2abfbd" ;;
    *) echo "unknown platform: $1" >&2; exit 2 ;;
  esac
}

host_platform() {
  local os arch
  os="$(uname -s)"; arch="$(uname -m)"
  case "$os" in
    Darwin) os="mac" ;;
    Linux) os="linux" ;;
    MINGW*|MSYS*|CYGWIN*) os="win" ;;
    *) echo "unsupported OS: $os" >&2; exit 2 ;;
  esac
  case "$arch" in
    arm64|aarch64) arch="arm64" ;;
    x86_64|amd64) arch="x64" ;;
    *) echo "unsupported arch: $arch" >&2; exit 2 ;;
  esac
  echo "$os-$arch"
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

PLATFORM="${1:-$(host_platform)}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="$ROOT/third_party/pdfium/$PLATFORM"
STAMP="$DEST/.build-$PDFIUM_BUILD"

if [[ -f "$STAMP" ]]; then
  echo "PDFium $PDFIUM_BUILD ($PLATFORM) already present at $DEST"
  exit 0
fi

if [[ "$PLATFORM" == "mac-universal" ]]; then
  command -v lipo >/dev/null || { echo "mac-universal needs lipo (Xcode command-line tools)" >&2; exit 2; }
  "$0" mac-arm64
  "$0" mac-x64
  ARM="$ROOT/third_party/pdfium/mac-arm64"
  rm -rf "$DEST"
  mkdir -p "$DEST/lib"
  lipo -create "$ARM/lib/libpdfium.dylib" "$ROOT/third_party/pdfium/mac-x64/lib/libpdfium.dylib" \
    -output "$DEST/lib/libpdfium.dylib"
  # Both packages come from the same build: one copy of the notices serves.
  cp -R "$ARM/LICENSE" "$ARM/VERSION" "$ARM/licenses" "$DEST/"
  touch "$STAMP"
  echo "PDFium $PDFIUM_BUILD ($PLATFORM: $(lipo -archs "$DEST/lib/libpdfium.dylib")) installed at $DEST"
  exit 0
fi

EXPECTED="$(checksum_for "$PLATFORM")"

URL="https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F$PDFIUM_BUILD/pdfium-$PLATFORM.tgz"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "Downloading $URL"
curl --fail --location --silent --show-error --output "$TMP/pdfium.tgz" "$URL"

ACTUAL="$(sha256_of "$TMP/pdfium.tgz")"
if [[ "$ACTUAL" != "$EXPECTED" ]]; then
  echo "SHA-256 mismatch for pdfium-$PLATFORM.tgz" >&2
  echo "  expected $EXPECTED" >&2
  echo "  actual   $ACTUAL" >&2
  exit 1
fi

rm -rf "$DEST"
mkdir -p "$DEST"
tar -xzf "$TMP/pdfium.tgz" -C "$DEST"
touch "$STAMP"
echo "PDFium $PDFIUM_BUILD ($PLATFORM) installed at $DEST"
