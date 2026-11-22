# 0044 — Certificate signatures (PAdES): library choice and prototype

Status: Proposed ([SPIKE], Milestone 5; prototype in `spikes/pades/`, no UI). Date: 2026-11-22.

## Context

Spec §5 lists certificate-based signatures (P2) and §10 says "do not hand-roll cryptography": established
Rust crypto crates, licences from the allowlist, current maintenance. The question: **which crates sign a
PDF as PAdES, validate a certificate chain, and add a time-stamp, and does the result verify elsewhere?**

What the product has to produce: an **incremental update** (so the file as it was stays byte for byte, ADR
0014) that adds a signature field and a `/Sig` dictionary with `/SubFilter /ETSI.CAdES.detached`, a
`/ByteRange` over everything but the `/Contents` value, and in `/Contents` a **CMS `SignedData`** (RFC 5652)
with the CAdES signed attributes (content type, message digest, signing time, ESS signing-certificate-v2),
the certificate chain, and, for baseline B-T, an RFC 3161 time-stamp token as an unsigned attribute.

## Candidates

Facts read from crates.io and the RustSec database on 2026-11-22.

| Crate | Version | Licence | Updated | Role | Verdict |
|---|---|---|---|---|---|
| `cms` (RustCrypto) | 0.3.0-rc.2 (0.2.3: Jan 2024) | Apache-2.0 OR MIT | 2026-11-09 | CMS types, `SignedDataBuilder`, `SignerInfoBuilder` | **Used.** Release candidate; the 0.2 line is tied to `rsa` 0.9 |
| `x509-cert` | 0.3.1 | Apache-2.0 OR MIT | 2026-11-20 | certificates, PEM, name types | **Used** |
| `x509-tsp` | 0.2.0-rc.0 | Apache-2.0 OR MIT | 2026-11-05 | RFC 3161 request, response, `TSTInfo` | **Used.** Release candidate |
| `x509-verify` | 0.4.10 | Apache-2.0 OR MIT | 2026-11-21 | verify a signature with a certificate's key (RSA, ECDSA, Ed25519) | **Used**; its README calls parts "early, experimental"; fine for the signature check, not a path validator |
| `rsa` | 0.10.0 | MIT OR Apache-2.0 | 2026-11-08 | RSA PKCS#1 v1.5 signing | **Used.** 0.10 fixes RUSTSEC-2023-0071 (Marvin timing leak in private-key use); 0.9 and so `cms` 0.2 do not |
| `p256`, `p384`, `ecdsa`, `sha2`, `der`, `spki`, `pkcs8` | 0.14 / 0.17 / 0.11 / 0.8 | MIT OR Apache-2.0 | 2026 | curves, hashes, encodings | **Used** (`sha2` is already in the tree) |
| `cryptographic-message-syntax`, `x509-certificate` | 0.28.0 / 0.25.0 | **MPL-2.0** (allowed) | 2025-08-17 | CMS sign and verify, one author | Fallback; one maintainer, no release for 15 months; not tried |
| `openssl` | 0.10.82 | Apache-2.0 | 2026-11-01 | everything (CMS, OCSP, chain), through OpenSSL | Fallback for validation; a C library to build and ship on three systems, and `openssl-src` to vendor it; not tried |
| `rustls-webpki` | 0.103.17 | ISC | 2026-11-12 | path validation with CRLs | Candidate for **chain building** later; TLS-shaped (EKU rules), no OCSP, no CMS |
| `x509-ocsp` | 0.3.0-rc.1 | Apache-2.0 OR MIT | 2026-11-09 | OCSP request and response types | For revocation (B-LT), not tried |
| `pdf_signing`, `pades` | 0.3.0 / 0.1.10 | MIT OR Apache-2.0 / Apache-2.0 | 2024-11 / 2026-11 | PDF-level signing | Not used: 1,360 and 89 recent downloads, and the PDF half is ours anyway |
| `p12-keystore`, `pkcs12`, `cryptoki`, `security-framework` | 0.4.1 / 0.2.0-rc.0 / 0.10.1 / 3.7.0 | MIT / Apache-2.0 / MIT | 2026 | PKCS#12 files, PKCS#11 smart cards, the macOS keychain | Key *sources*; see "Not done" |

All of the "Used" rows are on the allowlist; `cargo deny check licenses bans sources` passes with the spike
in the workspace. Nothing needed asking about. One judgement for the owner: two of the used crates are
**release candidates** (`cms`, `x509-tsp`; the rest of their generation, `x509-cert`, `rsa`, `p256`, `ecdsa`,
is stable). The spike pins them with `=`. The stable `cms` 0.2.3 would mean RSA signing with a key-leaking
`rsa` 0.9, which is worse.

## Prototype (`spikes/pades`)

About 600 lines. `pdf.rs` reads the file with lopdf and writes the update by hand (the product would use
`pdf::write::Update`, which writes both cross-reference forms; the spike handles classic tables only):
a `/Sig` object with fixed-width `/ByteRange` and a 16 KiB `/Contents` placeholder, an invisible widget
(`/F 132`) on the first page, the page with the widget in `/Annots`, the catalog with
`/AcroForm /Fields` and `/SigFlags 3`. `cms_sign.rs` builds the CMS with the `cms` builder, hand-encodes the
ESS attribute (4 lines of DER), does the RFC 3161 exchange (request from `x509-tsp`; the reply here comes
from `openssl ts -reply` standing in for the HTTP call to a time-stamping authority), and verifies: message
digest, signature over the signed attributes, ESS binding, chain to a trust anchor, and the time-stamp
token's imprint, signature and chain.

### Checked by other tools

