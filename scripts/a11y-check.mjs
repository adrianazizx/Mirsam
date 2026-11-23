#!/usr/bin/env node
// Accessibility check in a real browser: runs axe-core over the app's main
// screens in headless Chrome, in English and Arabic, light and dark. Unlike the
// jsdom unit tests, the page is really laid out and painted, so colour contrast
// is checked too.
//
// The UI runs on Vite's dev server with the mock backend (as `pnpm dev`), and
// the script drives it from inside the page through the app's own store and
// command registry. Screens, per combination: the empty window; the viewer
// with a document (and its sepia and night reading themes); the left panels;
// the search panel and replace preview; the tools; the OCR tool's states (a
// job, the review list, review mode, an area, layer checks, the model prompt);
// the dialogs; Settings' OCR models, crash reports and the certificate dialog's
// time-stamp field. Commands that are disabled are reported as skipped.
// If the store has `setUiScale`, the viewer and Settings are also checked at
// interface size 2, where clipped text counts as a failure as well.
//
// Motion is reduced (no theme fades mid-measurement), and a colour-contrast
// failure counts only if it is still there a second later. axe's colour-contrast
// rule is made to check Arabic text too (see checkArabicContrast).
//
// Exit status: 0 clean, 1 violations or clipped text, 2 the check itself failed.
//
// Usage: node scripts/a11y-check.mjs [--out <dir>] [--only en-light,ar-dark]
//   --out   where report.json goes (default: a new temporary directory)
//   --only  combinations to check, of en-light, en-dark, ar-light, ar-dark
// Needs Chrome (set CHROME_BIN if it is not found) and `pnpm install` in apps/desktop.

import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { createServer as createNetServer } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { findChrome } from "../fixtures/lib.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = path.join(ROOT, "apps", "desktop");
const requireApp = createRequire(path.join(APP, "package.json"));

const COMBOS = ["en-light", "en-dark", "ar-light", "ar-dark"];
/** Combinations that also get the interface-size pass, when the app has one. */
const UI_SCALE_COMBOS = ["en-light", "ar-dark"];
const VIEWPORT = { width: 1280, height: 820 };
/** Wait after each state change, before axe runs. */
const SETTLE_MS = 400;
/** A colour-contrast failure must still be there this long after the first run. */
const CONTRAST_RETRY_MS = 1000;

// The search panel is checked with its replace row and preview (below).
const PANELS = [
  "view.showThumbnails",
  "view.showOutline",
  "view.showComments",
  "view.showSignatures",
];
// The OCR tool is checked with its jobs, review mode and areas (checkOcr, below).
const TOOLS = ["edit", "comment", "pages", "forms", "sign", "protect"];
const DIALOGS = [
  "app.settings",
  "app.commandPalette",
  "pages.split",
  "security.open",
  "certSign.open",
  "signature.new",
  "pages.watermark",
  "file.compress",
  "file.exportImages",
  "app.toggleGallery",
  "app.shortcuts",
];
/** Store fields that close every dialog above (the certificate dialog closes by command). */
const CLOSED_DIALOGS = {
  settingsOpen: false,
  paletteOpen: false,
  splitPrompt: false,
  securityPrompt: false,
  outputDialog: null,
  galleryOpen: false,
  signatureDialog: null,
  shortcutsOpen: false,
};

// Known, justified exclusions: a violation of `rule` on an element matching
// `selector` is reported as excluded, not as a failure. Keep each one argued.
const EXCLUSIONS = [
  {
    rule: "scrollable-region-focusable",
    selector: '[data-testid="palette-list"]',
    reason:
      "The command palette's list is a combobox listbox with virtual focus: focus stays in " +
      "the search field and the arrow keys move through (and scroll) the list.",
  },
];

// ------------------------------------------------------------------ arguments

const args = process.argv.slice(2);
if (args.includes("--help") || args.includes("-h")) {
  console.log("Usage: node scripts/a11y-check.mjs [--out <dir>] [--only en-light,ar-dark]");
  process.exit(0);
}
const flag = (name) => {
  const index = args.indexOf(name);
  return index === -1 ? null : args[index + 1];
};
const only =
  flag("--only")
    ?.split(",")
    .map((combo) => combo.trim()) ?? COMBOS;
const unknown = only.filter((combo) => !COMBOS.includes(combo));
if (unknown.length > 0 || only.length === 0) {
  console.error(`--only takes a list of ${COMBOS.join(", ")}; got ${flag("--only")}`);
  process.exit(2);
}
const outDir = flag("--out")
  ? path.resolve(flag("--out"))
  : mkdtempSync(path.join(tmpdir(), "mirsam-a11y-"));

// ------------------------------------------------------------------- clean-up

