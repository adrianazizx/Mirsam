# Mirsam (مرسام) — PDF viewer & editor

A desktop PDF viewer and editor aiming at Acrobat-class capability with three differences: it is
fast, it treats Arabic and mixed Arabic/English documents as first-class, and its OCR is built to
be trusted. Offline by default.

- **What we are building and why:** [docs/SPEC.md](docs/SPEC.md)
- **Where we are:** [PROGRESS.md](PROGRESS.md)
- **Why it is built this way:** [docs/adr/](docs/adr/README.md)
- **How to work in this repo:** [CLAUDE.md](CLAUDE.md)
- **See it run:** [the viewer (Milestone 1)](docs/demos/m1.md) ·
  [foundations (Milestone 0)](docs/demos/m0.md)

Status: Milestone 1, the viewer, is merged: tiled continuous scrolling, thumbnails and outline,
search and copy that handle Arabic correctly, tabs, recent files, reading themes, and
password-protected files. Milestone 2 (annotate and organize) is in progress, and the groundwork
for the text engine (Milestone 3) is in.

Mirsam was called Safha (صفحة) while it had a working name. Code identifiers (the `safha-core`
crate, `SAFHA_*` environment variables, the `safha:` URI scheme) still carry that name; they will
be renamed once the milestone branches now in progress have merged.
