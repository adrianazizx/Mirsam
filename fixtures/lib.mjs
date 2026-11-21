// Shared by fixtures/generate.mjs (PDF corpus) and ocr-eval/generate.mjs (page
// images): the content files, the fixture fonts, and their HTML layout.

import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const HERE = path.dirname(fileURLToPath(import.meta.url));
export const ROOT = path.resolve(HERE, "..");
export const FONTS_DIR = path.join(ROOT, "assets", "fonts");
export const FONTS = JSON.parse(readFileSync(path.join(HERE, "fonts.json"), "utf8"));

export function which(command) {
  try {
    return execFileSync(process.platform === "win32" ? "where" : "which", [command], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    })
      .split(/\r?\n/)[0]
      .trim();
  } catch {
    return null;
  }
}

export function findChrome() {
  const candidates = [
    process.env.CHROME_BIN,
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
    which("google-chrome"),
    which("google-chrome-stable"),
    which("chromium"),
    which("chromium-browser"),
  ];
  return candidates.find((candidate) => candidate && existsSync(candidate)) ?? null;
}

/** Every content file, sorted by id. */
export function loadContents() {
  return readdirSync(path.join(HERE, "content"))
    .filter((name) => name.endsWith(".json"))
    .sort()
    .map((name) => JSON.parse(readFileSync(path.join(HERE, "content", name), "utf8")));
}

export const hasArabic = (text) => /[\u0600-\u06FF]/.test(text);

const escapeHtml = (text) =>
  text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

function fontFaces() {
  return Object.values(FONTS)
    .map(({ family, file, boldFile }) => {
      const url = (name) => pathToFileURL(path.join(FONTS_DIR, name)).href;
      if (file === boldFile) {
        // Variable font: one file covers every weight.
        return `@font-face { font-family: "${family}"; src: url("${url(file)}"); font-weight: 100 900; }`;
      }
      return [
        `@font-face { font-family: "${family}"; src: url("${url(file)}"); font-weight: 400; }`,
        `@font-face { font-family: "${family}"; src: url("${url(boldFile)}"); font-weight: 700; }`,
      ].join("\n");
    })
    .join("\n");
}

/**
 * Lays a content file out as an A4 HTML page.
 *
 * options.arabicFont  font key that replaces every non-Latin font in the content
 * options.css         extra CSS appended to the page styles
 * options.wrap        (bodyHtml) => html, to wrap the content (e.g. in a degradation layer)
 */
export function contentHtml(content, options = {}) {
  const fontStack = (key) => {
    const resolved = key !== "latin" && options.arabicFont ? options.arabicFont : key;
    const family = FONTS[resolved]?.family;
    if (!family) throw new Error(`unknown font key "${resolved}"`);
    // Latin letters inside Arabic text fall back to the Latin fixture face.
    return resolved === "latin" ? `"${family}"` : `"${family}", "${FONTS.latin.family}"`;
  };

  const textBlock = (block) => {
    const tag = block.kind === "heading" ? "h1" : "p";
    const lang = block.dir === "rtl" ? "ar" : "en";
    return `<${tag} dir="${block.dir}" lang="${lang}" style='font-family: ${fontStack(block.font)}'>${escapeHtml(block.text)}</${tag}>`;
  };

  const anyBlock = (block) => {
    if (block.kind === "table") {
      const rows = block.rows
        .map((row) => {
          const cells = row
            .map((cell) => {
              const rtl = hasArabic(cell);
              const font = fontStack(rtl ? block.font : "latin");
              return `<td dir="${rtl ? "rtl" : "ltr"}" style='font-family: ${font}'>${escapeHtml(cell)}</td>`;
            })
            .join("");
          return `<tr>${cells}</tr>`;
        })
        .join("\n");
      return `<table dir="${block.dir}">\n${rows}\n</table>`;
    }
    if (block.kind === "columns") {
      const columns = block.columns
        .map((column) => `<div dir="${column.dir}">${column.blocks.map(textBlock).join("\n")}</div>`)
        .join("\n");
      return `<div class="columns" style="grid-template-columns: repeat(${block.columns.length}, 1fr)">\n${columns}\n</div>`;
    }
    return textBlock(block);
  };

  const body = content.blocks.map(anyBlock).join("\n");
  return `<!doctype html>
<html>
<head>
<meta charset="utf-8">
<title>${escapeHtml(content.id)}</title>
<style>
${fontFaces()}
@page { size: A4; margin: 56pt; }
body { margin: 0; font-size: 12pt; line-height: 1.7; color: #000; }
h1 { margin: 0 0 12pt; font-size: 20pt; font-weight: 700; line-height: 1.4; }
p { margin: 0 0 12pt; }
table { inline-size: 100%; border-collapse: collapse; }
td { padding: 6pt; border: 0.5pt solid #999; vertical-align: top; }
/* Column order is physical (first column on the left) whatever the text direction. */
.columns { display: grid; gap: 24pt; direction: ltr; }
${options.css ?? ""}
</style>
</head>
<body>
${options.wrap ? options.wrap(body) : body}
</body>
</html>
`;
}

/** Logical-order text of a content file, block by block, in reading order. */
export function expectedBlocks(content) {
  const out = [];
  const visit = (block) => {
    if (block.kind === "table") {
      for (const row of block.rows) {
        for (const cell of row) out.push({ dir: hasArabic(cell) ? "rtl" : "ltr", text: cell });
      }
    } else if (block.kind === "columns") {
      // Reading order of a bilingual contract: finish one language, then the other.
      for (const column of block.columns) column.blocks.forEach(visit);
    } else {
      out.push({ dir: block.dir, text: block.text });
    }
  };
  content.blocks.forEach(visit);
  return out;
}
