#!/usr/bin/env node
// Builds Mirsam's installers for one target with the pinned PDFium inside, then
// opens every installer it produced and checks it (docs/adr/0060-bundling-pdfium.md):
//
// - PDFium is where the engine looks for it (crates/safha-core/src/engine.rs):
//     macOS    Mirsam.app/Contents/Frameworks/libpdfium.dylib
//     Windows  pdfium.dll beside the executable
//     Linux    /usr/lib/<productName>/libpdfium.so (deb, rpm, AppImage)
// - it is the pinned build, byte for byte, for the target's architectures;
// - PDFium's licence notices travel with it;
// - the installer is within the spec's 60 MB budget (section 11, no OCR models).
//
// Usage: node scripts/bundle.mjs [--target <triple>] [--no-build] [<tauri build args>…]
//   --target    aarch64-apple-darwin | x86_64-apple-darwin | universal-apple-darwin |
//               x86_64-pc-windows-msvc | aarch64-pc-windows-msvc |
//               x86_64-unknown-linux-gnu | aarch64-unknown-linux-gnu
//               (default: the host; the output then goes to target/release/bundle)
//   --no-build  only check the installers already built for the target
//   anything else goes to `tauri build`, e.g. --features devtools, --bundles app,deb
//
// Writes <bundle dir>/bundle-report.json. Exits non-zero if a check fails.
//
// `tauri build` on its own refuses to bundle (the before-bundle hook below):
// without this script's overlay the installer would carry no PDFium.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP_DIR = path.join(ROOT, "apps/desktop");
const TAURI_DIR = path.join(APP_DIR, "src-tauri");
/** Spec section 11: installer size without OCR models ≤ 60 MB. */
const BUDGET_BYTES = 60_000_000;
/**
 * Installers reported against the budget without failing the build, and why.
 * An AppImage carries WebKitGTK and GTK, which every other installer takes
 * from the system. The owner decided (2026-11-23) to ship it alongside the
 * deb and rpm; the spec's budget excludes a bundled web engine.
 */
const BUDGET_EXCEPTIONS = {
  appimage: "carries WebKitGTK and GTK, which the other installers take from the system",
};

const TARGETS = {
  "aarch64-apple-darwin": { os: "macos", pdfium: "mac-arm64", archs: ["arm64"] },
  "x86_64-apple-darwin": { os: "macos", pdfium: "mac-x64", archs: ["x86_64"] },
  "universal-apple-darwin": { os: "macos", pdfium: "mac-universal", archs: ["arm64", "x86_64"] },
  "x86_64-pc-windows-msvc": { os: "windows", pdfium: "win-x64" },
  "aarch64-pc-windows-msvc": { os: "windows", pdfium: "win-arm64" },
  "x86_64-unknown-linux-gnu": { os: "linux", pdfium: "linux-x64" },
  "aarch64-unknown-linux-gnu": { os: "linux", pdfium: "linux-arm64" },
};
/** The library inside each pdfium-binaries package. */
const LIBRARY = {
  macos: "lib/libpdfium.dylib",
  windows: "bin/pdfium.dll",
  linux: "lib/libpdfium.so",
};

// ------------------------------------------------------------------ guard

if (process.argv[2] === "--guard") {
  // build.beforeBundleCommand in tauri.conf.json: runs inside `tauri build`.
  const library = process.env.SAFHA_BUNDLE_PDFIUM;
  if (!library || !fs.existsSync(library)) {
    console.error(
      "Refusing to bundle without PDFium. Build installers with `node scripts/bundle.mjs` " +
        "(it adds the pinned PDFium to the bundle; docs/adr/0060-bundling-pdfium.md).",
    );
    process.exit(1);
  }
  process.exit(0);
}

// ------------------------------------------------------------------ arguments

