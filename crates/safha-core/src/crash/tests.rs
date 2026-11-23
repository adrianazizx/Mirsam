use super::*;

fn about() -> About {
    About { version: "0.0.1".into(), build: "release, aarch64".into(), os: "macOS 26.1".into() }
}

/// `{:#}` output of a backtrace in a build with symbols, as std prints it.
const BACKTRACE: &str = "   0:        0x102f9ea24 - <std[d28d9deb04fda6f8]::backtrace::Backtrace>::create
   1:        0x102f90a6c - safha_desktop_lib[c8e987b83851e3b8]::crash::install::{closure#0}
   2:        0x102faa3f8 - std[d28d9deb04fda6f8]::panicking::panic_with_hook
   3:        0x102f9bb50 - std[d28d9deb04fda6f8]::panicking::panic_handler::{closure#0}
   4:        0x102f9c394 - __rustc[45371133cddedda4]::rust_begin_unwind
   5:        0x102fc78f0 - core[e78f7293afa7e62d]::panicking::panic_fmt
   6:        0x102fc7594 - core[e78f7293afa7e62d]::str::slice_error_fail
   7:        0x102fc2e80 - safha_core[0123456789abcdef]::text::layout::visual_order
                               at /Users/someone/src/Mirsam/crates/safha-core/src/text/layout.rs:120:9
   8:        0x102fc2e34 - safha_core[0123456789abcdef]::text::page_text
   9:        0x102f90a2c - std[d28d9deb04fda6f8]::sys::backtrace::__rust_begin_short_backtrace::<fn(), ()>
  10:        0x102f90a44 - std[d28d9deb04fda6f8]::rt::lang_start::<()>::{closure#0}
  11:        0x102f90d10 - _main
";

/// The same backtrace from a stripped binary.
const STRIPPED: &str = "   0:        0x102d2ea24 - __mh_execute_header
   1:        0x102d20a6c - __mh_execute_header
   2:        0x102d3a3f8 - __mh_execute_header
";

#[test]
fn keeps_the_frames_between_the_panic_machinery_and_the_thread_start() {
    assert_eq!(
        rust_frames(BACKTRACE),
        [
            "core::str::slice_error_fail",
            "safha_core::text::layout::visual_order (crates/safha-core/src/text/layout.rs:120:9)",
            "safha_core::text::page_text",
        ]
    );
}

/// A panic on a spawned thread: `catch_unwind` (panic machinery by name)
/// wraps the thread's closure, below it.
const THREAD: &str = "   0:        0x1 - <std[d28d9deb04fda6f8]::backtrace::Backtrace>::create
   1:        0x2 - safha_desktop_lib[c8e987b83851e3b8]::crash::install_panic_hook::{closure#0}
   2:        0x2 - <alloc[1234567890abcdef]::boxed::Box<dyn for<'a, 'b> core::ops::function::Fn<(&'a std::panic::PanicHookInfo<'b>,)>> as core::ops::function::Fn<(&std::panic::PanicHookInfo,)>>::call
   2:        0x3 - std[d28d9deb04fda6f8]::panicking::panic_with_hook
   3:        0x4 - core[e78f7293afa7e62d]::panicking::panic_fmt
   4:        0x5 - safha_core[0123456789abcdef]::render::worker::{closure#0}
   5:        0x6 - std[d28d9deb04fda6f8]::sys::backtrace::__rust_begin_short_backtrace::<_, ()>
   6:        0x7 - std[d28d9deb04fda6f8]::panicking::catch_unwind::<_>
   7:        0x8 - std[d28d9deb04fda6f8]::thread::lifecycle::spawn_unchecked::{closure#1}
";

#[test]
fn keeps_the_frames_of_a_panic_on_another_thread() {
    assert_eq!(rust_frames(THREAD), ["safha_core::render::worker::{closure#0}"]);
}

#[test]
fn keeps_addresses_when_the_binary_has_no_symbols() {
    assert_eq!(rust_frames(STRIPPED), ["0x102d2ea24", "0x102d20a6c", "0x102d3a3f8"]);
}

#[test]
fn a_panic_report_holds_no_document_text_or_paths() {
    let panic = PanicDetails {
        message: "byte index 1 is not a char boundary; it is inside 'ب' (bytes 0..2 of `عقد الإيجار`)",
        location: Some(("/Users/someone/.cargo/registry/src/index.crates.io-1/foo-1.0.0/src/lib.rs", 3, 7)),
        thread: Some("safha-render"),
        backtrace: BACKTRACE,
    };
    let scrubber = Scrubber::new(["someone".to_string()]);
    let report = CrashReport::from_panic(
        &about(),
        1_763_894_530,
        panic,
        vec!["open_document_dialog".into()],
        &scrubber,
    );
    let text = report.text();
    assert!(text.contains("byte index 1 is not a char boundary; it is inside <quoted>"), "{text}");
    assert!(text.contains("at foo-1.0.0/src/lib.rs:3:7, thread safha-render"), "{text}");
    assert!(text.contains("Time: 2025-11-23 10:42 UTC"), "{text}");
    assert!(text.contains("  open_document_dialog\n"), "{text}");
    assert!(!text.contains("someone") && !text.contains("عقد") && !text.contains('ب'), "{text}");
    assert!(is_valid_id(&report.id), "{}", report.id);
}

#[test]
fn an_interface_report_keeps_the_apps_own_script_positions_only() {
    let report = CrashReport::from_interface(
        &about(),
        0,
        "TypeError: undefined is not an object (evaluating 'doc.name')",
        "render@tauri://localhost/assets/index-AbC12.js:1:2345\n@https://evil.example/x.js:1:1\n",
        vec!["file.open".into(), "edit.bold ×3".into()],
        &Scrubber::default(),
    );
    assert_eq!(report.message, "TypeError: undefined is not an object (evaluating <quoted>)");
    assert_eq!(report.stack, ["render@assets/index-AbC12.js:1:2345", "@<address>"]);
    assert_eq!(report.commands, ["file.open", "edit.bold ×3"]);
}

#[test]
fn breadcrumbs_fold_repeats_and_keep_the_last_ones() {
    let mut crumbs = Breadcrumbs::new();
    assert!(!crumbs.take_dirty());
    crumbs.push("open_document_dialog");
    for _ in 0..120 {
        crumbs.push("update_viewport");
    }
    crumbs.push("edit_text_block");
    assert_eq!(crumbs.list(), ["open_document_dialog", "update_viewport ×120", "edit_text_block"]);
    assert!(crumbs.take_dirty());
    assert!(!crumbs.take_dirty());
    for i in 0..100 {
        crumbs.push(&format!("command_{i}"));
    }
    let list = crumbs.list();
    assert_eq!(list.len(), MAX_COMMANDS);
    assert_eq!(list.last().map(String::as_str), Some("command_99"));
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("safha-crash-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn report_at(created: i64) -> CrashReport {
    CrashReport::from_native(
        &about(),
        created,
        NativeCrash { exception: "EXC_BAD_ACCESS (SIGSEGV)".into(), thread: None, frames: vec![] },
        vec![],
    )
}

#[test]
fn the_store_keeps_the_newest_reports_and_offers_each_once() {
    let dir = temp_dir("store");
    let store = CrashStore::open(&dir).expect("store");
    for created in 0..(MAX_REPORTS as i64 + 5) {
        store.write(&report_at(created)).expect("write");
    }
    let reports = store.list();
    assert_eq!(reports.len(), MAX_REPORTS);
    assert_eq!(reports[0].created, MAX_REPORTS as i64 + 4);

    let offered = store.take_unoffered();
    assert_eq!(offered.len(), MAX_REPORTS);
    assert!(store.take_unoffered().is_empty());

    let id = reports[0].id.clone();
    assert_eq!(store.get(&id).map(|r| r.offered), Some(true));
    assert!(store.delete(&id).expect("delete"));
    assert!(!store.delete(&id).expect("delete again"));
    // Ids from the UI cannot reach outside the folder.
    assert!(store.get("../running").is_none());
    assert!(!store.delete("../../etc").expect("refused"));
    let _ = fs::remove_dir_all(dir);
}

fn run(pid: u32, updated: i64) -> RunRecord {
    RunRecord {
        pid,
        started: updated - 10,
        updated,
        about: about(),
        exe: "safha-desktop".into(),
        commands: vec!["page_sizes".into()],
        reported: false,
    }
}

#[test]
fn a_run_that_did_not_end_is_found_at_the_next_start() {
    let dir = temp_dir("runs");
    let store = CrashStore::open(&dir).expect("store");
    assert!(store.begin_run(&run(1, 100)).is_empty());
    // The run crashed: no end_run. The next one finds it.
    let earlier = store.begin_run(&run(2, 200));
    assert_eq!(earlier, [run(1, 100)]);
    // That one ends cleanly; the one after finds nothing but what was kept.
    store.keep_unconfirmed(&[run(1, 100)]);
    store.end_run();
    assert_eq!(store.begin_run(&run(3, 300)), [run(1, 100)]);
    store.end_run();
    assert!(store.begin_run(&run(4, 400)).is_empty());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn earlier_runs_become_crashes_wait_for_a_log_or_are_dropped() {
    let now = 10 * 86_400;
    let crash = NativeCrash { exception: "EXC_CRASH (SIGABRT)".into(), thread: None, frames: vec![] };
    let mut reported = run(5, now - 60);
    reported.reported = true;
    let runs = vec![run(1, now - 60), run(2, now - 60), run(3, now - 3 * 86_400), reported];
    let resolved = resolve_earlier_runs(runs, now, |run| (run.pid == 1).then(|| crash.clone()));
    assert_eq!(resolved.crashes, [(run(1, now - 60), crash)]);
    if os_logs::SUPPORTED {
        assert_eq!(resolved.unconfirmed, [run(2, now - 60)]);
    } else {
        assert!(resolved.unconfirmed.is_empty());
    }
}

#[test]
fn issue_links_carry_the_report_and_fit_in_a_url() {
    let destination = Destination::parse(DEFAULT_DESTINATION).expect("default");
    assert_eq!(destination.github(), Some("adrianazizx/Mirsam"));
    let mut report = report_at(0);
    let (url, truncated) = destination.issue_url(&report).expect("url");
    assert!(
        url.starts_with("https://github.com/adrianazizx/Mirsam/issues/new?title=Crash%3A%20EXC_BAD_ACCESS")
    );
    assert!(url.contains("No%20document%20contents"));
    assert!(!truncated);

    report.stack = (0..2_000).map(|i| format!("frame_number_{i}::with::a::long::name")).collect();
    let (url, truncated) = destination.issue_url(&report).expect("url");
    assert!(truncated && url.len() <= MAX_ISSUE_URL, "{}", url.len());

    assert_eq!(Destination::parse("manual"), Ok(Destination::Manual { github: None }));
    assert!(Destination::parse("manual").expect("manual").issue_url(&report).is_none());
    assert!(Destination::parse("https://crash.example/api").is_err());
    assert!(Destination::parse("github:../x").is_err());
    assert!(Destination::parse("github:a/b/c").is_err());
}

#[test]
fn formats_times_in_utc() {
    assert_eq!(utc_minute(0), "1970-01-01 00:00 UTC");
    assert_eq!(utc_minute(1_763_894_530), "2025-11-23 10:42 UTC");
    assert_eq!(utc_minute(951_782_400), "2000-02-29 00:00 UTC");
}

#[test]
fn tells_follow_on_panics_from_first_ones() {
    assert!(is_follow_on_panic("panic in a function that cannot unwind"));
    assert!(is_follow_on_panic("panic in a destructor during cleanup"));
    assert!(!is_follow_on_panic("index out of bounds: the len is 3 but the index is 7"));
}
