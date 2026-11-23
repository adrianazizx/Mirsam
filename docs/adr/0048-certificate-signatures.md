# 0048 — Certificate signatures (PAdES): signing, checking, keys

Status: Accepted (Milestone 5). Date: 2026-11-23. Builds on ADR 0044 (the spike and its library choice),
ADR 0014 (edits as incremental updates), ADR 0043 (encryption) and ADR 0045 (kept signatures).

## Context

Spec §5.6 makes certificate signatures (visible and invisible), validating existing signatures with what
changed after signing, and incremental saves that keep signatures valid P1; trusted time-stamps P2. The
spike (ADR 0044) chose RustCrypto (`cms` 0.3 RC, `x509-cert`, `x509-tsp` RC, `rsa` 0.10, `p256`) and
showed Poppler and OpenSSL accept its output. This records how the feature is built on it.

## Decision

`safha_core::sign`, UI-independent, with thin shell commands (`signing.rs`) and UI in
`components/signing/`.

### Signing

- **One incremental update** (`sign/pdf.rs`, through `pdf::write::Update`, so classic tables and
  cross-reference streams both work): a signature field merged with its widget (`/FT /Sig`, `/F 132`,
  print and locked), the page's `/Annots`, the form's `/Fields` and `/SigFlags 3`, and a `/Sig`
  dictionary (`/Filter /Adobe.PPKLite`, `/SubFilter /ETSI.CAdES.detached`, `/M`, `/Name` from the
  certificate, optional `/Reason`, `/Location`, `/ContactInfo`, `/Prop_Build /App /Name /Mirsam`).
  `/ByteRange` and `/Contents` are written as placeholders at offsets the writer returns
  (`Update::raw_object`), then filled in. Nothing before the update changes, so earlier signatures stay
  valid; the signed result is checked before it is returned (ours verifies, every signature that was
  intact still is).
- **Encrypted files:** the update's strings are encrypted like any edit's (ADR 0043 `Crypt`), except
  `/Contents`, which never is (ISO 32000-2, 7.6.2). Tested on an AES file; `pdfsig` verifies it with
  the password.
- **What is signed:** a signed document's bytes as they are. An unsigned document with unsaved edits is
  first rewritten in full by PDFium, as saving it would be, so content an edit removed does not stay in an
  earlier revision of the signed file. A file whose structure our writer cannot read is rewritten too
  (unsigned only). Signing **saves**: over the file or to one the user picks (the same Rust-side dialog
  as Save As); the signed file is then what is shown and undo starts over (as with protection, ADR 0043).
- **CMS** (`sign/cms.rs`): detached `SignedData`, signed attributes content-type, message-digest and ESS
  signing-certificate-v2; the signer's certificate and the chain the key source had. PAdES baseline
  leaves the CMS `signing-time` out (EN 319 142-1, 5.4): the claimed time is `/M`. SHA-256 (SHA-384 for
  P-384 keys); RSA PKCS#1 v1.5, ECDSA P-256 and P-384.
- **B-T:** with a time-stamping authority, an RFC 3161 request (SHA-256 of the signature value, a random
  nonce, `certReq`) goes out over HTTP(S) (`ureq`, the OS trust store for HTTPS, 30 s, 1 MiB reply cap);
  the reply must be granted, over that imprint, with that nonce, and the token goes in as the
  `signatureTimeStampToken` unsigned attribute. **The authority is a setting (`signing.tsaUrl`).** Since the
  owner's decision of 2026-11-23 (item 21; amendment below), the dialog suggests DigiCert's
  `http://timestamp.digicert.com`, with the time-stamp **off** until the user turns it on. Nothing
  is sent anywhere by default. An unreachable authority fails the
  signing with a message; it never silently produces a B-B signature the user did not ask for. What is
  sent is a hash of the signature and a nonce, nothing of the document.
- **Placeholder:** certificates + 4 KiB, + 16 KiB with a time-stamp; if the CMS is larger, the update is
  prepared once more with room (at most 256 KiB). Our signatures measure 1.3–2.9 KB.
- **Visible signatures** (`sign/appearance.rs`): a box chosen by a press on the page (200 × 72 pt,
  centred on it) shows one of the user's kept signatures (drawn, typed or image, ADR 0045, drawn by the
  same code as Fill & Sign) above a caption ("Digitally signed by", the signer's name, the date, worded
  and with digits as the UI shows them), or the caption alone. The caption is shaped by `typeset` and
  embedded as font subsets, so an Arabic name joins and reads right to left; the box stands upright on
  turned pages, as Fill & Sign items do. Invisible signatures get a zero-size widget with an empty
  appearance.

