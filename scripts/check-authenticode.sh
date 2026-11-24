#!/usr/bin/env bash
# Checks that a Windows file carries a valid Authenticode signature from the
# expected signer (release.yml, ADR 0073). Windows only (PowerShell's
# Get-AuthenticodeSignature).
#
# Usage: FILE=<path> [SIGNER="SignPath Foundation"] scripts/check-authenticode.sh
set -euo pipefail

: "${FILE:?FILE must name the file to check}"
export FILE SIGNER="${SIGNER:-SignPath Foundation}"
[[ -f "$FILE" ]] || { echo "check-authenticode: no such file: $FILE" >&2; exit 1; }

# The path and the signer reach PowerShell through the environment, never
# through the command text.
pwsh -NoProfile -NonInteractive -Command '
  $s = Get-AuthenticodeSignature -LiteralPath $env:FILE
  $subject = if ($s.SignerCertificate) { $s.SignerCertificate.Subject } else { "(none)" }
  $stamped = if ($s.TimeStamperCertificate) { "time-stamped" } else { "not time-stamped" }
  Write-Output "$($env:FILE): $($s.Status); $subject; $stamped"
  if ($s.Status -ne "Valid") { Write-Output "::error::$($env:FILE) is not validly signed: $($s.StatusMessage)"; exit 1 }
  if (-not $subject.Contains($env:SIGNER)) { Write-Output "::error::$($env:FILE) is signed by $subject, not $($env:SIGNER)"; exit 1 }
  if (-not $s.TimeStamperCertificate) { Write-Output "::error::$($env:FILE) has no time-stamp, so its signature ends with the certificate"; exit 1 }
'
