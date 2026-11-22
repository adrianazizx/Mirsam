#!/usr/bin/env node
// Seeds the fuzz corpora from the generated fixture corpus (fixtures/out):
//
//   corpus/open, corpus/file   every fixture PDF as it is (encrypted, linearized,
//                              truncated and incrementally updated ones included)
//   corpus/objects             each fixture's objects, uncompressed (qpdf --qdf),
//                              separated by `endobj`: the `objects` target's format
//   corpus/content             every page's content stream, uncompressed, plus a few
//                              written here for operators the fixtures lack
//   corpus/cmap                every ToUnicode CMap
//
// Needs qpdf and `node fixtures/generate.mjs`. Usage: node fuzz/seed.mjs

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const FUZZ = path.dirname(fileURLToPath(import.meta.url));
const FIXTURES = path.join(FUZZ, "../fixtures/out");
/** Larger files slow every run down more than they add (the 1,000-page fixture). */
const MAX_SEED_BYTES = 512 * 1024;

if (!fs.existsSync(path.join(FIXTURES, "manifest.json"))) {
  console.error("missing fixtures/out: run node fixtures/generate.mjs first");
  process.exit(2);
}

const counts = {};
const add = (target, bytes) => {
  if (bytes.length === 0 || bytes.length > MAX_SEED_BYTES) return;
  const dir = path.join(FUZZ, "corpus", target);
  fs.mkdirSync(dir, { recursive: true });
  const name = createHash("sha1").update(bytes).digest("hex");
  fs.writeFileSync(path.join(dir, name), bytes);
  counts[target] = (counts[target] ?? 0) + 1;
};

const pdfs = walk(FIXTURES).filter((file) => file.endsWith(".pdf"));
for (const file of pdfs) {
  const raw = fs.readFileSync(file);
  add("open", raw);
  add("file", raw);

  // Encrypted fixtures have no readable plain objects; qpdf fails on them.
  const qdf = spawnSync("qpdf", ["--qdf", "--object-streams=disable", file, "-"], { maxBuffer: 1 << 28 });
  if (qdf.status !== 0 && qdf.status !== 3) continue; // 3: warnings
  const text = qdf.stdout.toString("latin1");

  const objects = new Map();
  for (const match of text.matchAll(/^(\d+) 0 obj\n([\s\S]*?)\nendobj$/gm)) {
    objects.set(Number(match[1]), match[2]);
  }
  const highest = Math.max(0, ...objects.keys());
  const bodies = Array.from({ length: highest }, (_, i) => objects.get(i + 1) ?? "null");
  add("objects", Buffer.from(bodies.join("endobj"), "latin1"));

  for (const match of text.matchAll(/%% Contents for page \d+\n(?:%%[^\n]*\n)*\d+ 0 obj\n[\s\S]*?\nstream\n([\s\S]*?)\nendstream/g)) {
    add("content", Buffer.from(match[1], "latin1"));
  }
  for (const match of text.matchAll(/\/ToUnicode (\d+) 0 R/g)) {
    const body = objects.get(Number(match[1]));
    const data = body?.match(/\nstream\n([\s\S]*?)\nendstream/)?.[1];
    if (data) add("cmap", Buffer.from(data, "latin1"));
  }
}

// Content the fixtures do not draw: inline images, marked content with
// properties, text render modes, clipping, shading and type 3 style operators.
const handWritten = [
  "q 0 0 1 rg 10 10 100 50 re f Q BT /F1 24 Tf 3 Tr 50 50 Td (invisible) Tj 0 Tr ET",
  "BI /W 2 /H 2 /CS /G /BPC 8 ID \x00\xff\xff\x00 EI Q",
  "/Span << /ActualText (\xfe\xff\x06\x27) >> BDC BT /F2 12 Tf 100 100 Td <0001> Tj ET EMC",
  "q 100 100 200 200 re W n BT /F1 12 Tf 0 0 Td [(A) 120 (V) -50 (A)] TJ ET Q",
  "BT /F1 12 Tf 1 0 0 -1 0 842 Tm 12 TL (flipped) Tj T* 0.5 Tc 2 Tw 80 Tz 3 Ts (spaced) Tj ET",
  "q 1 0 0 1 50 50 cm q 2 0 0 2 0 0 cm /X1 Do Q Q /Im1 Do /G1 gs 0.5 g 0 G 4 w 1 J 1 j [3 1] 0 d",
  "BT /F2 10 Tf 0 1 -1 0 300 300 Tm <00100011001200130014> Tj ET",
  "%comment\nBT\r/F1\t12\fTf ( nested ( parens ) and \\) escapes \\101 ) Tj <4142 43> Tj ET",
];
for (const content of handWritten) add("content", Buffer.from(content, "latin1"));

console.log(`seeded from ${pdfs.length} fixtures:`, counts);

function walk(dir) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(dir, entry.name);
    return entry.isDirectory() ? walk(full) : [full];
  });
}
