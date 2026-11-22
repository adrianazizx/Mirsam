#!/usr/bin/env node
// Asserts the spec section 11 viewing budgets on a scripts/scroll-benchmark.sh
// report, and prints the per-phase table.
//
// Budgets:
//   - 60 fps while scrolling and zooming: at most 1% of all frames are dropped
//     (an interval over 25 ms: at least one 60 Hz vsync missed), and in every
//     phase of 40 frames or more the 95th-percentile interval is at most 20 ms. WKWebView's frame
//     timestamps jitter by 1-2 ms even on an idle page (p95 18-19 ms with
//     nothing happening), so a p95 just above 16.7 ms cannot tell smooth from
//     not; a missed vsync can (ADR 0011).
//   - No page area stays blank (neither preview nor sharp tiles) for more than
//     100 ms at any point. While scrolling this is sampled every frame; for
//     random jumps across the document it is timed by the viewer's own events
//     (jump -> preview drawn, plus one frame to reach the screen), which is
//     valid even when the window was hidden.
//   - Memory bounded under 500 MB on the 1,000-page document: the app's own
//     (shell) process RSS. The webview's processes are measured and printed
//     but not budgeted (owner decision, session 6; ADR 0011).
//
// Frame and blank-time numbers are only meaningful when the window was on
// screen the whole time; a run without real animation frames fails as invalid.
//
// --ci: for hosted CI runners, which are virtual machines without a real GPU
// running a debug build. Frame timing is reported as warnings there; blank
// time, jump-to-preview time, and the shell's memory still fail the run. The
// frame budgets are enforced on real hardware.
//
// Usage: node scripts/check-scroll-benchmark.mjs <out-dir> [--ci]

import { readFileSync } from "node:fs";
import path from "node:path";

const BUDGET = {
  p95FrameMs: 20,
  p95MinFrames: 40,
  droppedFrameShare: 0.01,
  droppedFrameMs: 25,
  blankMs: 100,
  memoryMb: 500,
};

const [outDir, ...flags] = process.argv.slice(2);
const ci = flags.includes("--ci");
const report = JSON.parse(readFileSync(path.join(outDir, "report.json"), "utf8"));
const r = report.results;
const failures = [];
const warnings = [];
/** `soft`: a budget that only real hardware can judge (see --ci above). */
const check = (ok, message, soft = false) => {
  if (!ok) (ci && soft ? warnings : failures).push(message);
};

if (r.spike !== "scroll") {
  console.error(`not a scroll benchmark report: ${JSON.stringify(r).slice(0, 200)}`);
  process.exit(1);
}

const pad = (value, width) => String(value).padStart(width);
console.log(
  `${"phase".padEnd(30)} ${pad("frames", 6)} ${pad("median", 7)} ${pad("p95", 6)} ${pad("max", 6)} ` +
    `${pad(">20ms", 6)} ${pad(">25ms", 6)} ${pad("blank max", 9)} ${pad(">100ms", 6)} ${pad("pages", 6)} ${pad("zoom", 9)}`,
);
for (const p of r.phases) {
  console.log(
    `${p.name.padEnd(30)} ${pad(p.frames, 6)} ${pad(p.medianFrameMs, 7)} ${pad(p.p95FrameMs, 6)} ` +
      `${pad(p.longestFrameMs, 6)} ${pad(p.framesOver20Ms, 6)} ${pad(p.droppedFrames ?? "?", 6)} ${pad(p.maxBlankMs, 9)} ` +
      `${pad(p.blankEpisodesOver100Ms, 6)} ${pad(p.pagesSeen, 6)} ${pad(`${p.zoomStart}→${p.zoomEnd}`, 9)}`,
  );
}
const { summary, memory, randomAccess } = r;
const FRAME_MS = 1000 / 60;
console.log(
  `random access (${randomAccess.jumps.length} jumps): blank until preview p95 ${randomAccess.p95PreviewMs} ms, ` +
    `max ${randomAccess.maxPreviewMs} ms; sharp median ${randomAccess.medianSharpMs} ms, max ${randomAccess.maxSharpMs} ms`,
);
console.log(
  `memory (MB): shell ${memory.atStart.shellRssMb} → ${memory.afterJumps.shellRssMb} after jumps → ` +
    `${memory.atEnd.shellRssMb} at end → ${memory.afterIdle2s.shellRssMb} after 2 s idle; ` +
    `webview ${memory.atStart.webviewRssMb} → ${memory.afterJumps.webviewRssMb} → ${memory.atEnd.webviewRssMb} → ` +
    `${memory.afterIdle2s.webviewRssMb}`,
);
const stats = r.renderStats?.delta;
if (stats) {
  console.log(
    `engine: ${stats.rendered} renders (${(stats.renderMicros / 1000).toFixed(0)} ms), ${stats.cacheHits} cache hits, ` +
      `${stats.coalesced} coalesced, ${stats.cancelled} cancelled, ${stats.pageLoads} page loads, ` +
      `cache ${(r.renderStats.after.cacheBytes / 1048576).toFixed(0)} MB in ${r.renderStats.after.cacheEntries} tiles`,
  );
}