### Keys

- **PKCS#12 files** (`.p12`, `.pfx`) through **`p12-keystore` 0.4.1** (MIT OR Apache-2.0; 10.7 M
  downloads, 2.7 M recent, updated 2026-10-13; on RustCrypto's `pkcs12`, `pkcs5`, `cms`): PBES2/AES and
  the legacy 3DES and RC2 schemes Windows and older macOS exports use; a MAC failure is reported as a
  wrong password. The file is picked by a native dialog in Rust (the UI gets its name, never its path),
  its password asked in the dialog, and the identity kept **in memory for this run only**. The key is the
  first private key whose public key matches a certificate in the file; the other certificates that
  issued it form the chain.
- **The macOS keychain** (`sign/keychain.rs`, `security-framework` 3.7, already in the tree): identities
  with a supported key, a key usage that allows signing, in force now, and an extended key usage that
  allows documents (e-mail protection, Adobe's, Microsoft's, RFC 9336 document signing, or any) are
  listed beside the files. The key never leaves the keychain: `SecKeyCreateSignature` signs, and macOS
  asks the user the first time. Any key that signs elsewhere plugs in through one trait
  (`KeyOperation`); the CMS builder sees one `Signer` for all of them.
- **Not done:** the Windows certificate store (signing with a CNG key needs `unsafe` FFI, denied
  workspace-wide; `rustls-cng`, MIT/Apache, wraps it and is the candidate), Linux (NSS databases), smart
  cards (PKCS#11, `cryptoki`).

### Checking

- **Finding signatures** (`sign/pdf.rs`): through the form's field tree (field name, the widget's page and
  box), and by scanning the file for `/ByteRange` in objects the cross-reference table points at, for forms
  our reader cannot open (encrypted object streams). Signature dictionaries can never be in object
  streams (their byte offsets are signed), so this finds them all.
- **Per signature** (`sign/verify.rs`): the byte range must start the file and its gap must be exactly a
  hex string equal to this dictionary's `/Contents`, with the dictionary starting inside the signed bytes
  (no signature-wrapping); the message digest over the range; the signature over the signed attributes as
  the signer encoded them (raw DER, re-tagged as a SET); the ESS certificate binding (v2, or v1 for older
  signers); key usage; the time-stamp token (imprint over the signature value, its signature, its
  authority's certificate path and time-stamping key usage); document time-stamps (`ETSI.RFC3161`).
  `adbe.pkcs7.detached` signatures are checked the same way; SHA-1 and DSA signatures are reported as
  "not checked", never valid.
- **Trust:** the operating system's trusted roots (`rustls-native-certs`: macOS trust settings, Windows'
  root store, the Linux bundle), read once; no list is bundled. The path check is ours (ADR 0044 item 7):
  issuer by name and signature (`x509-verify`, features trimmed to RSA, P-256, P-384), CA constraints, and
  validity at the trusted time-stamp's time, else **now** (the signer's clock proves nothing).
- **Verdict:** *valid* (intact, signer trusted), *unknown* (intact, signer not vouched for: self-signed,
  a private CA, expired without a time-stamp, a key not for signing), *invalid* (the bytes or the
  signature do not check out). **Revocation (OCSP, CRL) is not checked**, and the panel says so under
  every verdict.
- **What changed after a signature:** the objects whose cross-reference entries differ between the signed
  revision and the newest one, by kind: signatures, annotations, form fields (by field, a widget counted
  by its parent), pages whose content or properties changed (a page rewritten only for its `/Annots` is
  not), pages added, removed or moved, and anything unreadable as "other".
- **On open:** the shell's `DocumentInfo` says whether a document is signed; the status bar checks a
  signed document at once and shows its verdict, which opens the Signatures panel (the side panel's fifth
  view) with each signature's verdict, problems, signer and issuer (names in `<bdi>`), time, revision and
  changes.

### UI

Commands `certSign.*`, `signatures.check|reveal`, `view.showSignatures` in the registry; certificate
signing needs the file's fill-in-forms permission (ISO 32000-2, table 22, bit 9). The dialog: identity
(keychain and files), reason, place, contact, invisible or visible (with a kept signature or the caption
alone), time-stamp and its authority; Sign / Sign as…; a visible signature closes the dialog and waits for
a press with the Sign tool (`fillTool: "certificate"`). English and Arabic strings; logical CSS only;
`--color-success` and `--color-warning` added with their contrast pairs.

