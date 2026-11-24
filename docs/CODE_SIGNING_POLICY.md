# Code signing policy

Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by
[SignPath Foundation](https://signpath.org/).

**Status:** Mirsam is applying to the SignPath Foundation's programme for open-source projects.
Until it is accepted, the Windows installers are unsigned. Each release's notes say whether
its installers are signed.

## What is signed

Windows releases on [GitHub Releases](https://github.com/adrianazizx/Mirsam/releases) are signed
with the SignPath Foundation's certificate. **The publisher shown by Windows is "SignPath
Foundation"**; it signs for this project, and only builds of this repository:

| File | Signed |
|---|---|
| `Mirsam-Windows-Setup.exe` (setup program) | Yes |
| `Mirsam-Windows.msi` (Windows Installer package) | Yes |
| The app's own executable, inside both installers | Yes, before it is packaged |
| `pdfium.dll`, the PDF engine inside both installers | No: an unmodified build by the [pdfium-binaries](https://github.com/bblanchon/pdfium-binaries) project (BSD-3-Clause and Apache-2.0), pinned by version and checked byte for byte when the installers are built |

The macOS disk image is signed with the maintainer's Apple Developer ID and notarized by Apple
(the release notes say when a build is not). The Linux packages are not signed. Each release
lists every file's SHA-256 checksum in `SHA256SUMS.txt`.

**Not covered by this signature:** files you choose to download later inside the app: the OCR
models and the OCR runtime (`llama-server`, an unmodified release of
[llama.cpp](https://github.com/ggml-org/llama.cpp), MIT). The app pins each one by address, size
and SHA-256 and refuses any file that does not match. See [Privacy](PRIVACY.md).

## How a release is signed

1. Every installer is built from this public repository by GitHub Actions on GitHub's own
   runners ([`.github/workflows/release.yml`](../.github/workflows/release.yml)), from a version
   tag on `main`. Nothing built on a personal computer is ever signed.
2. The workflow builds the app's executable, sends it to SignPath, and waits.
3. An approver checks the signing request on SignPath (the commit, the workflow run, the
   version) and approves it.
4. The workflow packages the signed executable into the setup program and the MSI, and sends
   both for signing. An approver approves again.
5. The workflow checks every signature and its time-stamp, then leaves a **draft** release. The
   maintainer publishes it.

Pull requests and forks are never signed.

## Team and roles

| Role | Who | What they do |
|---|---|---|
| Committers and reviewers | [Adrian Aziz](https://github.com/adrianazizx) | Commit to this repository; review and merge every change from anyone else (pull requests) |
| Approvers | [Adrian Aziz](https://github.com/adrianazizx) | Approve each signing request on SignPath |

Mirsam has one maintainer at present. Everyone in a role uses multi-factor authentication on
GitHub and on SignPath.

## Privacy

Mirsam has no accounts and no telemetry, and it sends nothing about you or your documents. It
connects to the internet only when you ask it to, for example to download OCR models. The
details are in the [privacy statement](PRIVACY.md).

## Reporting a problem

If you find a release signed by the SignPath Foundation for Mirsam that was not built from this
repository, or anything else wrong with a signature,
[open an issue](https://github.com/adrianazizx/Mirsam/issues) or write to
[support@signpath.io](mailto:support@signpath.io).
