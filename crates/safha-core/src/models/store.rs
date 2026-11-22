//! The install folder: what is installed, installing, deleting.
//!
//! A package is installed when its folder holds a receipt
//! ([`RECEIPT`]), which is written last, inside the staging folder, before the
//! folder is renamed into place. A crash at any point leaves either the old
//! install or the new one, plus downloads to resume and leftovers that
//! [`ModelStore::clean`] removes.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::archive;
use super::{Asset, Catalog, Downloader, ModelError, Package, Role, Unpack};

/// The receipt's file name in an installed package's folder.
pub const RECEIPT: &str = ".mirsam-package.json";

/// Free space kept on top of what an install needs, so it never fills the disk.
const SPARE: u64 = 256 << 20;

/// Where a package stands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PackageState {
    NotInstalled,
    /// Part of it is downloaded (`received` bytes); installing resumes.
    Partial {
        received: u64,
    },
    Installed,
    /// Another version (`version`) is installed; installing replaces it.
    Outdated {
        version: String,
    },
}

/// One package as the settings panel shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageStatus {
    pub id: String,
    pub role: Role,
    pub name: String,
    pub version: String,
    pub licence: String,
    pub homepage: String,
    /// Bytes to download.
    pub download_size: u64,
    /// An upper bound of the disk space it takes once installed.
    pub installed_size: u64,
    pub state: PackageState,
}

/// What an install is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// Re-reading what an earlier attempt downloaded, to resume after it.
    Checking,
    Downloading,
    /// Unpacking an archive (the runtime).
    Unpacking,
    /// Moving the package into place.
    Installing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub phase: Phase,
    /// Bytes of the package downloaded so far, earlier attempts included.
    pub received: u64,
    /// Bytes of the whole package.
    pub total: u64,
}

/// An installed package's files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledPackage {
    pub id: String,
    pub version: String,
    /// The package's folder: a model's `.gguf` files are directly in it.
    pub dir: PathBuf,
    /// A runtime's program.
    pub program: Option<PathBuf>,
}

impl InstalledPackage {
    /// A file of the package by name.
    pub fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

/// The receipt, as written into the package's folder.
#[derive(Debug, Serialize, Deserialize)]
struct Receipt {
    id: String,
    version: String,
    files: Vec<ReceiptFile>,
    /// Relative to the package's folder, `/`-separated.
    program: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ReceiptFile {
    name: String,
    size: u64,
    sha256: String,
}

/// Free space on the volume holding a folder.
type FreeSpace = fn(&Path) -> io::Result<u64>;

/// The OCR models' folder in the app's data directory.
#[derive(Clone, Debug)]
pub struct ModelStore {
    root: PathBuf,
    free_space: FreeSpace,
}

impl ModelStore {
    /// The store in `root` (created when something is first installed).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into(), free_space: |path| fs4::available_space(path) }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn downloads(&self) -> PathBuf {
        self.root.join(".downloads")
    }

    fn staging(&self) -> PathBuf {
        self.root.join(".staging")
    }

    fn trash(&self) -> PathBuf {
        self.root.join(".trash")
    }

    /// Where `asset` downloads to: named by its checksum, so a changed pin never
    /// resumes onto the old file's bytes.
    fn part(&self, asset: &Asset) -> PathBuf {
        self.downloads().join(format!("{}.part", asset.sha256))
    }

    fn received(&self, asset: &Asset) -> u64 {
        fs::metadata(self.part(asset)).map_or(0, |meta| meta.len().min(asset.size))
    }

    /// Every package of `catalog` and where it stands.
    pub fn status(&self, catalog: &Catalog) -> Vec<PackageStatus> {
        catalog
            .packages()
            .iter()
            .map(|package| PackageStatus {
                id: package.id.clone(),
                role: package.role,
                name: package.name.clone(),
                version: package.version.clone(),
                licence: package.licence.clone(),
                homepage: package.homepage.clone(),
                download_size: package.download_size(),
                installed_size: package.installed_size(),
                state: self.state(package),
            })
            .collect()
    }