const cleanups = [];
let cleaning = null;
/** Undoes everything started, last first; once, however many times it is called. */
function cleanUp() {
  cleaning ??= (async () => {
    while (cleanups.length > 0) {
      try {
        await cleanups.pop()();
      } catch (error) {
        console.error(`clean-up: ${error.message}`);
      }
    }
  })();
  return cleaning;
}
let interrupted = false;
for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => {
    interrupted = true;
    console.error(`\n${signal}: stopping`);
    cleanUp().finally(() => process.exit(130));
  });
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// ------------------------------------------------------------------ dev server

function freePort() {
  return new Promise((resolve, reject) => {
    const server = createNetServer();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => resolve(port));
    });
  });
}

/** Vite's dev server for the UI, from the app's own Vite and config, without file watching. */
async function startVite(work) {
  const vite = await import(pathToFileURL(requireApp.resolve("vite")).href);
  // vite.config.ts resolves paths from the working directory.
  process.chdir(APP);
  const server = await vite.createServer({
    configFile: path.join(APP, "vite.config.ts"),
    root: APP,
    logLevel: "warn",
    // Its own dependency cache, so a `pnpm dev` running alongside is not disturbed.
    cacheDir: path.join(work, "vite"),
    // The page's console goes to the report, not to this terminal.
    server: { port: await freePort(), strictPort: false, hmr: false, forwardConsole: false },
    plugins: [
      {
        // No file watching: the checkout may be cloud-synced (OneDrive touches
        // file times), and a reload mid-run would check a half-loaded page.
        // Set here because an inline `watch: null` does not override the config
        // file's `watch` (Vite's merge skips null values).
        name: "a11y-check:no-watch",
        enforce: "post",
        config(config) {
          config.server ??= {};
          config.server.watch = null;
        },
      },
    ],
  });
  cleanups.push(() => server.close());
  await server.listen();
  const url = server.resolvedUrls?.local[0];
  if (!url) throw new Error("Vite started without a local URL");
  return url;
}

// ---------------------------------------------------------------------- Chrome

async function startChrome(work) {
  const chrome = findChrome();
  if (!chrome) throw new Error("Chrome/Chromium not found (set CHROME_BIN)");
  const profile = path.join(work, "chrome");
  const child = spawn(
    chrome,
    [
      "--headless=new",
      "--disable-gpu",
      // CI containers run as root, where Chrome refuses its sandbox (as in fixtures/generate.mjs).
      ...(process.env.CI || process.getuid?.() === 0 ? ["--no-sandbox"] : []),
      `--user-data-dir=${profile}`,
      "--remote-debugging-port=0",
      `--window-size=${VIEWPORT.width},${VIEWPORT.height}`,
      "--no-first-run",
      "--no-default-browser-check",
      "about:blank",
    ],
    // Its own process group (not on Windows), so stopping it stops its helpers too.
    { stdio: ["ignore", "ignore", "pipe"], detached: process.platform !== "win32" },
  );
  const stop = (signal) => {
    try {
      if (process.platform === "win32") child.kill(signal);
      else process.kill(-child.pid, signal);
    } catch {
      // Already gone.
    }
  };
  let stderr = "";
  child.stderr.on("data", (chunk) => {
    stderr = (stderr + chunk).slice(-4000);
  });
  let exited = false;
  child.on("exit", () => {
    exited = true;
  });
  cleanups.push(async () => {
    stop("SIGTERM");
    for (let waited = 0; !exited && waited < 5000; waited += 100) await sleep(100);
    stop("SIGKILL");
  });

  // Chrome writes the port it picked to <profile>/DevToolsActivePort.
  const portFile = path.join(profile, "DevToolsActivePort");
  const deadline = Date.now() + 30_000;
  let port = null;
  while (!port) {
    if (exited) throw new Error(`Chrome exited at start:\n${stderr}`);
    if (Date.now() > deadline) throw new Error(`Chrome did not open its DevTools port:\n${stderr}`);
    if (existsSync(portFile)) port = Number(readFileSync(portFile, "utf8").split("\n")[0]) || null;
    if (!port) await sleep(100);
  }
  const base = `http://127.0.0.1:${port}`;
  const version = (await (await fetch(`${base}/json/version`)).json()).Browser;
  const targets = await (await fetch(`${base}/json/list`)).json();
  const target = targets.find((candidate) => candidate.type === "page");
  if (!target) throw new Error("Chrome has no page to drive");
  return { version, wsUrl: target.webSocketDebuggerUrl };
}

// ------------------------------------------------------- DevTools Protocol client

