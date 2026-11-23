//! The model downloader against the real Hugging Face servers: HTTPS with the
//! OS trust store, the redirect to their CDN, and resuming with `Range` there.
//!
//! Ignored by default (it needs the network). It fetches the Nuqta repository's
//! 1.1 MB logo, never a model:
//!
//! ```text
//! cargo test -p mirsam-core --test model_download_live -- --ignored
//! ```

use std::sync::atomic::{AtomicBool, Ordering};

use mirsam_core::models::{
    Asset, Catalog, Downloader, ModelError, ModelStore, Package, PackageState, Role, Unpack,
};

/// `nuqta_logo.png`, stored with Git LFS (so it is served through the CDN redirect).
fn logo() -> Package {
    Package {
        id: "live-logo".into(),
        role: Role::Draft,
        name: "Nuqta logo".into(),
        version: "4f72054".into(),
        licence: "Apache-2.0".into(),
        homepage: "https://huggingface.co/SalahALHaismawi/Nuqta-Arabic-OCR-0.8B-GGUF".into(),
        files: vec![Asset {
            name: "nuqta_logo.png".into(),
            url: "https://huggingface.co/SalahALHaismawi/Nuqta-Arabic-OCR-0.8B-GGUF/resolve/\
                  4f720541afdf85461f78bdc705509fe4382a1806/nuqta_logo.png"
                .into(),
            size: 1_164_512,
            sha256: "53399862b0bca580abb3c16299cd2661542313e1f90355904e64fe599bae5c66".into(),
            unpack: Unpack::None,
        }],
        program: None,
    }
}

#[test]
#[ignore = "needs the network; run with --ignored"]
fn downloads_and_resumes_from_hugging_face() {
    let root = std::env::temp_dir().join(format!("mirsam-models-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let store = ModelStore::new(&root);
    let package = logo();
    let downloader = Downloader::default();

    let cancel = AtomicBool::new(false);
    let first = store.install(&package, &downloader, &cancel, &mut |progress| {
        if progress.received > 400_000 {
            cancel.store(true, Ordering::Relaxed);
        }
    });
    assert!(matches!(first, Err(ModelError::Cancelled)), "{first:?}");
    let PackageState::Partial { received } = store.state(&package) else {
        panic!("{:?}", store.state(&package))
    };
    assert!(received < package.download_size());

    let installed =
        store.install(&package, &downloader, &AtomicBool::new(false), &mut |_| {}).expect("resumes");
    assert_eq!(std::fs::metadata(installed.file("nuqta_logo.png")).expect("file").len(), 1_164_512);
    assert_eq!(store.status(&Catalog::new(vec![package]))[0].state, PackageState::Installed);
    let _ = std::fs::remove_dir_all(&root);
}
