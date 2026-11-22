/**
 * Scroll and zoom benchmark (spec section 11: 60 fps, no page blank for more
 * than 100 ms, memory bounded under 500 MB on a 1,000-page document).
 *
 * Runs inside the real webview of a `devtools` build: `scripts/scroll-benchmark.sh`
 * launches the app with `SAFHA_SELFTEST_SPIKE=scroll` on the 1,000-page fixture,
 * and `scripts/check-scroll-benchmark.mjs` asserts the budgets on the report.
 *
 * Scrolling is driven by setting `scrollTop` once per animation frame at a
 * given speed; zooming by dispatching Ctrl+wheel events, which is what a
 * trackpad pinch delivers in WKWebView. Each frame records the gap since the
 * previous one and, for every page intersecting the viewport, whether anything
 * (low-resolution preview or sharp tiles) is drawn on it.
 */
import { backend, type RenderStats } from "../backend";
import { runCommand } from "../commands/registry";
import { waitForPagePainted } from "../selftest";
import { useAppStore } from "../state/appStore";
import { emptyProfile, type ViewerController, type ViewerProfile } from "../viewer/controller";
import {
  PAGE_PREVIEW_EVENT,
  type PagePaintedDetail,
  type PagePreviewDetail,
} from "../viewer/events";
import { getViewer } from "../viewer/registry";

const round = (value: number) => Math.round(value * 100) / 100;

function quantile(values: number[], q: number): number {
  if (values.length === 0) return Number.NaN;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))] ?? Number.NaN;
}

/**
 * Main-thread time of each frame: from its animation-frame callback to the
 * first task after it, which runs once the rendering update (the step's own
 * work, style, layout, paint, and the commit to the compositor) is done.
 */
const afterRender = new MessageChannel();
let renderStartedAt = 0;
let onRendered: ((ms: number) => void) | null = null;
afterRender.port1.onmessage = () => {
  onRendered?.(performance.now() - renderStartedAt);
  onRendered = null;
};

/** Next animation frame's timestamp; a timer stands in if the window gets no frames. */
function frame(rendered?: (ms: number) => void): Promise<{ time: number; fallback: boolean }> {
  return new Promise((resolve) => {
    const timer = window.setTimeout(
      () => resolve({ time: performance.now(), fallback: true }),
      250,
    );
    requestAnimationFrame((time) => {
      window.clearTimeout(timer);
      if (rendered) {
        renderStartedAt = performance.now();
        onRendered = rendered;
        afterRender.port2.postMessage(null);
      }
      resolve({ time, fallback: false });
    });
  });
}

/** One frame of a traced phase. */
interface FrameRecord {
  /** The frame's timestamp, relative to the phase's first frame. */
  at: number;
  /** Interval since the previous frame. */
  gap: number;
  /** Main-thread time of the frame (see `afterRender`); null if never measured. */
  workMs: number | null;
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/**
 * Phases that would start after this long are skipped, so a run always ends
 * with a report: a hidden webview (locked screen, occluded window) throttles
 * timers as well as frames and would otherwise outlast the harness's timeout.
 */
const DEADLINE_MS = 120_000;

interface PhaseResult {
  name: string;
  /** Where the viewer's own time went during the phase. */
  profile: ViewerProfile;
  memoryAfter: { shellRssMb: number | null; webviewRssMb: number | null };
  durationMs: number;
  frames: number;
  /** Frames delivered by the timer fallback: if any, frame statistics are invalid. */
  fallbackFrames: number;
  medianFrameMs: number;
  p95FrameMs: number;
  longestFrameMs: number;
  framesOver20Ms: number;
  /**
   * Frames that took over 25 ms: at 60 Hz (16.7 ms) at least one vsync was
   * missed. WebKit's frame timestamps jitter by 1–2 ms even on an idle page,
   * so this, not a p95 a hair above 16.7 ms, is what a reader sees as a stutter.
   */
  droppedFrames: number;
  /** Longest stretch any page spent on screen with nothing drawn on it. */
  maxBlankMs: number;
  blankEpisodes: number;
  blankEpisodesOver100Ms: number;
  /** Pages that came on screen during the phase. */
  pagesSeen: number;
  scrolledPx: number;
  startPage: number;
  endPage: number;
  zoomStart: number;
  zoomEnd: number;
  /** Traced phases (zoom gestures): every frame, and the viewer's steps relative to the first. */
  frameTrace?: FrameRecord[];
}

/** Per-frame bookkeeping of frame gaps and blank pages. */
class Monitor {
  private readonly gaps: number[] = [];
  private last: number | null = null;
  private fallbacks = 0;
  private readonly blankSince = new Map<number, number>();
  private readonly blanks: number[] = [];
  private readonly seen = new Set<number>();

  constructor(private readonly viewer: ViewerController) {}

