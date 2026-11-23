#!/usr/bin/env bash
# End-to-end check of crash reports (ADR 0061) on the real desktop shell.
#
# 1. Launches the app (scripts/selftest.sh, SAFHA_SELFTEST_SPIKE=crash:<kind>):
#    it runs a few commands, then fails on purpose:
#      panic         a Rust panic on the main thread (the panic hook's report);
#      abort         the process aborts without a panic, as a native crash does
#                    (the system's crash log, read at the next start; macOS and
#                    Windows only);
#      thread-panic  a panic on another thread; the app keeps running.
# 2. Launches it again on the same state directory (crash:check, Arabic UI):
#    the prompt must offer the report, and the report must hold the version,
#    the system, the last commands, and nothing private: the panic's message
#    names a file and quotes Arabic text, which must not appear.
#
# Usage: scripts/crash-selftest.sh <out-dir> [panic|abort|thread-panic]
# Needs: cargo build -p safha-desktop --features devtools (or SAFHA_SELFTEST_APP
#        and SAFHA_SELFTEST_UI=bundled, as for scripts/selftest.sh).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$1"
OUT="$(cd "$1" && pwd)"
KIND="${2:-panic}"
PDF="$ROOT/fixtures/out/chrome/en-paragraphs.pdf"
[[ -f "$PDF" ]] || { echo "missing $PDF: run node fixtures/generate.mjs" >&2; exit 2; }
case "$KIND" in panic|abort|thread-panic) ;; *) echo "kind must be panic, abort or thread-panic" >&2; exit 2 ;; esac

export SAFHA_SELFTEST_STATE="$OUT/state"
rm -rf "$SAFHA_SELFTEST_STATE"

# 1. Fail on purpose. A crash writes no self-test report, so selftest.sh's
#    "no report" is expected here.
SAFHA_SELFTEST_SPIKE="crash:$KIND" "$ROOT/scripts/selftest.sh" "$PDF" "$OUT/crash" en light || true

# 2. Start again: the report is offered. After an abort the system's crash
#    log can take minutes to appear (macOS writes it late under load); the
#    app keeps looking for it and offers the report once it is there.
if [[ "$KIND" == abort ]]; then
  SAFHA_SELFTEST_TIMEOUT=240 SAFHA_SELFTEST_SPIKE=crash:check-late "$ROOT/scripts/selftest.sh" "$PDF" "$OUT/check" ar dark
else
  SAFHA_SELFTEST_SPIKE=crash:check "$ROOT/scripts/selftest.sh" "$PDF" "$OUT/check" ar dark
fi

node - "$OUT" "$KIND" <<'NODE'
const fs = require("node:fs");
const [out, kind] = process.argv.slice(2);
const read = (path) => JSON.parse(fs.readFileSync(path, "utf8")).results;
const failures = [];
const check = (ok, message) => { if (!ok) failures.push(message); };
if (kind === "thread-panic") {
  const crashed = read(`${out}/crash/report.json`);
  check(crashed.after?.reports === 1, `the app kept ${crashed.after?.reports} reports after the thread's panic`);
}
const r = read(`${out}/check/report.json`);
check(r.offered && r.dialogShown, "the prompt after the crash was not shown");
check(r.dialogDir === "rtl", `the Arabic UI ran ${r.dialogDir}`);
check(r.reports?.length === 1, `${r.reports?.length} reports offered, expected 1`);
const report = r.reports?.[0] ?? { kind: "", text: "" };
const text = report.text;
check(report.kind === (kind === "abort" ? "native" : "panic"), `the report is a ${report.kind} report`);
check(/^Version: \S+ \((debug|release), /m.test(text), "the report has no version");
check(/^System: \S/m.test(text), "the report has no system");
check(text.includes("search_document"), "the report's last commands miss search_document");
if (kind === "abort") check(/SIGABRT|c0000409|abort/i.test(text), "the native report names no abort");
else check(text.includes("self-test panic opening <path> <path>: <quoted>"), "the panic's message was not redacted as expected");
for (const secret of ["someone", "Secret", "Contract", "عقد", process.env.HOME ?? "/nonexistent", process.env.USER ?? "/nonexistent"]) {
  check(!text.includes(secret), `the report contains ${JSON.stringify(secret)}`);
}
console.log(`crash self-test (${kind}): ${report.kind} report offered in the Arabic UI\n${text}`);
if (failures.length) {
  console.error(failures.map((failure) => `FAIL: ${failure}`).join("\n"));
  process.exit(1);
}
console.log("crash self-test passed");
NODE
