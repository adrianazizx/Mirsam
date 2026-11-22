#!/usr/bin/env node
// Generates the synthetic half of the OCR evaluation set (docs/SPEC.md section 8)
// into ocr-eval/out/: page images whose text is known exactly, because we
// rendered it ourselves.
//
// Each sample is one content file (fixtures/content/, plus the OCR-only pages in
// ocr-eval/content/), set in one Arabic face, rasterized by Chrome at a given
// resolution, optionally degraded the way real scans are (blur, skew + speckle,
// bilevel fax, photocopy shadow; and, since set v2, severe conditions built
// from SVG filters: broken fax strokes, second-generation photocopies, faded
// toner, warped phone-photo pages). Ground truth is the content's logical-order
// text; table samples also list their cells, row by row in reading order. Real
// scans with human-verified truth live in ocr-eval/scans/ and are merged into
// the same manifest.
//
// Usage: node ocr-eval/generate.mjs

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { contentHtml, expectedBlocks, findChrome, FONTS, loadContents } from "../fixtures/lib.mjs";

/** Bump when samples or ground truth change: results across versions are not comparable. */
const SET_VERSION = 2;

const HERE = path.dirname(fileURLToPath(import.meta.url));
const OUT = path.join(HERE, "out");
const FIELDS = JSON.parse(readFileSync(path.join(HERE, "fields.json"), "utf8"));

// A4 at CSS resolution (96 dpi). Chrome's device scale factor sets the scan resolution.
const PAGE_CSS = { width: 794, height: 1123 };

/** Black specks: fractal noise thresholded to sparse dots. Lower `cut` = more specks. */
const speckle = (id, seed, cut, frequency = 0.85) => `<svg class="overlay" xmlns="http://www.w3.org/2000/svg" width="100%" height="100%">
  <filter id="${id}" x="0" y="0" width="100%" height="100%">
    <feTurbulence type="fractalNoise" baseFrequency="${frequency}" numOctaves="2" seed="${seed}"/>
    <feColorMatrix type="matrix" values="0 0 0 0 0  0 0 0 0 0  0 0 0 0 0  0 0 0 -22 ${cut}"/>
  </filter>
  <rect width="100%" height="100%" filter="url(#${id})"/>
</svg>`;

/** White dropouts: the same noise in white, punching holes in strokes. */
const dropouts = (id, seed, cut) => `<svg class="overlay" xmlns="http://www.w3.org/2000/svg" width="100%" height="100%">
  <filter id="${id}" x="0" y="0" width="100%" height="100%">
    <feTurbulence type="fractalNoise" baseFrequency="0.6" numOctaves="2" seed="${seed}"/>
    <feColorMatrix type="matrix" values="0 0 0 0 1  0 0 0 0 1  0 0 0 0 1  0 0 0 -22 ${cut}"/>
  </filter>
  <rect width="100%" height="100%" filter="url(#${id})"/>
</svg>`;

/** Large, soft blotches (uneven toner or lighting), multiplied over the page. */
const mottle = (id, seed, strength) => `<svg class="overlay" style="mix-blend-mode: multiply" xmlns="http://www.w3.org/2000/svg" width="100%" height="100%">
  <filter id="${id}" x="0" y="0" width="100%" height="100%">
    <feTurbulence type="fractalNoise" baseFrequency="0.006" numOctaves="3" seed="${seed}"/>
    <feColorMatrix type="matrix" values="0 0 0 0 0.45  0 0 0 0 0.43  0 0 0 0 0.38  ${strength} 0 0 0 0"/>
  </filter>
  <rect width="100%" height="100%" filter="url(#${id})"/>
</svg>`;

/** A filter the `.page` element refers to with `filter: url(#warp)`: wavy text lines. */
const WARP_FILTER = `<svg width="0" height="0" style="position: absolute" xmlns="http://www.w3.org/2000/svg">
  <filter id="warp" x="-5%" y="-5%" width="110%" height="110%">
    <feTurbulence type="fractalNoise" baseFrequency="0.004 0.009" numOctaves="1" seed="11" result="field"/>
    <feDisplacementMap in="SourceGraphic" in2="field" scale="30" xChannelSelector="R" yChannelSelector="G"/>
  </filter>
</svg>`;