const args = process.argv.slice(2);
let target = null;
let build = true;
const tauriArgs = [];
for (let i = 0; i < args.length; i++) {
  if (args[i] === "--target") target = args[++i];
  else if (args[i].startsWith("--target=")) target = args[i].slice("--target=".length);
  else if (args[i] === "--no-build") build = false;
  else tauriArgs.push(args[i]);
}
const explicitTarget = target !== null;
target ??= hostTriple();
const spec = TARGETS[target];
if (!spec) fail(`unsupported target ${target}; one of: ${Object.keys(TARGETS).join(", ")}`);
const profile = tauriArgs.includes("--debug") ? "debug" : "release";
const targetDir = process.env.CARGO_TARGET_DIR
  ? path.resolve(process.env.CARGO_TARGET_DIR)
  : path.join(ROOT, "target");
const bundleDir = path.join(targetDir, ...(explicitTarget ? [target] : []), profile, "bundle");

const tauriConfig = readJson(path.join(TAURI_DIR, "tauri.conf.json"));
const platformConfig = readJson(path.join(TAURI_DIR, `tauri.${spec.os}.conf.json`), {});
const productName = tauriConfig.productName;
if (spec.os === "linux") {
  // The engine looks in /usr/lib/<productName> by a name of its own.
  const engine = fs.readFileSync(path.join(ROOT, "crates/safha-core/src/engine.rs"), "utf8");
  const dir = engine.match(/LINUX_RESOURCE_DIR: &str = "([^"]*)"/)?.[1];
  if (dir !== productName) fail(`LINUX_RESOURCE_DIR in engine.rs is ${dir}; productName is ${productName}`);
}
const pdfiumDir = path.join(ROOT, "third_party/pdfium", spec.pdfium);
const library = path.join(pdfiumDir, LIBRARY[spec.os]);

// ------------------------------------------------------------------ build

if (build) {
  // A relative path with forward slashes: Git Bash on Windows cannot take the
  // script's directory from a backslashed one.
  run("bash", ["scripts/fetch-pdfium.sh", spec.pdfium], { cwd: ROOT });
  const overlay = overlayConfig();
  const overlayPath = path.join(targetDir, "bundle-config", `${target}.json`);
  fs.mkdirSync(path.dirname(overlayPath), { recursive: true });
  fs.writeFileSync(overlayPath, `${JSON.stringify(overlay, null, 2)}\n`);
  console.log(`bundle: config overlay ${overlayPath}\n${JSON.stringify(overlay, null, 2)}`);
  // Old installers (another version, another bundle type) would be checked too.
  fs.rmSync(bundleDir, { recursive: true, force: true });
  const cli = path.join(APP_DIR, "node_modules/@tauri-apps/cli/tauri.js");
  if (!fs.existsSync(cli)) fail(`missing ${cli}: run pnpm install in apps/desktop`);
  run(
    process.execPath,
    [cli, "build", ...(explicitTarget ? ["--target", target] : []), "--config", overlayPath, ...tauriArgs],
    { cwd: APP_DIR, env: { ...process.env, SAFHA_BUNDLE_PDFIUM: library, SAFHA_BUILD_COMMIT: buildCommit() } },
  );
}

/** The commit being built, for crash reports (ADR 0061); the environment's when set. */
function buildCommit() {
  if (process.env.SAFHA_BUILD_COMMIT) return process.env.SAFHA_BUILD_COMMIT;
  const git = spawnSync("git", ["rev-parse", "--short=12", "HEAD"], { cwd: ROOT, encoding: "utf8" });
  return git.status === 0 ? git.stdout.trim() : "";
}

