#!/usr/bin/env node
// Asserts on a scripts/selftest.sh report: page 1 reached the screen as sharp
// tiles, the layout is mirrored the right way (and the page area is not), and
// the first page appeared inside its budget.
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
// Every on-screen tile of page 1 is drawn (more may be prefetched below the fold).
check(results.tiles > 0 && results.tiles >= results.painted?.tiles, `tiles on screen: ${results.tiles}`);
check(results.inkRatio > 0.002 && results.inkRatio < 0.5, `implausible ink ratio ${results.inkRatio}`);
check(results.canvasPixels?.width === results.painted?.width, "composited page size differs from the page");
// Document direction is independent of UI direction: pages never mirror.
check(results.viewerDirection === "ltr", `page area direction is ${results.viewerDirection}`);

const { toolRail, inspector, window: win } = results;
if (direction === "ltr") {
  check(toolRail.x === 0, `LTR: tool rail should be at the left edge (x=${toolRail.x})`);
  check(Math.round(inspector.x + inspector.width) === win.width, "LTR: inspector should be at the right edge");
} else {
  check(Math.round(toolRail.x + toolRail.width) === win.width, `RTL: tool rail should be at the right edge (x=${toolRail.x})`);
  check(inspector.x === 0, "RTL: inspector should be at the left edge");
}

// Spec budget: first page visible within 300 ms of opening. Measured from the
// viewer's creation to page 1's low-resolution preview and to its sharp tiles.
const preview = results.painted?.previewSinceOpenMs;
check(preview === null || preview < 300, `first page preview took ${preview} ms`);
check(results.painted?.sinceOpenMs < 300, `first page took ${results.painted?.sinceOpenMs} ms to be sharp`);

if (failures.length > 0) {
  console.error(`self-test (${direction}) FAILED:\n  ${failures.join("\n  ")}`);
  process.exit(1);
}
const { painted } = results;
console.log(
  `self-test (${direction}) ok: ${painted.width}x${painted.height} px in ${painted.tiles} tiles, ` +
    `render ${(painted.renderMicros / 1000).toFixed(1)} ms, preview after ${painted.previewSinceOpenMs?.toFixed(0)} ms, ` +
    `sharp after ${painted.sinceOpenMs.toFixed(0)} ms, reported after ${report.reportedAfterMs} ms`,
);