/**
 * Scan conditions. `css` degrades the `.page` element; `overlay` is drawn on top of it.
 * `severe` conditions (set v2) are meant to separate engines that v1's degraded
 * category could not: every engine tried in M0 read those almost perfectly.
 */
const VARIANTS = {
  "clean-300": { dpi: 300, degradation: "none" },
  "lowres-150": { dpi: 150, degradation: "low resolution" },
  "blur-200": { dpi: 200, degradation: "defocus blur", css: ".page { filter: blur(0.55px); }" },
  "skew-noise-200": {
    dpi: 200,
    degradation: "1.3° skew, speckle",
    css: ".page { transform: rotate(1.3deg) scale(0.97); }",
    overlay: `<svg class="overlay" xmlns="http://www.w3.org/2000/svg" width="100%" height="100%">
  <filter id="speckle" x="0" y="0" width="100%" height="100%">
    <feTurbulence type="fractalNoise" baseFrequency="0.85" numOctaves="2" seed="7"/>
    <feColorMatrix type="matrix" values="0 0 0 0 0  0 0 0 0 0  0 0 0 0 0  0 0 0 -22 6.6"/>
  </filter>
  <rect width="100%" height="100%" filter="url(#speckle)" opacity="0.8"/>
</svg>`,
  },
  "fax-100": {
    dpi: 100,
    degradation: "bilevel fax, 100 dpi",
    css: ".page { filter: blur(0.35px) grayscale(1) contrast(3000%); }",
  },
  "shadow-200": {
    dpi: 200,
    degradation: "photocopy: grey paper, faded ink, edge shadow",
    css: `.page { background: #e7e4dc; color: #3a3a3a; }
.page h1, .page p, .page td { color: #3a3a3a; }`,
    overlay: `<div class="overlay" style="background: linear-gradient(90deg, rgba(0,0,0,0.42), rgba(0,0,0,0.08) 9%, transparent 22%), linear-gradient(0deg, rgba(0,0,0,0.25), transparent 12%)"></div>`,
  },
  "fax-broken-100": {
    dpi: 100,
    degradation: "bilevel fax, 100 dpi, thinned and broken strokes, specks",
    css: ".page { filter: blur(0.45px) grayscale(1) brightness(1.18) contrast(3000%); }",
    overlay: speckle("specks", 3, 6.4) + dropouts("holes", 5, 6.2),
  },
  "photocopy2-150": {
    dpi: 150,
    degradation: "second-generation photocopy, 150 dpi: ink bleed, grey mottled paper, 2° skew, specks, edge shadow",
    css: `.page { transform: rotate(-2deg) scale(0.96); filter: blur(0.55px) grayscale(1) contrast(180%) brightness(0.92); }`,
    overlay:
      mottle("toner", 21, 0.55) +
      speckle("specks", 9, 6.3) +
      `<div class="overlay" style="background: linear-gradient(270deg, rgba(0,0,0,0.55), rgba(0,0,0,0.12) 7%, transparent 18%), linear-gradient(180deg, rgba(0,0,0,0.3), transparent 10%)"></div>`,
  },
  "faded-200": {
    dpi: 200,
    degradation: "faded toner on off-white paper, 200 dpi: low contrast, uneven background, slight blur",
    css: `.page { background: #f1eee6; filter: blur(0.6px) contrast(0.3) brightness(1.25); }`,
    overlay: mottle("toner", 33, 0.35),
  },
  "warp-200": {
    dpi: 200,
    degradation: "phone photo of a curved page, 200 dpi: perspective, wavy lines, uneven light, blur",
    css: `body { background: #6b6259; }
.page { transform: perspective(1600px) rotateX(5deg) rotateY(-6deg) rotate(1.2deg) scale(0.9); filter: url(#warp) blur(0.85px); box-shadow: 0 0 24px rgba(0,0,0,0.5); }`,
    overlay:
      WARP_FILTER +
      `<div class="overlay" style="background: radial-gradient(ellipse at 20% 15%, rgba(255,255,255,0.0), rgba(0,0,0,0.0) 40%, rgba(40,30,20,0.38) 100%)"></div>`,
  },
};

