#!/usr/bin/env bash
# One-time local setup.
#
# 1. Downloads the pinned PDFium build for this machine.
# 2. If the checkout lives in a cloud-synced folder (OneDrive, iCloud Drive,
#    Dropbox, Google Drive), moves the heavy build outputs out of it so the sync
#    client does not try to upload gigabytes of object files:
#      - Cargo's target/ and the fuzzing outputs (fuzz/target, corpus, artifacts)
#        become symlinks into the user cache directory.
#      - pnpm's virtual store (the bulk of node_modules) goes to the same cache
#        via generated, git-ignored apps/desktop/.npmrc and website/.npmrc.
#    Normal checkouts and CI are left untouched.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

scripts/fetch-pdfium.sh

case "$ROOT" in
  */Library/CloudStorage/*|*/Library/Mobile\ Documents/*|*OneDrive*|*Dropbox*|*Google\ Drive*)
    case "$(uname -s)" in
      Darwin) CACHE_HOME="$HOME/Library/Caches" ;;
      *) CACHE_HOME="${XDG_CACHE_HOME:-$HOME/.cache}" ;;
    esac
    # Keyed by checkout path so two checkouts never share build outputs.
    KEY="$(printf '%s' "$ROOT" | cksum | cut -d' ' -f1)"
    CACHE="$CACHE_HOME/mirsam-dev/$KEY"
    mkdir -p "$CACHE/target" "$CACHE/pnpm-virtual-store" "$CACHE/fuzz/target" "$CACHE/fuzz/corpus" \
      "$CACHE/fuzz/artifacts"

    # The fuzz crate is its own workspace with its own target/, and fuzzing
    # writes thousands of small files (fuzz/README.md).
    for pair in "target:$CACHE/target" "fuzz/target:$CACHE/fuzz/target" "fuzz/corpus:$CACHE/fuzz/corpus" \
      "fuzz/artifacts:$CACHE/fuzz/artifacts"; do
      link="${pair%%:*}"
      if [[ -L "$link" ]]; then
        :
      elif [[ -e "$link" ]]; then
        echo "note: $link/ already exists as a real directory; leaving it in place" >&2
      else
        ln -s "${pair#*:}" "$link"
      fi
    done

    printf 'virtual-store-dir=%s\n' "$CACHE/pnpm-virtual-store" > apps/desktop/.npmrc
    # The website (website/, deployed to Vercel) gets its own store beside it.
    mkdir -p "$CACHE/website-pnpm-virtual-store"
    printf 'virtual-store-dir=%s\n' "$CACHE/website-pnpm-virtual-store" > website/.npmrc
    echo "Cloud-synced checkout detected: build outputs redirected to $CACHE"
    ;;
esac

echo "Setup complete. Next: (cd apps/desktop && pnpm install)"
