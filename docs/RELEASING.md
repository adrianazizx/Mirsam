# Releasing Mirsam

Installers reach people through **GitHub Releases** on `adrianazizx/Mirsam`
(`.github/workflows/release.yml`, ADR 0072). A version tag on `main` builds every installer and
leaves a **draft** release; nothing is public until you press Publish.

## Download links

Once a release is published:

| Link | Points at |
|---|---|
| https://github.com/adrianazizx/Mirsam/releases/latest | The newest release's page: every download, notes, checksums |
| https://github.com/adrianazizx/Mirsam/releases/latest/download/Mirsam-macOS.dmg | Mac, Apple silicon and Intel (macOS 13+) |
| https://github.com/adrianazizx/Mirsam/releases/latest/download/Mirsam-Windows-Setup.exe | Windows 10/11, 64-bit |
| https://github.com/adrianazizx/Mirsam/releases/latest/download/Mirsam-Windows.msi | Windows, for managed installs |
| https://github.com/adrianazizx/Mirsam/releases/latest/download/Mirsam-Linux-x86_64.deb | Ubuntu, Debian |
| https://github.com/adrianazizx/Mirsam/releases/latest/download/Mirsam-Linux-x86_64.rpm | Fedora, openSUSE |
| https://github.com/adrianazizx/Mirsam/releases/latest/download/Mirsam-Linux-x86_64.AppImage | Other Linux |

The file names carry no version, so `latest/download/…` links keep working from one release to
the next: put them on a website or in a message once. `latest` skips drafts and pre-releases
(`0.2.0-1`); a pre-release has its own `releases/download/v0.2.0-1/…` links.

## Cutting a release

1. Pick the version and set it everywhere (Tauri config, `package.json`, `Cargo.toml`,
   `Cargo.lock`):

   ```bash
   node scripts/set-version.mjs 0.1.0
   ```

   `MAJOR.MINOR.PATCH`, or `MAJOR.MINOR.PATCH-N` for a pre-release (Windows Installer accepts only
   a number there, so not `-beta`).
2. Commit, open a pull request, and merge it once CI is green.
3. Tag the merged commit on `main` and push the tag:

   ```bash
   git fetch origin && git tag v0.1.0 origin/main && git push origin v0.1.0
   ```

   The workflow refuses a tag that does not match the version or is not on `main`.
4. Wait for **Actions → Release** (about 20–30 minutes; notarization adds a few). It leaves a draft
   under **Releases** with the six installers, `SHA256SUMS.txt`, install notes in English and Arabic,
   and the changes since the last release.
5. Read the draft, download one installer and try it, then press **Publish release**.

A failed run can be re-run from the Actions page; it replaces the draft's files. To start over,
delete the draft and the tag (`git push origin :v0.1.0 && git tag -d v0.1.0`) and tag again.

**Dry run:** Actions → Release → **Run workflow** builds and signs everything and keeps the
installers on the run (Artifacts), publishing nothing. Pull requests that change the release
files run it the same way.

## Signing

| Platform | State | Without it |
|---|---|---|
| macOS | Signed with the Developer ID and notarized by Apple **when the secrets below are set** | Gatekeeper blocks the app; users need System Settings → Privacy & Security → Open Anyway |
| Windows | Unsigned (needs a code-signing certificate, PROGRESS item 17) | SmartScreen: "More info" → "Run anyway" |
| Linux | Packages are not signed by convention for direct downloads | Nothing |

The release notes switch their Mac instructions to match: the run records whether the disk image
was signed.

### Setting up Apple signing (once)

You need the **Account Holder** role on the Apple Developer account to create a Developer ID
certificate.

1. **Developer ID Application certificate.** In Xcode: Settings → Accounts → your team → Manage
   Certificates → **+** → **Developer ID Application**. (Or developer.apple.com → Certificates →
   **+** → Developer ID Application, with a signing request from Keychain Access.)
2. **Export it** from Keychain Access → login → My Certificates → right-click
   "Developer ID Application: …" → Export → `.p12`, with a strong password.
3. **A notarization key.** App Store Connect → Users and Access → Integrations → App Store Connect
   API → Team Keys → **+**, access **Developer**. Download `AuthKey_<KEY ID>.p8` (Apple offers it
   once). Note the **Key ID** and the **Issuer ID** shown above the table.
4. **Store them as repository secrets** (from the repository folder; `gh` asks for each value
   that is not piped in, so nothing lands in your shell history):

   ```bash
   base64 -i ~/Desktop/Certificates.p12 | gh secret set APPLE_CERTIFICATE
   ```

   ```bash
   gh secret set APPLE_CERTIFICATE_PASSWORD
   ```

   ```bash
   gh secret set APPLE_API_PRIVATE_KEY < ~/Downloads/AuthKey_XXXXXXXXXX.p8
   ```

   ```bash
   gh secret set APPLE_API_KEY
   ```

   ```bash
   gh secret set APPLE_API_ISSUER
   ```

   Then delete the `.p12` and `.p8` files from Desktop and Downloads (keep a copy in your password
   manager if you like).
5. **Dry run** (above) and check the macOS job's log: "Signing as Developer ID Application:
   …", the notarization result `Accepted`, and `spctl` reporting `source=Notarized Developer ID`.

Notarizing with an Apple ID instead of a key also works: set `APPLE_ID`, `APPLE_PASSWORD` (an
app-specific password from account.apple.com) and `APPLE_TEAM_ID` instead of the three
`APPLE_API_*` secrets.

What the workflow does with them: imports the certificate into a keychain of the run's own
(deleted at the end), lets `tauri build` sign the app and PDFium with the hardened runtime and
notarize and staple the app, checks every signature, then signs, notarizes and staples the disk
image itself. `scripts/bundle.mjs` accepts the re-signed PDFium as the pinned build when its
Mach-O UUIDs match and its signature verifies.

**The bundle identifier** (`app.mirsam.desktop`, PROGRESS item 6) does not have to be registered
for Developer ID signing, but changing it after people install the app moves where it keeps its
recent files and settings. Settle it before the first public release.

## Not done yet

- **Windows signing**: needs the certificate (DigiCert suggested, PROGRESS item 17); `pdfium.dll`
  needs its own signature.
- **Auto-update**: Tauri's updater can read the same GitHub Releases; needs an updater key pair
  and a check in the app.
- The release installers carry no self-test harness, so they are not launched in CI; the Bundles
  workflow runs the same build with the harness on every push to `main`.
