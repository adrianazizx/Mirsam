import { defineConfig, passthroughImageService } from "astro/config";

export default defineConfig({
  site: "https://mirsam-pdf.vercel.app",
  output: "static",
  // Screenshots are encoded ahead of time (scripts/encode-screens.sh), so no
  // image pipeline (and no sharp/libvips) runs at build time.
  image: { service: passthroughImageService() },
  devToolbar: { enabled: false },
  build: { inlineStylesheets: "auto" },
  // In a cloud-synced checkout pnpm's store lives outside the project
  // (scripts/dev-setup.sh), so the dev server must be allowed to serve from it.
  vite: { server: { fs: { strict: false } } },
});