    pub fn state(&self, package: &Package) -> PackageState {
        match self.installed(&package.id) {
            Some(installed) if installed.version == package.version => return PackageState::Installed,
            Some(installed) => return PackageState::Outdated { version: installed.version },
            None => {}
        }
        let received: u64 = package.files.iter().map(|asset| self.received(asset)).sum();
        if received > 0 { PackageState::Partial { received } } else { PackageState::NotInstalled }
    }

    /// The installed package `id`, if its receipt is there and its files are.
    pub fn installed(&self, id: &str) -> Option<InstalledPackage> {
        let dir = self.root.join(id);
        let receipt: Receipt = serde_json::from_slice(&fs::read(dir.join(RECEIPT)).ok()?).ok()?;
        if receipt.id != id
            || receipt.files.iter().any(|file| file.size > 0 && !dir.join(&file.name).exists())
        {
            return None;
        }
        let program = match &receipt.program {
            Some(program) => {
                let path = program.split('/').fold(dir.clone(), |path, part| path.join(part));
                if !path.is_file() {
                    return None;
                }
                Some(path)
            }
            None => None,
        };
        Some(InstalledPackage { id: receipt.id, version: receipt.version, dir, program })
    }

    /// Bytes of free space installing `package` needs: what is still to
    /// download, room to unpack archives, and a margin.
    pub fn space_needed(&self, package: &Package) -> u64 {
        let download: u64 = package.files.iter().map(|asset| asset.size - self.received(asset)).sum();
        let unpack: u64 = package
            .files
            .iter()
            .filter(|asset| asset.unpack != Unpack::None)
            .map(Asset::installed_size)
            .sum();
        download + unpack + SPARE
    }

