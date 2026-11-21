import { realpathSync } from "node:fs";
import path from "node:path";
import react from "@vitejs/plugin-react";
import { searchForWorkspaceRoot } from "vite";
import { defineConfig } from "vitest/config";

// When scripts/dev-setup.sh has moved pnpm's virtual store out of a cloud-synced
// checkout, dependency files (fonts, CSS) resolve to real paths outside the
// project, which Vite's dev server refuses to serve unless explicitly allowed.
function externalStoreDirs(): string[] {
  try {
    // <store>/<name>@<version>/node_modules/<name> -> <store>
    const store = path.resolve(realpathSync("node_modules/react"), "../../..");
    return store.startsWith(process.cwd()) ? [] : [store];
  } catch {
    return [];
  }
}

const tauriDevHost = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  // Keep Rust compiler output visible when running under `tauri dev`.
  clearScreen: false,
  server: {
    // Tauri's devUrl expects exactly this port.
    port: 1420,
    strictPort: true,
    host: tauriDevHost || false,
    hmr: tauriDevHost ? { protocol: "ws", host: tauriDevHost, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
    fs: { allow: [searchForWorkspaceRoot(process.cwd()), ...externalStoreDirs()] },
  },
  build: {
    // WKWebView on macOS 13 is Safari 16; WebView2 and WebKitGTK are newer.
    target: "safari16",
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    css: true,
  },
});