/** A minimal Chrome DevTools Protocol client: `send` awaits the reply, `once` the next event. */
async function connect(wsUrl) {
  const ws = new WebSocket(wsUrl);
  await new Promise((resolve, reject) => {
    ws.onopen = resolve;
    ws.onerror = () => reject(new Error(`cannot connect to ${wsUrl}`));
  });
  let nextId = 1;
  const pending = new Map();
  const listeners = new Map();
  ws.onmessage = (event) => {
    const message = JSON.parse(event.data);
    if (message.id !== undefined) {
      const call = pending.get(message.id);
      if (!call) return;
      pending.delete(message.id);
      clearTimeout(call.timer);
      if (message.error) call.reject(new Error(`${call.method}: ${message.error.message}`));
      else call.resolve(message.result);
    } else {
      for (const listener of listeners.get(message.method) ?? []) listener(message.params);
    }
  };
  ws.onclose = () => {
    for (const call of pending.values()) {
      clearTimeout(call.timer);
      call.reject(new Error(`${call.method}: the DevTools connection closed`));
    }
    pending.clear();
  };
  cleanups.push(() => ws.close());

  const on = (method, listener) => {
    if (!listeners.has(method)) listeners.set(method, new Set());
    listeners.get(method).add(listener);
    return () => listeners.get(method).delete(listener);
  };
  return {
    send(method, params = {}, timeoutMs = 120_000) {
      const id = nextId++;
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          pending.delete(id);
          reject(new Error(`${method} timed out after ${timeoutMs} ms`));
        }, timeoutMs);
        pending.set(id, { resolve, reject, method, timer });
        ws.send(JSON.stringify({ id, method, params }));
      });
    },
    on,
    once(method, timeoutMs = 60_000) {
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          off();
          reject(new Error(`no ${method} within ${timeoutMs} ms`));
        }, timeoutMs);
        const off = on(method, (params) => {
          clearTimeout(timer);
          off();
          resolve(params);
        });
      });
    },
  };
}

async function evaluate(cdp, expression) {
  const { result, exceptionDetails } = await cdp.send("Runtime.evaluate", {
    expression,
    awaitPromise: true,
    returnByValue: true,
  });
  if (exceptionDetails) {
    throw new Error(
      `in the page: ${exceptionDetails.exception?.description ?? exceptionDetails.text}`,
    );
  }
  return result.value;
}

/** Calls `fn` (which must be self-contained) in the page with a JSON argument. */
const call = (cdp, fn, arg = null) => evaluate(cdp, `(${fn})(${JSON.stringify(arg)})`);

// --------------------------------------------------------- functions run in the page