/** The configuration `tauri build` merges over tauri.conf.json (JSON merge patch). */
function overlayConfig() {
  const rel = (file) => path.relative(TAURI_DIR, file).split(path.sep).join("/");
  // Resources are a map here; a list in the base config would be replaced
  // wholesale by the merge, silently dropping its entries.
  for (const config of [tauriConfig, platformConfig]) {
    if (Array.isArray(config.bundle?.resources) && config.bundle.resources.length > 0) {
      fail("bundle.resources must be a map (source → destination) for scripts/bundle.mjs to add to it");
    }
  }
  const resources = { [rel(path.join(pdfiumDir, "LICENSE"))]: "licenses/pdfium/LICENSE" };
  resources[rel(path.join(pdfiumDir, "licenses"))] = "licenses/pdfium";
  if (spec.os === "macos") {
    const frameworks = platformConfig.bundle?.macOS?.frameworks ?? tauriConfig.bundle?.macOS?.frameworks ?? [];
    return { bundle: { resources, macOS: { frameworks: [...frameworks, rel(library)] } } };
  }
  // Windows: resources sit beside the executable. Linux: in /usr/lib/<productName>.
  resources[rel(library)] = path.basename(library);
  return { bundle: { resources } };
}

// ------------------------------------------------------------------ checks

const expectedHash = sha256(library);
const report = { target, productName, pdfium: rel(library), pdfiumSha256: expectedHash, artifacts: [] };
const problems = [];
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "mirsam-bundle-"));
try {
  for (const artifact of artifactsIn(bundleDir)) {
    const entry = { file: path.relative(ROOT, artifact.file), kind: artifact.kind };
    entry.bytes = artifact.directory ? directorySize(artifact.file) : fs.statSync(artifact.file).size;
    entry.mib = Number((entry.bytes / 2 ** 20).toFixed(1));
    if (!artifact.directory) {
      entry.withinBudget = entry.bytes <= BUDGET_BYTES;
      const exception = BUDGET_EXCEPTIONS[artifact.kind];
      if (!entry.withinBudget && exception) entry.budgetException = exception;
      else if (!entry.withinBudget) problems.push(`${entry.file}: ${entry.mib} MiB is over the 60 MB budget`);
    }
    const inspected = inspect(artifact);
    Object.assign(entry, inspected);
    if (inspected.problem) problems.push(`${entry.file}: ${inspected.problem}`);
    report.artifacts.push(entry);
  }
} finally {
  fs.rmSync(scratch, { recursive: true, force: true });
}
if (report.artifacts.length === 0) problems.push(`no installers in ${bundleDir}`);
report.ok = problems.length === 0;
report.problems = problems;
if (fs.existsSync(bundleDir)) fs.writeFileSync(path.join(bundleDir, "bundle-report.json"), `${JSON.stringify(report, null, 2)}\n`);

console.log(`\nbundle: ${target}, PDFium ${spec.pdfium} (sha256 ${expectedHash.slice(0, 12)}…)`);
for (const a of report.artifacts) {
  const size = `${a.mib.toFixed(1).padStart(6)} MiB`;
  const budget =
    a.withinBudget === undefined
      ? ""
      : a.withinBudget
        ? " within budget"
        : a.budgetException
          ? ` over budget (known: ${a.budgetException})`
          : " OVER BUDGET";
  const archs = a.archs ? ` (${a.archs.join(", ")})` : "";
  console.log(`  ${size}${budget}  ${a.file}\n           PDFium: ${a.pdfium ?? "-"}${archs}${a.note ? `; ${a.note}` : ""}`);
}
if (problems.length > 0) {
  console.error(`\nbundle: ${problems.length} problem(s):\n  ${problems.join("\n  ")}`);
  process.exit(1);
}
console.log("bundle: every installer carries the pinned PDFium and fits the budget");

/** Installers and app bundles that `tauri build` wrote. */
function artifactsIn(dir) {
  const found = [];
  const list = (sub, test) => {
    const d = path.join(dir, sub);
    if (!fs.existsSync(d)) return [];
    return fs.readdirSync(d).filter(test).map((name) => path.join(d, name));
  };
  for (const file of list("macos", (n) => n.endsWith(".app"))) found.push({ kind: "app", file, directory: true });
  for (const file of list("dmg", (n) => n.endsWith(".dmg"))) found.push({ kind: "dmg", file });
  for (const file of list("deb", (n) => n.endsWith(".deb"))) found.push({ kind: "deb", file });
  for (const file of list("rpm", (n) => n.endsWith(".rpm"))) found.push({ kind: "rpm", file });
  for (const file of list("appimage", (n) => n.endsWith(".AppImage"))) found.push({ kind: "appimage", file });
  for (const file of list("nsis", (n) => n.endsWith(".exe"))) found.push({ kind: "nsis", file });
  for (const file of list("msi", (n) => n.endsWith(".msi"))) found.push({ kind: "msi", file });
  return found;
}

