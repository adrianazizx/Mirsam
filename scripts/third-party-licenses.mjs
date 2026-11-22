#!/usr/bin/env node
// Generates THIRD_PARTY_LICENSES.md from what the build actually depends on and
// fails on any licence outside the allowlist (docs/SPEC.md section 10).
//
//   node scripts/third-party-licenses.mjs          write THIRD_PARTY_LICENSES.md
//   node scripts/third-party-licenses.mjs --check  fail if the committed file is stale
//
// Sources: `cargo metadata` (Rust crates, all platforms), `pnpm licenses`
// (npm packages shipped in the app), and the bundled binaries and fonts listed
// in BUNDLED below. The allowlist must match deny.toml.

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const OUTPUT = path.join(ROOT, "THIRD_PARTY_LICENSES.md");

const ALLOWED = new Set([
  "MIT",
  "MIT-0",
  "Apache-2.0",
  "Apache-2.0 WITH LLVM-exception",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "ISC",
  "Zlib",
  "MPL-2.0",
  "Unicode-3.0",
  "CC0-1.0",
  "0BSD",
  "OFL-1.1",
  // SQLite's public-domain dedication. Pending the owner's confirmation (PROGRESS.md).
  "blessing",
]);

/** Components that ship in the app but come from neither Cargo nor npm. */
const BUNDLED = [
  {
    name: "PDFium",
    version: "chromium/8125 (bblanchon/pdfium-binaries)",
    license: "BSD-3-Clause AND Apache-2.0",
    source: "https://pdfium.googlesource.com/pdfium/",
    note: "Its own third-party notices ship in third_party/pdfium/<platform>/licenses/.",
  },
  {
    name: "SQLite",
    version: "bundled by libsqlite3-sys",
    license: "blessing",
    source: "https://sqlite.org/copyright.html",
    note: "Compiled into the app from source by rusqlite's `bundled` feature; public domain.",
  },
  ...[
    ["Amiri", "https://github.com/aliftype/amiri"],
    ["IBM Plex Sans", "https://github.com/IBM/plex"],
    ["IBM Plex Sans Arabic", "https://github.com/IBM/plex"],
    ["Noto Naskh Arabic", "https://github.com/notofonts/arabic"],
    ["Noto Sans Arabic", "https://github.com/notofonts/arabic"],
  ].map(([name, source]) => ({
    name: `${name} (font)`,
    version: "see assets/fonts/README.md",
    license: "OFL-1.1",
    source,
  })),
];

/**
 * True when an SPDX expression can be satisfied using allowed licences only:
 * every AND operand must be allowed, at least one OR operand must be.
 */
function isAllowed(expression) {
  if (!expression) return false;
  const text = expression.replace(/[()]/g, " ").replace(/\s+/g, " ").trim().replace(/\//g, " OR ");
  return text.split(/ OR /i).some((alternative) =>
    alternative.split(/ AND /i).every((licence) => ALLOWED.has(licence.trim())),
  );
}

function rustPackages() {
  const metadata = JSON.parse(
    execFileSync("cargo", ["metadata", "--format-version", "1", "--locked"], {
      cwd: ROOT,
      encoding: "utf8",
      maxBuffer: 256 * 1024 * 1024,
    }),
  );
  const workspace = new Set(metadata.workspace_members);
  return metadata.packages
    .filter((pkg) => !workspace.has(pkg.id))
    .map((pkg) => ({
      name: pkg.name,
      version: pkg.version,
      license: pkg.license ?? "",
      source: pkg.repository ?? `https://crates.io/crates/${pkg.name}`,
    }));
}

function npmPackages() {
  const listing = JSON.parse(
    execFileSync("pnpm", ["licenses", "list", "--json", "--prod"], {
      cwd: path.join(ROOT, "apps", "desktop"),
      encoding: "utf8",
      maxBuffer: 256 * 1024 * 1024,
    }),
  );
  const rows = Object.entries(listing).flatMap(([license, packages]) =>
    packages.flatMap((pkg) =>
      (pkg.versions ?? [pkg.version]).map((version) => ({
        name: platformNeutral(pkg.name),
        version,
        license,
        source: pkg.homepage ?? `https://www.npmjs.com/package/${pkg.name}`,
      })),
    ),
  );
  // One row per package, whichever platform's binary this machine installed.
  const seen = new Set();
  return rows.filter((row) => {
    const key = `${row.name}@${row.version}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

/**
 * Native binaries ship as one package per platform (`@typescript/typescript-
 * darwin-arm64`, `…-linux-x64`, …) and pnpm installs only this machine's. Named
 * neutrally, the report is the same on every OS, so CI can check it anywhere.
 */
function platformNeutral(name) {
  return name.replace(
    /-(darwin|linux|win32|freebsd|android)-(arm64|x64|ia32|arm)(-(gnu|musl|msvc|gnueabihf))?$/,
    "-<platform>",
  );
}

const byName = (a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version);
const sections = [
  ["Bundled binaries and fonts", BUNDLED],
  ["Rust crates", rustPackages().sort(byName)],
  ["npm packages", npmPackages().sort(byName)],
];

const violations = sections.flatMap(([title, packages]) =>
  packages
    .filter((pkg) => !isAllowed(pkg.license))
    .map((pkg) => `${title}: ${pkg.name} ${pkg.version} — "${pkg.license || "no licence declared"}"`),
);
if (violations.length > 0) {
  console.error("Disallowed or missing licences (docs/SPEC.md section 10):");
  for (const violation of violations) console.error(`  ${violation}`);
  process.exit(1);
}

const lines = [
  "# Third-party licences",
  "",
  "Generated by `scripts/third-party-licenses.mjs`. Do not edit by hand.",
  "",
  "Every dependency below is under a licence permitted by the project policy",
  "(docs/SPEC.md section 10, enforced by `deny.toml` and this script).",
];
for (const [title, packages] of sections) {
  lines.push("", `## ${title} (${packages.length})`, "", "| Component | Version | Licence | Source |", "|---|---|---|---|");
  for (const pkg of packages) {
    lines.push(`| ${pkg.name} | ${pkg.version} | ${pkg.license} | ${pkg.source} |`);
  }
  for (const pkg of packages.filter((entry) => entry.note)) {
    lines.push("", `${pkg.name}: ${pkg.note}`);
  }
}
const markdown = `${lines.join("\n")}\n`;

if (process.argv.includes("--check")) {
  let committed = "";
  try {
    committed = readFileSync(OUTPUT, "utf8");
  } catch {
    // Treated as stale below.
  }
  if (committed !== markdown) {
    console.error("THIRD_PARTY_LICENSES.md is out of date: run `node scripts/third-party-licenses.mjs`.");
    process.exit(1);
  }
  console.log("THIRD_PARTY_LICENSES.md is up to date.");
} else {
  writeFileSync(OUTPUT, markdown);
  const total = sections.reduce((sum, [, packages]) => sum + packages.length, 0);
  console.log(`THIRD_PARTY_LICENSES.md: ${total} components, all licences allowed.`);
}
