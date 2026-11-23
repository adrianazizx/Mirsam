# Spike: tile transport (Milestone 0)

Question, method, results, and decision: [docs/adr/0004-tile-transport.md](../../docs/adr/0004-tile-transport.md).

- Benchmark (runs inside the webview): `apps/desktop/src/spikes/tileTransport.ts`
- Transports under test (Rust, `devtools` feature only): `apps/desktop/src-tauri/src/spike_tiles.rs`
- Raw report behind the ADR's tables: `results-2026-11-22-macos-arm64.json`

Re-run (needed on Windows and Linux before M1 closes):

```bash
cargo build -p mirsam-desktop --features devtools
node fixtures/generate.mjs
scripts/spike-tile-transport.sh        # writes spikes/out/tile-transport/report.json
```

Keep the app window visible while it runs: an occluded window gets no animation frames, which
invalidates the frame-pacing numbers (the report records `visibility`).
