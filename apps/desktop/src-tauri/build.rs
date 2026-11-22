//! Tauri's build step, and the Windows application manifest for every binary
//! this crate links.

use std::path::Path;

fn main() {
    // Tauri embeds its manifest (a dependency on Common Controls v6, which the
    // windowing runtime and the dialogs need) into the app's binary only. A
    // test binary whose code reaches the runtime then fails to start on
    // Windows (STATUS_ENTRYPOINT_NOT_FOUND). So the same manifest goes to the
    // linker for every binary instead: the app, its tests, and examples.
    let windows = tauri_build::WindowsAttributes::new_without_app_manifest();
    if let Err(err) = tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows)) {
        panic!("tauri build step failed: {err:#}");
    }
    let target = |key: &str| std::env::var(key).unwrap_or_default();
    if target("CARGO_CFG_TARGET_OS") == "windows" && target("CARGO_CFG_TARGET_ENV") == "msvc" {
        let manifest = Path::new(&target("CARGO_MANIFEST_DIR")).join("windows-app-manifest.xml");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
    println!("cargo:rerun-if-changed=windows-app-manifest.xml");
}
