import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { backend } from "./backend";
import { runCommand } from "./commands/registry";
import { useAppStore } from "./state/appStore";
import { PAGE_PAINTED_EVENT, type PagePaintedDetail } from "./viewer/events";
import { getViewer } from "./viewer/registry";

/**
 * Resolves on the next animation frame. A webview whose window is occluded or
 * whose display is asleep gets no animation frames at all, so fall back to a
 * timer rather than hanging the self-test.
 */
export function nextFrame(): Promise<void> {
  return new Promise((resolve) => {
    const timer = window.setTimeout(resolve, 250);
    requestAnimationFrame(() => {
      window.clearTimeout(timer);
      resolve();
    });
  });
}

/**
 * Resolves with the first painted event for `page` that `accept` takes, or
 * null after `timeoutMs`. Start waiting before whatever causes the paint: a
 * fast paint can otherwise come and go before the wait begins.
 */
export function waitForPagePainted(
  page: number,
  timeoutMs: number,
  accept: (detail: PagePaintedDetail) => boolean = () => true,
): Promise<PagePaintedDetail | null> {
  return new Promise((resolve) => {
    const onPainted = (event: Event) => {
      const detail = (event as CustomEvent<PagePaintedDetail>).detail;
      if (detail.page !== page || !accept(detail)) return;
      window.clearTimeout(timeout);
      window.removeEventListener(PAGE_PAINTED_EVENT, onPainted);
      resolve(detail);
    };
    const timeout = window.setTimeout(() => {
      window.removeEventListener(PAGE_PAINTED_EVENT, onPainted);
      resolve(null);
    }, timeoutMs);
    window.addEventListener(PAGE_PAINTED_EVENT, onPainted);
  });
}

/**
 * Draws page `page` as it is on screen — its full-resolution tiles at their
 * positions — into one canvas of the whole page's pixel size, and measures the
 * ink in the tiled area.
 */
function compositePage(documentId: number, painted: PagePaintedDetail) {
  const viewer = getViewer(documentId);
  if (!viewer) return null;
  const { tiles } = viewer.pageCanvases(painted.page);
  const canvas = document.createElement("canvas");
  canvas.width = painted.width;
  canvas.height = painted.height;
  const context = canvas.getContext("2d");
  if (!context) return null;
  context.fillStyle = "#ffffff";
  context.fillRect(0, 0, canvas.width, canvas.height);
  let ink = 0;
  let pixels = 0;
  for (const { canvas: tileCanvas, tile } of tiles) {
    context.drawImage(tileCanvas, tile.x, tile.y);
    const data = tileCanvas.getContext("2d")?.getImageData(0, 0, tile.width, tile.height).data;
    if (!data) continue;
    for (let i = 0; i < data.length; i += 4) {
      if ((data[i] ?? 255) < 200 || (data[i + 1] ?? 255) < 200 || (data[i + 2] ?? 255) < 200) {
        ink += 1;
      }
    }
    pixels += data.length / 4;
  }
  return { canvas, tiles: tiles.length, inkRatio: pixels > 0 ? ink / pixels : null };
}

/** Polls `ready` once per frame until it holds or `timeoutMs` passes. */
async function waitFor(ready: () => boolean, timeoutMs: number): Promise<boolean> {
  const until = performance.now() + timeoutMs;
  while (!ready()) {
    if (performance.now() > until) return false;
    await nextFrame();
  }
  return true;
}

/**
 * Milestone 1 navigation on the real engine: the text layer, selecting and
 * copying the first page's text, a search for one of its words with hits
 * drawn on the page, the outline, and thumbnails.
 */
