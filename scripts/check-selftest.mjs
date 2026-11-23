#!/usr/bin/env node
// Asserts on a scripts/selftest.sh report: page 1 reached the screen as sharp
// tiles, the layout is mirrored the right way (and the page area is not), and
// the first page appeared inside its budget.
//
// --ci: on hosted CI runners (virtual machines, a debug build, Vite serving
// the UI) the first-page time budget only warns; the behaviour checks still
// fail the run. The budget is enforced on real hardware.
//
// Usage: node scripts/check-selftest.mjs <out-dir> <ltr|rtl> [--ci]

import { readFileSync } from "node:fs";
import path from "node:path";

const [outDir, direction, ...flags] = process.argv.slice(2);
const ci = flags.includes("--ci");
const warnings = [];
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
/** A time budget: on CI runners it warns (see --ci above). */
const budget = (ok, message) => {
  if (!ok) (ci ? warnings : failures).push(message);
};
budget(preview === null || preview < 300, `first page preview took ${preview} ms`);
budget(results.painted?.sinceOpenMs < 300, `first page took ${results.painted?.sinceOpenMs} ms to be sharp`);

// Milestone 1 navigation (selftest.ts, navigationChecks): selecting page 1
// copies exactly its text layer, a search finds a word of it and marks it on
// the page, and the thumbnail strip draws.
const nav = results.navigation;
check(nav !== null && nav !== undefined, "no navigation results");
if (nav) {
  if (nav.textBlocks > 0) {
    check(nav.selection.length > 0, "select all on page 1 selected nothing");
    check(nav.selectionMatchesLayer, "the selected text differs from the page's text layer");
    check(nav.selectionRects > 0, "the selection was not drawn");
    check(nav.selectionToolbar, "no selection toolbar");
    check(nav.searchStatus === "done", `search ended as ${nav.searchStatus}`);
    check(nav.searchHitsOnPage1 > 0, `searching "${nav.query}" found nothing on page 1`);
    check(nav.searchHitMarks > 0, "search hits were not drawn on page 1");
  }
  check(nav.thumbnailDrawn, "no thumbnail was drawn");
  check((nav.windows?.length ?? 0) >= 2, `a new window did not open (windows: ${JSON.stringify(nav.windows)})`);
  // Markup (ADR 0017): highlighting the selection paints yellow in the same view; undo removes it.
  const markup = nav.markup;
  if (nav.textBlocks > 0) {
    check(markup !== undefined && markup !== null, "no markup results");
    if (markup) {
      check(markup.kinds.length === 1 && markup.kinds[0] === "highlight", `markup listed: ${JSON.stringify(markup.kinds)}`);
      check(markup.quads === markup.runs, `the highlight has ${markup.quads} quads for ${markup.runs} selected runs`);
      check(markup.painted && markup.sameViewer, "the highlight was not painted by the same view");
      check(markup.yellowPixels > 500, `only ${markup.yellowPixels} highlighted pixels on screen`);
      check(markup.modified === true, "a highlighted document does not count as modified");
      check(markup.removedByUndo, "undo did not remove the highlight");
    }
  }
  // Notes and ink (ADR 0018): an Arabic note and a stroke, painted and read back, erased, undone.
  const notes = nav.notesAndInk;
  check(notes !== undefined && notes !== null, "no notes-and-ink results");
  if (notes) {
    check(notes.noteMatches, "the note's Arabic text did not come back as written");
    // Pixels enough for a small mark at the view's render scale (selftest.ts `enoughAt`).
    const enough = notes.enough ?? 50;
    const of = (px) => `${px} px, ${enough} needed at ${notes.scale?.toFixed(2)} px/pt`;
    check(notes.notePainted && notes.notePixels > enough, `the note's icon was not painted (${of(notes.notePixels)})`);
    check(notes.inkPainted && notes.inkPixels > enough, `the stroke was not painted (${of(notes.inkPixels)})`);
    check(notes.inkPoints === 3, `the stroke came back with ${notes.inkPoints} points`);
    check(notes.boxMatches, "the text box's Arabic text did not come back as written");
    check(notes.boxPainted && notes.boxPixels > enough, `the text box was not painted (${of(notes.boxPixels)})`);
    check(notes.erased, "the eraser did not remove the stroke");
    // The comments panel (ADR 0041).
    const comments = notes.comments;
    check(
      JSON.stringify(comments?.kinds) === JSON.stringify(["ink", "note", "textBox"]),
      `the comments listed were ${JSON.stringify(comments?.kinds)}`,
    );
    check(comments?.replied, "the reply to the note did not read back");
    check(comments?.resolved, "the note was not resolved");
    check(comments?.panelItems >= 3, `the comments panel showed ${comments?.panelItems} comments`);
    check(comments?.replyOnPage === 3, `replies and states counted as page annotations (${comments?.replyOnPage})`);
    check(notes.cleared, "undo did not take the note, the text box, and the stroke away");
  }
  // New page text (ADR 0021): written into the page with the Edit tool.
  const added = nav.addedText;
  check(added !== undefined && added !== null, "no added-text results");
  if (added) {
    check(added.painted, `the added text was not painted (${added.pixels} px, ${added.enough} needed)`);
    check(added.readBack, "the added Arabic text did not read back as page text");
    check(added.undone, "undo did not take the added text away");
    // In-place editing (ADR 0023): the paragraph is found, edited, and read back.
    check(added.blockFound, "the added paragraph was not found as one right-to-left block in logical order");
    check(added.blockEdited && added.blockReadBack, "the edited paragraph did not read back as page text");
    // Bold from the editor (ADR 0025): the browser's own command, saved, read back.
    check(added.boldApplied, "the editor's bold command did not make the selected word bold in the field");
    check(added.boldReadBack, "the word made bold in the editor did not read back as a bold run");
    // Spec section 11: typing in the paragraph field, 32 ms a keystroke (the
    // median). It must have been measured; the budget is a time budget, so on
    // hosted runners (software rendering under Xvfb) it warns.
    check(added.typingMedianMs >= 0, "typing in the paragraph field was not measured");
    budget(
      added.typingMedianMs <= 32,
      `typing in the paragraph field took ${added.typingMedianMs} ms a keystroke (median; budget 32 ms)`,
    );
  }
  // Page objects and links (ADR 0051): through the real shell's commands.
  const objects = nav.objectsAndLinks;
  check(objects !== undefined && objects !== null, "no objects-and-links results");
  if (objects) {
    check(objects.objects > 0 && objects.kinds.includes("text"), `the page's objects were ${JSON.stringify(objects.kinds)}`);
    check(objects.moved, "the text block did not move where it was dragged");
    // CoreText's Arabic draws dots as glyphs of their own with no text ("suspect", ADR 0023): a
    // block moved on their line regroups a dot with it. Only a page that reads well must read the same.
    if (nav.textQuality === "good") check(objects.textKept, "moving a text block changed the page's words");
    check(objects.undone, "undo did not put the text block back byte for byte");
    check(objects.linkAdded, "the link to an Arabic address did not read back, percent-encoded");
    check(objects.linkUndone, "undo did not remove the link");
    check(objects.errorCode === null, `objects and links reported ${objects.errorCode}`);
  }
  // Editing (ADR 0014): duplicate a page, the new revision paints, undo restores the file.
  const editing = nav.editing;
  check(editing !== undefined && editing !== null, "no editing results");
  if (editing) {
    check(editing.pagesAfterEdit === editing.pagesBefore + 1, `duplicating gave ${editing.pagesAfterEdit} pages`);
    check(editing.repainted, "the edited document was not painted");
    check(editing.closePrompted, "closing a window with unsaved changes did not ask first");
    if (editing.pagesBefore >= 2) {
      check(editing.organizer?.gridImages > 0, "the page organizer drew no page");
      check(editing.organizer?.moved, "moving the first page later did not put it second");
    }
    check(editing.pagesAfterUndo === editing.pagesBefore, `undo left ${editing.pagesAfterUndo} pages`);
    check(editing.modifiedAfterUndo === false, "undo back to the file still counts as modified");
    // Autosave (ADR 0040): an unsaved change gets a recovery file; undo back to the file removes it.
    check(editing.recoveryAfterEdit === true, "the unsaved change was not kept in a recovery file");
    check(editing.recoveryAfterUndo === false, "undo back to the file left its recovery file");
  }
}

