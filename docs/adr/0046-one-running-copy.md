# 0046 — One running copy per state directory

Status: Accepted. Date: 2026-11-22. Closes the gap ADR 0040 left open.

## Context

The state directory holds the state store and the recovery files of documents with unsaved
changes (ADR 0040). Until now nothing stopped a second copy of the app from starting beside the
first, for instance when the user opened a PDF from the file manager or a shell while the app
ran. The second copy then had a writer of its own on the same store and offered the first
copy's recovery files as if a session had crashed.

Users start the app with documents in three ways:

- **Command line, and the file manager on Windows and Linux.** A new process starts with the
  paths as arguments.
- **Finder on macOS** ("Open", "Open With", a double-click). LaunchServices does not start a
  second process; it sends the running app an "open documents" Apple Event. Tauri reports it
  as `RunEvent::Opened`. Until now the app ignored it.
- **A plain second launch** (an icon, a shell). A new process starts with no documents.

## Options

- **`tauri-plugin-single-instance` 2.5.5** (Tauri's own plugin, MIT OR Apache-2.0). Rejected:
  - It is keyed by the bundle identifier only (a named mutex on Windows, a D-Bus name on
    Linux, `/tmp/<identifier>_si.sock` on macOS). Self-tests and benchmarks run with their own
    `SAFHA_STATE_DIR`, often several at once from different checkouts, and beside the owner's
    own copy. Keyed by identifier, each would hand its PDF to whichever copy started first.
  - On macOS it is not a lock. A copy that cannot connect to the socket deletes it and listens
    itself, so two copies started together can both run, and a later one can take the socket
    from an earlier one.
  - Arguments go through lossy UTF-8, so a path that is not valid Unicode cannot be opened.
- **Our own, about 150 lines of `std`** (chosen). It is keyed by what has to have one writer:
  the state directory. It uses no new crates: `getrandom` and `dirs` are already in the tree
  (MIT OR Apache-2.0).

## Decision

### The lock

- The first copy takes an exclusive OS file lock (`File::try_lock`: `flock` on macOS and
  Linux, `LockFileEx` on Windows) on `<state dir>/instance.lock` and holds it until it exits.
  The kernel releases it when the process ends, crash included, so a stale lock cannot block
  the next start.
- The state directory is `SAFHA_STATE_DIR` when set (self-tests, benchmarks), else the app data
  directory as Tauri resolves it (`dirs::data_dir()` joined with the bundle identifier). It is
  worked out before Tauri starts, so a later copy exits before it creates a window.

### Handing over

- The running copy listens on a loopback TCP port. It writes the port and a random 128-bit
  token to `<state dir>/instance.json`, mode 0600 on Unix (Windows: the user profile's ACL).
- A later copy that finds the lock held reads that file, connects, and sends one JSON line: the
  token and its documents' absolute paths. Paths travel as the OS spells them (bytes on Unix,
  UTF-16 on Windows), so any path survives. The running copy checks the token, answers `ok`,
  and the later copy exits with status 0.
- The later copy retries for up to 10 s. This covers a running copy that holds the lock but has
  not written its port yet (it is starting), and one that is quitting: a quitting copy leaves
  requests unanswered, so the later copy waits and then takes the lock itself.
- If no answer comes in 10 s (the running copy is hung), the later copy runs without a state
  directory: no recents, session or recovery. It never becomes a second writer.
- Loopback TCP and a file lock behave the same on macOS, Windows and Linux. Another local user
  could connect to the port, but cannot read the token, and a request without it is dropped.
- A Unix socket would avoid the port. It needs a short path (the state directory under
  OneDrive is too long for `sun_path`), and Windows would need named pipes.

### Documents that arrive

- Arguments that are not flags are the launch documents, made absolute in the process that
  received them. `SAFHA_OPEN` is still the development fallback.
- Documents handed over, and macOS `RunEvent::Opened` files, go the same way
  (`commands::open_arrivals`):
  - **Before the main window's UI has asked for its startup documents,** they join those.
  - **After that,** they open in the document window the user was last in (tracked from
    `WindowEvent::Focused`; print windows do not count). The window gets a
    `documents-arrived` event with the outcomes and is brought forward. The last document
    becomes the active tab. A file that cannot be opened shows the usual error banner.
- The UI subscribes to `documents-arrived` before it asks for its startup documents, so no
  document can fall between the two. A document that arrives while the startup documents are
  still on their way stays the active tab: it is the one the user just asked for.
- A later copy with no documents still connects; the running copy comes forward.

## Consequences

- Recovery files and the store have one writer per state directory, on every platform.
- **Self-tests in parallel checkouts never meet,** because each has its own state directory.
  The owner's own copy and a benchmark never meet either.
- **Raising the window is best effort.** Windows lets a background process take the foreground
  only in some cases, and may flash the taskbar button instead. Wayland compositors may refuse
  too.
- **`open -n` on macOS** forces a new process and delivers its files by Apple Event once that
  process runs. It hands over before the event arrives, so the running copy comes forward
  without the file (seen). Finder, plain `open` and the command line are not affected. Handling
  it would mean starting the event loop in the later copy, which shows a Dock icon.
- **File associations** (`bundle.fileAssociations`) belong to packaging (M7). The shell already
  opens what the platform passes: arguments on Windows and Linux, Apple Events on macOS.

## Verified

macOS (Apple-silicon MacBook, macOS 26), release build:

- **Unit tests** (`instance.rs`): a second claim hands its documents to the first; Arabic and
  non-UTF-8 paths arrive intact; the lock passes on when the first copy ends, even with a stale
  `instance.json`; two state directories do not meet; a request with the wrong token is
  dropped; a copy that is quitting leaves requests unanswered; `instance.json` is mode 0600.
  `state.rs`: arrivals wait for the UI's startup documents, then open directly, in the window
  last used.
- **End to end** (`scripts/instance-selftest.sh`): the running copy shows one PDF; a second copy
  started from another directory with a relative path exits with status 0 in under a second;
  the running copy shows the second PDF as its active tab.
- **Apple Events:** with the app started through LaunchServices, `open -a Mirsam.app file.pdf`
  (what Finder does) opens the file in the running copy.

Not tested here: Windows (`LockFileEx`, the loopback port, raising the window) and Linux.
`scripts/instance-selftest.sh` runs unchanged under Git Bash and on Linux, so CI can run it on
both.