async function navigationChecks(documentId: number) {
  const viewer = getViewer(documentId);
  const layer = await backend.pageText(documentId, 0);
  const layerText = layer.blocks
    .map((block) => block.lines.map((line) => line.text).join("\n"))
    .join("\n\n");

  // Select all of page 1 through the command, as a user would.
  await runCommand("edit.selectAll");
  await waitFor(() => viewer?.hasSelection() ?? false, 3_000);
  const selection = viewer?.selectedText() ?? "";
  const overlay = document.querySelector('[data-page="0"] [class*="overlay"]');
  const selectionRects = overlay?.childElementCount ?? 0;
  const toolbar = document.querySelector('[data-testid="selection-toolbar"]') !== null;
  await runCommand("edit.deselect");

  // Search for the longest word on the page's first line.
  const words = (layer.blocks[0]?.lines[0]?.text ?? "")
    .split(/\s+/u)
    .filter((w) => /\p{L}{3,}/u.test(w));
  const query =
    words.sort((a, b) => b.length - a.length)[0]?.replace(/[^\p{L}\p{M}\p{N}]/gu, "") ?? "";
  const started = performance.now();
  await runCommand("search.run", { query });
  const search = useAppStore.getState().documents.find((open) => open.id === documentId)?.search;
  const searchMs = performance.now() - started;
  await nextFrame();
  const hitMarks = document.querySelectorAll('[data-page="0"] [class*="searchHit"]').length;

  const outline = await backend.documentOutline(documentId);
  const labels = await backend.pageLabels(documentId);

  // Thumbnails: the panel's first one gets its image.
  await runCommand("view.showThumbnails");
  const thumbnailDrawn = await waitFor(
    () => document.querySelector('[data-testid="thumbnails"] canvas') !== null,
    5_000,
  );
  const thumbnails = document.querySelectorAll('[data-testid="thumbnails"] button').length;

  // Markup (ADR 0017): highlight the page's text from the selection, see it
  // painted by the same view (no rebuild), find it listed, then undo it.
  const markup = layer.blocks.length === 0 ? null : await markupChecks(documentId);
  // Notes and ink (ADR 0018): an Arabic note and a pen stroke, painted and read
  // back; the stroke erased; everything undone.
  const notesAndInk = await notesAndInkChecks(documentId);
  // New page text (ADR 0021): Arabic written into the page's content with the
  // Edit tool, painted, read back as the page's own text, then undone.
  const addedText = await addedTextChecks(documentId);
  // Editing in place (ADR 0024): the page's own text opened, typed into on the
  // engine's surface, written back, read back, undone; and cancelled.
  const inPlace = await inPlaceChecks(documentId);

  // Editing (ADR 0014): duplicate the page, see the new revision painted,
  // then undo back to the file as opened.
  const editing = await (async () => {
    const pagesBefore = (await backend.pageSizes(documentId)).length;
    const started = performance.now();
    // The new revision gets a new viewer; only its paint counts.
    const before = getViewer(documentId);
    const repaint = waitForPagePainted(0, 15_000, () => getViewer(documentId) !== before);
    await runCommand("pages.duplicate");
    const pagesAfterEdit = (await backend.pageSizes(documentId)).length;
    const editMs = Math.round(performance.now() - started);
    const repainted = (await repaint) !== null;
    // Autosave (ADR 0040): the unsaved change is kept in a recovery file.
    const recoveryAfterEdit = await recoveryFile(documentId, true);
    // Closing the window now must ask first (ADR 0015) and leave it open.
    let closePrompted = false;
    if (isTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("selftest_request_close");
      closePrompted = await waitFor(
        () => useAppStore.getState().closePrompt?.kind === "window",
        5_000,
      );
      await runCommand("app.cancelClose");
    }
    await runCommand("edit.undo");
    const pagesAfterUndo = (await backend.pageSizes(documentId)).length;
    // Back to the file as opened: nothing left to recover.
    const recoveryAfterUndo = await recoveryFile(documentId, false);

    // The page organizer, when there is more than one page: its grid draws,
    // and moving the first page later puts it second.
    let organizer: { gridImages: number; moved: boolean } | null = null;
    if (pagesBefore >= 2) {
      const text = async (page: number) =>
        (await backend.pageText(documentId, page)).blocks
          .map((block) => block.lines.map((line) => line.text).join("\n"))
          .join("\n\n");
      const first = await text(0);
      useAppStore.getState().setActiveTool("pages");
      const drawn = await waitFor(
        () => document.querySelector('[role="grid"] canvas') !== null,
        10_000,
      );
      const gridImages = drawn ? document.querySelectorAll('[role="grid"] canvas').length : 0;
      useAppStore.getState().setSelectedPages(documentId, [0]);
      await runCommand("pages.moveLater");
      const moved = (await text(1)) === first && first.length > 0;
      await runCommand("edit.undo");
      const shown = waitForPagePainted(0, 15_000);
      useAppStore.getState().setActiveTool("select");
      await shown;
      organizer = { gridImages, moved };
    }
    const open = useAppStore.getState().documents.find((candidate) => candidate.id === documentId);
    return {
      pagesBefore,
      pagesAfterEdit,
      pagesAfterUndo,
      editMs,
      repainted,
      closePrompted,
      organizer,
      modifiedAfterUndo: open?.modified ?? null,
      recoveryAfterEdit,
      recoveryAfterUndo,
      revision: open?.revision ?? null,
    };
  })();

  // A second window opens, and the shell has both.
  await runCommand("window.new");
  let windows: string[] | null = null;
  await waitFor(() => {
    void backend.windowLabels().then((labels) => {
      windows = labels;
    });
    return (windows?.length ?? 0) >= 2;
  }, 5_000);

  return {
    textQuality: layer.quality,
    textBlocks: layer.blocks.length,
    selection,
    selectionMatchesLayer: selection === layerText,
    selectionRects,
    selectionToolbar: toolbar,
    query,
    searchStatus: search?.status ?? null,
    searchHits: search?.hits.length ?? 0,
    searchHitsOnPage1: search?.hits.filter((hit) => hit.page === 0).length ?? 0,
    searchHitMarks: hitMarks,
    searchMs,
    outlineItems: outline.length,
    labels: labels?.slice(0, 5) ?? null,
    thumbnailDrawn,
    thumbnails,
    markup,
    notesAndInk,
    addedText,
    inPlace,
    editing,
    windows,
  };
}

