# 0043 — Encryption and permissions on save

Status: Accepted (Milestone 5). Date: 2026-11-22.

## Context

Spec §5 (P1) asks for encrypting a file with AES-256, passwords, and permissions (print, copy, edit).
Opening encrypted files and encrypting what an edit adds already exist (ADR 0017, `pdf::crypt`). What
was missing is **changing the protection of a whole file**: set a password, change it, remove it, set
which actions a reader may take.

PDFium cannot do it. Its save writes a file with the protection it was opened with, and the
`FPDF_REMOVE_SECURITY` flag is not exposed by `pdfium-render` (calling it would take `unsafe`, which is
denied workspace-wide). `lopdf` drops encryption silently on re-save (PROGRESS, session 3). So it is our
own code, as the writer has been since ADR 0014.

## Decision

### 1. Rewrite the file once, object by object (`pdf::protect`)

A save with another protection takes PDFium's full rewrite (which is also what the unprotected save writes,
so unreachable objects are still dropped), then reads it back with our parser and writes it again:

1. open the rewrite with the document's password (`Crypt::of`; the owner or the user password);
2. for every object: decrypt its strings and its stream with that object's key; unpack object streams
   (decrypting them first), whose contents become ordinary objects; drop cross-reference streams and the
   old `/Encrypt` dictionary;
3. encrypt each object for the new settings, or leave it plain for "remove";
4. write them under a classic cross-reference table with a trailer that keeps `/Root`, `/Info`, `/ID`
   and adds `/Encrypt`.

Streams keep their filters and bytes; only the encryption layer changes. The header says `%PDF-2.0` for
revision 6 and `%PDF-1.6` for revision 4.

### 2. Ciphers

| Choice | Revision | Key | Passwords |
|---|---|---|---|
| **AES-256** (default) | 6 (ISO 32000-2) | random 256-bit file key, wrapped by each password | any Unicode, normalised (§3) |
| **AES-128** (old readers) | 4 (ISO 32000-1) | derived from the user password (Algorithm 2) | ASCII only |
| RC4, AES-256 revision 5 | - | read, never written | - |

R6 follows Algorithms 8–10: `U`/`UE` for the user password, `O`/`OE` for the owner password (hashed over
`U`), `/Perms` (permissions, `adb`, random, encrypted with the file key) and the 64-round hash 2.B. The file
key is wrapped, so a later change of password re-wraps instead of re-encrypting (not used yet: a save
re-encrypts everything). Strings and streams use `/StdCF` with `/AESV3` or `/AESV2`; the metadata stream is
encrypted (`/EncryptMetadata` left true).

AES-128 passwords are limited to ASCII on purpose: PDFium opens a file with the password's UTF-8 bytes, and
the legacy handler compares bytes, so a Latin-1 "café" written to the file would never open in PDFium.
The error is `password_not_encodable`, and the dialog says to use AES-256.

### 3. Unicode passwords (spec: Arabic is never a follow-up)

ISO 32000-2 stores the password as UTF-8 after **SASLprep** (RFC 4013, NFKC based), cut at 127 bytes.
Readers differ in whether they normalise, so:

- **Writing R6** stores the normalised form: NFKC; non-ASCII spaces become U+0020; the "mapped to nothing"
  characters (soft hyphen, joiners, variation selectors, the byte order mark) are dropped. The characters
  SASLprep *prohibits* are not refused: a password that mixes Arabic and Latin letters is still a password.
- **Reading** tries the password as typed, then its normalised form (`password_bytes`), for our own
  decryption; and `Engine::open_shared` retries PDFium with the normalised form when PDFium says "wrong
  password". So a password set with `ﻛﻠﻤﺔ` (presentation forms) opens when typed as `كلمة`, and one set as
  `o` + U+0308 opens when typed as `ö`. Both are tested, and the file also opens in qpdf with the password as
  typed (UTF-8).
- Legacy (R2–4) tries Latin-1 then UTF-8.

### 4. Owner, user, permissions

`Permissions` is the `/P` flags: print, print at full quality, edit content, copy, annotate, fill forms,
accessibility, assemble. `bits()` sets the reserved bits the standard requires. Accessibility stays on in
the dialog (ISO 32000-2 no longer lets it be refused).

- An empty **owner password** becomes a random one nobody knows, so the restrictions cannot be lifted.
  The dialog says so.
- An empty **user password** means anyone may open it; only the permissions then bind.
- **Changing or removing protection needs the owner**: a document opened with the user password whose
  `/P` restricts anything refuses (`not_permitted`). `Document::security()` reports whether the opening
  password was the owner's.
- **A signed file is refused** (`edit_unsupported`): re-encrypting changes every byte a signature covers.
- **The UI honours the flags** for a file opened as user: Print, Copy, page edits (assemble), text edits,
  redaction and sanitize (modify), and markup and comments (annotate) are disabled by one table in the
  command registry (`PERMISSION_GATES`); the owner is never restricted. What this does not stop: the
  viewer's own text selection handles, drag-and-drop out of the page, and anything a different program
  does with the file. Permission flags are a courtesy that conforming readers observe, not protection:
  only the password protects.

### 5. In the app

`security.open` opens the dialog (Protect → Password and permissions, and the palette); `security.apply`
saves. The form (passwords included) lives only in the dialog's component state. **Save** rewrites the
file in place with the new protection, **Save as…** writes a copy and the tab becomes that file. The saved
bytes are opened again with the new password and must have the same page count before anything is
written; undo starts over from the saved file (its bytes are encrypted differently, so earlier updates no
longer apply). The recovery file is replaced like after any save. All strings exist in English and Arabic.

## Measured

- `qpdf --check` passes for AES-256 and AES-128 files with the user and with the owner password, with an
  owner-only (no user password) file, with an Arabic password, and after unlocking; `qpdf --show-encryption`
  reports `R = 6`/`AESv3` and `R = 4`/`AESv2` and the permissions set (extract, modify, print low
  resolution).
- Text and the rendered pixels of a page are identical before and after encrypting; a file with object
  streams (`qpdf/rtl-book-objstm`) round-trips protected and plain.
- Cost: protection adds 4–5 % (16–32 bytes per object for the IV and padding): `ar-plain` 61,491 →
  65,852 B (AES-256), the 1,000-page file 542,845 → 568,422 B in **21 ms** (release).
- Seven integration tests (`tests/security.rs`), unit tests for normalisation and a new handler's keys,
  and UI tests for the form, the gating and the dialog.

## Not done

- **Public-key security handlers**, **RC4 for writing**, and per-stream `/Crypt` filters (read as identity).
- **Verifying `/Perms`** on read (a tampered `/P` is not detected).
- **Re-wrapping the key** for a password change without re-encrypting: a save re-encrypts everything,
  which is the simple and correct path at 21 ms per 1,000 pages.
- **Encrypting and then signing in one session** (the order matters, and signing is not built: ADR 0044).
- **Linearized output**: the rewrite is not linearized; PDFium's rewrite is not either.
