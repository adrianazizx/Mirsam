# 0060 — Bundling PDFium per platform, checked installers, and fuzzing the open path

Status: Accepted. Date: 2026-11-22. Supersedes the bundling note in ADR 0002's consequences.

## Context

PDFium is loaded at run time from a prebuilt, checksum-pinned library (ADR 0002). Until now only
macOS arm64 bundles carried it: `tauri.macos.conf.json` listed
`third_party/pdfium/mac-arm64/lib/libpdfium.dylib` under `bundle.macOS.frameworks`. That static
path had two problems beyond the missing platforms:

- Tauri's build step (`tauri-build`) copies `frameworks` and `resources` at **every** `cargo build`
  and fails when a file is missing. A static path names one architecture, so an Intel Mac (whose
  `fetch-pdfium.sh` fetches `mac-x64`) could not build the app at all, and no cross build could
  bundle the right library.
- Nothing checked that an installer really contained PDFium, or that the installed app loaded
  that copy rather than one found elsewhere.

Spec §11 also asks for fuzzing of the open/parse path and caps installers at 60 MB without OCR
models. Code signing and auto-update are out of scope here (they need the owner's certificates).

## Decision

**One command builds installers:** `node scripts/bundle.mjs [--target <triple>] [tauri args]`.

1. It fetches the pinned PDFium for the target (`scripts/fetch-pdfium.sh`, SHA-256 verified).
   `mac-universal` is a new pseudo-platform: both macOS downloads, each verified, joined with
   `lipo -create`.
2. It writes a **config overlay** for that target (`target/bundle-config/<triple>.json`) and passes
   it to `tauri build --config` (a JSON merge patch over `tauri.conf.json`). The static configs carry
   no PDFium path any more, so `cargo build` and `tauri dev` no longer depend on one (debug builds
   find `third_party/pdfium/<host>` as before).
3. Where PDFium goes, and where the engine (`crates/safha-core/src/engine.rs`) looks:

   | Platform | Tauri mechanism | Installed at | Engine candidate |
   |---|---|---|---|
   | macOS (arm64, x86_64, universal) | `bundle.macOS.frameworks` | `Mirsam.app/Contents/Frameworks/libpdfium.dylib` | `<exe>/../Frameworks` |
   | Windows (NSIS, MSI) | `bundle.resources` (map) | `pdfium.dll` beside `safha-desktop.exe` | `<exe dir>` |
   | Linux (deb, rpm, AppImage) | `bundle.resources` (map) | `/usr/lib/Mirsam/libpdfium.so` (`$APPDIR/usr/lib/Mirsam` in an AppImage) | `<exe>/../lib/Mirsam` (new) |

   Tauri's own bundler puts resources in `usr/lib/<productName>` for all three Linux formats
   (read in tauri-bundler 2.12.6: `debian.rs`, `rpm.rs`, and the AppImage, which reuses the deb's
   data directory), and its runtime resolves its resource directory the same way. The engine's
   name for that directory (`LINUX_RESOURCE_DIR`) must equal `productName`; `bundle.mjs` refuses to
   build a Linux bundle when they differ.
4. **PDFium's licence and its own third-party notices** (`LICENSE`, `licenses/`) ship in every
   installer under `licenses/pdfium/` in the resource directory (BSD-3-Clause requires the notice
   with binaries).
5. **Every installer is opened and checked** after the build: PDFium is at the path above, byte for
   byte the pinned build, built for the target's architectures (`lipo -archs` on macOS, also for
   the app's executable), its notices are present, and the installer is within 60 MB. DMGs are
   mounted, debs and rpms unpacked, AppImages extracted, MSIs unpacked by an administrative
   install, NSIS installers unpacked by 7-Zip. The result is `bundle-report.json` beside them.
6. **A before-bundle hook refuses a bare `tauri build`**, which would now produce an installer
   without PDFium.
7. **The installed app is run:** `scripts/bundle-selftest.sh <installer>` installs or unpacks it
   (NSIS silently into a directory, then uninstalls), runs the end-to-end self-test (ADR 0007) in
   English/LTR and Arabic/RTL, and checks that the PDFium library the process loaded (now in the
   self-test report, with the architecture the process ran as) lies inside what the installer
   delivered. On macOS it also launches the app as `x86_64` under Rosetta: the Intel build, and the
   Intel half of the universal build.
8. **CI** (`.github/workflows/bundle.yml`): one job per target — macOS arm64, x86_64 and universal
   (on the Apple-silicon runner, Intel code under Rosetta), Windows x64, Linux x64 — each building
   with `--features devtools` (the self-test harness) and self-testing every installer. Linux
   packages are built on Ubuntu 22.04 (the oldest supported glibc) and self-tested on the latest
   Ubuntu, whose qpdf can generate the fixtures. Pull requests run it only when packaging changes;
   every push to `main` and "Run workflow" run it in full. It is not a required check.

**Fuzzing** (`fuzz/`, `fuzz/README.md`): cargo-fuzz (libFuzzer with AddressSanitizer) targets
for the open path through PDFium (`open`), every PDFium-free reader in safha-core on a whole file
(`file`), fuzzed objects inside a correct cross-reference table (`objects`), one page's content
stream (`content`) and one ToUnicode CMap (`cmap`), seeded from the fixture corpus. The fuzz crate is
its own Cargo workspace on a pinned nightly; it never ships. `fuzz.yml` runs each target for two
minutes weekly, on demand, and on pull requests that touch the readers.

## Evidence

Measured on an Apple-silicon MacBook (macOS 26) and on GitHub's hosted runners, 2026-11-22.
Installers built with `--features devtools`; sizes in MiB (the budget is 60 MB = 57.2 MiB).

| Target | Installer | Size | PDFium inside | Self-test (en + ar) |
|---|---|---|---|---|
| macOS arm64 | `.dmg` | 10.5 MiB (app 22.6 MiB) | arm64, pinned bytes | passes from the mounted DMG; PDFium loaded from its `Contents/Frameworks` |

Fuzzing, about 75 minutes of CPU per target on the MacBook (five targets, several hundred thousand
executions in all), found **three crashes in safha-core**, all fixed with regression tests:

1. `Parser::word` sliced out of bounds when a `/Prev`, `/XRefStm` or xref entry pointed past the
   end of the file. The catalog reader runs on every open, so a damaged file could crash opening it.
2. A stream whose `/Length` refers to its own object (or to another stream whose length refers
   back) recursed until the stack overflowed. Lengths and filters are now read as values only.
3. Text extraction's left-to-right ordering compares glyph positions with a tolerance, which is not
   transitive; a page whose glyphs make it circular panicked the standard sort ("does not correctly
   implement a total order"). A small stable merge sort now does that ordering: it never panics and
   gives the stable sort's exact result for a proper order (extraction over the fixture corpus is
   byte-identical before and after). Found seven times through `open`.

No crash came from PDFium itself, and none from the content-stream or CMap readers.

## Alternatives considered

- **A static per-platform config pointing at a host-staged copy** (`third_party/pdfium/host/`):
  `pnpm tauri build` would work unchanged on any host, but cross and universal builds need an overlay
  anyway, and a staged copy can silently be the wrong architecture after one.
- **Always bundling the universal dylib on macOS:** one config for every Mac build, but arm64-only
  installers would carry 7.4 MiB of x86_64 code they never run.
- **`deb`/`rpm`/`appimage` `files` entries** to a lower-case `/usr/lib/mirsam`: equivalent, but three
  entries instead of one, and a second directory beside Tauri's own resource directory.
- **Linking PDFium statically:** no prebuilt static build exists for every target; building PDFium
  ourselves means a Chromium toolchain in CI. Not worth it while dynamic loading works.
- **honggfuzz or AFL++ instead of libFuzzer:** both work with Rust, but need extra system packages and
  do not run as easily on macOS, where development happens.

## Consequences

- Build installers with `node scripts/bundle.mjs`; `pnpm tauri build` alone stops at the hook. The
  release scroll benchmark (`scripts/scroll-benchmark.sh … release`) needs
  `node scripts/bundle.mjs --features devtools --bundles app`.
- The pinned-bytes check will need to compare unsigned slices once the owner signs bundles: signing
  rewrites the dylib. Frameworks are signed by Tauri's bundler with the app; `pdfium.dll` as a
  resource will need its own `signCommand` step on Windows.
- Renaming the product means renaming `LINUX_RESOURCE_DIR` with it (the bundle script says so).
- CI bundles are test builds (self-test harness compiled in, unsigned); they are not uploaded as
  installers.
- `libfuzzer-sys` is (MIT OR Apache-2.0) AND NCSA — NCSA is LLVM's BSD-style licence. It is a
  development tool in its own workspace and never ships.
- Fork-mode fuzzing (`-fork=N`) breaks on paths with spaces, such as this OneDrive checkout; run the
  built binaries from the cache directory (fuzz/README.md).