async function markupChecks(documentId: number) {
  const viewer = getViewer(documentId);
  await runCommand("edit.selectAll");
  await waitFor(() => viewer?.hasSelection() ?? false, 3_000);
  const runs = viewer?.selectionMarks().reduce((sum, page) => sum + page.rects.length, 0) ?? 0;
  const started = performance.now();
  await runCommand("markup.highlight");
  const markMs = Math.round(performance.now() - started);
  // Highlighter yellow (multiplied over white), once the repainted tiles show it.
  // A highlight of a page's text covers far more than a few tiles' worth.
  const highlighted = 500;
  const yellowPixels = await pixelsOnScreen(
    documentId,
    (r, g, b) => r > 200 && g > 150 && b < 120,
    highlighted,
  );
  const listed = await backend.pageAnnotations(documentId, 0);
  const open = useAppStore.getState().documents.find((candidate) => candidate.id === documentId);
  const result = {
    runs,
    markMs,
    painted: yellowPixels > highlighted,
    sameViewer: getViewer(documentId) === viewer,
    yellowPixels,
    kinds: listed.map((item) => item.kind),
    quads: listed[0]?.rects.length ?? 0,
    paint: open?.paint ?? null,
    modified: open?.modified ?? null,
    removedByUndo: false,
  };
  await runCommand("edit.undo");
  result.removedByUndo = (await backend.pageAnnotations(documentId, 0)).length === 0;
  return result;
}

/**
 * Enough matching pixels for a small mark (a note's icon, a line of text) at
 * the view's render scale (device pixels per point). A CI runner's small
 * window at 1x draws a page at about 1 pixel per point, where a note's icon
 * has a few dozen pure-yellow pixels; this Mac draws three times larger in
 * each direction. Nothing matches before the mark is drawn, so a small floor
 * still tells drawn from not drawn.
 */
function enoughAt(scale: number): number {
  return Math.max(8, Math.round(8 * scale * scale));
}

/**
 * Page 0's on-screen pixels that `match` takes, once there are more than
 * `enough` (or when `timeoutMs` runs out). A repaint after an edit replaces
 * tiles one by one, and a slow machine may still be repainting for another
 * reason (a zoom change) when the edit lands: waiting for one "painted" event
 * can catch the wrong one. What is on screen is what counts.
 */