    /// Downloads and installs `package`, replacing an older version.
    ///
    /// Blocks until done; run it off the UI thread. `progress` gets every step.
    /// Setting `cancel` stops it at the next step with [`ModelError::Cancelled`];
    /// what was downloaded stays for the next try.
    pub fn install(
        &self,
        package: &Package,
        downloader: &Downloader,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<InstalledPackage, ModelError> {
        if let Some(installed) = self.installed(&package.id)
            && installed.version == package.version
        {
            return Ok(installed);
        }
        fs::create_dir_all(self.downloads())?;
        let needed = self.space_needed(package);
        let available = (self.free_space)(&self.root)?;
        if available < needed {
            return Err(ModelError::DiskFull { needed, available });
        }

        let total = package.download_size();
        let mut done = 0;
        for asset in &package.files {
            let resuming = self.received(asset);
            if resuming > 0 {
                progress(Progress { phase: Phase::Checking, received: done + resuming, total });
            }
            let result = downloader.fetch(asset, &self.part(asset), cancel, &mut |have| {
                progress(Progress { phase: Phase::Downloading, received: done + have, total });
            });
            match result {
                // Out of space while writing: say how much it needs.
                Err(ModelError::DiskFull { .. }) => {
                    return Err(ModelError::DiskFull {
                        needed: self.space_needed(package),
                        available: (self.free_space)(&self.root).unwrap_or(0),
                    });
                }
                other => other?,
            }
            done += asset.size;
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(ModelError::Cancelled);
        }
        self.put_in_place(package, cancel, &mut |phase| progress(Progress { phase, received: total, total }))
    }

    /// Assembles the downloaded files in a staging folder and swaps it in.
    fn put_in_place(
        &self,
        package: &Package,
        cancel: &AtomicBool,
        phase: &mut dyn FnMut(Phase),
    ) -> Result<InstalledPackage, ModelError> {
        let staging = self.staging().join(format!("{}-{}", package.id, nonce()));
        fs::create_dir_all(&staging)?;
        // Downloads moved into the staging folder, to move back if this fails.
        let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
        let result = (|| {
            for asset in &package.files {
                let part = self.part(asset);
                if asset.unpack == Unpack::None {
                    let target = staging.join(&asset.name);
                    fs::rename(&part, &target)?;
                    moved.push((target, part));
                } else {
                    phase(Phase::Unpacking);
                    archive::unpack(asset.unpack, &part, &staging, cancel)?;
                }
            }
            phase(Phase::Installing);
            let program = match &package.program {
                Some(program) => Some(find_program(&staging, program)?),
                None => None,
            };
            let receipt = Receipt {
                id: package.id.clone(),
                version: package.version.clone(),
                files: package
                    .files
                    .iter()
                    .filter(|asset| asset.unpack == Unpack::None)
                    .map(|asset| ReceiptFile {
                        name: asset.name.clone(),
                        size: asset.size,
                        sha256: asset.sha256.clone(),
                    })
                    .collect(),
                program,
            };
            {
                // Closed before the folder moves: Windows refuses to rename a
                // folder that holds an open file.
                let mut file = fs::File::create(staging.join(RECEIPT))?;
                file.write_all(&serde_json::to_vec_pretty(&receipt).map_err(io::Error::other)?)?;
                file.sync_all()?;
            }
            self.swap_in(&package.id, &staging)
        })();
        match result {
            Ok(()) => {
                // Unpacked archives are spent.
                for asset in package.files.iter().filter(|asset| asset.unpack != Unpack::None) {
                    let _ = fs::remove_file(self.part(asset));
                }
                self.installed(&package.id)
                    .ok_or_else(|| ModelError::Io(io::Error::other("the installed package cannot be read")))
            }
            Err(err) => {
                for (target, part) in moved {
                    let _ = fs::rename(target, part);
                }
                let _ = fs::remove_dir_all(&staging);
                Err(err)
            }
        }
    }

    /// Renames `staging` to the package's folder, moving an older install aside first.
    fn swap_in(&self, id: &str, staging: &Path) -> Result<(), ModelError> {
        let target = self.root.join(id);
        let old = if fs::symlink_metadata(&target).is_ok() {
            fs::create_dir_all(self.trash())?;
            let old = self.trash().join(format!("{id}-{}", nonce()));
            fs::rename(&target, &old)?;
            Some(old)
        } else {
            None
        };
        if let Err(err) = fs::rename(staging, &target) {
            if let Some(old) = &old {
                let _ = fs::rename(old, &target);
            }
            return Err(err.into());
        }
        if let Some(old) = old {
            let _ = fs::remove_dir_all(old);
        }
        Ok(())
    }

    /// Removes `package`: its install and any partial download.
    pub fn delete(&self, package: &Package) -> Result<(), ModelError> {
        let target = self.root.join(&package.id);
        if fs::symlink_metadata(&target).is_ok() {
            // Out of sight first, so a half-deleted folder never reads as installed.
            fs::create_dir_all(self.trash())?;
            let old = self.trash().join(format!("{}-{}", package.id, nonce()));
            fs::rename(&target, &old)?;
            fs::remove_dir_all(old)?;
        }
        self.discard_download(package)
    }

    /// Removes what was downloaded of `package` but not installed.
    pub fn discard_download(&self, package: &Package) -> Result<(), ModelError> {
        for asset in &package.files {
            match fs::remove_file(self.part(asset)) {
                Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err.into()),
                _ => {}
            }
        }
        Ok(())
    }

