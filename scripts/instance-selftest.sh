#!/usr/bin/env bash
# End-to-end check of the single running copy (ADR 0046): a second launch with
# a document hands it to the running copy and exits.
#
# 1. Starts the app through scripts/selftest.sh with SAFHA_SELFTEST_SPIKE=handoff
#    on the first PDF; its UI then waits for a second document.
# 2. Once that copy holds its state directory, launches a second copy on the
#    same state directory with the second PDF as a relative command-line
#    argument, from another working directory.
# 3. Checks that the second copy exited at once and successfully, and that the
#    first one shows the second PDF as its active tab.
#
# Usage: scripts/instance-selftest.sh <out-dir>
# Needs: cargo build -p safha-desktop --features devtools (or SAFHA_SELFTEST_APP,
#        as for scripts/selftest.sh).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$1"
OUT="$(cd "$1" && pwd)"
FIRST="$ROOT/fixtures/out/chrome/en-paragraphs.pdf"
SECOND_DIR="$ROOT/fixtures/out/derived"
SECOND_NAME="rotated.pdf"
APP="${SAFHA_SELFTEST_APP:-$ROOT/target/debug/safha-desktop}"
[[ -f "$APP.exe" ]] && APP="$APP.exe"
for file in "$FIRST" "$SECOND_DIR/$SECOND_NAME"; do
  [[ -f "$file" ]] || { echo "missing $file: run node fixtures/generate.mjs" >&2; exit 2; }
done
native() { if command -v cygpath >/dev/null; then cygpath -w "$1"; else printf '%s' "$1"; fi; }

SAFHA_SELFTEST_SPIKE=handoff "$ROOT/scripts/selftest.sh" "$FIRST" "$OUT" en light &
FIRST_PID=$!

# The first copy writes instance.json once it holds the lock and listens.
for _ in $(seq 1 300); do
  [[ -f "$OUT/state/instance.json" ]] && break
  kill -0 "$FIRST_PID" 2>/dev/null || break
  sleep 0.2
done
[[ -f "$OUT/state/instance.json" ]] || { echo "the first copy never claimed $OUT/state" >&2; wait "$FIRST_PID" || true; exit 1; }
# Let its UI ask for its startup documents first, so the second one arrives
# as an event (the path a user takes when the app is already open).
sleep 3

started=$(date +%s)
set +e
(cd "$SECOND_DIR" && SAFHA_STATE_DIR="$(native "$OUT/state")" "$APP" "$SECOND_NAME" >"$OUT/second.log" 2>&1)
second_status=$?
set -e
second_seconds=$(( $(date +%s) - started ))
wait "$FIRST_PID" || true

node - "$OUT/report.json" "$second_status" "$second_seconds" "$SECOND_NAME" <<'EOF'
const [report, status, seconds, second] = process.argv.slice(2);
const r = JSON.parse(require("node:fs").readFileSync(report, "utf8")).results;
const failures = [];
if (status !== "0") failures.push(`the second copy exited with status ${status}`);
if (Number(seconds) > 5) failures.push(`the second copy took ${seconds} s to hand over`);
if (r.spike !== "handoff") failures.push(`unexpected report ${JSON.stringify(r).slice(0, 200)}`);
else {
  if (!r.documents.includes(second)) failures.push(`the running copy did not open ${second}: ${r.documents}`);
  if (r.active !== second) failures.push(`the active tab is ${r.active}, not ${second}`);
  if (r.errorCode) failures.push(`error shown: ${r.errorCode}`);
}
console.log(`documents ${JSON.stringify(r.documents)}, active ${r.active}; second copy exited ${status} after ${seconds} s`);
if (failures.length) {
  console.error(`instance self-test FAILED:\n  ${failures.join("\n  ")}`);
  process.exit(1);
}
console.log("instance self-test ok");
EOF