async function pixelsOnScreen(
  documentId: number,
  match: (r: number, g: number, b: number) => boolean,
  enough: number,
  timeoutMs = 15_000,
): Promise<number> {
  let count = 0;
  await waitFor(() => {
    count = countPixels(documentId, match);
    return count > enough;
  }, timeoutMs);
  return count;
}

/** Waits until page 0's visible part is covered by current tiles. */
function pageSettled(documentId: number, timeoutMs = 15_000): Promise<boolean> {
  return waitFor(
    () =>
      getViewer(documentId)
        ?.snapshot()
        .visible.some((page) => page.page === 0 && page.sharp) ?? false,
    timeoutMs,
  );
}

/** Pixels of page 0's on-screen tiles that `match` takes. */
function countPixels(documentId: number, match: (r: number, g: number, b: number) => boolean) {
  let count = 0;
  for (const { canvas, tile } of getViewer(documentId)?.pageCanvases(0).tiles ?? []) {
    const data = canvas.getContext("2d")?.getImageData(0, 0, tile.width, tile.height).data;
    for (let i = 0; data && i < data.length; i += 4) {
      if (match(data[i] ?? 0, data[i + 1] ?? 0, data[i + 2] ?? 0)) count += 1;
    }
  }
  return count;
}

/**
 * Whether the document has a recovery file, polled until it is `expected`
 * (autosaves wait for edits to pause, ADR 0040), or null outside devtools.
 */
