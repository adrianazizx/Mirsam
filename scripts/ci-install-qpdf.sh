#!/usr/bin/env bash
# Installs qpdf on a CI runner (Chocolatey on Windows, Homebrew on macOS, apt on
# Linux), retrying with a short back-off, and fails the step if qpdf is still
# not on PATH afterwards.
#
# Package mirrors fail now and then. On 2026-11-24 community.chocolatey.org
# answered 504, choco printed "installed 0/0 packages" and exited 0, and the
# missing qpdf surfaced only much later as unrelated-looking failures (Rust
# tests "qpdf runs (install qpdf)", skipped derived fixtures, a self-test
# `io_error`). Checking `qpdf --version` here, not the package manager's exit
# code, catches that.
#
# Usage: scripts/ci-install-qpdf.sh   (CI only; on a dev machine see CLAUDE.md)
set -uo pipefail

attempts=4

case "$(uname -s)" in
  Darwin) install_qpdf() { brew install qpdf; } ;;
  MINGW* | MSYS* | CYGWIN*) install_qpdf() { choco install qpdf --no-progress -y; } ;;
  Linux)
    install_qpdf() {
      sudo apt-get -o Acquire::Retries=3 update &&
        sudo apt-get -o Acquire::Retries=3 install -y qpdf
    }
    ;;
  *)
    echo "::error title=qpdf install::unsupported OS: $(uname -s)"
    exit 1
    ;;
esac

for attempt in $(seq 1 "$attempts"); do
  echo "Installing qpdf (attempt $attempt/$attempts)"
  install_qpdf || echo "::warning title=qpdf install::attempt $attempt/$attempts exited with status $?"
  if version="$(qpdf --version 2>/dev/null)"; then
    echo "${version%%$'\n'*} at $(command -v qpdf)"
    exit 0
  fi
  if ((attempt < attempts)); then
    delay=$((attempt * 15))
    echo "qpdf is not on PATH after attempt $attempt; retrying in ${delay}s"
    sleep "$delay"
  fi
done

echo "::error title=qpdf missing::qpdf is still not on PATH after $attempts install attempts (package mirror down?). Tests, fixtures and self-tests need it; re-run the job."
exit 1
