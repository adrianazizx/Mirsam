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
| Windows | Signed through the SignPath Foundation (free for open source, ADR 0073) **once SignPath has accepted the project and the settings below are set**; each signing waits for your approval on signpath.io | SmartScreen: "More info" → "Run anyway" |
| Linux | Packages are not signed by convention for direct downloads | Nothing |

The release notes switch their Mac and Windows instructions to match: the run records whether the
disk image and the Windows installers were signed.

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

### Setting up Windows signing through SignPath (once)

The SignPath Foundation signs open-source projects for free. Its certificate names **SignPath
Foundation** as the publisher, not Mirsam. Its conditions are on
[signpath.org/terms](https://signpath.org/terms); our side of them is
[`docs/CODE_SIGNING_POLICY.md`](CODE_SIGNING_POLICY.md) and [`docs/PRIVACY.md`](PRIVACY.md),
linked from the README and from every release's notes.

1. **Release once unsigned.** SignPath takes only projects that have already released what they
   want signed: cut `v0.1.0` as above and publish it with its unsigned Windows installers.
2. **Turn on two-factor authentication** on your GitHub account (SignPath requires it of everyone
   in a role, and on SignPath itself).
3. **Apply** at [signpath.org/apply](https://signpath.org/apply) with the repository, the release,
   and the code signing policy page.
4. **Once accepted**, on [app.signpath.io](https://app.signpath.io):
   - **Trusted build systems:** add the predefined **GitHub.com** one and link it to the project.
   - **Project** slug `Mirsam` and **signing policy** slug `release-signing` (other names work if
     you set the variables in step 5). Make yourself the policy's approver.
   - **Two artifact configurations**, with these slugs and contents. GitHub hands every artifact
     over as a zip, so each starts with `<zip-file>`:

     `app` (the app's executable, signed before it goes into the installers):

     ```xml
     <artifact-configuration xmlns="http://signpath.io/artifact-configuration/v1">
       <zip-file>
         <pe-file path="*.exe" product-name="Mirsam">
           <authenticode-sign/>
         </pe-file>
       </zip-file>
     </artifact-configuration>
     ```

     `installers` (the setup program and the MSI):

     ```xml
     <artifact-configuration xmlns="http://signpath.io/artifact-configuration/v1">
       <zip-file>
         <pe-file path="Mirsam-Windows-Setup.exe" product-name="Mirsam">
           <authenticode-sign/>
         </pe-file>
         <msi-file path="Mirsam-Windows.msi">
           <authenticode-sign/>
         </msi-file>
       </zip-file>
     </artifact-configuration>
     ```

     `product-name` is SignPath's metadata restriction: it refuses a file whose version resource
     names another product. If the first dry run is refused over it, the message shows the name the
     file carries. Add a `product-version` restriction too if SignPath asks for one: the dry run's
     signing request shows the version string Tauri wrote.
   - **An API token** for a CI user that may submit to the signing policy.
5. **Store the settings in the repository:**

   ```bash
   gh secret set SIGNPATH_API_TOKEN
   ```

   ```bash
   gh variable set SIGNPATH_ORGANIZATION_ID --body "<organization ID from app.signpath.io>"
   ```

   Only if you chose other slugs: `gh variable set SIGNPATH_PROJECT_SLUG --body "…"` and
   `gh variable set SIGNPATH_SIGNING_POLICY_SLUG --body "…"`.
6. **Dry run** (Actions → Release → Run workflow). The Windows job asks SignPath twice, first for
   the executable and then for the installers, and waits up to an hour each time: approve both
   requests on app.signpath.io. Its log then shows, for each file, `Valid`, the signer `CN=SignPath
   Foundation, …` and `time-stamped` (`scripts/check-authenticode.sh`).

What the workflow does: builds the executable (`scripts/bundle.mjs --no-bundle`), uploads it as a
workflow artifact, has SignPath sign it, checks the signature, makes the installers around the
signed copy (`--bundle-only`, with the same PDFium checks as always), and has SignPath sign those.
Pull requests build unsigned and never ask. Without `SIGNPATH_API_TOKEN` the installers are built
unsigned with a warning, as before.

## Not done yet

- **Windows signing** is ready for SignPath's acceptance and settings (above; PROGRESS item 17).
  `pdfium.dll` stays unsigned: SignPath signs only the project's own files, so it stays as
  pdfium-binaries ships it. The uninstaller that NSIS writes inside the setup program is unsigned
  too.
- **Auto-update**: Tauri's updater can read the same GitHub Releases; needs an updater key pair
  and a check in the app.
- The release installers carry no self-test harness, so they are not launched in CI; the Bundles
  workflow runs the same build with the harness on every push to `main`.