async function recoveryFile(documentId: number, expected: boolean): Promise<boolean | null> {
  const deadline = performance.now() + 10_000;
  for (;;) {
    const kept = await backend.hasRecoveryFile(documentId);
    if (kept === null || kept === expected || performance.now() > deadline) return kept;
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
}

async function addedTextChecks(documentId: number) {
  const text = "أضيف هذا النص إلى الصفحة";
  const pageText = async () =>
    (await backend.pageText(documentId, 0)).blocks
      .map((block) => block.lines.map((line) => line.text).join(" "))
      .join(" ");
  const dark = (r: number, g: number, b: number) => r < 90 && g < 90 && b < 90;
  const enough = enoughAt(getViewer(documentId)?.snapshot().scale ?? 1);
  await pageSettled(documentId);
  const darkBefore = countPixels(documentId, dark);
  useAppStore.getState().setActiveTool("edit");
  await runCommand("text.place", { page: 0, x: 60, y: 40, width: 220 });
  await runCommand("note.save", { text });
  let pixels = 0;
  await waitFor(() => {
    pixels = countPixels(documentId, dark) - darkBefore;
    return pixels > enough;
  }, 15_000);
  const readBack = (await pageText()).replace(/\s+/gu, " ").includes(text);
  await runCommand("edit.undo");
  const undone = !(await pageText()).includes(text);
  useAppStore.getState().setActiveTool("select");
  return { painted: pixels > enough, pixels, enough, readBack, undone };
}

async function inPlaceChecks(documentId: number) {
  const blocks = await backend.textBlocks(documentId, 0);
  const refusals = [...new Set(blocks.map((block) => block.refusal).filter(Boolean))];
  const index = blocks.findIndex((block) => block.refusal === null);
  if (index < 0) return { editable: false, blocks: blocks.length, refusals };
  const pageText = async () =>
    (await backend.pageText(documentId, 0)).blocks
      .map((block) => block.lines.map((line) => line.text).join(" "))
      .join(" ")
      .replace(/\s+/gu, " ");
  const editorOpen = () => document.querySelector<HTMLElement>('[data-testid="block-editor"]');
  useAppStore.getState().setActiveTool("edit");
  await pageSettled(documentId);
  await runCommand("text.editBlock", { page: 0, index, refusal: null });
  const session = useAppStore.getState().blockEdit;
  const editor = editorOpen();
  const field = editor?.querySelector("textarea") ?? null;
  const opened = session !== null && field !== null && document.activeElement === field;
  if (!session || !editor || !field || !opened) {
    await runCommand("text.cancelBlock");
    useAppStore.getState().setActiveTool("select");
    return { editable: true, opened: false, blocks: blocks.length, refusals };
  }
  const original = field.value;
  const rtl = field.dir === "rtl";
  const word = rtl ? "تعديل" : "edited";
  const before = await pageText();
  // Typed at the end, one keystroke at a time, each timed to the frame that
  // shows it (as the editor spike measured, ADR 0022).
  const drawn = (length: number) =>
    new Promise<void>((resolve) => {
      const done = () => editor.dataset.laidOutLength === String(length);
      if (done()) return resolve();
      const observer = new MutationObserver(() => {
        if (!done()) return;
        observer.disconnect();
        resolve();
      });
      observer.observe(editor, { attributes: true, attributeFilter: ["data-laid-out-length"] });
      setTimeout(() => {
        observer.disconnect();
        resolve();
      }, 5_000);
    });
  field.setSelectionRange(original.length, original.length);
  const latencies: number[] = [];
  for (const char of ` ${word}`) {
    const started = performance.now();
    document.execCommand("insertText", false, char);
    await drawn(field.value.length);
    await nextFrame();
    latencies.push(performance.now() - started);
  }
  const typed = field.value === `${original} ${word}`;
  const canvas = editor.querySelector("canvas");
  const pixels = canvas?.getContext("2d")?.getImageData(0, 0, canvas.width, canvas.height).data;
  let inked = 0;
  for (let i = 3; pixels && i < pixels.length; i += 4) if ((pixels[i] ?? 0) > 128) inked += 1;

  await runCommand("text.commitBlock");
  // The editor stays until the page shows the new text.
  const repainted = await waitFor(() => editorOpen() === null, 15_000);
  const after = await pageText();
  const readBack = !before.includes(word) && after.includes(word);
  // The rest of the block is still there.
  const kept = original
    .split(/\s+/u)
    .filter((part) => part.length > 2)
    .slice(0, 3)
    .every((part) => after.includes(part));
  await runCommand("edit.undo");
  const undone = !(await pageText()).includes(word);

  // Escape leaves the block as it was.
  await runCommand("text.editBlock", { page: 0, index, refusal: null });
  const again = editorOpen()?.querySelector("textarea");
  again?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  const cancelled =
    again !== null &&
    again !== undefined &&
    (await waitFor(
      () => useAppStore.getState().blockEdit === null && editorOpen() === null,
      5_000,
    ));
  useAppStore.getState().setActiveTool("select");
  const sorted = [...latencies].sort((a, b) => a - b);
  const pick = (q: number) =>
    Math.round(sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))] ?? 0);
  return {
    editable: true,
    opened,
    blocks: blocks.length,
    refusals,
    rtl,
    font: session.font,
    typed,
    inked,
    readBack,
    kept,
    repainted,
    undone,
    cancelled,
    keystrokes: latencies.length,
    latencyMedianMs: pick(0.5),
    latencyP95Ms: pick(0.95),
  };
}

