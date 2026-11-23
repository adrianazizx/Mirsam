# 0071 — Onboarding: a first-run guide, one-time tool hints, a start screen with first steps

Status: Accepted. Date: 2026-11-23. Milestone 7's onboarding (spec §12).

## Context

A new user met an empty window with one button ("Open PDF…"), a rail of eight icon-only tools,
an Arabic switch in the status bar and nothing about OCR, whose first use downloads models. Two
things a first-time user cannot guess are also the ones that matter most for trust: that the
Arabic interface is complete (and its digits a choice), and that OCR runs on this computer.

Constraints: no cost on cold start (spec §11, 1.5 s); English and Arabic, LTR and RTL, light and
dark; reduced motion; keyboard and screen readers (ADR 0070's checks); design tokens; every action
a registry command; state through the shell's settings store, whose keys are an allowlist.

## Decision

- **A four-step guide** in a modal dialog: open a PDF; the tool rail (its eight tools listed with
  the rail's own icons) and the command palette; Arabic and English, with the language and the
  Arabic interface's digits chosen right there; OCR (models download once, with the user's OK;
  recognition runs here; documents never leave the computer). Next/Back/Skip/Done and "Open
  PDF…" are commands (`onboarding.next`, `.back`, `.skip`, `.finish`; `app.languageEnglish`,
  `app.languageArabic` for the radio buttons). Escape skips; a click outside does nothing.
- **When it opens by itself:** on a launch whose window starts empty, in the main window only,
  once the stored state has been read, and when nothing else is asking — recovered changes, a
  crash report, a password prompt. A launch with a document ("Open with", a restored session)
  waits for a later empty one: a modal over a file the user just asked for is in the way. Never
  again once finished or skipped; skipping a guide reopened after finishing keeps "finished".
  `help.guide` ("Welcome guide" in the command palette) and the start screen reopen it. The app
  has no native menu, so there is no Help menu to put it in.
- **Steps keep focus where it was.** Pressing Next leaves focus on Next (the same button becomes
  Done), so Enter walks the guide; the new step is announced politely ("Step 2 of 4: The tool
  rail"). If the focused button disappears (Back on step 1, Skip on the last), focus goes to the
  main button. The dialog is named by its title and the step's heading.
- **One-time hints** for OCR, Sign and Edit: the first time the tool is active with a document on
  screen. A card at the page area's inline-end bottom corner (near the tool's panel), over the
  page so nothing moves; it never takes focus and is announced politely; "Got it" or choosing
  another tool closes it. A hint counts as seen when it is shown, so each appears at most once
  even if the app quits with it open.
- **The start screen** gains four first steps under "Open PDF…": the tour, the command palette,
  the other language (named in that language, with `lang`) and Settings. Each card's accessible
  name is all its text (WCAG 2.5.3; axe flagged a name of the title alone).
- **State:** two keys on the shell's allowlist, `onboarding.guide` (`done` | `skipped`; absent =
  never seen) and `onboarding.hints` (`ocr,sign,edit`), in `state/onboarding.ts`, followed across
  windows through the settings-changed event. No first-paint cache: nothing opens before the
  store answers, so a cleared store means a first run again.
- **Motion:** the step fades and the hint slides in by `--motion-slow`; reduced motion sets the
  token to 0 and the components also drop the animation.
- **Tests:** the unit tests' screens run with the guide and hints off (`setOnboardingAutomatic`
  in `src/test/setup.ts`), so they do not start under a modal; the onboarding tests turn them on.
  `scripts/a11y-check.mjs` checks each guide step on its fresh start, in all four combinations,
  then skips it; the Edit and Sign hints are on screen in its tool screens.

## Consequences

- Cold start is unchanged: the guide reads the settings the app already reads at startup, and
  draws nothing until it opens. The self-test, which always launches with a document, measures
  it (PROGRESS.md, M7 onboarding).
- The guide is text only; screenshots or animations of the real UI would go stale with every
  design change and need translating twice.
- A user who only ever launches Mirsam by opening files never sees the guide by itself; the
  start screen and the palette still offer it, and the hints still explain the three tools.