/** Waits for the app's first render; says whether the store has an interface size. */
async function pageReady() {
  const deadline = Date.now() + 30_000;
  while (!document.getElementById("root")?.firstElementChild) {
    if (Date.now() > deadline) throw new Error("the app did not render within 30 s");
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  const { useAppStore } = await import("/src/state/appStore.ts");
  return { hasUiScale: typeof useAppStore.getState().setUiScale === "function" };
}

/**
 * Makes axe check the contrast of Arabic text. axe skips text it takes for an
 * icon-font ligature: it draws the first letter alone, then the whole text
 * clipped to that width, and calls it a ligature when the two differ. Arabic
 * letters change shape when joined, so most Arabic text would go unchecked.
 * The app has no icon fonts (its icons are SVG), so before axe decides whether
 * to check an element, its font is recorded as a text font (axe's own cache of
 * that decision, `fonts[family]`, read by its isIconLigature).
 */
function checkArabicContrast() {
  const rule = axe._audit?.rules.find((candidate) => candidate.id === "color-contrast");
  if (!rule || typeof axe._cache?.get !== "function") {
    throw new Error("axe internals changed: revisit checkArabicContrast in scripts/a11y-check.mjs");
  }
  const matches = rule.matches;
  rule.matches = (node, virtualNode, context) => {
    const fonts = axe._cache.get("fonts", () => ({}));
    fonts[getComputedStyle(node).getPropertyValue("font-family")] = {
      occurrences: 3,
      numLigatures: 0,
    };
    return matches(node, virtualNode, context);
  };
}

/** Lets fonts load, React commit, and the page paint, then waits `ms`. */
async function settle(ms) {
  await document.fonts.ready;
  await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  await new Promise((resolve) => setTimeout(resolve, ms));
}

/** One state change, through the app's store and command registry. */
async function act(step) {
  const { useAppStore } = await import("/src/state/appStore.ts");
  const R = await import("/src/commands/registry.ts");
  const state = useAppStore.getState();
  switch (step.kind) {
    case "combo":
      state.setLocale(step.locale);
      state.setTheme(step.theme);
      return {};
    case "command":
      if (!R.isCommandEnabled(step.id)) return { skipped: "command disabled" };
      await R.runCommand(step.id);
      return {};
    case "open": {
      await R.runCommand("file.open");
      const deadline = Date.now() + 15_000;
      while (useAppStore.getState().documents.length === 0) {
        if (Date.now() > deadline) throw new Error("file.open opened no document");
        await new Promise((resolve) => setTimeout(resolve, 50));
      }
      return {};
    }
    case "state":
      useAppStore.setState(step.state);
      return {};
    case "tool":
      state.setActiveTool(step.tool);
      return {};
    case "readingTheme":
      state.setReadingTheme(step.theme);
      return {};
    case "replacePreview":
      // The search panel with its replace row, then the replace preview (ADR 0052).
      await R.runCommand("search.replaceOpen");
      await R.runCommand("search.run", { query: step.query });
      await R.runCommand("search.setReplacement", { text: step.replacement });
      if (step.preview) await R.runCommand("search.replaceAll");
      return {};
    case "replaceClose":
      await R.runCommand("search.replaceCancel");
      await R.runCommand("search.replaceClose");
      useAppStore.setState({ leftPanel: null });
      return {};
    case "uiScale":
      state.setUiScale(step.scale);
      return {};
    case "ocr":
      // Installed by loadApp: this function is sent to the page alone.
      return window.__a11yOcrStep(step, R);
    case "click": {
      const element = document.querySelector(step.selector);
      if (!element) throw new Error(`nothing matches ${step.selector}`);
      element.click();
      return {};
    }
    default:
      throw new Error(`unknown step ${step.kind}`);
  }
}

/**
 * The OCR tool's and Settings' states (ADR 0070's second pass), set up through
 * the mock shell's own test hooks, which the dev server serves to the page.
 */
async function ocrStep(step, R) {
  const { useAppStore } = await import("/src/state/appStore.ts");
  const { useOcrJobs } = await import("/src/state/ocrJobs.ts");
  const { useOcrModels } = await import("/src/state/ocrModels.ts");
  const { resetMockOcr } = await import("/src/backend/mockOcr.ts");
  const waitFor = async (what, test, ms = 15_000) => {
    const deadline = Date.now() + ms;
    while (!test()) {
      if (Date.now() > deadline) throw new Error(`OCR: ${what} did not happen within ${ms} ms`);
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
  };
  const installed = { kind: "installed" };
  const active = () => useAppStore.getState().activeDocumentId;
  switch (step.action) {
    case "models": {
      // "ready": the models in; "missing": none; "linux": the draft models' row, one model in.
      if (step.models === "ready")
        resetMockOcr({ states: { "nuqta-4b": installed, "llama-cpp": installed } });
      else if (step.models === "linux")
        resetMockOcr({
          states: { "nuqta-4b": installed },
          hardware: { accelerators: [], runtimeGpu: null, os: "linux", arch: "x86_64" },
        });
      else resetMockOcr();
      useOcrModels.setState({ models: null, jobs: {}, failures: {}, prompt: false });
      await R.refreshOcrModels();
      return {};
    }
    case "recognizePage": {
      await R.runCommand("ocr.recognizePage");
      await waitFor("a recognised page", () =>
        Object.values(useOcrJobs.getState().jobs).some((job) => job.state === "done"),
      );
      await waitFor("the review list", () => (useOcrJobs.getState().review[active()] ?? []).length > 0);
      return {};
    }
    case "decideAll": {
      for (let i = 0; i < 20 && R.isCommandEnabled("ocr.reviewConfirm"); i += 1)
        await R.runCommand("ocr.reviewConfirm");
      return {};
    }
    case "checkLayers": {
      await R.runCommand("ocr.checkLayers");
      await waitFor("a poor text layer found", () =>
        (useOcrJobs.getState().checks[active()] ?? []).some((check) => check.verdict === "poor"),
      );
      return {};
    }
    case "crashReports": {
      const { sampleCrashReport, setMockCrashReports } = await import("/src/backend/mockCrash.ts");
      const { refreshCrashReports } = await import("/src/state/crashReports.ts");
      setMockCrashReports(step.reports ? [sampleCrashReport()] : []);
      await refreshCrashReports();
      return {};
    }
    case "timestamp": {
      const { useCertSign } = await import("/src/state/certSign.ts");
      useCertSign.getState().updateForm({ timestamp: true, tsaUrl: "" });
      return {};
    }
    case "reset":
      useOcrJobs.setState({ reviewing: false, focus: null, drawingRegion: false, region: null });
      return {};
    default:
      throw new Error(`unknown OCR step ${step.action}`);
  }
}

/**
 * Runs axe over the document. Colour-contrast failures are checked again after
 * `contrastRetryMs`; only those that fail both times count. With `clipped`,
 * also lists visible text that is cut off.
 */
async function audit({ exclusions, contrastRetryMs, clipped }) {
  // A readable locator that is the same on every load: axe's own selectors use
  // ids, and React Aria's ids change with each page load.
  const generatedId = /^react-aria|_r_|:r|\u00abr/;
  const locate = (element) => {
    const parts = [];
    for (let node = element; node && node !== document.body && parts.length < 4; ) {
      if (node.id && !generatedId.test(node.id)) {
        parts.unshift(`#${CSS.escape(node.id)}`);
        break;
      }
      const testId = node.getAttribute("data-testid");
      if (testId) {
        parts.unshift(`[data-testid="${testId}"]`);
        break;
      }
      const role = node.getAttribute("role");
      const classes = [...node.classList].slice(0, 2).map((name) => `.${CSS.escape(name)}`);
      parts.unshift(node.localName + classes.join("") + (role ? `[role="${role}"]` : ""));
      node = node.parentElement;
    }
    return parts.join(" > ");
  };
  const describe = (violations) =>
    violations.flatMap((violation) =>
      violation.nodes.map((node) => {
        const selector = node.target
          .map((part) => (Array.isArray(part) ? part.join(" >>> ") : part))
          .join(" | ");
        const single = node.target.length === 1 && typeof node.target[0] === "string";
        const element = single ? document.querySelector(node.target[0]) : null;
        const exclusion = exclusions.find(
          (ex) => ex.rule === violation.id && element?.matches(ex.selector),
        );
        return {
          rule: violation.id,
          impact: node.impact ?? violation.impact,
          help: violation.help,
          helpUrl: violation.helpUrl,
          target: element ? locate(element) : selector,
          selector,
          html: node.html,
          failureSummary: node.failureSummary,
          excludedBecause: exclusion?.reason ?? null,
        };
      }),
    );

  let found = describe((await axe.run(document, { resultTypes: ["violations"] })).violations);
  let transientContrast = [];
  if (found.some((entry) => entry.rule === "color-contrast")) {
    await new Promise((resolve) => setTimeout(resolve, contrastRetryMs));
    const again = await axe.run(document, {
      runOnly: { type: "rule", values: ["color-contrast"] },
      resultTypes: ["violations"],
    });
    const still = new Set(describe(again.violations).map((entry) => entry.selector));
    const isTransient = (entry) => entry.rule === "color-contrast" && !still.has(entry.selector);
    transientContrast = found.filter(isTransient);
    found = found.filter((entry) => !isTransient(entry));
  }

  // Visible elements whose own text overflows a box that hides the overflow.
  // An ellipsis is fine when a title, or an accessible name given explicitly
  // (aria-label, aria-labelledby) on the element or its control, carries the
  // whole text. A name computed from the content does not count: it is always
  // the whole text, whatever the screen shows.
  const clippedText = [];
  if (clipped) {
    const squash = (text) => (text ?? "").replace(/\s+/g, " ").trim();
    const hides = (value) => value === "hidden" || value === "clip";
    const givenName = (element) => {
      const ids = element.getAttribute("aria-labelledby")?.split(/\s+/) ?? [];
      const labelledBy = ids.map((id) => document.getElementById(id)?.textContent ?? "");
      return squash([element.getAttribute("aria-label"), ...labelledBy].join(" "));
    };
    for (const element of document.body.querySelectorAll("*")) {
      const ownText = [...element.childNodes].some(
        (child) => child.nodeType === Node.TEXT_NODE && child.textContent.trim() !== "",
      );
      if (!ownText) continue;
      if (!element.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true })) continue;
      // Text for screen readers only (`.visually-hidden`: a 1px box that clips) is meant unseen.
      if (element.closest(".visually-hidden")) continue;
      const style = getComputedStyle(element);
      const overX = hides(style.overflowX) && element.scrollWidth > element.clientWidth + 1;
      const overY = hides(style.overflowY) && element.scrollHeight > element.clientHeight + 1;
      if (!overX && !overY) continue;
      const text = squash(element.textContent);
      if (style.textOverflow === "ellipsis") {
        const control = element.closest("button, a, label, [role]");
        const carriers = [
          element.closest("[title]")?.getAttribute("title"),
          givenName(element),
          control ? givenName(control) : null,
        ];
        if (carriers.some((carrier) => carrier && squash(carrier).includes(text))) continue;
      }
      clippedText.push({
        target: locate(element),
        text: text.slice(0, 120),
        html: element.outerHTML.slice(0, 240),
        overflow: [overX ? "x" : null, overY ? "y" : null].filter(Boolean).join("+"),
        ellipsis: style.textOverflow === "ellipsis",
        size: `${element.scrollWidth}x${element.scrollHeight} in ${element.clientWidth}x${element.clientHeight}`,
      });
    }
  }

  return {
    violations: found.filter((entry) => !entry.excludedBecause),
    excluded: found.filter((entry) => entry.excludedBecause),
    transientContrast,
    clippedText,
  };
}