async function notesAndInkChecks(documentId: number) {
  const viewer = getViewer(documentId);
  const scale = viewer?.snapshot().scale ?? 1;
  const enough = enoughAt(scale);
  const text = "راجع البند ٤ — check clause 4";
  // The note and the stroke go near the page's top: show it, so its tiles are on screen.
  if (viewer) {
    const shown = waitForPagePainted(0, 15_000);
    viewer.goToPage(0, 0);
    await Promise.race([shown, new Promise((resolve) => setTimeout(resolve, 2_000))]);
  }

  await runCommand("note.place", { page: 0, x: 24, y: 24 });
  await runCommand("note.save", { text });
  // The icon's yellow.
  const notePixels = await pixelsOnScreen(
    documentId,
    (r, g, b) => r > 220 && g > 180 && b < 80,
    enough,
  );
  const notePainted = notePixels > enough;
  const note = (await backend.pageAnnotations(documentId, 0)).find((a) => a.kind === "note");

  // A text box (ADR 0019): Arabic, shaped and drawn with embedded fonts.
  const boxText = "تمت المراجعة والموافقة";
  const dark = (r: number, g: number, b: number) => r < 90 && g < 90 && b < 90;
  // Counted against the page as it is before, every tile in place.
  await pageSettled(documentId);
  const darkBefore = countPixels(documentId, dark);
  await runCommand("textBox.place", { page: 0, x: 60, y: 24, width: 200 });
  await runCommand("note.save", { text: boxText });
  let boxPixels = 0;
  await waitFor(() => {
    boxPixels = countPixels(documentId, dark) - darkBefore;
    return boxPixels > enough;
  }, 15_000);
  const boxPainted = boxPixels > enough;
  const box = (await backend.pageAnnotations(documentId, 0)).find((a) => a.kind === "textBox");

  const stroke = [
    { x: 60, y: 60, pressure: 0.5 },
    { x: 140, y: 64, pressure: 0.8 },
    { x: 220, y: 60, pressure: 1 },
  ];
  await runCommand("ink.add", { page: 0, strokes: [stroke] });
  // The pen's dark blue.
  const inkPixels = await pixelsOnScreen(
    documentId,
    (r, g, b) => r < 70 && g < 110 && b > 110,
    enough,
  );
  const inkPainted = inkPixels > enough;
  const ink = (await backend.pageAnnotations(documentId, 0)).find((a) => a.kind === "ink");

  // The comments panel (ADR 0041): the three listed, a reply and a resolve
  // read back from the file, then both undone (two of the undos below).
  await runCommand("view.showComments");
  const replyText = "تم — done";
  if (note) await runCommand("comment.reply", { page: 0, index: note.index, text: replyText });
  if (note) await runCommand("comment.resolve", { page: 0, index: note.index, resolved: true });
  const listed = await backend.documentComments(documentId);
  const thread = listed.find((comment) => comment.kind === "note");
  await waitFor(() => document.querySelectorAll('[data-testid="comment"]').length >= 3, 5_000);
  const comments = {
    kinds: listed.map((comment) => comment.kind).sort(),
    replied: thread?.replies[0]?.contents === replyText,
    resolved: thread?.resolved ?? false,
    panelItems: document.querySelectorAll('[data-testid="comment"]').length,
    replyOnPage: (await backend.pageAnnotations(documentId, 0)).length,
  };
  await runCommand("view.showThumbnails");

  await runCommand("ink.erase", { page: 0, indices: ink ? [ink.index] : [] });
  const erased = !(await backend.pageAnnotations(documentId, 0)).some((a) => a.kind === "ink");
  // The erase, the resolve, the reply, the stroke, the text box, the note.
  for (let i = 0; i < 6; i += 1) await runCommand("edit.undo");
  const cleared = (await backend.pageAnnotations(documentId, 0)).length === 0;
  return {
    scale,
    enough,
    noteMatches: note?.contents === text,
    boxMatches: box?.contents === boxText,
    boxPainted,
    boxPixels,
    notePainted,
    notePixels,
    inkPainted,
    inkPixels,
    inkPoints: ink?.strokes[0]?.length ?? 0,
    erased,
    cleared,
    comments,
  };
}

/**
 * UI half of the developer self-test (see src-tauri/src/diagnostics.rs).
 * When the shell says a self-test is requested, wait for the startup
 * document's first page to be sharp on screen, then report what is actually
 * there: layout geometry, directions, and the page's pixels themselves.
 * With `SAFHA_SELFTEST_SPIKE=scroll`, run the scroll benchmark instead.
 */
