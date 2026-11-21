#!/usr/bin/env node
// Generates the synthetic half of the OCR evaluation set (docs/SPEC.md section 8)
// into ocr-eval/out/: page images whose text is known exactly, because we
// rendered it ourselves.
//
// Each sample is one fixtures/content/ file, set in one Arabic face, rasterized
// by Chrome at a given resolution, optionally degraded the way real scans are
// (blur, skew + speckle, bilevel fax, photocopy shadow). Ground truth is the
// content's logical-order text. Real scans with human-verified truth live in
// ocr-eval/scans/ and are merged into the same manifest.
//
// Usage: node ocr-eval/generate.mjs

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { contentHtml, expectedBlocks, findChrome, FONTS, loadContents } from "../fixtures/lib.mjs";

/** Bump when samples or ground truth change: results across versions are not comparable. */
const SET_VERSION = 1;

const HERE = path.dirname(fileURLToPath(import.meta.url));
const OUT = path.join(HERE, "out");
const FIELDS = JSON.parse(readFileSync(path.join(HERE, "fields.json"), "utf8"));

// A4 at CSS resolution (96 dpi). Chrome's device scale factor sets the scan resolution.
const PAGE_CSS = { width: 794, height: 1123 };

/** Scan conditions. `css` degrades the `.page` element; `overlay` is drawn on top of it. */
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
};

const DEGRADED = ["lowres-150", "blur-200", "skew-noise-200", "fax-100", "shadow-200"];

/** The sample matrix: which content, in which face, under which scan condition, for which category. */
function sampleSpecs() {
  const specs = [];
  const add = (content, font, variant, categories) => specs.push({ content, font, variant, categories });

  for (const content of ["ar-plain", "ar-lam-alef"]) {
    for (const font of ["naskh", "amiri"]) add(content, font, "clean-300", ["clean-arabic-300dpi"]);
    for (const font of ["sans", "kufi"]) add(content, font, "clean-300", ["other-typefaces"]);
    for (const variant of DEGRADED) add(content, "naskh", variant, ["degraded-arabic"]);
  }
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

const contents = new Map(loadContents().map((content) => [content.id, content]));
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

  samples.push({
    id,
    image: `images/${id}.png`,
    truth: `truth/${id}.txt`,
    categories: spec.categories,
    fields: FIELDS[spec.content] ?? [],
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