// ---------------------------------------------------------------------- driving

async function loadApp(cdp, url, axeSource) {
  // A fresh start: no cached settings, no mock documents.
  await cdp.send("Storage.clearDataForOrigin", {
    origin: new URL(url).origin,
    storageTypes: "local_storage",
  });
  const loaded = cdp.once("Page.loadEventFired");
  await cdp.send("Page.navigate", { url });
  await loaded;
  await evaluate(cdp, axeSource);
  await evaluate(cdp, `window.__a11yOcrStep = ${ocrStep}`);
  await call(cdp, checkArabicContrast);
  const ready = await call(cdp, pageReady);
  await call(cdp, settle, SETTLE_MS);
  return ready;
}

async function checkCombo(cdp, combo, { url, axeSource, uiScale }) {
  const [locale, theme] = combo.split("-");
  const result = { combo, locale, theme, screens: [] };
  const step = async (kind, extra = {}) => call(cdp, act, { kind, ...extra });
  const check = async (screen, { clipped = false, settleMs = SETTLE_MS } = {}) => {
    await call(cdp, settle, settleMs);
    const found = await call(cdp, audit, {
      exclusions: EXCLUSIONS,
      contrastRetryMs: CONTRAST_RETRY_MS,
      clipped,
    });
    result.screens.push({ screen, ...found });
    const marks = found.violations.length + found.clippedText.length;
    process.stdout.write(marks === 0 ? "." : "x");
  };
  const skip = (screen, reason) => {
    result.screens.push({ screen, skipped: reason });
    process.stdout.write("s");
  };

  process.stdout.write(`${combo.padEnd(9)} `);
  await loadApp(cdp, url, axeSource);
  // Theme switches end without transitions (they would be measured mid-fade).
  await cdp.send("Emulation.setEmulatedMedia", {
    features: [
      { name: "prefers-reduced-motion", value: "reduce" },
      { name: "prefers-color-scheme", value: theme },
    ],
  });
  await step("combo", { locale, theme });
  await check("empty");

  await step("open");
  await check("viewer", { settleMs: 1000 });
  for (const reading of ["sepia", "night"]) {
    await step("readingTheme", { theme: reading });
    await check(`viewer (reading theme ${reading})`);
  }
  await step("readingTheme", { theme: "normal" });

  for (const id of PANELS) {
    const outcome = await step("command", { id });
    if (outcome.skipped) skip(`panel ${id}`, outcome.skipped);
    else await check(`panel ${id}`);
    await step("state", { state: { leftPanel: null } });
  }

  await step("replacePreview", { query: "4,500", replacement: "4,750", preview: false });
  await check("panel search.focus with the replace row");
  await step("replacePreview", { query: "4,500", replacement: "4,750", preview: true });
  await check("replace preview");
  await step("replaceClose");

  for (const tool of TOOLS) {
    await step("tool", { tool });
    await check(`tool ${tool}`);
  }
  await step("tool", { tool: "select" });

  await checkOcr(step, check);

  const closeDialogs = async () => {
    await step("state", { state: CLOSED_DIALOGS });
    await step("command", { id: "certSign.cancel" });
    await call(cdp, settle, SETTLE_MS);
  };
  for (const id of DIALOGS) {
    const outcome = await step("command", { id });
    if (outcome.skipped) skip(`dialog ${id}`, outcome.skipped);
    else await check(`dialog ${id}`);
    await closeDialogs();
  }

  await checkSettings(step, check, closeDialogs);

  if (uiScale && UI_SCALE_COMBOS.includes(combo)) {
    await step("uiScale", { scale: 2 });
    await check("interface size 2: viewer", { clipped: true });
    await step("ocr", { action: "models", models: "ready" });
    await step("tool", { tool: "ocr" });
    await check("interface size 2: tool ocr", { clipped: true });
    await step("ocr", { action: "reset" });
    await step("tool", { tool: "select" });
    const outcome = await step("command", { id: "app.settings" });
    if (outcome.skipped) skip("interface size 2: dialog app.settings", outcome.skipped);
    else await check("interface size 2: dialog app.settings", { clipped: true });
    await closeDialogs();
    await step("uiScale", { scale: 1 });
  }
  process.stdout.write("\n");
  return result;
}