export async function runSelftestIfRequested(): Promise<void> {
  const config = await backend.selftestConfig();
  if (!config) return;
  // Only the main window runs it; windows it opens itself must not.
  if (isTauri() && getCurrentWebviewWindow().label !== "main") return;

  const store = useAppStore.getState();
  store.setLocale(config.locale === "ar" ? "ar" : "en");
  store.setTheme(config.theme === "dark" ? "dark" : "light");

  void backend.selftestLog(`started (${document.visibilityState})`);
  const painted = await waitForPagePainted(0, 15_000);
  void backend.selftestLog(`first page painted: ${JSON.stringify(painted)}`);
  // Let layout settle after the paint (fit-width, fonts).
  await nextFrame();
  await nextFrame();

  if (config.spike === "tile-transport" && painted) {
    const { runTileTransportSpike } = await import("./spikes/tileTransport");
    await backend.selftestReport(await runTileTransportSpike(painted.documentId), null);
    return;
  }
  if (config.spike === "print" && painted) {
    // Print the open document. On Linux the shell resolves once GTK has
    // finished (with SAFHA_PRINT_EXPORT, into a PDF file the harness checks;
    // ADR 0013). On macOS and Windows it resolves once the print window opens,
    // so the harness waits for screenshots (ADR 0012).
    const started = performance.now();
    let error: string | null = null;
    try {
      await backend.printDocument(painted.documentId);
    } catch (caught) {
      error = JSON.stringify(caught);
    }
    const printMs = Math.round(performance.now() - started);
    void backend.selftestLog(`print_document returned after ${printMs} ms`);
    if (!navigator.userAgent.includes("Linux")) {
      await new Promise((resolve) => window.setTimeout(resolve, 12_000));
    }
    await backend.selftestReport({ spike: "print", error, printMs }, null);
    return;
  }
  if (config.spike?.startsWith("tile-memory:") && painted) {
    const { runTileMemorySpike } = await import("./spikes/tileMemory");
    const variant = config.spike.slice("tile-memory:".length);
    try {
      await backend.selftestReport(await runTileMemorySpike(painted.documentId, variant), null);
    } catch (error) {
      void backend.selftestLog(`tile-memory failed: ${String(error)}`);
      await backend.selftestReport({ spike: "tile-memory", variant, error: String(error) }, null);
    }
    return;
  }
  if (config.spike === "editor-surface") {
    const { runEditorSurfaceSpike } = await import("./spikes/editorSurface");
    try {
      await backend.selftestReport(await runEditorSurfaceSpike(), null);
    } catch (error) {
      void backend.selftestLog(`editor-surface failed: ${String(error)}`);
      await backend.selftestReport({ spike: "editor-surface", error: String(error) }, null);
    }
    return;
  }
  if (config.spike === "scroll" && painted) {
    const { runScrollBenchmark } = await import("./benchmarks/scroll");
    await backend.selftestReport(await runScrollBenchmark(painted), null);
    return;
  }

  const rect = (testId: string) => {
    const box = document.querySelector(`[data-testid="${testId}"]`)?.getBoundingClientRect();
    return box ? { x: box.x, y: box.y, width: box.width, height: box.height } : null;
  };
  const state = useAppStore.getState();
  const composite = painted ? compositePage(painted.documentId, painted) : null;
  const viewerElement = document.querySelector('[data-testid="viewer"]');

  const results = {
    painted,
    locale: state.locale,
    dir: document.documentElement.dir,
    viewerDirection: viewerElement ? getComputedStyle(viewerElement).direction : null,
    visibility: document.visibilityState,
    theme: document.documentElement.dataset.theme,
    documents: state.documents.map(({ name, pageCount, zoom, pageSize, viewMode, pageOrder }) => ({
      name,
      pageCount,
      zoom,
      pageSize,
      viewMode,
      pageOrder,
    })),
    errorCode: state.errorCode,
    window: { width: window.innerWidth, height: window.innerHeight, dpr: window.devicePixelRatio },
    toolRail: rect("tool-rail"),
    inspector: rect("inspector"),
    viewerCss: rect("viewer"),
    pageCss: rect("page-0"),
    canvasPixels: composite
      ? { width: composite.canvas.width, height: composite.canvas.height }
      : null,
    tiles: composite?.tiles ?? 0,
    inkRatio: composite?.inkRatio ?? null,
    viewer: painted ? getViewer(painted.documentId)?.snapshot() : null,
    renderStats: await backend.renderStats(),
    uiReadyMs: performance.now(),
    fontsLoaded: [...document.fonts]
      .filter((font) => font.status === "loaded")
      .map((f) => f.family),
  };

  const canvasPng = composite ? composite.canvas.toDataURL("image/png") : null;
  const navigation = painted ? await navigationChecks(painted.documentId) : null;
  await backend.selftestReport({ ...results, navigation }, canvasPng);
}