const valid = r.visibility === "visible" && summary.fallbackFrames === 0 && r.skipped.length === 0;
check(
  valid,
  `the window was not on screen (visibility ${r.visibility}, ${summary.fallbackFrames} timer-driven frames, ` +
    `${r.skipped.length} phases skipped): ` +
    "frame and blank-time numbers are invalid; rerun with the screen unlocked and the window visible",
);
if (valid) {
  // A p95 over a short idle phase (~20 frames) is its second-slowest frame;
  // short phases count through the dropped-frame share below instead.
  for (const p of r.phases.filter((phase) => phase.frames >= BUDGET.p95MinFrames)) {
    check(
      p.p95FrameMs <= BUDGET.p95FrameMs,
      `${p.name}: p95 frame ${p.p95FrameMs} ms > ${BUDGET.p95FrameMs} ms`,
      true,
    );
  }
  const share = summary.droppedFrames / Math.max(1, summary.frames);
  check(
    share <= BUDGET.droppedFrameShare,
    `${summary.droppedFrames} of ${summary.frames} frames dropped (over ${BUDGET.droppedFrameMs} ms, ${(share * 100).toFixed(1)}%)`,
    true,
  );
  check(summary.maxBlankMs <= BUDGET.blankMs, `a page stayed blank for ${summary.maxBlankMs} ms`);
}
check(
  randomAccess.maxPreviewMs !== null && randomAccess.maxPreviewMs + FRAME_MS <= BUDGET.blankMs,
  `a jumped-to page stayed blank for ${randomAccess.maxPreviewMs} ms (+ one frame)`,
);
const shellPeak = Math.max(
  memory.afterJumps.shellRssMb ?? 0,
  memory.atEnd.shellRssMb ?? 0,
  memory.afterIdle2s.shellRssMb ?? 0,
);
check(shellPeak < BUDGET.memoryMb, `shell RSS ${shellPeak} MB`);

for (const warning of warnings) {
  // A GitHub Actions annotation: shown on the run and the pull request.
  console.log(`::warning title=Scroll benchmark (CI runner)::${warning}`);
}
if (failures.length > 0) {
  console.error(`scroll benchmark FAILED:\n  ${failures.join("\n  ")}`);
  process.exit(1);
}
console.log(
  `scroll benchmark ${warnings.length > 0 ? `ok on its hard budgets, ${warnings.length} warnings` : "ok"}: worst p95 ${summary.worstP95FrameMs} ms, longest frame ${summary.longestFrameMs} ms, ` +
    `${summary.framesOver20Ms}/${summary.frames} frames over 20 ms, max blank ${summary.maxBlankMs} ms, ` +
    `shell RSS ≤ ${shellPeak} MB`,
);