/**
 * The OCR tool (ADR 0070's second pass): the panel, a finished job with the
 * pages-needing-review list, review mode's card (a word, then nothing left),
 * an area set from the keyboard, a layer check, and the model prompt.
 */
async function checkOcr(step, check) {
  const ocr = (action, extra = {}) => step("ocr", { action, ...extra });
  await ocr("models", { models: "ready" });
  await step("tool", { tool: "ocr" });
  await check("tool ocr");
  await ocr("recognizePage");
  await check("tool ocr: a finished job and the review list");
  await step("command", { id: "ocr.startReview" });
  await check("tool ocr: review card");
  await ocr("decideAll");
  await check("tool ocr: review card, nothing left");
  await step("command", { id: "ocr.stopReview" });
  await step("command", { id: "ocr.drawRegion" });
  await check("tool ocr: drawing an area");
  await step("command", { id: "ocr.regionWholePage" });
  await check("tool ocr: an area set, with its fields");
  await step("command", { id: "ocr.cancelRegion" });
  await ocr("checkLayers");
  await check("tool ocr: layer checks");
  await ocr("models", { models: "missing" });
  await step("command", { id: "ocr.recognize" });
  await check("dialog ocr model prompt");
  await step("command", { id: "ocrModels.dismissPrompt" });
  await ocr("reset");
  await step("tool", { tool: "select" });
}