/**
 * Looks inside one artifact for PDFium and its notices. Returns what it found;
 * `problem` is set when something is missing or wrong, `pdfium: null` with a
 * `note` when this machine has no tool to look inside.
 */
function inspect(artifact) {
  const dir = fs.mkdtempSync(path.join(scratch, `${artifact.kind}-`));
  switch (artifact.kind) {
    case "app":
      return checkRoot(artifact.file, "Contents/Frameworks/libpdfium.dylib", "Contents/Resources");
    case "dmg": {
      if (!have("hdiutil")) return { pdfium: null, note: "hdiutil not available" };
      const mount = path.join(dir, "mount");
      fs.mkdirSync(mount);
      run("hdiutil", ["attach", "-nobrowse", "-readonly", "-noautoopen", "-mountpoint", mount, artifact.file], {
        quiet: true,
      });
      try {
        const app = fs.readdirSync(mount).find((n) => n.endsWith(".app"));
        if (!app) return { problem: "no .app inside the disk image" };
        return checkRoot(path.join(mount, app), "Contents/Frameworks/libpdfium.dylib", "Contents/Resources");
      } finally {
        run("hdiutil", ["detach", mount, "-quiet"], { quiet: true, allowFailure: true });
      }
    }
    case "deb":
      if (!have("dpkg-deb")) return { pdfium: null, note: "dpkg-deb not available" };
      run("dpkg-deb", ["-x", artifact.file, dir], { quiet: true });
      return checkRoot(dir, `usr/lib/${productName}/libpdfium.so`, `usr/lib/${productName}`, "usr/bin");
    case "rpm": {
      // rpm2cpio | cpio where they work, else libarchive (bsdtar reads rpms).
      let unpacked = false;
      const errors = [];
      if (have("rpm2cpio") && have("cpio")) {
        const cpio = spawnSync("rpm2cpio", [artifact.file], { maxBuffer: 1 << 30 });
        const unpack =
          cpio.status === 0 ? spawnSync("cpio", ["-idm", "--quiet"], { cwd: dir, input: cpio.stdout }) : cpio;
        unpacked = unpack.status === 0;
        if (!unpacked) errors.push(`rpm2cpio/cpio: ${String(unpack.stderr).trim() || `exit ${unpack.status}`}`);
      }
      if (!unpacked && have("bsdtar")) {
        const tar = spawnSync("bsdtar", ["-xf", artifact.file, "-C", dir], { encoding: "utf8" });
        unpacked = tar.status === 0;
        if (!unpacked) errors.push(`bsdtar: ${tar.stderr.trim()}`);
      }
      if (!unpacked) {
        return errors.length > 0 ? { problem: `could not unpack: ${errors.join("; ")}` } : { pdfium: null, note: "no rpm2cpio or bsdtar" };
      }
      return checkRoot(dir, `usr/lib/${productName}/libpdfium.so`, `usr/lib/${productName}`, "usr/bin");
    }
    case "appimage": {
      if (process.platform !== "linux") return { pdfium: null, note: "AppImages are inspected on Linux" };
      fs.chmodSync(artifact.file, 0o755);
      // Extraction needs no FUSE.
      run(artifact.file, ["--appimage-extract"], { cwd: dir, quiet: true });
      // linuxdeploy rewrites every library it bundles (run path, symbols), so
      // the AppImage's copy is checked as the same build, not the same bytes.
      return checkRoot(path.join(dir, "squashfs-root"), `usr/lib/${productName}/libpdfium.so`, `usr/lib/${productName}`, "usr/bin", {
        sameBuild: true,
      });
    }
    case "msi": {
      if (process.platform !== "win32") return { pdfium: null, note: "MSIs are inspected on Windows" };
      // An administrative installation only unpacks the files.
      const log = path.join(dir, "msiexec.log");
      run("msiexec", ["/a", artifact.file, "/qn", `TARGETDIR=${path.join(dir, "msi")}`, "/l*", log], { quiet: true });
      return checkBesideExe(path.join(dir, "msi"));
    }
    case "nsis": {
      const sevenZip = ["7z", "C:\\Program Files\\7-Zip\\7z.exe"].find(have);
      if (!sevenZip) return { pdfium: null, note: "7-Zip not available" };
      run(sevenZip, ["x", "-y", `-o${path.join(dir, "nsis")}`, artifact.file], { quiet: true });
      return checkBesideExe(path.join(dir, "nsis"));
    }
    default:
      return { pdfium: null };
  }
}

