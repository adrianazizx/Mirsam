#!/usr/bin/env node
// Generates the PDF test corpus (docs/SPEC.md section 6.6) into fixtures/out/.
//
// Every fixture comes from a content file in fixtures/content/, which is also
// the ground truth: each block's `text` is the logical-order string that
// extraction, search, and copy must reproduce. The same content is laid out by
// several *producers*, because each PDF producer stores text differently:
//
//   chrome    Chrome headless print-to-PDF (Skia PDF backend)
//   coretext  macOS CoreText into a Quartz PDF context (macOS only)
//
// Derived fixtures (encrypted, linearized, damaged, 1,000 pages) are made from
// those with qpdf. Nothing generated is committed; only OFL fonts are embedded.
//
// Usage: node fixtures/generate.mjs [--producer chrome|coretext] [--only <id>]

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import {
  contentHtml,
  expectedBlocks,
  findChrome,
  FONTS,
  FONTS_DIR,
  HERE,
  loadContents,
  which,
} from "./lib.mjs";

const OUT = path.join(HERE, "out");

/** Password of the encrypted fixture. Test data, not a secret. */
const ENCRYPTED_PASSWORD = "safha-test";

const args = process.argv.slice(2);
const flag = (name) => {
  const index = args.indexOf(name);
  return index === -1 ? null : args[index + 1];
};
const onlyProducer = flag("--producer");
const onlyId = flag("--only");

// ---------------------------------------------------------------- producers

const chrome = findChrome();
const swift = process.platform === "darwin" ? which("swift") : null;
const qpdf = which("qpdf");

const PRODUCERS = {
  chrome: {
    available: Boolean(chrome),
    unavailableReason: "Chrome/Chromium not found (set CHROME_BIN)",
    run(content, outFile, workDir) {
      const htmlFile = path.join(workDir, `${content.id}.html`);
      writeFileSync(htmlFile, contentHtml(content));
      execFileSync(
        chrome,
        [
          "--headless=new",
          "--disable-gpu",
          // CI containers run as root, where Chrome refuses its sandbox.
          ...(process.env.CI ? ["--no-sandbox"] : []),
          "--no-pdf-header-footer",
          // Headings become the PDF outline (bookmarks).
          "--generate-pdf-document-outline",
          `--user-data-dir=${path.join(workDir, "chrome-profile")}`,
          `--print-to-pdf=${outFile}`,
          pathToFileURL(htmlFile).href,
        ],
        { stdio: ["ignore", "ignore", "ignore"], timeout: 120_000 },
      );
    },
  },
  coretext: {
    available: Boolean(swift),
    unavailableReason: "needs macOS with the Swift toolchain",
    run(content, outFile) {
      execFileSync(
        swift,
        [
          path.join(HERE, "producers", "coretext.swift"),
          path.join(HERE, "content", `${content.id}.json`),
          path.join(HERE, "fonts.json"),
          FONTS_DIR,
          outFile,
        ],
        { stdio: ["ignore", "inherit", "inherit"], timeout: 300_000 },
      );
    },
  },
};

// ---------------------------------------------------------------- helpers

const sha256 = (file) => createHash("sha256").update(readFileSync(file)).digest("hex");

function qpdfCheck(file, password) {
  if (!qpdf) return null;
  try {
    execFileSync(qpdf, [...(password ? [`--password=${password}`] : []), "--check", file], {
      stdio: ["ignore", "ignore", "ignore"],
    });
    return true;
  } catch (error) {
    // qpdf exits 3 for warnings-only, which still means a readable file.
    return error.status === 3;
  }
}

function pageCount(file, password) {
  if (!qpdf) return null;
  const out = execFileSync(
    qpdf,
    [...(password ? [`--password=${password}`] : []), "--show-npages", file],
    { encoding: "utf8" },
  );
  return Number.parseInt(out.trim(), 10);
}

// ---------------------------------------------------------------- main

const contents = loadContents().filter((content) => !onlyId || content.id === onlyId);

const workDir = path.join(tmpdir(), `safha-fixtures-${process.pid}`);
mkdirSync(workDir, { recursive: true });
if (!onlyId && !onlyProducer) rmSync(OUT, { recursive: true, force: true });

const entries = [];
const skipped = [];
let failed = false;