/**
 * Settings' sections added since the first pass: OCR models with the Tesseract
 * note and a delete being confirmed, crash reports kept and shown; and the
 * certificate dialog's time-stamp field left empty.
 */
async function checkSettings(step, check, closeDialogs) {
  const ocr = (action, extra = {}) => step("ocr", { action, ...extra });
  await ocr("models", { models: "linux" });
  await ocr("crashReports", { reports: true });
  await step("command", { id: "app.settings" });
  await check("settings: OCR models on Linux, a crash report kept");
  await step("click", { selector: '[data-testid="ocr-model-nuqta-4b"] button' });
  await step("click", { selector: '[data-testid="crash-report"] button' });
  await check("settings: deleting a model, a crash report shown");
  await closeDialogs();
  await ocr("crashReports", { reports: false });
  await ocr("models", { models: "ready" });
  await step("command", { id: "certSign.open" });
  await ocr("timestamp");
  await check("dialog certSign.open: an empty time-stamp address");
  await closeDialogs();
}

// --------------------------------------------------------------------- reporting

/** "en-light, en-dark: viewer, tool edit" lines: combinations with the same screens share one. */
function whereLines(hits) {
  const byScreens = new Map();
  for (const [combo, screens] of hits) {
    const key = [...screens].join(", ");
    byScreens.set(key, [...(byScreens.get(key) ?? []), combo]);
  }
  return [...byScreens].map(([screens, combos]) => `${combos.join(", ")}: ${screens}`);
}

/** Groups entries by `keyOf`, then by target, recording the combinations and screens hit. */
function group(entries, keyOf) {
  const groups = new Map();
  for (const { combo, screen, entry } of entries) {
    const key = keyOf(entry);
    if (!groups.has(key)) groups.set(key, { first: entry, targets: new Map() });
    const targets = groups.get(key).targets;
    if (!targets.has(entry.target)) targets.set(entry.target, { entry, hits: new Map() });
    const hits = targets.get(entry.target).hits;
    if (!hits.has(combo)) hits.set(combo, new Set());
    hits.get(combo).add(screen);
  }
  return groups;
}

function printSummary(report) {
  const flat = (field) =>
    report.combos.flatMap((combo) =>
      combo.screens.flatMap((screen) =>
        (screen[field] ?? []).map((entry) => ({
          combo: combo.combo,
          screen: screen.screen,
          entry,
        })),
      ),
    );
  const violations = flat("violations");
  const clipped = flat("clippedText");
  const excluded = flat("excluded");
  const transient = flat("transientContrast");
  const short = (html) => (html.length > 160 ? `${html.slice(0, 157)}...` : html);

  console.log("");
  if (violations.length === 0) {
    console.log("axe: no violations.");
  } else {
    const rules = group(violations, (entry) => entry.rule);
    console.log(`axe: ${rules.size} rule(s) violated:`);
    for (const [rule, { first, targets }] of rules) {
      console.log(`\n  ${rule} [${first.impact}] ${first.help}`);
      console.log(`  ${first.helpUrl}`);
      for (const [target, { entry, hits }] of targets) {
        console.log(`    ${target}`);
        console.log(`      ${short(entry.html)}`);
        for (const line of whereLines(hits)) console.log(`      - ${line}`);
      }
    }
  }

  if (report.uiScale.ran) {
    console.log("");
    if (clipped.length === 0) {
      console.log("Clipped text at interface size 2: none.");
    } else {
      console.log("Clipped text at interface size 2:");
      const [{ targets }] = group(clipped, () => "clipped").values();
      for (const [target, { entry, hits }] of targets) {
        const how = `${entry.overflow}${entry.ellipsis ? ", ellipsis" : ""}, ${entry.size}`;
        console.log(`    ${target}  (${how}) "${entry.text}"`);
        for (const line of whereLines(hits)) console.log(`      - ${line}`);
      }
    }
  } else {
    console.log(`\nInterface size pass skipped: ${report.uiScale.reason}.`);
  }

  const skipped = report.combos.flatMap((combo) =>
    combo.screens
      .filter((screen) => screen.skipped)
      .map((screen) => `${combo.combo}: ${screen.screen} (${screen.skipped})`),
  );
  if (skipped.length > 0) console.log(`\nSkipped:\n${skipped.map((s) => `  ${s}`).join("\n")}`);
  if (excluded.length > 0) {
    console.log("\nExcluded (known and justified, see EXCLUSIONS in the script):");
    for (const [rule, { targets }] of group(excluded, (entry) => entry.rule)) {
      for (const [target, { hits }] of targets) {
        console.log(`  ${rule} on ${target} (${[...hits.keys()].join(", ")})`);
      }
    }
  }
  if (transient.length > 0) {
    console.log(
      `\nNote: ${transient.length} colour-contrast failure(s) passed on the retry (timing, not counted):`,
    );
    for (const { combo, screen, entry } of transient) {
      console.log(`  ${combo} / ${screen}: ${entry.target}`);
    }
  }
  if (report.pageMessages.length > 0) {
    console.log("\nPage console errors and warnings (not counted):");
    for (const { message, count } of report.pageMessages) {
      console.log(`  ${count}x ${message.split("\n")[0].slice(0, 200)}`);
    }
  }
  return {
    rules: new Set(violations.map(({ entry }) => entry.rule)).size,
    violations: violations.length,
    clipped: clipped.length,
  };
}