const DEGRADED = ["lowres-150", "blur-200", "skew-noise-200", "fax-100", "shadow-200"];
const SEVERE = ["fax-broken-100", "photocopy2-150", "faded-200", "warp-200"];

/** The sample matrix: which content, in which face, under which scan condition, for which category. */
function sampleSpecs() {
  const specs = [];
  const add = (content, font, variant, categories) => specs.push({ content, font, variant, categories });

  for (const content of ["ar-plain", "ar-lam-alef"]) {
    for (const font of ["naskh", "amiri"]) add(content, font, "clean-300", ["clean-arabic-300dpi"]);
    for (const font of ["sans", "kufi"]) add(content, font, "clean-300", ["other-typefaces"]);
    for (const variant of DEGRADED) add(content, "naskh", variant, ["degraded-arabic"]);
    for (const variant of SEVERE) add(content, "naskh", variant, ["degraded-arabic-severe"]);
  }
  for (const variant of SEVERE) add("ar-plain", "amiri", variant, ["degraded-arabic-severe"]);
  for (const font of ["amiri", "naskh"]) add("ar-vocalized", font, "clean-300", ["diacritized"]);
  add("ar-vocalized", "amiri", "blur-200", ["diacritized", "degraded-arabic"]);

  add("en-paragraphs", "latin", "clean-300", ["clean-english-300dpi", "numbers-dates-ids"]);
  for (const font of ["naskh", "sans"]) {
    add("mixed-inline", font, "clean-300", ["mixed-arabic-english", "numbers-dates-ids"]);
    add("contract-two-column", font, "clean-300", ["mixed-arabic-english", "numbers-dates-ids"]);
  }
  add("mixed-inline", "naskh", "fax-100", ["degraded-mixed", "numbers-dates-ids"]);
  add("mixed-inline", "naskh", "skew-noise-200", ["degraded-mixed", "numbers-dates-ids"]);
  add("table-bilingual", "naskh", "clean-300", ["tables-forms", "numbers-dates-ids"]);
  add("table-bilingual", "naskh", "lowres-150", ["tables-forms", "numbers-dates-ids"]);
  // Set v2: severe conditions for mixed lines and tables, and an Arabic-only form.
  add("mixed-inline", "naskh", "photocopy2-150", ["degraded-mixed", "numbers-dates-ids"]);
  add("mixed-inline", "sans", "warp-200", ["degraded-mixed", "numbers-dates-ids"]);
  add("table-bilingual", "sans", "fax-broken-100", ["tables-forms", "numbers-dates-ids"]);
  add("form-leave-request", "naskh", "clean-300", ["tables-forms", "numbers-dates-ids"]);
  add("form-leave-request", "sans", "photocopy2-150", ["tables-forms", "numbers-dates-ids"]);
  return specs;
}

const PAGE_CSS_RULES = `
html, body { inline-size: ${PAGE_CSS.width}px; block-size: ${PAGE_CSS.height}px; overflow: hidden; background: #fff; }
.page { position: absolute; inset: 0; box-sizing: border-box; padding: 56pt; background: #fff; transform-origin: center; }
.overlay { position: absolute; inset: 0; pointer-events: none; }
`;

const chrome = findChrome();
if (!chrome) {
  console.error("Chrome/Chromium not found (set CHROME_BIN); cannot rasterize samples.");
  process.exit(1);
}

// OCR-only pages (forms and the like) live beside this script, so they never
// enter the PDF fixture corpus and its extraction tests.
const ocrContents = readdirSync(path.join(HERE, "content"))
  .filter((name) => name.endsWith(".json"))
  .sort()
  .map((name) => JSON.parse(readFileSync(path.join(HERE, "content", name), "utf8")));
