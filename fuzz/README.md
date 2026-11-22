# Fuzzing the open and parse path

Spec §11 asks for fuzzing of the open/parse path with malformed PDFs. This directory holds
[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) targets (libFuzzer with AddressSanitizer)
for everything Mirsam does with a file's bytes before and after PDFium reads them.

| Target | The fuzzer controls | What runs |
|---|---|---|
| `open` | a whole file | `Engine::open_bytes` (PDFium, after our own catalog reader), then what the shell asks of a new document: page sizes, outline, labels, comments, form fields; and of its first two pages: text in reading order, annotations, text blocks, a small render |
| `file` | a whole file | every PDFium-free reader in safha-core (`safha_fuzz::exercise`): catalog hints, the page tree and each page edit (then the edited file read back), text blocks, ink strokes, form fields and filling them in (decrypting with the fixtures' password), sanitize, redaction |
| `objects` | the objects of a file, separated by `endobj`; the harness writes a correct xref table around them | the same readers, without the fuzzer having to guess byte offsets: page trees, inheritance, resources, streams and filters |
| `content` | one page's content stream, on a page with real resources (simple and composite fonts, a form, an image, a hidden layer) | the content tokenizers and interpreters behind text blocks, redaction and sanitizing |
| `cmap` | a composite font's ToUnicode CMap | the font and CMap readers, through text extraction, redaction and scrubbing |

A panic, an arithmetic overflow, an out-of-bounds access, a stack overflow, a hang (20 s) or
4 GB of memory is a finding. An `Err` is the right answer to a damaged file.

## Running

Once per machine: a nightly toolchain (pinned in `fuzz/rust-toolchain.toml`, installed by rustup on
first use) and cargo-fuzz:

```bash
cargo install cargo-fuzz --locked
```

Then, from the repository root (`scripts/dev-setup.sh` has fetched PDFium; the `open` target
needs it):

```bash
node fixtures/generate.mjs   # the fixture corpus, if not generated yet
node fuzz/seed.mjs           # seeds fuzz/corpus/<target> from it (needs qpdf)
cp Cargo.lock fuzz/          # the app's dependency versions (fuzz/Cargo.lock is not committed)
cd fuzz
cargo fuzz run file -- -max_total_time=600 -max_len=600000 -dict=pdf.dict -timeout=20
```

`content` and `cmap` want `-max_len=65536`. A crash is written to `fuzz/artifacts/<target>/`;
replay it with `cargo fuzz run <target> fuzz/artifacts/<target>/crash-…`.

- **Several cores:** `-fork=N -ignore_crashes=1` keeps fuzzing after a crash and collects them all.
  libFuzzer's fork mode breaks on paths with spaces (this checkout's OneDrive path has them); run
  the built binary from a directory without (`fuzz/target`, `corpus` and `artifacts` are symlinks
  into the cache directory after `scripts/dev-setup.sh`), e.g.
  `cd fuzz/target/.. && target/<host triple>/release/file corpus/file -fork=4 -ignore_crashes=1 …`.
- **The templates:** `cargo test --lib` (in `fuzz/`) checks that the files `content`, `cmap` and
  `objects` build are read in full; without that the fuzzer would spend its time on the first error.
- **CI:** `.github/workflows/fuzz.yml` runs every target for two minutes weekly, on pull requests
  that touch the readers, and on "Run workflow" (with a time of your choice).

## A crash found

Fix it in safha-core and add the smallest input that showed it as a regression test next to the
code (see `pdf::tests::a_section_past_the_end_of_the_file_is_not_read`). Do not commit fuzzer
outputs: corpora and artifacts are regenerated, and the tests are the record.

## Licence

cargo-fuzz and `arbitrary` are MIT OR Apache-2.0; `libfuzzer-sys` is (MIT OR Apache-2.0) AND NCSA,
the BSD-style licence LLVM's libFuzzer was released under. All are development tools: the fuzz crate
is its own Cargo workspace, nothing in it ships, and the app's licence report does not include it.