    /// Removes leftovers of interrupted installs and deletes. Call it when no
    /// install is running (at startup).
    pub fn clean(&self) {
        let _ = fs::remove_dir_all(self.staging());
        let _ = fs::remove_dir_all(self.trash());
    }
}

/// The runtime's program in an unpacked package: where the catalog says, or
/// else the one file of that name (a release that moved it into a folder).
fn find_program(dir: &Path, declared: &str) -> Result<String, ModelError> {
    if declared.split('/').fold(dir.to_path_buf(), |path, part| path.join(part)).is_file() {
        return Ok(declared.to_owned());
    }
    let name = declared.rsplit('/').next().unwrap_or(declared);
    let mut found = Vec::new();
    let mut folders = vec![(dir.to_path_buf(), String::new())];
    while let Some((folder, prefix)) = folders.pop() {
        for entry in fs::read_dir(&folder)? {
            let entry = entry?;
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let relative =
                if prefix.is_empty() { file_name.clone() } else { format!("{prefix}/{file_name}") };
            let kind = entry.file_type()?;
            if kind.is_dir() && relative.matches('/').count() < 2 {
                folders.push((entry.path(), relative));
            } else if kind.is_file() && file_name == name {
                found.push(relative);
            }
        }
    }
    match found.as_slice() {
        [only] => Ok(only.clone()),
        _ => Err(ModelError::Archive(format!("{declared} is not in the archive"))),
    }
}

/// A name no other install or delete of this process uses.
fn nonce() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let time = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_nanos());
    format!("{}-{time:x}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use sha2::{Digest, Sha256};

    use super::super::archive::tests::{TarItem, make_tar_gz, make_zip};
    use super::super::download::hex;
    use super::super::test_server::{Server, Trouble};
    use super::super::{DownloadConfig, Downloader};
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("safha-models-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn downloader() -> Downloader {
        Downloader::new(DownloadConfig {
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(5),
            retries: 3,
            retry_delay: Duration::from_millis(1),
            user_agent: "Mirsam-test".into(),
        })
    }

    /// Deterministic bytes that are not all the same.
    fn data(len: usize, seed: u8) -> Vec<u8> {
        (0..len).map(|i| (i as u32).wrapping_mul(2_654_435_761).to_le_bytes()[1] ^ seed).collect()
    }

    fn asset(server: &Server, path: &str, bytes: &[u8], unpack: Unpack) -> Asset {
        let name = path.rsplit('/').next().unwrap_or(path).to_owned();
        Asset {
            name,
            url: server.url(path),
            size: bytes.len() as u64,
            sha256: hex(&Sha256::digest(bytes)),
            unpack,
        }
    }

    fn model(id: &str, version: &str, files: Vec<Asset>) -> Package {
        Package {
            id: id.into(),
            role: Role::Primary,
            name: "Test model".into(),
            version: version.into(),
            licence: "Apache-2.0".into(),
            homepage: "https://example.invalid".into(),
            files,
            program: None,
        }
    }

    fn install(
        store: &ModelStore,
        package: &Package,
    ) -> (Result<InstalledPackage, ModelError>, Vec<Progress>) {
        let mut seen = Vec::new();
        let result = store.install(package, &downloader(), &AtomicBool::new(false), &mut |p| seen.push(p));
        (result, seen)
    }

    #[test]
    fn installs_a_model_and_reports_it() {
        let server = Server::start();
        let weights = data(1_300_000, 1);
        let encoder = data(70_000, 2);
        server.add("/m/weights.gguf", &weights);
        server.add("/m/mmproj.gguf", &encoder);
        let package = model(
            "test-model",
            "v1",
            vec![
                asset(&server, "/m/weights.gguf", &weights, Unpack::None),
                asset(&server, "/m/mmproj.gguf", &encoder, Unpack::None),
            ],
        );
        let catalog = Catalog::new(vec![package.clone()]);
        let store = ModelStore::new(scratch("fresh"));
        assert_eq!(store.status(&catalog)[0].state, PackageState::NotInstalled);
        assert_eq!(store.status(&catalog)[0].download_size, 1_370_000);

        let (result, seen) = install(&store, &package);
        let installed = result.expect("installs");
        assert_eq!(fs::read(installed.file("weights.gguf")).expect("weights"), weights);
        assert_eq!(fs::read(installed.file("mmproj.gguf")).expect("encoder"), encoder);
        assert_eq!(installed.dir, store.root().join("test-model"));
        assert_eq!(installed.program, None);
        assert_eq!(store.status(&catalog)[0].state, PackageState::Installed);
        assert_eq!(store.installed("test-model"), Some(installed));

        // Progress only grows, and ends with the whole package in place.
        assert!(seen.windows(2).all(|pair| pair[0].received <= pair[1].received), "{seen:?}");
        assert!(seen.iter().all(|p| p.total == 1_370_000));
        assert_eq!(seen.last().map(|p| p.phase), Some(Phase::Installing));
        // Nothing left behind.
        assert_eq!(fs::read_dir(store.downloads()).expect("downloads").count(), 0);
        assert_eq!(fs::read_dir(store.staging()).expect("staging").count(), 0);

        // Installing again downloads nothing.
        let before = server.requests().len();
        install(&store, &package).0.expect("already there");
        assert_eq!(server.requests().len(), before);
    }

    #[test]
    fn cancel_keeps_the_download_and_install_resumes_it() {
        let server = Server::start();
        let weights = data(2_000_000, 3);
        server.add("/files/weights.gguf", &weights);
        // Through a redirect, as Hugging Face serves files: the range must survive it.
        let mut file = asset(&server, "/files/weights.gguf", &weights, Unpack::None);
        file.url = server.url("/redirect/files/weights.gguf");
        let package = model("resumable", "v1", vec![file]);
        let store = ModelStore::new(scratch("resume"));

        let cancel = AtomicBool::new(false);
        let result = store.install(&package, &downloader(), &cancel, &mut |p| {
            if p.received > 600_000 {
                cancel.store(true, Ordering::Relaxed);
            }
        });
        assert!(matches!(result, Err(ModelError::Cancelled)), "{result:?}");
        let PackageState::Partial { received } = store.state(&package) else {
            panic!("expected a partial download, got {:?}", store.state(&package));
        };
        assert!(received > 600_000 && received < 2_000_000, "{received}");

        let (result, seen) = install(&store, &package);
        result.expect("resumes");
        assert_eq!(seen.first().map(|p| (p.phase, p.received)), Some((Phase::Checking, received)));
        let ranges: Vec<String> = server.requests().into_iter().filter_map(|(_, range)| range).collect();
        assert_eq!(ranges, [format!("bytes={received}-")], "asked only for the rest");
        assert_eq!(fs::read(store.root().join("resumable/weights.gguf")).expect("file"), weights);
    }

    #[test]
    fn dropped_connections_and_busy_servers_are_retried() {
        let server = Server::start();
        let weights = data(900_000, 4);
        server.add("/w.gguf", &weights);
        server.trouble(Trouble::CutAfter { bytes: 300_000, times: 2 });
        let package = model("flaky", "v1", vec![asset(&server, "/w.gguf", &weights, Unpack::None)]);
        let store = ModelStore::new(scratch("flaky"));
        install(&store, &package).0.expect("retries past the drops");
        assert_eq!(fs::read(store.root().join("flaky/w.gguf")).expect("file"), weights);
        let ranges: Vec<Option<String>> = server.requests().into_iter().map(|(_, range)| range).collect();
        assert_eq!(ranges, [None, Some("bytes=300000-".into()), Some("bytes=600000-".into())]);

        let busy = Server::start();
        busy.add("/w.gguf", &weights);
        busy.trouble(Trouble::Status { status: 503, times: 2 });
        let package = model("busy", "v1", vec![asset(&busy, "/w.gguf", &weights, Unpack::None)]);
        install(&store, &package).0.expect("retries past 503");
        assert_eq!(busy.requests().len(), 3);
    }

    #[test]
    fn a_server_without_ranges_starts_over() {
        let server = Server::start();
        let weights = data(800_000, 5);
        server.add("/w.gguf", &weights);
        server.trouble(Trouble::NoRanges);
        let package = model("no-ranges", "v1", vec![asset(&server, "/w.gguf", &weights, Unpack::None)]);
        let store = ModelStore::new(scratch("no-ranges"));
        fs::create_dir_all(store.downloads()).expect("downloads");
        fs::write(store.part(&package.files[0]), &weights[..1000]).expect("partial");
        install(&store, &package).0.expect("installs");
        assert_eq!(fs::read(store.root().join("no-ranges/w.gguf")).expect("file"), weights);
    }

    #[test]
    fn a_different_file_is_refused_and_deleted() {
        let server = Server::start();
        let weights = data(500_000, 6);
        server.add("/w.gguf", &weights);
        let store = ModelStore::new(scratch("mismatch"));

        let mut wrong_hash = asset(&server, "/w.gguf", &weights, Unpack::None);
        wrong_hash.sha256 = hex(&Sha256::digest(b"something else"));
        let package = model("mismatch", "v1", vec![wrong_hash]);
        let (result, _) = install(&store, &package);
        assert!(matches!(result, Err(ModelError::ChecksumMismatch { .. })), "{result:?}");
        assert_eq!(store.state(&package), PackageState::NotInstalled, "the bad file is gone");

        let mut wrong_size = asset(&server, "/w.gguf", &weights, Unpack::None);
        wrong_size.size -= 1;
        let package = model("short", "v1", vec![wrong_size]);
        assert!(matches!(install(&store, &package).0, Err(ModelError::ChecksumMismatch { .. })));
        assert!(store.installed("short").is_none());
    }

    #[test]
    fn missing_files_fail_without_retrying() {
        let server = Server::start();
        let package = model("missing", "v1", vec![asset(&server, "/nope.gguf", b"x", Unpack::None)]);
        let store = ModelStore::new(scratch("missing"));
        let (result, _) = install(&store, &package);
        assert!(matches!(result, Err(ModelError::Http { status: 404, .. })), "{result:?}");
        assert_eq!(server.requests().len(), 1);
        assert_eq!(result.map_err(|e| e.code()).err(), Some("model_download_failed"));
    }

    #[test]
    fn too_little_disk_space_downloads_nothing() {
        let server = Server::start();
        let weights = data(100_000, 7);
        server.add("/w.gguf", &weights);
        let package = model("big", "v1", vec![asset(&server, "/w.gguf", &weights, Unpack::None)]);
        let mut store = ModelStore::new(scratch("disk"));
        store.free_space = |_| Ok(1 << 20);
        let (result, _) = install(&store, &package);
        let Err(ModelError::DiskFull { needed, available }) = result else { panic!("{result:?}") };
        assert_eq!((needed, available), (100_000 + SPARE, 1 << 20));
        assert!(server.requests().is_empty());
    }

    #[test]
    fn runtime_archives_unpack_and_name_their_program() {
        let server = Server::start();
        let tar = make_tar_gz(&[
            TarItem::Dir("./"),
            TarItem::Dir("./llama-b1/"),
            TarItem::File("./llama-b1/llama-server", b"#!/bin/sh\n", 0o755),
            TarItem::File("./llama-b1/libggml.dylib", b"lib", 0o644),
        ]);
        server.add("/llama.tar.gz", &tar);
        let store = ModelStore::new(scratch("runtime"));
        let mut package =
            model("runtime-tar", "b1", vec![asset(&server, "/llama.tar.gz", &tar, Unpack::TarGz)]);
        package.role = Role::Runtime;
        package.program = Some("llama-b1/llama-server".into());
        let installed = install(&store, &package).0.expect("tar installs");
        let program = installed.program.expect("program");
        assert_eq!(fs::read(&program).expect("program"), b"#!/bin/sh\n");
        assert!(program.starts_with(store.root().join("runtime-tar")));
        assert_eq!(fs::read_dir(store.downloads()).expect("downloads").count(), 0, "the archive is removed");

        // A zip whose program is not where the catalog says: found by name.
        let zip = make_zip(&[("bin/llama-server.exe", b"MZ", None), ("bin/ggml.dll", b"dll", None)], false);
        server.add("/llama.zip", &zip);
        let mut package = model("runtime-zip", "b1", vec![asset(&server, "/llama.zip", &zip, Unpack::Zip)]);
        package.program = Some("llama-server.exe".into());
        let installed = install(&store, &package).0.expect("zip installs");
        assert_eq!(
            installed.program,
            Some(store.root().join("runtime-zip").join("bin").join("llama-server.exe"))
        );

        // No such program: the install fails and leaves nothing half-done.
        let mut package = model("runtime-none", "b1", vec![asset(&server, "/llama.zip", &zip, Unpack::Zip)]);
        package.program = Some("llama-cli".into());
        assert!(matches!(install(&store, &package).0, Err(ModelError::Archive(_))));
        assert!(!store.root().join("runtime-none").exists());
        assert_eq!(fs::read_dir(store.staging()).expect("staging").count(), 0);
    }

    #[test]
    fn a_new_version_replaces_the_old_and_delete_removes_everything() {
        let server = Server::start();
        let old = data(10_000, 8);
        let new = data(12_000, 9);
        server.add("/v1/w.gguf", &old);
        server.add("/v2/w.gguf", &new);
        let store = ModelStore::new(scratch("versions"));
        let v1 = model("versioned", "v1", vec![asset(&server, "/v1/w.gguf", &old, Unpack::None)]);
        let v2 = model("versioned", "v2", vec![asset(&server, "/v2/w.gguf", &new, Unpack::None)]);
        install(&store, &v1).0.expect("v1");
        assert_eq!(store.state(&v2), PackageState::Outdated { version: "v1".into() });
        let installed = install(&store, &v2).0.expect("v2");
        assert_eq!(installed.version, "v2");
        assert_eq!(fs::read(installed.file("w.gguf")).expect("file"), new);
        assert_eq!(fs::read_dir(store.trash()).expect("trash").count(), 0);

        // A partial download of something else is removed with it too.
        fs::write(store.part(&v1.files[0]), b"partial").expect("partial");
        store.delete(&v1).expect("delete");
        assert!(!store.root().join("versioned").exists());
        assert_eq!(store.state(&v1), PackageState::NotInstalled);
        store.delete(&v1).expect("deleting twice is fine");
    }

    #[test]
    fn a_damaged_install_does_not_count() {
        let server = Server::start();
        let weights = data(5_000, 10);
        server.add("/w.gguf", &weights);
        let store = ModelStore::new(scratch("damaged"));
        let package = model("damaged", "v1", vec![asset(&server, "/w.gguf", &weights, Unpack::None)]);
        install(&store, &package).0.expect("installs");
        fs::remove_file(store.root().join("damaged/w.gguf")).expect("remove");
        assert_eq!(store.state(&package), PackageState::NotInstalled);
        fs::write(store.root().join("damaged").join(RECEIPT), b"{not json").expect("receipt");
        assert!(store.installed("damaged").is_none());
        install(&store, &package).0.expect("installs again over the damage");
        assert_eq!(store.state(&package), PackageState::Installed);
    }

    #[test]
    fn two_packages_install_at_once() {
        let server = Server::start();
        let a = data(600_000, 11);
        let b = data(700_000, 12);
        server.add("/a.gguf", &a);
        server.add("/b.gguf", &b);
        let store = Arc::new(ModelStore::new(scratch("parallel")));
        let first = model("a", "v1", vec![asset(&server, "/a.gguf", &a, Unpack::None)]);
        let second = model("b", "v1", vec![asset(&server, "/b.gguf", &b, Unpack::None)]);
        let threads: Vec<_> = [first, second]
            .into_iter()
            .map(|package| {
                let store = Arc::clone(&store);
                std::thread::spawn(move || install(&store, &package).0.map(|p| p.id))
            })
            .collect();
        for thread in threads {
            thread.join().expect("thread").expect("installs");
        }
        assert!(store.installed("a").is_some() && store.installed("b").is_some());
        store.clean();
        assert!(!store.staging().exists());
    }
}