const contents = new Map([...loadContents(), ...ocrContents].map((content) => [content.id, content]));
const workDir = path.join(tmpdir(), `safha-ocr-eval-${process.pid}`);
rmSync(OUT, { recursive: true, force: true });
mkdirSync(path.join(OUT, "images"), { recursive: true });
mkdirSync(path.join(OUT, "truth"), { recursive: true });
mkdirSync(workDir, { recursive: true });

const samples = [];
for (const spec of sampleSpecs()) {
  const content = contents.get(spec.content);
  if (!content) throw new Error(`unknown content "${spec.content}"`);
  const variant = VARIANTS[spec.variant];
  const id = `syn-${spec.content}-${spec.font}-${spec.variant}`;

  const html = contentHtml(content, {
    arabicFont: spec.font === "latin" ? undefined : spec.font,
    css: `${PAGE_CSS_RULES}\n${variant.css ?? ""}`,
    wrap: (body) => `<div class="page">${body}</div>${variant.overlay ?? ""}`,
  });
  const htmlFile = path.join(workDir, `${id}.html`);
  const imageFile = path.join(OUT, "images", `${id}.png`);
  writeFileSync(htmlFile, html);
  execFileSync(
    chrome,
    [
      "--headless=new",
      "--disable-gpu",
      ...(process.env.CI ? ["--no-sandbox"] : []),
      "--hide-scrollbars",
      `--user-data-dir=${path.join(workDir, "chrome-profile")}`,
      `--window-size=${PAGE_CSS.width},${PAGE_CSS.height}`,
      `--force-device-scale-factor=${variant.dpi / 96}`,
      `--screenshot=${imageFile}`,
      pathToFileURL(htmlFile).href,
    ],
    { stdio: ["ignore", "ignore", "ignore"], timeout: 120_000 },
  );
  if (!existsSync(imageFile)) throw new Error(`${id}: Chrome wrote no image`);

  const truth = expectedBlocks(content)
    .map((block) => block.text)
    .join("\n");
  writeFileSync(path.join(OUT, "truth", `${id}.txt`), `${truth}\n`);

  // Table cells, row by row in reading order, for cell-level scoring (ADR 0030).
  const cells = content.blocks.filter((block) => block.kind === "table").flatMap((block) => block.rows);

  samples.push({
    id,
    image: `images/${id}.png`,
    truth: `truth/${id}.txt`,
    categories: spec.categories,
    fields: FIELDS[spec.content] ?? [],
    ...(cells.length > 0 ? { cells } : {}),
    source: "synthetic",
    content: spec.content,
    font: FONTS[spec.font].family,
    dpi: variant.dpi,
    degradation: variant.degradation,
  });
  console.log(`ok    ${id}`);
}

// Real scans: ocr-eval/scans/<id>.png with <id>.txt (human-verified) and <id>.json (metadata).
const scansDir = path.join(HERE, "scans");
for (const name of readdirSync(scansDir).filter((file) => file.endsWith(".json")).sort()) {
  const id = name.replace(/\.json$/, "");
  const meta = JSON.parse(readFileSync(path.join(scansDir, name), "utf8"));
  for (const required of [`${id}.png`, `${id}.txt`]) {
    if (!existsSync(path.join(scansDir, required))) throw new Error(`scan ${id}: missing ${required}`);
  }
  if (!meta.verifiedBy) throw new Error(`scan ${id}: ground truth must name who verified it ("verifiedBy")`);
  samples.push({
    id,
    image: `../scans/${id}.png`,
    truth: `../scans/${id}.txt`,
    categories: meta.categories,
    fields: meta.fields ?? [],
    source: "scan",
    ...meta,
  });
  console.log(`ok    ${id} (scan)`);
}

rmSync(workDir, { recursive: true, force: true });
writeFileSync(
  path.join(OUT, "manifest.json"),
  `${JSON.stringify({ version: SET_VERSION, samples }, null, 2)}\n`,
);

const perCategory = {};
for (const sample of samples) {
  for (const category of sample.categories) perCategory[category] = (perCategory[category] ?? 0) + 1;
}
console.log(`\n${samples.length} samples (set v${SET_VERSION}):`);
for (const [category, count] of Object.entries(perCategory).sort()) {
  console.log(`  ${category.padEnd(24)} ${count}`);
}