// ------------------------------------------------------------------------- main

async function main() {
  const started = Date.now();
  mkdirSync(outDir, { recursive: true });
  const axeSource = readFileSync(requireApp.resolve("axe-core/axe.min.js"), "utf8");
  const axeVersion = JSON.parse(
    readFileSync(requireApp.resolve("axe-core/package.json"), "utf8"),
  ).version;

  // Chrome's profile and Vite's cache; removed at the end.
  const work = mkdtempSync(path.join(tmpdir(), "mirsam-a11y-work-"));
  cleanups.push(() => rmSync(work, { recursive: true, force: true, maxRetries: 5 }));
  const url = await startVite(work);
  const { version, wsUrl } = await startChrome(work);
  const cdp = await connect(wsUrl);
  // Uncaught exceptions and console errors and warnings (React Aria warns
  // about accessibility there), each message once with a count.
  const pageMessages = new Map();
  const note = (message) => pageMessages.set(message, (pageMessages.get(message) ?? 0) + 1);
  cdp.on("Runtime.exceptionThrown", ({ exceptionDetails }) => {
    note(`exception: ${exceptionDetails.exception?.description ?? exceptionDetails.text}`);
  });
  cdp.on("Runtime.consoleAPICalled", ({ type, args: values }) => {
    if (type === "error" || type === "warning") {
      note(`${type}: ${values.map((value) => value.value ?? value.description ?? "").join(" ")}`);
    }
  });
  await cdp.send("Page.enable");
  await cdp.send("Runtime.enable");
  await cdp.send("Emulation.setDeviceMetricsOverride", {
    ...VIEWPORT,
    deviceScaleFactor: 1,
    mobile: false,
  });

  console.log(`axe-core ${axeVersion} in ${version}, app at ${url}`);
  const { hasUiScale } = await loadApp(cdp, url, axeSource);
  const uiScale = hasUiScale
    ? { ran: UI_SCALE_COMBOS.some((combo) => only.includes(combo)) }
    : { ran: false, reason: "the store has no setUiScale" };
  if (hasUiScale && !uiScale.ran) uiScale.reason = `none of ${UI_SCALE_COMBOS.join(", ")} chosen`;
  console.log("Screens: . clean, x violations, s skipped");

  const combos = [];
  for (const combo of only) {
    combos.push(await checkCombo(cdp, combo, { url, axeSource, uiScale: uiScale.ran }));
  }

  const report = {
    generatedAt: new Date().toISOString(),
    durationMs: Date.now() - started,
    axeVersion,
    browser: version,
    viewport: VIEWPORT,
    exclusions: EXCLUSIONS,
    uiScale,
    combos,
    pageMessages: [...pageMessages].map(([message, count]) => ({ message, count })),
  };
  writeFileSync(path.join(outDir, "report.json"), `${JSON.stringify(report, null, 2)}\n`);
  const totals = printSummary(report);
  const screens = combos.reduce((sum, combo) => sum + combo.screens.length, 0);
  console.log(
    `\n${screens} screens in ${combos.length} combination(s), ${Math.round(report.durationMs / 1000)} s.`,
  );
  console.log(`Report: ${path.join(outDir, "report.json")}`);
  if (totals.violations > 0 || totals.clipped > 0) {
    console.log(
      `FAILED: ${totals.rules} axe rule(s) violated (${totals.violations} element(s) over all ` +
        `screens), ${totals.clipped} clipped text element(s).`,
    );
    return 1;
  }
  console.log("Passed.");
  return 0;
}

let status = 2;
try {
  status = await main();
} catch (error) {
  if (!interrupted) console.error(`\na11y-check failed: ${error.stack ?? error}`);
} finally {
  await cleanUp();
}
process.exit(interrupted ? 130 : status);
