#!/usr/bin/env node
// Ground-truth word boxes for the geometry-preserving samples of ocr-eval set v2 (M4 word-box spike).
//
// The set has no boxes. Chrome knows where it laid out every word, so this re-renders each
// sample's HTML (same content, face and page rules as generate.mjs; the degradations that keep
// geometry are pixel filters, so boxes carry over) and reads the word rectangles from the DOM.
// Samples whose degradation moves pixels (skew, warp, photocopy shadow) are left out.
//
//   node word_boxes_truth.mjs  ->  spikes/out/ocr-engines/word-boxes-truth.json
//
// { "<sample id>": [ { "text": "...", "box": [x0, y0, x1, y1] }, ... ] }  boxes in image pixels,
// words in DOM order (which is logical order within each block).

import { execFileSync } from "node:child_process";
import { mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { contentHtml, findChrome, loadContents } from "../../fixtures/lib.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, "../..");
const MANIFEST = JSON.parse(readFileSync(path.join(REPO, "ocr-eval/out/manifest.json"), "utf8"));
const OUT = path.join(REPO, "spikes/out/ocr-engines");
const KEEP = ["none", "low resolution", "defocus blur", "bilevel fax, 100 dpi"];
const PAGE = { width: 794, height: 1123 };

const CSS = `
html, body { inline-size: ${PAGE.width}px; block-size: ${PAGE.height}px; overflow: hidden; background: #fff; }
.page { position: absolute; inset: 0; box-sizing: border-box; padding: 56pt; background: #fff; transform-origin: center; }
`;
const SCRIPT = `<script>
document.fonts.ready.then(() => {
  const words = [];
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const text = node.nodeValue;
    for (const m of text.matchAll(/\\S+/g)) {
      const range = document.createRange();
      range.setStart(node, m.index);
      range.setEnd(node, m.index + m[0].length);
      const rects = [...range.getClientRects()].filter((r) => r.width > 0);
      if (!rects.length) continue;
      words.push({
        text: m[0],
        box: [
          Math.min(...rects.map((r) => r.left)), Math.min(...rects.map((r) => r.top)),
          Math.max(...rects.map((r) => r.right)), Math.max(...rects.map((r) => r.bottom)),
        ],
      });
    }
  }
  const pre = document.createElement("pre");
  pre.id = "boxes";
  pre.textContent = JSON.stringify(words);
  document.body.appendChild(pre);
});
</script>`;

const chrome = findChrome();
const ocrContents = readdirSync(path.join(REPO, "ocr-eval/content"))
  .filter((n) => n.endsWith(".json"))
  .map((n) => JSON.parse(readFileSync(path.join(REPO, "ocr-eval/content", n), "utf8")));
const contents = new Map([...loadContents(), ...ocrContents].map((c) => [c.id, c]));
const work = path.join(tmpdir(), `mirsam-wordboxes-${process.pid}`);
mkdirSync(work, { recursive: true });
const result = {};
for (const sample of MANIFEST.samples) {
  if (sample.source !== "synthetic" || !KEEP.includes(sample.degradation)) continue;
  const content = contents.get(sample.content);
  const fontKey = sample.id.slice(`syn-${sample.content}-`.length).split("-")[0];
  const html = contentHtml(content, {
    arabicFont: fontKey === "latin" ? undefined : fontKey,
    css: CSS,
    wrap: (body) => `<div class="page">${body}</div>${SCRIPT}`,
  });
  const file = path.join(work, `${sample.id}.html`);
  writeFileSync(file, html);
  const dom = execFileSync(
    chrome,
    ["--headless=new", "--disable-gpu", "--hide-scrollbars", `--user-data-dir=${path.join(work, "profile")}`,
      `--window-size=${PAGE.width},${PAGE.height}`, "--virtual-time-budget=4000", "--dump-dom", pathToFileURL(file).href],
    { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"], timeout: 120_000, maxBuffer: 1 << 26 },
  );
  const m = dom.match(/<pre id="boxes">([\s\S]*?)<\/pre>/);
  if (!m) throw new Error(`${sample.id}: no boxes in DOM dump`);
  const json = m[1].replace(/&quot;/g, '"').replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&");
  const k = sample.dpi / 96;
  result[sample.id] = JSON.parse(json).map((w) => ({ text: w.text, box: w.box.map((v) => Math.round(v * k * 10) / 10) }));
  console.log(`ok  ${sample.id}  ${result[sample.id].length} words`);
}
rmSync(work, { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });
writeFileSync(path.join(OUT, "word-boxes-truth.json"), JSON.stringify(result));
console.log(`${Object.keys(result).length} samples`);
