# 0003 — UI stack

Status: Accepted. Date: 2026-11-22.

## Decision

| Concern | Choice | Version checked | Licence | Why |
|---|---|---|---|---|
| Framework / build | React + TypeScript + Vite | 19.3 / 7.0 / 8.3 | MIT / Apache-2.0 / MIT | Spec starting choice; current and maintained |
| Headless primitives | **React Aria Components** | 1.22 | Apache-2.0 | See below |
| Styling | CSS Modules + CSS custom properties | — | — | Tokens in one file, no runtime, logical properties |
| i18n | i18next + react-i18next | 27.2 / 18.0 | MIT | Arabic has six plural forms; needs real plural rules |
| State | zustand | 5.1 | MIT | Small; readable outside React (the command registry needs that) |
| Icons | lucide-react | 1.77 | ISC | Consistent stroke icons; tree-shaken |
| UI fonts | IBM Plex Sans + IBM Plex Sans Arabic (`@fontsource`) | 5.3 | OFL-1.1 | One design across both scripts; bundled, so offline |
| Lint / format | Biome | 2.6 | MIT OR Apache-2.0 | One fast tool, no plugin tree |
| Unit tests | Vitest + Testing Library + jsdom | 5.1 | MIT | |

**Why React Aria Components over Radix / Base UI.** The spec asked for headless primitives with
RTL support. All three are permissively licensed and maintained (Base UI reached 1.x this year).
React Aria is the one built around internationalization: direction-aware keyboard navigation
(arrow keys flip in RTL), logical tooltip/popover placement (`start`/`end`), locale-aware
formatting, and the most thorough screen-reader testing of the three. For a product whose
differentiator is Arabic, that is the deciding property. Cost: a larger bundle than Radix.

## Rules this stack enforces

- `src/styles/tokens.css` is the only place colours, spacing, radii, type sizes, and motion are
  defined. `tokens.test.ts` computes WCAG contrast for every text/background pair in both themes
  and fails below AA; it also fails if the file mentions a physical direction.
- Layout uses logical properties only, so `dir="rtl"` on the root mirrors everything. Verified in
  the real WKWebView: tool rail at x=0 in English, x=1232 of 1280 in Arabic.
- The page canvas does **not** mirror: document direction is independent of UI direction.
- Every action is a command in `src/commands/registry.ts`; the command palette (M1) will list the
  same entries buttons and shortcuts already run.
- `src/backend/` is the only module that talks to Tauri. A mock backend lets the UI run in a plain
  browser (`pnpm dev`) and in component tests.
- Locale files are tested for identical keys and placeholders in English and Arabic.

## Open points

- ~~**Digits in the Arabic UI.**~~ **Decided by the owner (2026-11-23):** chrome text (page
  counts, zoom, sizes, dates) uses Arabic-Indic digits in Arabic (`ar-u-nu-arab`) by default,
  and Western digits (`ar-u-nu-latn`) if chosen in Settings (`uiDigits`, i18n `formatTag`).
  Document content is unaffected.
- **TypeScript 7** (the native compiler) is new. It typechecks this codebase, but two React Aria
  callbacks needed explicit parameter types that inference should have supplied. If friction
  grows, pin TypeScript 6 (what the official Tauri template still uses).
- Preferences are in `localStorage` for now; they move to the core's SQLite store in M1.
- Native window chrome is used as-is. Tabs-in-titlebar is an M1 decision.
