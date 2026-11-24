# 0073 — Windows code signing through the SignPath Foundation

Status: Accepted (ready; waiting for SignPath to accept the project). Date: 2026-11-24.
Milestone 7's distribution (PROGRESS item 17; builds on ADR 0072).

## Context

ADR 0072 ships Windows installers unsigned, so SmartScreen warns, and Windows 11's Smart App
Control can block an unsigned app outright. A code-signing certificate from a commercial
authority (DigiCert, Sectigo, …) costs a few hundred dollars a year and, since 2023, has to live
on a hardware token or in a cloud signing service. Mirsam is MIT-licensed and public (spec D2).

## Options

- **SignPath Foundation** (chosen): free for open-source projects. It signs with its own
  certificate through SignPath.io, and only builds that GitHub Actions made from the public
  repository, with each signing approved by a person. Its conditions
  ([signpath.org/terms](https://signpath.org/terms)) fit Mirsam:
  - an OSI licence with no proprietary parts;
  - already released;
  - a public code signing policy and privacy statement;
  - named roles, and two-factor authentication.

  The cost:
  - **the publisher Windows shows is "SignPath Foundation"**, not Mirsam;
  - every release waits for two approvals on signpath.io;
  - SignPath may pause or revoke the certificate if the terms are broken.
- **Azure Artifact Signing** (formerly Trusted Signing): about $10 a month and shows our own
  name, but its identity validation was open only to organisations and individuals in a few
  countries when last checked. Kept as the paid alternative.
- **Certum's open-source certificate**: cheap (tens of euros a year) and shows the maintainer's
  name, but the key is on a smart card or in Certum's cloud signing service, so signing in CI
  needs extra tooling and a subscription.
- **Stay unsigned**: free, but SmartScreen warns on every download, and Smart App Control can
  block the app.

## Decision

- **Two signing passes in `release.yml`**, both waiting for approval:
  1. `scripts/bundle.mjs --no-bundle` builds the executable (`tauri build --no-bundle`). It is
     uploaded as a workflow artifact, and SignPath signs it (artifact configuration `app`).
  2. The signed copy replaces the built one. `bundle.mjs --bundle-only` (`tauri bundle`, the
     same overlay and checks) makes the setup program and the MSI around it. SignPath signs both
     (artifact configuration `installers`).

  The app on disk is then signed too, not only the installers people download.
- **Checked after each pass:** `scripts/check-authenticode.sh` requires a `Valid` signature, the
  signer `SignPath Foundation`, and a time-stamp, so the signature outlives the certificate.
- **Switched on by the repository's settings:** the `SIGNPATH_API_TOKEN` secret and the
  `SIGNPATH_ORGANIZATION_ID` variable. Project and policy slugs default to `Mirsam` and
  `release-signing`, and variables can change them. Without the token the run builds unsigned
  with a warning, as before.
- **Pull requests never ask for a signature.** Every request needs a person's approval.
- **The release notes** carry a signed and an unsigned Windows paragraph, like the Mac one. The
  run records which applies.
- **Public pages SignPath requires:** `docs/CODE_SIGNING_POLICY.md` and `docs/PRIVACY.md`, linked
  from the README and from every release's notes.
- **Not signed:**
  - `pdfium.dll`: a third party's unmodified build, which SignPath allows inside a signed
    package;
  - the NSIS uninstaller inside the setup program;
  - files the app downloads at the user's request, which are pinned by SHA-256 (ADR 0034).

## Consequences

- **Owner steps** (`docs/RELEASING.md`):
  - publish an unsigned `v0.1.0`;
  - turn on two-factor authentication;
  - apply to SignPath;
  - once accepted, set up the trusted build system, the two artifact configurations and the
    token;
  - do a dry run, approving both requests.
- **Not run:** the signing steps themselves. They need SignPath's acceptance and a Windows
  runner. The two-stage build was checked on macOS: `--no-bundle`, then `--bundle-only`, made the
  same `Mirsam.app` with the pinned PDFium, and the packaged executable is byte-identical to the
  built one.
- **SmartScreen reputation** builds on SignPath's certificate, which is shared with other
  projects. A new release can still warn for a while.
- **Moving to a certificate in our own name later** (Azure Artifact Signing or a commercial
  authority) changes only the two signing steps and the expected signer in
  `check-authenticode.sh`.