for (const warning of warnings) {
  // A GitHub Actions annotation: shown on the run and the pull request.
  console.log(`::warning title=Self-test (CI runner)::${warning}`);
}
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
if (nav) {
  console.log(
    `  navigation: selected ${nav.selection.length} chars in ${nav.selectionRects} runs; ` +
      `"${nav.query}": ${nav.searchHits} hits in ${nav.searchMs.toFixed(0)} ms; ` +
      `outline ${nav.outlineItems} items; ${nav.thumbnails} thumbnails mounted; windows ${JSON.stringify(nav.windows)}`,
  );
  if (nav.markup) {
    const m = nav.markup;
    console.log(
      `  markup: highlight of ${m.runs} runs in ${m.markMs} ms, ${m.yellowPixels} px painted, ` +
        `same view ${m.sameViewer}, undo removed it ${m.removedByUndo}`,
    );
  }
  if (nav.notesAndInk) {
    const n = nav.notesAndInk;
    console.log(
      `  notes, text boxes, ink: note ${n.notePixels} px, text kept ${n.noteMatches}; ` +
        `text box ${n.boxPixels} px, text kept ${n.boxMatches}; stroke ${n.inkPixels} px, ` +
        `erased ${n.erased}, undone ${n.cleared}; comments ${JSON.stringify(n.comments?.kinds)}, ` +
        `reply ${n.comments?.replied}, resolved ${n.comments?.resolved}, ${n.comments?.panelItems} in the panel`,
    );
  }
  if (nav.addedText) {
    const a = nav.addedText;
    console.log(
      `  added text: ${a.pixels} px painted, read back ${a.readBack}, undone ${a.undone}; ` +
        `edited in place: found ${a.blockFound}, read back ${a.blockReadBack}; bold in the field ${a.boldApplied}, read back ${a.boldReadBack}; typing ${a.typingMedianMs} ms (p95 ${a.typingP95Ms})`,
    );
  }
  if (nav.objectsAndLinks) {
    const o = nav.objectsAndLinks;
    console.log(
      `  objects and links: ${o.objects} objects ${JSON.stringify(o.kinds)}; a block moved ${o.moved}, words kept ${o.textKept}, undone ${o.undone}; ` +
        `a link added ${o.linkAdded}, undone ${o.linkUndone}`,
    );
  }
  if (nav.editing) {
    const e = nav.editing;
    console.log(
      `  editing: duplicate ${e.pagesBefore} → ${e.pagesAfterEdit} pages in ${e.editMs} ms, ` +
        `undo → ${e.pagesAfterUndo}, repainted ${e.repainted}, close asked ${e.closePrompted}, ` +
        `recovery file kept ${e.recoveryAfterEdit} then removed ${e.recoveryAfterUndo === false}` +
        (e.organizer ? `; organizer: ${e.organizer.gridImages} pages drawn, move ${e.organizer.moved}` : ""),
    );
  }
}
