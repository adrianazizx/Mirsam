#!/usr/bin/env node
// Mirsam's version lives in four places that must agree (docs/RELEASING.md):
//   apps/desktop/src-tauri/tauri.conf.json  the installers' and the About box's version
//   apps/desktop/package.json
//   Cargo.toml [workspace.package]          the crates that take version.workspace
//   Cargo.lock                              those crates' entries (CI builds --locked)
//
// Usage: node scripts/set-version.mjs <version>   set it everywhere
//        node scripts/set-version.mjs --check     print it, or fail if they disagree
//
// A version is MAJOR.MINOR.PATCH, or MAJOR.MINOR.PATCH-N for a pre-release:
// Windows Installer (MSI) takes a numeric pre-release only, at most 65535.

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TAURI_CONF = path.join(ROOT, "apps/desktop/src-tauri/tauri.conf.json");
const PACKAGE_JSON = path.join(ROOT, "apps/desktop/package.json");
const CARGO_TOML = path.join(ROOT, "Cargo.toml");
const CARGO_LOCK = path.join(ROOT, "Cargo.lock");
const VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(0|[1-9]\d*))?$/;
/** `version = "…"` inside [workspace.package]. */
const WORKSPACE_VERSION = /(\[workspace\.package\][^[]*?\nversion = ")([^"]*)(")/;

const arg = process.argv[2];
if (!arg) fail("usage: node scripts/set-version.mjs <version> | --check");

if (arg === "--check") {
  const found = versions();
  const distinct = [...new Set(Object.values(found))];
  if (distinct.length !== 1) {
    fail(`versions disagree:\n${Object.entries(found).map(([where, v]) => `  ${where}: ${v}`).join("\n")}\n` +
      "Set one with node scripts/set-version.mjs <version>.");
  }
  if (!VERSION.test(distinct[0])) fail(`${distinct[0]} is not MAJOR.MINOR.PATCH or MAJOR.MINOR.PATCH-N`);
  console.log(distinct[0]);
  process.exit(0);
}

const version = arg.replace(/^v/, "");
if (!VERSION.test(version)) {
  fail(`${arg} is not MAJOR.MINOR.PATCH or MAJOR.MINOR.PATCH-N (Windows Installer takes numeric pre-releases only)`);
}
const preRelease = version.split("-")[1];
if (preRelease !== undefined && Number(preRelease) > 65535) fail("a pre-release number is at most 65535 (Windows Installer)");

editJson(TAURI_CONF, (config) => {
  config.version = version;
});
editJson(PACKAGE_JSON, (pkg) => {
  pkg.version = version;
});
const cargoToml = fs.readFileSync(CARGO_TOML, "utf8");
if (!WORKSPACE_VERSION.test(cargoToml)) fail("no version in Cargo.toml's [workspace.package]");
fs.writeFileSync(CARGO_TOML, cargoToml.replace(WORKSPACE_VERSION, `$1${version}$3`));
// Rewrites only the workspace's own entries in Cargo.lock; no dependency moves.
const cargo = spawnSync("cargo", ["update", "--workspace", "--offline"], { cwd: ROOT, stdio: "inherit" });
if (cargo.error || cargo.status !== 0) fail("cargo update --workspace failed; Cargo.lock still has the old version");

const after = versions();
const wrong = Object.entries(after).filter(([, v]) => v !== version);
if (wrong.length > 0) fail(`still not ${version}: ${wrong.map(([where, v]) => `${where} (${v})`).join(", ")}`);
console.log(`Mirsam ${version}: ${Object.keys(after).join(", ")}`);

function versions() {
  const found = {
    "tauri.conf.json": readJson(TAURI_CONF).version,
    "package.json": readJson(PACKAGE_JSON).version,
    "Cargo.toml": fs.readFileSync(CARGO_TOML, "utf8").match(WORKSPACE_VERSION)?.[2],
  };
  const lock = fs.readFileSync(CARGO_LOCK, "utf8");
  for (const name of workspaceVersionedCrates()) {
    const entry = lock.match(new RegExp(`\\[\\[package\\]\\]\\nname = "${name}"\\nversion = "([^"]*)"`));
    found[`Cargo.lock ${name}`] = entry?.[1];
  }
  return found;
}

/** Workspace members whose Cargo.toml takes the workspace's version. */
function workspaceVersionedCrates() {
  const members = fs.readFileSync(CARGO_TOML, "utf8").match(/^members = \[([^\]]*)\]/m)?.[1] ?? "";
  return [...members.matchAll(/"([^"]+)"/g)]
    .map((m) => path.join(ROOT, m[1], "Cargo.toml"))
    .filter((file) => fs.existsSync(file) && /^version\.workspace = true$/m.test(fs.readFileSync(file, "utf8")))
    .map((file) => fs.readFileSync(file, "utf8").match(/^name = "([^"]+)"/m)?.[1])
    .filter(Boolean);
}

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

/** Keeps the file's own formatting: two-space JSON with a final newline. */
function editJson(file, edit) {
  const value = readJson(file);
  edit(value);
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`);
}

function fail(message) {
  console.error(`set-version: ${message}`);
  process.exit(1);
}