## Validation

`scripts/check-signatures.sh` signs the corpus with a throwaway PKI made in Rust
(`examples/sign_fixtures.rs`, `tests/common/pki.rs`: a CA, P-256 and RSA signers, one with an Arabic name,
an RFC 3161 authority) and checks every file with tools that are not ours. 2026-11-23, this Mac:

| File | Poppler `pdfsig` 26.06 (NSS), test root trusted | OpenSSL 4.1 `cms -verify -cades` | other |
|---|---|---|---|
| invisible (P-256, classic xref) | Valid, Trusted | successful | `qpdf --check` clean |
| rsa (RSA, object streams, xref stream) | Valid, Trusted | successful | qpdf clean |
| visible (Arabic signer, turned page) | Valid, Trusted; Arabic name shown | successful | qpdf clean |
| timestamped (B-T) | Valid, Trusted | successful | `openssl ts -verify`: OK |
| encrypted (AES, with password) | Valid, Trusted | successful | qpdf clean |
| two signatures | #1 Valid, "Not total document signed"; #2 Valid, total | both successful | qpdf clean |

`tests/sign.rs` (11 tests) covers the same cases through our own checker plus a changed byte, an expired
signer with and without a time-stamp, later comments and page turns listed as changes, an edited
document rewritten before signing, PKCS#12 files (PBES2 and legacy 3DES, Arabic password) and a key that
signs elsewhere or refuses.

**Not checked:** Adobe Acrobat and Reader, Foxit, Preview (not installed here; Acrobat matters most and
will not trust a private root, which is policy, not a defect). Signing with a real keychain key (no
document-signing identity on this Mac; listing works). The signing and import flows in the native shell
(component tests and the browser preview against the mock only).

## Consequences

- 28 components new to the licence report (620 → 645): `p12-keystore` and what it brings (`x509-parser`,
  `asn1-rs`, `der-parser`, `nom`, `pkcs12`, `pkcs5`, `pbkdf2`, `scrypt`, `des`, `rc2`, AES-GCM), all
  MIT or Apache-2.0; `cargo deny` passes. The spike's RustCrypto crates were in the tree already. `cms` and `x509-tsp` stay pinned release candidates (ADR 0044): move to
  stable when they are released.
- Signing an unsigned, edited document rewrites it first, like saving; a signed one is never rewritten.
  Protection cannot be changed after signing (ADR 0043 refuses), so the dialog says so.
- Open: revocation and long-term validation (`/DSS`, OCSP/CRL, B-LT and B-LTA), certification (DocMDP)
  signatures and their permission checks, signing an existing empty signature field, trusting a
  certificate inside the app (today only the system's roots), PKCS#12 identities that survive a restart,
  the Windows store and smart cards.

## Amendment (2026-11-23): DigiCert as the suggested time-stamping server

The owner chose DigiCert's free public server as the suggestion (owner item 21).

- **The suggestion:** `SUGGESTED_TSA_URL = "http://timestamp.digicert.com"` (UI,
  `commands/registry.ts`). It fills the address while `signing.tsaUrl` is unset. The hint names
  it as a suggestion that the user may replace.
- **Off by default:** the time-stamp checkbox is off until the user turns it on, so nothing is
  sent by default. With it off, the address field is not shown and the request says
  `timestamp: false`; the shell contacts an authority only when that is true.
- **The user's choice is kept:**
  - A new setting, `signing.timestamp` (`on`/`off`), holds the on/off choice. Before, turning
    the time-stamp off was forgotten whenever an address was saved.
  - The address is saved as the user leaves it, changed or cleared. A cleared address stays
    cleared and is not refilled.
  - A file signed before this change has `signing.tsaUrl` set and no choice saved. Its saved
    address still means on.
- **HTTP is fine for RFC 3161:** the reply is a signed token checked against its imprint and
  nonce, and what goes out is a hash.
- **Tests:** `commands/signing.test.tsx`:
  - the suggestion shown, off, and a signature with no time-stamp;
  - turning it on with the suggestion, clearing it, and entering another;
  - off remembered after being on;
  - a cleared address kept;
  - the earlier settings.