/**
 * PDFium at `libraryPath` under `root`, notices under `resources`, executables
 * under `bin`. `sameBuild`: a rewritten copy passes if its ELF build ID is the
 * pinned library's.
 */
function checkRoot(root, libraryPath, resources, bin = null, { sameBuild = false } = {}) {
  const found = path.join(root, libraryPath);
  const result = { pdfium: libraryPath };
  if (!fs.existsSync(found)) return { ...result, problem: `PDFium missing at ${libraryPath}` };
  const problem = [];
  if (sha256(found) !== expectedHash) {
    const id = sameBuild ? elfBuildId(found) : null;
    const signed = spec.os === "macos" ? resignedSameBuild(found) : null;
    if (id && id === elfBuildId(library)) result.note = `rewritten by the bundler; same build (build ID ${id})`;
    else if (signed) result.note = `re-signed (${signed.signer}); same build (UUID ${signed.uuids.join(", ")})`;
    else problem.push("PDFium differs from the pinned build");
  }
  if (spec.archs) {
    result.archs = archs(found);
    if (result.archs.join() !== [...spec.archs].sort().join()) {
      problem.push(`PDFium is built for ${result.archs.join(", ")}, expected ${spec.archs.join(", ")}`);
    }
    const exes = path.join(root, "Contents/MacOS");
    for (const exe of fs.existsSync(exes) ? fs.readdirSync(exes) : []) {
      const exeArchs = archs(path.join(exes, exe));
      if (exeArchs.join() !== [...spec.archs].sort().join()) {
        problem.push(`${exe} is built for ${exeArchs.join(", ")}, expected ${spec.archs.join(", ")}`);
      }
    }
  }
  if (!fs.existsSync(path.join(root, resources, "licenses/pdfium/LICENSE"))) {
    problem.push("PDFium's licence notices are missing");
  }
  if (bin && !fs.existsSync(path.join(root, bin))) problem.push(`no ${bin}`);
  return problem.length > 0 ? { ...result, problem: problem.join("; ") } : result;
}

/** Windows installers: pdfium.dll in the same directory as the app's executable. */
function checkBesideExe(root) {
  const exe = walk(root).find((file) => /safha-desktop\.exe$|mirsam\.exe$/i.test(file));
  if (!exe) return { problem: "no application executable inside" };
  const dir = path.dirname(exe);
  return checkRoot(dir, "pdfium.dll", ".");
}

// ------------------------------------------------------------------ helpers