  sample(time: number, fallback: boolean): void {
    if (this.last !== null) this.gaps.push(time - this.last);
    this.last = time;
    if (fallback) this.fallbacks += 1;
    const visible = this.viewer.snapshot().visible;
    const onScreen = new Set<number>();
    for (const { page, preview, sharp } of visible) {
      onScreen.add(page);
      this.seen.add(page);
      const painted = preview || sharp;
      const since = this.blankSince.get(page);
      if (!painted && since === undefined) this.blankSince.set(page, time);
      if (painted && since !== undefined) {
        this.blanks.push(time - since);
        this.blankSince.delete(page);
      }
    }
    // A page that scrolled away while still blank ends its episode there.
    for (const [page, since] of this.blankSince) {
      if (!onScreen.has(page)) {
        this.blanks.push(time - since);
        this.blankSince.delete(page);
      }
    }
  }

  finish(time: number) {
    for (const since of this.blankSince.values()) this.blanks.push(time - since);
    this.blankSince.clear();
    return {
      frames: this.gaps.length,
      fallbackFrames: this.fallbacks,
      medianFrameMs: round(quantile(this.gaps, 0.5)),
      p95FrameMs: round(quantile(this.gaps, 0.95)),
      longestFrameMs: round(Math.max(0, ...this.gaps)),
      framesOver20Ms: this.gaps.filter((gap) => gap > 20).length,
      droppedFrames: this.gaps.filter((gap) => gap > 25).length,
      maxBlankMs: round(Math.max(0, ...this.blanks)),
      blankEpisodes: this.blanks.length,
      blankEpisodesOver100Ms: this.blanks.filter((ms) => ms > 100).length,
      pagesSeen: this.seen.size,
    };
  }
}

/**
 * `zoomOnly`: just the zoom gestures, three times over, with no random access,
 * scrolling, or long idle: a quick loop for work on zooming. Its report is not
 * a budget check.
 */
export async function runScrollBenchmark(first: PagePaintedDetail, { zoomOnly = false } = {}) {
  const viewer = getViewer(first.documentId);
  const container = document.querySelector<HTMLElement>('[data-testid="viewer"]');
  if (!viewer || !container) throw new Error("no viewer is mounted");

  const megabytes = (kb: number | null) => (kb === null ? null : round(kb / 1024));
  const memory = async () => ({
    shellRssMb: megabytes(await backend.shellRssKb()),
    webviewRssMb: megabytes(await backend.webviewRssKb()),
  });
  const statsBefore = await backend.renderStats();
  const memoryAtStart = await memory();
  const phases: PhaseResult[] = [];
  const skipped: string[] = [];
  const startedAt = performance.now();

  /**
   * Runs `step` once per frame until it returns false or `maxMs` passes.
   * `step` gets the time since the phase began and since the last frame.
   */
  const phase = async (
    name: string,
    maxMs: number,
    step: (elapsed: number, delta: number) => boolean | undefined,
    trace = false,
  ) => {
    const since = Math.round(performance.now() - startedAt);
    // A hidden webview gets no frames and throttles timers ever harder:
    // frame-driven phases would measure nothing and might never finish.
    if (since > DEADLINE_MS || document.visibilityState !== "visible") {
      skipped.push(name);
      void backend.selftestLog(`skipping "${name}" at ${since} ms (${document.visibilityState})`);
      return;
    }
    void backend.selftestLog(`phase "${name}" at ${since} ms (${document.visibilityState})`);
    const monitor = new Monitor(viewer);
    viewer.profile = emptyProfile(trace);
    const startTop = container.scrollTop;
    const startPage = viewer.snapshot().currentPage;
    const zoomStart = viewer.snapshot().zoom;
    let scrolled = 0;
    let previousTop = startTop;
    const frames: FrameRecord[] = [];
    const record = (time: number, gap: number) => {
      const entry: FrameRecord = { at: time, gap: round(gap), workMs: null };
      frames.push(entry);
      return (ms: number) => {
        entry.workMs = round(ms);
      };
    };
    const begin = await frame(trace ? record(0, 0) : undefined);
    let last = begin.time;
    monitor.sample(begin.time, begin.fallback);
    for (;;) {
      // The callback fills in the frame's record once its rendering is done.
      let fill: ((ms: number) => void) | undefined;
      const { time, fallback } = await frame(trace ? (ms) => fill?.(ms) : undefined);
      if (trace) fill = record(time - begin.time, time - last);
      monitor.sample(time, fallback);
      scrolled += Math.abs(container.scrollTop - previousTop);
      previousTop = container.scrollTop;
      const elapsed = time - begin.time;
      if (elapsed >= maxMs || step(elapsed, time - last) === false) {
        last = time;
        break;
      }
      last = time;
    }
    // The last frame's rendering finishes before the next task.
    if (trace) await frame();
    const snapshot = viewer.snapshot();
    const profile = viewer.profile ?? emptyProfile();
    for (const event of profile.trace ?? []) {
      event.at = round(event.at - begin.time);
      if (event.ms !== undefined) event.ms = round(event.ms);
    }
    phases.push({
      name,
      durationMs: round(last - begin.time),
      ...monitor.finish(last),
      scrolledPx: Math.round(scrolled),
      startPage,
      endPage: snapshot.currentPage,
      zoomStart: round(zoomStart),
      zoomEnd: round(snapshot.zoom),
      profile,
      ...(trace ? { frameTrace: frames } : {}),
      // Sampled after the phase's last frame, so it does not disturb it.
      memoryAfter: await memory(),
    });
    viewer.profile = null;
  };

  /** Scrolls at `speed` CSS px/s (a function of elapsed ms) for `ms`. */
  const scroll = (name: string, ms: number, speed: (elapsed: number) => number) =>
    phase(name, ms, (elapsed, delta) => {
      container.scrollTop += (speed(elapsed) * Math.min(delta, 50)) / 1000;
      return container.scrollTop + container.clientHeight < container.scrollHeight - 1;
    });

  /** Dispatches one Ctrl+wheel (pinch) event per frame at the viewport centre. */
  const pinch = (name: string, frames: number, deltaY: number) => {
    const bounds = container.getBoundingClientRect();
    let count = 0;
    return phase(
      name,
      frames * 50,
      () => {
        container.dispatchEvent(
          new WheelEvent("wheel", {
            deltaY,
            ctrlKey: true,
            clientX: bounds.left + bounds.width / 2,
            clientY: bounds.top + bounds.height / 2,
            bubbles: true,
            cancelable: true,
          }),
        );
        count += 1;
        return count < frames;
      },
      true,
    );
  };

  const idle = (name: string, ms: number, trace = false) => phase(name, ms, () => true, trace);

  // Random access, timed by the viewer's own events rather than by frames, so
  // it is valid even when the window is hidden: jump to 40 pages spread over
  // the document (none prefetched) and time jump -> preview drawn (how long
  // the page is blank, before the next frame shows it) and jump -> sharp.
  const pageCount =
    useAppStore.getState().documents.find((open) => open.id === first.documentId)?.pageCount ?? 1;
  const jumps: { page: number; previewMs: number | null; sharpMs: number | null }[] = [];
  // Every render call during the jumps, to tell an IPC stall from a UI stall.
  const calls: {
    page: number;
    w: number;
    sentAt: number;
    doneAt: number;
    micros: number | null;
  }[] = [];
  const renderTile = backend.renderTile;
  backend.renderTile = async (documentId, generation, page, tile, priority, lane) => {
    const sentAt = performance.now();
    const result = await renderTile(documentId, generation, page, tile, priority, lane);
    calls.push({
      page,
      w: tile.width,
      sentAt,
      doneAt: performance.now(),
      micros: result.renderMicros,
    });
    return result;
  };
  const slowJumps: unknown[] = [];
  for (let i = 1; i <= (zoomOnly ? 0 : 40); i += 1) {
    const page = (i * 397) % pageCount;
    const jumpedAt = performance.now();
    calls.length = 0;
    let previewMs: number | null = null;
    const onPreview = (event: Event) => {
      const detail = (event as CustomEvent<PagePreviewDetail>).detail;
      if (detail.page === page && previewMs === null) previewMs = performance.now() - jumpedAt;
    };
    window.addEventListener(PAGE_PREVIEW_EVENT, onPreview);
    const sharp = waitForPagePainted(page, 5000);
    viewer.goToPage(page);
    const painted = await sharp;
    window.removeEventListener(PAGE_PREVIEW_EVENT, onPreview);
    jumps.push({
      page,
      previewMs: previewMs === null ? null : round(previewMs),
      sharpMs: painted ? round(performance.now() - jumpedAt) : null,
    });
    if ((previewMs ?? Number.POSITIVE_INFINITY) > 100) {
      slowJumps.push({
        page,
        previewMs,
        calls: calls.map((call) => ({
          page: call.page,
          w: call.w,
          sent: round(call.sentAt - jumpedAt),
          done: round(call.doneAt - jumpedAt),
          renderMs: call.micros === null ? null : round(call.micros / 1000),
        })),
      });
    }
  }
  backend.renderTile = renderTile;
  const memoryAfterJumps = await memory();
  void backend.selftestLog(`random access done: ${JSON.stringify(jumps.slice(0, 5))}`);
  viewer.goToPage(0);

  await idle("settle", 500);
  if (zoomOnly) {
    for (let round = 1; round <= 3; round += 1) {
      await pinch(`pinch in ${round}`, 45, -4);
      await idle(`after pinch in ${round}`, 400, true);
      await scroll(`scroll zoomed in ${round}`, 1000, () => 3000);
      await pinch(`pinch out ${round}`, 75, 4);
      await idle(`after pinch out ${round}`, 400, true);
      await runCommand("view.fitWidth");
      await idle(`fit width ${round}`, 300);
    }
  }
  if (!zoomOnly) {
    await scroll("scroll 4,000 px/s", 8000, () => 4000);
    await scroll("scroll 12,000 px/s", 4000, () => 12_000);
    // A fling: 30,000 px/s decaying with a 600 ms time constant.
    await scroll("fling", 2500, (elapsed) => 30_000 * Math.exp(-elapsed / 600));
    await idle("after fling", 300);

    // Scrollbar drags: land in the middle of nowhere, nothing prefetched.
    for (const page of [500, 900, 250]) {
      await phase(`jump to page ${page + 1}`, 400, (elapsed) => {
        if (elapsed < 20) viewer.goToPage(page);
        return true;
      });
    }

    await pinch("pinch in", 45, -4);
    await idle("after pinch in", 400, true);
    await scroll("scroll zoomed in 3,000 px/s", 2000, () => 3000);
    await pinch("pinch out", 75, 4);
    await idle("after pinch out", 400, true);
    await scroll("scroll zoomed out 6,000 px/s", 2000, () => 6000);
    await runCommand("view.fitWidth");
    await idle("fit width", 500);
  }

  const memoryAtEnd = await memory();
  await sleep(2000);
  const memoryAfterIdle = await memory();
  // WebKit frees garbage and returns memory to the system lazily; a longer
  // idle separates retained memory from not-yet-reclaimed memory.
  if (!zoomOnly) await sleep(13_000);
  const memoryAfterIdle15s = await memory();
  const statsAfter = await backend.renderStats();
  const snapshot = viewer.snapshot();

  const scrollPhases = phases.filter((p) => !p.name.startsWith("jump"));
  return {
    spike: "scroll",
    userAgent: navigator.userAgent,
    // Frame numbers only mean something when the window was on screen.
    visibility: document.visibilityState,
    devicePixelRatio: window.devicePixelRatio,
    viewport: { width: container.clientWidth, height: container.clientHeight },
    pageCount:
      useAppStore.getState().documents.find((open) => open.id === first.documentId)?.pageCount ??
      null,
    firstPage: first,
    phases,
    /** Phases not run because the deadline passed (a hidden, throttled webview). */
    skipped,
    summary: {
      longestFrameMs: Math.max(...scrollPhases.map((p) => p.longestFrameMs)),
      framesOver20Ms: scrollPhases.reduce((sum, p) => sum + p.framesOver20Ms, 0),
      droppedFrames: scrollPhases.reduce((sum, p) => sum + p.droppedFrames, 0),
      frames: scrollPhases.reduce((sum, p) => sum + p.frames, 0),
      worstP95FrameMs: Math.max(...scrollPhases.map((p) => p.p95FrameMs)),
      maxBlankMs: Math.max(...phases.map((p) => p.maxBlankMs)),
      blankEpisodesOver100Ms: phases.reduce((sum, p) => sum + p.blankEpisodesOver100Ms, 0),
      fallbackFrames: phases.reduce((sum, p) => sum + p.fallbackFrames, 0),
      pagesSeen: phases.reduce((sum, p) => sum + p.pagesSeen, 0),
    },
    randomAccess: {
      jumps,
      /** Render calls around jumps whose preview took over 100 ms. */
      slowJumps,
      maxPreviewMs: Math.max(...jumps.map((jump) => jump.previewMs ?? Number.POSITIVE_INFINITY)),
      p95PreviewMs: quantile(
        jumps.map((jump) => jump.previewMs ?? Number.POSITIVE_INFINITY),
        0.95,
      ),
      medianSharpMs: quantile(
        jumps.map((jump) => jump.sharpMs ?? Number.POSITIVE_INFINITY),
        0.5,
      ),
      maxSharpMs: Math.max(...jumps.map((jump) => jump.sharpMs ?? Number.POSITIVE_INFINITY)),
    },
    memory: {
      atStart: memoryAtStart,
      afterJumps: memoryAfterJumps,
      atEnd: memoryAtEnd,
      afterIdle2s: memoryAfterIdle,
      afterIdle15s: memoryAfterIdle15s,
    },
    renderStats: {
      before: statsBefore,
      after: statsAfter,
      delta: diffStats(statsBefore, statsAfter),
    },
    viewerAtEnd: snapshot,
  };
}

function diffStats(before: RenderStats | null, after: RenderStats | null) {
  if (!before || !after) return null;
  const delta: Record<string, number> = {};
  for (const key of Object.keys(after) as (keyof RenderStats)[]) {
    delta[key] = after[key] - before[key];
  }
  return delta;
}