Signed with P-256 and RSA-2048 keys, with an Arabic subject name (`CN=الموقّع التجريبي`), and time-stamped:

| Check | Result |
|---|---|
| **Poppler `pdfsig`** (NSS), CA in its database | *Signature is Valid.* *Certificate is Trusted.* for P-256, RSA-2048, Arabic name and time-stamped; the Arabic common name is shown correctly; without the CA: *Certificate issuer isn't Trusted.* |
| **OpenSSL** `cms -verify -cades` on the extracted signature and signed bytes | *CMS Verification successful*, *CAdES Verification successful* (checks signing-certificate-v2) |
| **OpenSSL** `ts -verify` on the extracted token against the signature value | *Verification: OK* (policy, SHA-256, time, TSA name read back) |
| `qpdf --check` | no syntax or stream errors |
| One byte changed inside the signed range | `pdfsig`: *Digest Mismatch.* Our verifier: digest mismatch |
| Bytes appended after signing | `pdfsig`: *Signature is Valid*, "Not total document signed" |
| Our engine (`safha-core`) | opens all four, `is_signed()` is true; a page rotation appends to the signed bytes (the signed file is a prefix) and `pdfsig` still says *Valid* and *Trusted* |

Found on the way: NSS (so `pdfsig`) judges a document-signing certificate as an **e-mail signer** and
rejects one whose extended key usage lists only Adobe's document-signing OID. The test certificates carry
both `emailProtection` and `1.2.840.113583.1.1.5`. Certificates from commercial CAs for PDF signing
usually do.

**Not checked: Adobe Acrobat, Foxit, Preview.** Acrobat is the validator that matters most for users and
it is not installed here. Its trust list (AATL/EUTL) will not know a self-signed or private CA; that is
policy, not a defect. Needs a person with Acrobat before the feature is announced.

### Measurements (release build, M-series Mac)

| | P-256 | RSA-2048 | P-256 + time-stamp |
|---|---|---|---|
| CMS in `/Contents` | 1,346 B | 1,741 B | 2,875 B |
| Signing (key operation + CMS) | 0.2–0.5 ms | 1.6–1.8 ms | 6–9 ms (includes starting `openssl`) |
| Verifying (own code) | 0.4–0.8 ms | 0.4–0.7 ms | 0.9–1.2 ms |

Hashing the signed ranges: 0.3 ms for 600 KB (about 2 GB/s), so a 100 MB file is about 50 ms. The prototype's
whole `sign` is 2–8 ms on the three fixtures, almost all of it lopdf parsing the input, which the product
does not do. A 16 KiB placeholder holds 5× what these signatures need; it costs 32 KB of hex in the file.
**Dependency cost:** 55 crates are new to the tree (`THIRD_PARTY_LICENSES.md`: 497 → 552), 92 counting
duplicates by version; many are curve crates (`k256`, `p192`, `p224`, `p521`, `ed25519`) pulled in by
`x509-verify`'s features, which can be trimmed to P-256, P-384 and RSA. The spike's release binary (with
lopdf) is 2.7 MB; a cold release build of the spike takes 31 s.

## Recommendation

**Build PAdES on the RustCrypto stack above**, as `safha_core::sign` next to `edit` and `save`:

1. **PAdES-B-B first, then B-T.** The spike produces both. B-LT (OCSP/CRL values in a `/DSS` dictionary,
   which needs `x509-ocsp` and a fetcher) comes after, with the validation UI.
2. **The PDF half is ours** and uses `pdf::write::Update`: one new revision, both cross-reference forms,
   encrypted files too. In an encrypted file `/Contents` of the signature dictionary must **not** be
   encrypted (ISO 32000-1, 7.6) while everything else the update adds is; and protection has to be set
   *before* signing (ADR 0043 refuses to change a signed file).
3. **Signing with an `EncodePublicKey + Signer` object**, so the same code takes a software key (a
   `.p12`/PEM, with `p12-keystore` or `pkcs12` for the file) or a key that never leaves a device
   (`cryptoki` for smart cards, the macOS keychain, Windows CNG). Software keys first.
4. **Placeholder size:** 16 KiB by default; the signer knows its CMS size only after signing, so it
   computes it first (certificates + token estimate) and fails loudly rather than truncating.
5. **Time-stamp authority:** a setting, empty by default; an unreachable TSA fails the signing with a
   message instead of producing a signature without the time-stamp the user asked for.
6. **Appearance:** a visible signature uses `typeset` (Arabic name, date in both numeral systems per the
   UI setting), as text boxes do (ADR 0021). The invisible one the spike writes is the base case.
7. **Validation:** our verifier's checks plus **chain building with `rustls-webpki` or our own short
   path code** (the spike's is 20 lines: issuer, signature, validity). Revocation through OCSP/CRL is
   the open part; until it exists the UI must say "revocation not checked", never "valid".
   Trust anchors: the operating system's store plus the user's own, not a bundled list.

Risks: the RC crates (pinned; move to stable on release), `x509-verify` maturity (our checks over its
signature primitive, covered by tests against `openssl` and `pdfsig` as above), and RSA key operations
(only with `rsa` ≥ 0.10).

## Not done

- A signing UI (out of scope for the spike), key import (`.p12`, keychain, smart cards).
- Revocation (OCSP, CRL, `/DSS`), long-term validation, document time-stamps (`DocTimeStamp`), DocMDP
  certification levels.
- Visible signature appearances; signing an existing empty signature field; several signatures in a row
  (the second signs the first's revision; the spike signs once).
- Acrobat, Foxit and Preview checks; EdDSA and RSA-PSS (the crates have them; the spike did not try).
- Cross-reference *streams* in the spike (the product's writer has them).
