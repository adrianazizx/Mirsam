# Mirsam brand

| File | What it is |
|---|---|
| [`mirsam-logo.svg`](mirsam-logo.svg) | The logo: the page with the pen stroke, and the Mirsam wordmark under it |
| [`mirsam-logo-dark.svg`](mirsam-logo-dark.svg) | The logo for dark backgrounds: the wordmark in `#D4D1FC` instead of the ink colour |
| [`mirsam-envelope.svg`](mirsam-envelope.svg) | The page alone: the app icon |
| [`mirsam-nib.svg`](mirsam-nib.svg) | The pen stroke and nib alone, for small or single-colour use |

Colours: ink `#29265E`, page `#D4D1FC`, folded corner `#F7F6FF`.

## The app icon

`apps/desktop/src-tauri/icons/icon.svg` is `mirsam-envelope.svg` centred on a square canvas
(viewBox `303 113 650 650`, the page filling about 84% of the height). Every other file in that
folder is generated from it; after changing it, run from `apps/desktop`:

```bash
pnpm tauri icon src-tauri/icons/icon.svg -o src-tauri/icons
rm -rf src-tauri/icons/android src-tauri/icons/ios   # desktop only
```