for (const [name, producer] of Object.entries(PRODUCERS)) {
  if (onlyProducer && onlyProducer !== name) continue;
  if (!producer.available) {
    skipped.push({ producer: name, reason: producer.unavailableReason });
    console.warn(`skip  ${name}: ${producer.unavailableReason}`);
    continue;
  }
  mkdirSync(path.join(OUT, name), { recursive: true });
  for (const content of contents) {
    const outFile = path.join(OUT, name, `${content.id}.pdf`);
    try {
      producer.run(content, outFile, workDir);
      if (!existsSync(outFile)) throw new Error("producer wrote no file");
      const valid = qpdfCheck(outFile);
      if (valid === false) throw new Error("qpdf --check failed");
      entries.push({
        id: content.id,
        producer: name,
        kind: "source",
        path: path.relative(OUT, outFile).split(path.sep).join("/"),
        pages: pageCount(outFile),
        sha256: sha256(outFile),
        description: content.description,
        covers: content.covers,
        expected: expectedBlocks(content),
      });
      console.log(`ok    ${name}/${content.id}.pdf`);
    } catch (error) {
      failed = true;
      console.error(`FAIL  ${name}/${content.id}: ${error.message}`);
    }
  }
}

// Derived fixtures: structural variants of files produced above.
const base = (id) => entries.find((entry) => entry.id === id && entry.producer === "chrome");
if (qpdf && !onlyId && base("en-paragraphs") && base("contract-two-column") && base("mixed-inline")) {
  mkdirSync(path.join(OUT, "derived"), { recursive: true });
  const src = (id) => path.join(OUT, base(id).path);
  const derived = (name) => path.join(OUT, "derived", name);
  const add = (name, extra) => {
    entries.push({
      id: name.replace(/\.pdf$/, ""),
      producer: "qpdf",
      kind: "derived",
      path: `derived/${name}`,
      sha256: sha256(derived(name)),
      ...extra,
    });
    console.log(`ok    derived/${name}`);
  };

  execFileSync(qpdf, ["--linearize", src("en-paragraphs"), derived("linearized.pdf")]);
  add("linearized.pdf", {
    pages: 1,
    description: "Linearized (fast web view) copy of chrome/en-paragraphs.",
    expected: base("en-paragraphs").expected,
  });

  execFileSync(qpdf, [
    "--encrypt",
    ENCRYPTED_PASSWORD,
    ENCRYPTED_PASSWORD,
    "256",
    "--",
    src("en-paragraphs"),
    derived("encrypted.pdf"),
  ]);
  add("encrypted.pdf", {
    pages: 1,
    password: ENCRYPTED_PASSWORD,
    description: "AES-256 encrypted copy of chrome/en-paragraphs; needs the password to open.",
    expected: base("en-paragraphs").expected,
  });

  // 1,000 pages alternating the bilingual contract and the mixed-direction page:
  // the Milestone 1 exit-test document.
  // A two-page file, then its page range repeated 500 times in one argument:
  // one file argument per page overflowed Windows' command line, and an
  // `@file` argument list is not read by every qpdf build (Ubuntu's).
  const pair = derived("bilingual-pair.tmp.pdf");
  execFileSync(qpdf, ["--empty", "--pages", src("contract-two-column"), "1", src("mixed-inline"), "1", "--", pair]);
  const range = (pairs) => Array.from({ length: pairs }, () => "1-2").join(",");
  execFileSync(qpdf, ["--empty", "--pages", pair, range(500), "--", derived("bilingual-1000-pages.pdf")]);
  // The spec's other sizes (section 11): the first page of a 100-page text
  // PDF, and search in 500 pages.
  execFileSync(qpdf, ["--empty", "--pages", pair, range(250), "--", derived("bilingual-500-pages.pdf")]);
  execFileSync(qpdf, ["--empty", "--pages", pair, range(50), "--", derived("bilingual-100-pages.pdf")]);
  rmSync(pair);
  for (const pages of [1000, 500, 100]) {
    const name = `bilingual-${pages}-pages.pdf`;
    add(name, {
      pages: pageCount(derived(name)),
      description: `${pages.toLocaleString("en")} pages alternating chrome/contract-two-column and chrome/mixed-inline.`,
    });
  }

  // Page labels: roman front matter, then arabic numbering from 1.
  const three = derived("three-pages.tmp.pdf");
  execFileSync(qpdf, [
    "--empty",
    "--pages",
    src("en-paragraphs"),
    src("mixed-inline"),
    src("contract-two-column"),
    "--",
    three,
  ]);
  execFileSync(qpdf, [three, "--set-page-labels", "1:r", "2:D", "--", derived("page-labels.pdf")]);
  rmSync(three);
  add("page-labels.pdf", {
    pages: 3,
    pageLabels: ["i", "1", "2"],
    description: "Three pages labelled i, 1, 2 (qpdf --set-page-labels).",
  });

  // The same page four times with /Rotate 0, 90, 180, 270: text geometry must
  // follow the rotation onto the displayed page (selection, search highlights).
  const quarter = src("mixed-inline");
  execFileSync(qpdf, [
    "--empty",
    "--pages",
    quarter,
    quarter,
    quarter,
    quarter,
    "--",
    "--rotate=+90:2",
    "--rotate=+180:3",
    "--rotate=+270:4",
    derived("rotated.pdf"),
  ]);
  add("rotated.pdf", {
    pages: 4,
    rotations: [0, 90, 180, 270],
    description: "chrome/mixed-inline four times, rotated 0, 90, 180 and 270 degrees.",
  });

  // An Arabic "book" that states its own reading preferences in its catalog:
  // /ViewerPreferences /Direction /R2L (bound on the right) and /PageLayout
  // /TwoPageRight (spreads with the first page alone). Edited through qpdf's
  // JSON form; a second copy keeps the catalog in a compressed object stream
  // indexed by a cross-reference stream.
  if (["ar-plain", "ar-sans", "ar-vocalized", "ar-lam-alef"].every((id) => base(id))) {
    const book = derived("rtl-book.tmp.pdf");
    execFileSync(qpdf, [
      "--empty",
      "--pages",
      ...["ar-plain", "ar-sans", "ar-vocalized", "ar-lam-alef"].map(src),
      "--",
      book,
    ]);
    const json = JSON.parse(
      execFileSync(qpdf, ["--json-output=2", "--json-stream-data=inline", "--json-key=qpdf", book], {
        encoding: "utf8",
        maxBuffer: 256 * 1024 * 1024,
      }),
    );
    const objects = json.qpdf[1];
    const catalog = objects[`obj:${objects.trailer.value["/Root"]}`].value;
    catalog["/ViewerPreferences"] = { "/Direction": "/R2L" };
    catalog["/PageLayout"] = "/TwoPageRight";
    const edited = derived("rtl-book.tmp.json");
    writeFileSync(edited, JSON.stringify(json));
    execFileSync(qpdf, ["--json-input", edited, derived("rtl-book.pdf")]);
    execFileSync(qpdf, ["--object-streams=generate", derived("rtl-book.pdf"), derived("rtl-book-objstm.pdf")]);
    rmSync(book);
    rmSync(edited);
    const hints = { direction: "R2L", pageLayout: "TwoPageRight" };
    add("rtl-book.pdf", {
      pages: 4,
      catalog: hints,
      description: "Four Arabic pages whose catalog asks for right-to-left binding and two-page spreads.",
    });
    add("rtl-book-objstm.pdf", {
      pages: 4,
      catalog: hints,
      description: "rtl-book.pdf with its catalog in an object stream (qpdf --object-streams=generate).",
    });
  }

  // A file cut off mid-stream: opening it must fail cleanly or recover, never crash.
  const whole = readFileSync(src("mixed-inline"));
  writeFileSync(derived("truncated.pdf"), whole.subarray(0, Math.floor(whole.length * 0.6)));
  add("truncated.pdf", {
    damaged: true,
    description: "chrome/mixed-inline cut off at 60% of its length (no xref table or trailer).",
  });
} else if (!onlyId) {
  skipped.push({ producer: "qpdf", reason: "derived fixtures need qpdf and the chrome producer" });
  console.warn("skip  derived fixtures: need qpdf and the chrome producer");
}

rmSync(workDir, { recursive: true, force: true });

if (!onlyId && !onlyProducer) {
  const manifest = {
    note: "Generated by fixtures/generate.mjs. Do not edit; do not commit.",
    fonts: Object.fromEntries(Object.entries(FONTS).map(([key, { family }]) => [key, family])),
    skipped,
    fixtures: entries,
  };
  mkdirSync(OUT, { recursive: true });
  writeFileSync(path.join(OUT, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(`\n${entries.length} fixtures -> ${path.relative(process.cwd(), OUT)}/manifest.json`);
}

if (failed) process.exit(1);