/** The GNU build ID of a 64-bit little-endian ELF file, from its PT_NOTE segments. */
function elfBuildId(file) {
  const b = fs.readFileSync(file);
  if (b.length < 64 || b.toString("latin1", 0, 4) !== "\x7fELF" || b[4] !== 2 || b[5] !== 1) return null;
  const phoff = Number(b.readBigUInt64LE(0x20));
  const phentsize = b.readUInt16LE(0x36);
  const phnum = b.readUInt16LE(0x38);
  for (let i = 0; i < phnum; i++) {
    const header = phoff + i * phentsize;
    if (header + 0x28 > b.length || b.readUInt32LE(header) !== 4) continue; // PT_NOTE
    let at = Number(b.readBigUInt64LE(header + 8));
    const end = Math.min(b.length, at + Number(b.readBigUInt64LE(header + 0x20)));
    while (at + 12 <= end) {
      const nameSize = b.readUInt32LE(at);
      const descSize = b.readUInt32LE(at + 4);
      const type = b.readUInt32LE(at + 8);
      const desc = at + 12 + ((nameSize + 3) & ~3);
      if (type === 3 && b.toString("latin1", at + 12, at + 12 + nameSize) === "GNU\0") {
        return b.toString("hex", desc, desc + descSize);
      }
      at = desc + ((descSize + 3) & ~3);
    }
  }
  return null;
}

/**
 * A Developer ID build signs PDFium again (release.yml), which rewrites its
 * signature and so its bytes. The copy passes as the pinned build when its
 * Mach-O UUIDs (one per architecture, set by the linker, untouched by codesign)
 * are the pinned library's and its new signature verifies, so nothing but the
 * signature changed. Returns the UUIDs and the signer, or null.
 */
function resignedSameBuild(file) {
  if (!have("dwarfdump") || !have("codesign")) return null;
  const uuids = (f) =>
    [...run("dwarfdump", ["--uuid", f], { capture: true }).matchAll(/^UUID: (\S+) \((\S+)\)/gm)]
      .map((m) => `${m[1]} (${m[2]})`)
      .sort();
  const found = uuids(file);
  if (found.length === 0 || found.join() !== uuids(library).join()) return null;
  const verify = spawnSync("codesign", ["--verify", "--strict", file], { stdio: "ignore" });
  if (verify.status !== 0) return null;
  const details = spawnSync("codesign", ["-dv", "--verbose=2", file], { encoding: "utf8" }).stderr ?? "";
  const signer = details.match(/^Authority=(.+)$/m)?.[1] ?? (/Signature=adhoc/.test(details) ? "ad hoc" : "unknown");
  return { uuids: found, signer };
}

function archs(file) {
  if (!have("lipo")) return ["(lipo not available)"];
  return run("lipo", ["-archs", file], { capture: true }).trim().split(/\s+/).sort();
}

function walk(dir) {
  const out = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) out.push(...walk(full));
    else out.push(full);
  }
  return out;
}

function directorySize(dir) {
  return walk(dir).reduce((sum, file) => sum + fs.lstatSync(file).size, 0);
}

function sha256(file) {
  return createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

function rel(file) {
  return path.relative(ROOT, file).split(path.sep).join("/");
}

function hostTriple() {
  const out = run("rustc", ["-vV"], { capture: true, cwd: ROOT });
  const host = out.match(/^host: (\S+)$/m)?.[1];
  if (!host) fail("could not read the host triple from rustc -vV");
  return host;
}

function have(command) {
  if (path.isAbsolute(command)) return fs.existsSync(command);
  const probe = process.platform === "win32" ? "where" : "which";
  return spawnSync(probe, [command], { stdio: "ignore" }).status === 0;
}

function readJson(file, fallback) {
  if (!fs.existsSync(file) && fallback !== undefined) return fallback;
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

function run(command, commandArgs, { cwd, env, capture = false, quiet = false, allowFailure = false } = {}) {
  const result = spawnSync(command, commandArgs, {
    cwd,
    env,
    encoding: capture ? "utf8" : undefined,
    stdio: capture ? ["ignore", "pipe", "inherit"] : quiet ? ["ignore", "ignore", "inherit"] : "inherit",
    maxBuffer: 1 << 26,
  });
  if (result.error) fail(`${command}: ${result.error.message}`);
  if (result.status !== 0 && !allowFailure) fail(`${command} ${commandArgs.join(" ")} exited with ${result.status}`);
  return capture ? result.stdout : "";
}

function fail(message) {
  console.error(`bundle: ${message}`);
  process.exit(1);
}
