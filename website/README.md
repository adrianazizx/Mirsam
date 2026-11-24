# Mirsam website

The landing page at <https://mirsam.vercel.app>, in English (`/`) and Arabic (`/ar/`). Static
[Astro](https://astro.build) site, deployed by Vercel from `main` (project root directory:
`website/`). Not part of the app and never shipped with it.

```bash
pnpm install          # in a cloud-synced checkout, run scripts/dev-setup.sh first
pnpm dev              # http://localhost:4321
pnpm test             # English/Arabic string parity
pnpm build            # astro check + static build into dist/
```

- **Strings:** `src/i18n/en.json` and `src/i18n/ar.json` (same keys; the test enforces it).
- **Brand:** the logo SVGs are the repository's `docs/brand/` (`src/generated/brand.json` holds its paths, `public/favicon.svg` is the page alone); colours and type are in `src/styles/tokens.css`
  (indigo ink, lavender, pale lavender; Readex Pro headings, IBM Plex body).
- **Version and downloads:** the version label and installer sizes come from the latest
  *published* GitHub release at build time (falling back to `apps/desktop/package.json`, without
  sizes, if GitHub can't be reached); download links use `releases/latest/download/<asset>`, so
  they never need changing. Publishing a release doesn't rebuild the site: redeploy it on Vercel
  (Deployments → Redeploy) so the label shows the new version.
- **Screenshots:** real captures of the app with the fictional sample document in
  `screenshots/showcase.html` (`screenshots/make-showcase.sh out.pdf`), encoded by
  `scripts/encode-screens.sh <dir>` into `public/screens/<shot>-<en|ar>.webp`. A missing Arabic
  shot falls back to the English one; a missing shot shows a placeholder frame.
- **OCR demo scan:** a page from the OCR evaluation set (`ocr-eval/`, generated).
- No analytics, no third-party requests: fonts are self-hosted (`@fontsource`).
- Vercel skips the build when a commit changes nothing under `website/` or the app version
  (`vercel.json`, `ignoreCommand`).
