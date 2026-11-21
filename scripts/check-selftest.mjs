#!/usr/bin/env node
// Asserts on a scripts/selftest.sh report: the page reached the canvas, the
// layout is mirrored the right way, and startup stayed inside its budget.
//
// Usage: node scripts/check-selftest.mjs <out-dir> <ltr|rtl>

import { readFileSync } from "node:fs";
import path from "node:path";

const [outDir, direction] = process.argv.slice(2);
const report = JSON.parse(readFileSync(path.join(outDir, "report.json"), "utf8"));
const results = report.results;
const failures = [];
const check = (ok, message) => {
  if (!ok) failures.push(message);
};

check(results.errorCode === null, `the app reported an error: ${results.errorCode}`);
check(results.dir === direction, `document direction is ${results.dir}, expected ${direction}`);
check(results.painted?.width > 0, "no page was painted");
check(results.inkRatio > 0.002 && results.inkRatio < 0.5, `implausible ink ratio ${results.inkRatio}`);
check(results.canvasPixels?.width === results.painted?.width, "canvas size differs from the bitmap");

const { toolRail, inspector, window: win } = results;
if (direction === "ltr") {
  check(toolRail.x === 0, `LTR: tool rail should be at the left edge (x=${toolRail.x})`);
  check(Math.round(inspector.x + inspector.width) === win.width, "LTR: inspector should be at the right edge");
} else {
  check(Math.round(toolRail.x + toolRail.width) === win.width, `RTL: tool rail should be at the right edge (x=${toolRail.x})`);
  check(inspector.x === 0, "RTL: inspector should be at the left edge");
}

// Spec budget: first page visible within 300 ms of opening. The debug build
// and dev server are far slower than a release build, so this is a loose guard.
check(results.painted?.roundTripMs < 300, `first page took ${results.painted?.roundTripMs} ms`);

if (failures.length > 0) {
  console.error(`self-test (${direction}) FAILED:\n  ${failures.join("\n  ")}`);
  process.exit(1);
}
console.log(
  `self-test (${direction}) ok: ${results.painted.width}x${results.painted.height} px, ` +
    `render ${(results.painted.renderMicros / 1000).toFixed(1)} ms, ` +
    `round trip ${results.painted.roundTripMs.toFixed(0)} ms, reported after ${report.reportedAfterMs} ms`,
);
