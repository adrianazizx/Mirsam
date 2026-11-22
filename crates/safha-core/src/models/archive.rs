//! Unpacking the runtime's archives: llama.cpp ships `.tar.gz` for macOS and
//! Linux and `.zip` for Windows.
//!
//! The archives are checked against their pinned SHA-256 before they get here,
//! but unpacking still refuses anything that could land outside the target
//! folder: absolute paths, `..`, drive prefixes, links that point out, and
//! entry kinds other than files, folders and (tar, Unix) relative symlinks.

use std::fs::{self, File};
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use flate2::read::{DeflateDecoder, GzDecoder};

use super::{ModelError, Unpack};

/// The most an archive may unpack to (the runtime's builds unpack to ~120 MB).
const MAX_UNPACKED: u64 = 4 << 30;

/// Unpacks `archive` (of kind `kind`) into the existing folder `into`.
pub(crate) fn unpack(
    kind: Unpack,
    archive: &Path,
    into: &Path,
    cancel: &AtomicBool,
) -> Result<(), ModelError> {
    match kind {
        Unpack::None => Err(ModelError::Archive("not an archive".into())),
        Unpack::TarGz => unpack_tar_gz(archive, into, cancel),
        Unpack::Zip => unpack_zip(archive, into, cancel),
    }
}

fn bad(message: impl Into<String>) -> ModelError {
    ModelError::Archive(message.into())
}

/// `name` as a path below the target folder, or `None` if it could escape it.
/// Both `/` and `\` separate (zips made on Windows use either).
fn safe_relative(name: &str) -> Option<PathBuf> {
    if name.starts_with(['/', '\\']) {
        return None;
    }
    let mut out = PathBuf::new();
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => continue,
            ".." => return None,
            // A drive (`C:`) or an NTFS stream (`file:stream`).
            part if part.contains(':') || part.contains('\0') => return None,
            part => out.push(part),
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// macOS's tar adds `._name` files for extended attributes; they are not
/// part of the build.
fn is_apple_double(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with("._"))
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), ModelError> {
    if cancel.load(Ordering::Relaxed) { Err(ModelError::Cancelled) } else { Ok(()) }
}

/// Writes `reader` to `path`, at most `limit` bytes, creating its folder.
fn write_file(path: &Path, reader: &mut dyn Read, limit: u64, written: &mut u64) -> Result<(), ModelError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // A link or file already there (a duplicate entry) is replaced, never followed.
    if fs::symlink_metadata(path).is_ok() {
        fs::remove_file(path)?;
    }
    let mut file = File::create(path)?;
    let copied = io::copy(&mut reader.take(limit.saturating_add(1)), &mut file)?;
    if copied > limit {
        return Err(bad("an entry is larger than it says"));
    }
    *written += copied;
    if *written > MAX_UNPACKED {
        return Err(bad("the archive unpacks to too much"));
    }
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    // Keep the execute bits (the programs and libraries); nothing else.
    let mode = if mode & 0o111 != 0 { 0o755 } else { 0o644 };
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> io::Result<()> {
    Ok(())
}

fn unpack_tar_gz(archive: &Path, into: &Path, cancel: &AtomicBool) -> Result<(), ModelError> {
    let mut tar = tar::Archive::new(GzDecoder::new(BufReader::new(File::open(archive)?)));
    let mut written = 0;
    for entry in tar.entries().map_err(|err| bad(err.to_string()))? {
        check_cancel(cancel)?;
        let mut entry = entry.map_err(|err| bad(err.to_string()))?;
        let name = entry.path().map_err(|err| bad(err.to_string()))?.to_string_lossy().into_owned();
        let kind = entry.header().entry_type();
        if kind.is_pax_global_extensions() || kind.is_pax_local_extensions() {
            continue;
        }
        // The top folder itself (`./`).
        let Some(relative) = safe_relative(&name) else {
            if kind.is_dir() && name.split(['/', '\\']).all(|part| part.is_empty() || part == ".") {
                continue;
            }
            return Err(bad(format!("unsafe path {name:?}")));
        };
        if is_apple_double(&relative) {
            continue;
        }
        let target = into.join(&relative);
        if kind.is_dir() {
            fs::create_dir_all(&target)?;
        } else if kind.is_file() || kind == tar::EntryType::Continuous {
            let size = entry.header().size().map_err(|err| bad(err.to_string()))?;
            let mode = entry.header().mode().unwrap_or(0o644);
            write_file(&target, &mut entry, size, &mut written)?;
            set_mode(&target, mode)?;
        } else if kind.is_symlink() {
            let link = entry
                .link_name()
                .map_err(|err| bad(err.to_string()))?
                .ok_or_else(|| bad(format!("{name:?}: a link without a target")))?;
            symlink_inside(&relative, &link, into)?;
        } else {
            return Err(bad(format!("{name:?}: unsupported entry type")));
        }
    }
    Ok(())
}

/// Creates the symlink `relative` → `link` if `link` stays inside `into`.
fn symlink_inside(relative: &Path, link: &Path, into: &Path) -> Result<(), ModelError> {
    let mut resolved = relative.parent().map(Path::to_path_buf).unwrap_or_default();
    for component in link.components() {
        match component {
            Component::Normal(part) => resolved.push(part),
            Component::CurDir => {}
            Component::ParentDir if resolved.pop() => {}
            _ => return Err(bad(format!("{}: a link that points outside", relative.display()))),
        }
    }
    let target = into.join(relative);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::symlink_metadata(&target).is_ok() {
        fs::remove_file(&target)?;
    }
    make_symlink(link, &target)
}

#[cfg(unix)]
fn make_symlink(link: &Path, at: &Path) -> Result<(), ModelError> {
    Ok(std::os::unix::fs::symlink(link, at)?)
}

#[cfg(not(unix))]
fn make_symlink(_link: &Path, at: &Path) -> Result<(), ModelError> {
    Err(bad(format!("{}: symlinks are not unpacked here", at.display())))
}

// Zip: read through the central directory at the end of the file. Stored and
// deflated entries only, no encryption, no zip64 (the runtime's zips are tens
// of megabytes and 7-Zip writes plain records for them).

const END_OF_DIRECTORY: u32 = 0x0605_4b50;
const DIRECTORY_ENTRY: u32 = 0x0201_4b50;
const LOCAL_HEADER: u32 = 0x0403_4b50;

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

struct ZipEntry {
    name: String,
    method: u16,
    crc: u32,
    compressed: u64,
    size: u64,
    offset: u64,
    /// Unix permissions when the archive was made on Unix.
    mode: Option<u32>,
}

fn unpack_zip(archive: &Path, into: &Path, cancel: &AtomicBool) -> Result<(), ModelError> {
    let mut file = File::open(archive)?;
    let entries = zip_directory(&mut file)?;
    let mut written = 0;
    for entry in entries {
        check_cancel(cancel)?;
        let Some(relative) = safe_relative(&entry.name) else {
            return Err(bad(format!("unsafe path {:?}", entry.name)));
        };
        let target = into.join(&relative);
        if entry.name.ends_with(['/', '\\']) {
            fs::create_dir_all(&target)?;
            continue;
        }
        if entry.mode.is_some_and(|mode| mode & 0o170_000 == 0o120_000) {
            return Err(bad(format!("{:?}: links are not unpacked from zips", entry.name)));
        }
        // The data follows the local header, whose name and extra lengths may
        // differ from the directory's.
        file.seek(SeekFrom::Start(entry.offset))?;
        let mut local = [0; 30];
        file.read_exact(&mut local)?;
        if u32_at(&local, 0) != Some(LOCAL_HEADER) {
            return Err(bad("a damaged entry header"));
        }
        let skip = u64::from(u16_at(&local, 26).unwrap_or(0)) + u64::from(u16_at(&local, 28).unwrap_or(0));
        file.seek(SeekFrom::Current(skip as i64))?;
        let data = (&mut file).take(entry.compressed);
        let mut reader: Box<dyn Read + '_> = match entry.method {
            0 => Box::new(data),
            8 => Box::new(DeflateDecoder::new(data)),
            method => return Err(bad(format!("{:?}: compression method {method}", entry.name))),
        };
        let mut checked = CrcReader { inner: &mut reader, crc: flate2::Crc::new() };
        write_file(&target, &mut checked, entry.size, &mut written)?;
        if checked.crc.amount() as u64 != entry.size & 0xffff_ffff || checked.crc.sum() != entry.crc {
            return Err(bad(format!("{:?}: the data does not match its checksum", entry.name)));
        }
        set_mode(&target, entry.mode.unwrap_or(0o644))?;
    }
    Ok(())
}

/// Reads the central directory.
fn zip_directory(file: &mut File) -> Result<Vec<ZipEntry>, ModelError> {
    let len = file.seek(SeekFrom::End(0))?;
    // The end record is 22 bytes plus a comment of at most 65,535.
    let tail_len = len.min(22 + 65_535);
    file.seek(SeekFrom::Start(len - tail_len))?;
    let mut tail = vec![0; tail_len as usize];
    file.read_exact(&mut tail)?;
    let end = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&at| u32_at(&tail, at) == Some(END_OF_DIRECTORY))
        .ok_or_else(|| bad("not a zip file"))?;
    let count = u16_at(&tail, end + 10).ok_or_else(|| bad("a damaged end record"))?;
    let size = u32_at(&tail, end + 12).ok_or_else(|| bad("a damaged end record"))?;
    let offset = u32_at(&tail, end + 16).ok_or_else(|| bad("a damaged end record"))?;
    if count == u16::MAX || size == u32::MAX || offset == u32::MAX {
        return Err(bad("zip64 archives are not supported"));
    }
    if u64::from(offset) + u64::from(size) > len {
        return Err(bad("a damaged end record"));
    }
    file.seek(SeekFrom::Start(offset.into()))?;
    let mut directory = vec![0; size as usize];
    file.read_exact(&mut directory)?;

    let damaged = || bad("a damaged directory");
    let mut entries = Vec::with_capacity(count.into());
    let mut at = 0;
    for _ in 0..count {
        if u32_at(&directory, at) != Some(DIRECTORY_ENTRY) {
            return Err(damaged());
        }
        let made_by = u16_at(&directory, at + 4).ok_or_else(damaged)?;
        let flags = u16_at(&directory, at + 8).ok_or_else(damaged)?;
        let method = u16_at(&directory, at + 10).ok_or_else(damaged)?;
        let crc = u32_at(&directory, at + 16).ok_or_else(damaged)?;
        let compressed = u32_at(&directory, at + 20).ok_or_else(damaged)?;
        let size = u32_at(&directory, at + 24).ok_or_else(damaged)?;
        let name_len = usize::from(u16_at(&directory, at + 28).ok_or_else(damaged)?);
        let extra_len = usize::from(u16_at(&directory, at + 30).ok_or_else(damaged)?);
        let comment_len = usize::from(u16_at(&directory, at + 32).ok_or_else(damaged)?);
        let external = u32_at(&directory, at + 38).ok_or_else(damaged)?;
        let local = u32_at(&directory, at + 42).ok_or_else(damaged)?;
        let name = directory.get(at + 46..at + 46 + name_len).ok_or_else(damaged)?;
        if flags & 1 != 0 {
            return Err(bad("encrypted entries are not supported"));
        }
        if compressed == u32::MAX || size == u32::MAX || local == u32::MAX {
            return Err(bad("zip64 archives are not supported"));
        }
        entries.push(ZipEntry {
            name: String::from_utf8_lossy(name).into_owned(),
            method,
            crc,
            compressed: compressed.into(),
            size: size.into(),
            offset: local.into(),
            // "Made by" Unix: the permissions sit in the high half.
            mode: (made_by >> 8 == 3 && external >> 16 != 0).then_some(external >> 16),
        });
        at += 46 + name_len + extra_len + comment_len;
    }
    Ok(entries)
}

/// Computes the CRC-32 of what passes through.
struct CrcReader<'a> {
    inner: &'a mut dyn Read,
    crc: flate2::Crc,
}

impl Read for CrcReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.crc.update(&buffer[..read]);
        Ok(read)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Write;

    use flate2::Compression;
    use flate2::write::{DeflateEncoder, GzEncoder};

    use super::*;

    /// A zip of `(name, data, unix mode)` entries; deflated unless `stored`.
    pub(crate) fn make_zip(entries: &[(&str, &[u8], Option<u32>)], stored: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let mut directory = Vec::new();
        for &(name, data, mode) in entries {
            let mut crc = flate2::Crc::new();
            crc.update(data);
            let body = if stored {
                data.to_vec()
            } else {
                let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
                encoder.write_all(data).expect("deflate");
                encoder.finish().expect("deflate")
            };
            let method: u16 = if stored { 0 } else { 8 };
            let offset = out.len() as u32;
            out.extend_from_slice(&LOCAL_HEADER.to_le_bytes());
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0; 4]);
            out.extend_from_slice(&crc.sum().to_le_bytes());
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            // An extra field in the local header only, as some tools write.
            out.extend_from_slice(&4u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&[0xfe, 0xca, 0, 0]);
            out.extend_from_slice(&body);

            directory.extend_from_slice(&DIRECTORY_ENTRY.to_le_bytes());
            let made_by: u16 = if mode.is_some() { 3 << 8 | 20 } else { 20 };
            directory.extend_from_slice(&made_by.to_le_bytes());
            directory.extend_from_slice(&20u16.to_le_bytes());
            directory.extend_from_slice(&0u16.to_le_bytes());
            directory.extend_from_slice(&method.to_le_bytes());
            directory.extend_from_slice(&[0; 4]);
            directory.extend_from_slice(&crc.sum().to_le_bytes());
            directory.extend_from_slice(&(body.len() as u32).to_le_bytes());
            directory.extend_from_slice(&(data.len() as u32).to_le_bytes());
            directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
            directory.extend_from_slice(&[0; 2 + 2 + 2 + 2]);
            directory.extend_from_slice(&(mode.unwrap_or(0) << 16).to_le_bytes());
            directory.extend_from_slice(&offset.to_le_bytes());
            directory.extend_from_slice(name.as_bytes());
        }
        let directory_offset = out.len() as u32;
        out.extend_from_slice(&directory);
        out.extend_from_slice(&END_OF_DIRECTORY.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(directory.len() as u32).to_le_bytes());
        out.extend_from_slice(&directory_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    pub(crate) enum TarItem<'a> {
        File(&'a str, &'a [u8], u32),
        Dir(&'a str),
        Link(&'a str, &'a str),
    }

    /// A .tar.gz of the given items. Names are written raw, so unsafe ones can be tested.
    pub(crate) fn make_tar_gz(items: &[TarItem<'_>]) -> Vec<u8> {
        let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
        for item in items {
            let mut header = tar::Header::new_gnu();
            let (name, kind, data, mode, link): (&str, _, &[u8], u32, Option<&str>) = match *item {
                TarItem::File(name, data, mode) => (name, tar::EntryType::Regular, data, mode, None),
                TarItem::Dir(name) => (name, tar::EntryType::Directory, &[], 0o755, None),
                TarItem::Link(name, target) => (name, tar::EntryType::Symlink, &[], 0o777, Some(target)),
            };
            header.as_gnu_mut().expect("gnu").name[..name.len()].copy_from_slice(name.as_bytes());
            header.set_entry_type(kind);
            header.set_size(data.len() as u64);
            header.set_mode(mode);
            if let Some(link) = link {
                header.as_gnu_mut().expect("gnu").linkname[..link.len()].copy_from_slice(link.as_bytes());
            }
            header.set_cksum();
            builder.append(&header, data).expect("tar");
        }
        builder.into_inner().expect("tar").finish().expect("gzip")
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("safha-archive-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn unpack_bytes(kind: Unpack, bytes: &[u8], name: &str) -> (PathBuf, Result<(), ModelError>) {
        let dir = scratch(name);
        let archive = dir.join("archive");
        fs::write(&archive, bytes).expect("write archive");
        let into = dir.join("out");
        fs::create_dir_all(&into).expect("out");
        let result = unpack(kind, &archive, &into, &AtomicBool::new(false));
        (into, result)
    }

    #[test]
    fn safe_relative_paths() {
        assert_eq!(safe_relative("a/b.dll"), Some(PathBuf::from("a").join("b.dll")));
        assert_eq!(safe_relative("./llama/llama-server"), Some(PathBuf::from("llama").join("llama-server")));
        assert_eq!(safe_relative("bin\\x.exe"), Some(PathBuf::from("bin").join("x.exe")));
        for unsafe_name in
            ["../x", "a/../../x", "/etc/passwd", "C:/x", "a/b:stream", "", "./", "\\\\server\\x/.."]
        {
            assert_eq!(safe_relative(unsafe_name), None, "{unsafe_name:?}");
        }
    }

    #[test]
    fn zip_deflated_and_stored() {
        for stored in [false, true] {
            let program = b"#!/bin/sh\necho llama\n".repeat(100);
            let bytes = make_zip(
                &[
                    ("llama-server.exe", &program, None),
                    ("lib/", b"", None),
                    ("lib/ggml.dll", b"ggml", None),
                    ("tool", b"x", Some(0o100_755)),
                ],
                stored,
            );
            let (out, result) = unpack_bytes(Unpack::Zip, &bytes, if stored { "zip-stored" } else { "zip" });
            result.expect("unpacks");
            assert_eq!(fs::read(out.join("llama-server.exe")).expect("program"), program);
            assert_eq!(fs::read(out.join("lib").join("ggml.dll")).expect("lib"), b"ggml");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = fs::metadata(out.join("tool")).expect("tool").permissions().mode();
                assert_eq!(mode & 0o777, 0o755);
            }
        }
    }

    #[test]
    fn zip_refuses_escapes_and_damage() {
        let escape = make_zip(&[("../evil.dll", b"x", None)], false);
        let (out, result) = unpack_bytes(Unpack::Zip, &escape, "zip-escape");
        assert!(matches!(result, Err(ModelError::Archive(_))));
        assert!(!out.parent().expect("parent").join("evil.dll").exists());

        let mut corrupt = make_zip(&[("a.dll", b"hello hello hello", None)], true);
        // Flip a data byte: the CRC no longer matches.
        let at = corrupt.windows(5).position(|w| w == b"hello").expect("data");
        corrupt[at] = b'j';
        assert!(matches!(unpack_bytes(Unpack::Zip, &corrupt, "zip-crc").1, Err(ModelError::Archive(_))));

        assert!(matches!(unpack_bytes(Unpack::Zip, b"not a zip", "zip-junk").1, Err(ModelError::Archive(_))));
        let link = make_zip(&[("link", b"/etc/passwd", Some(0o120_777))], true);
        assert!(matches!(unpack_bytes(Unpack::Zip, &link, "zip-link").1, Err(ModelError::Archive(_))));
    }

    #[test]
    fn tar_gz_files_folders_and_links() {
        let mut items = vec![
            TarItem::Dir("./"),
            TarItem::Dir("llama-b1/"),
            TarItem::File("llama-b1/llama-server", b"program", 0o755),
            TarItem::File("llama-b1/libllama.0.dylib", b"lib", 0o644),
            TarItem::File("llama-b1/._llama-server", b"apple double", 0o644),
        ];
        // Tarballs are the macOS and Linux builds; symlinks are only unpacked there.
        if cfg!(unix) {
            items.push(TarItem::Link("llama-b1/libllama.dylib", "libllama.0.dylib"));
        }
        let bytes = make_tar_gz(&items);
        let (out, result) = unpack_bytes(Unpack::TarGz, &bytes, "tar");
        result.expect("unpacks");
        let dir = out.join("llama-b1");
        assert_eq!(fs::read(dir.join("llama-server")).expect("program"), b"program");
        assert!(!dir.join("._llama-server").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.join("llama-server")).expect("program").permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
            assert_eq!(fs::read(dir.join("libllama.dylib")).expect("through the link"), b"lib");
        }
    }

    #[test]
    fn tar_gz_refuses_escapes() {
        for (name, items) in [
            ("tar-dotdot", vec![TarItem::File("../evil", b"x", 0o644)]),
            ("tar-abs-link", vec![TarItem::Link("lib", "/etc/passwd")]),
            ("tar-up-link", vec![TarItem::Dir("a/"), TarItem::Link("a/lib", "../../outside")]),
        ] {
            let bytes = make_tar_gz(&items);
            let (out, result) = unpack_bytes(Unpack::TarGz, &bytes, name);
            assert!(matches!(result, Err(ModelError::Archive(_))), "{name}: {result:?}");
            assert!(!out.parent().expect("parent").join("evil").exists());
        }
        // A link that climbs but stays inside is fine on Unix. Elsewhere
        // (Windows, whose builds are zips) no link is unpacked at all.
        let bytes = make_tar_gz(&[
            TarItem::File("lib/real.so", b"so", 0o644),
            TarItem::Link("bin/real.so", "../lib/real.so"),
        ]);
        let inside = unpack_bytes(Unpack::TarGz, &bytes, "tar-inside-link").1;
        if cfg!(unix) {
            inside.expect("inside");
        } else {
            assert!(matches!(inside, Err(ModelError::Archive(_))), "{inside:?}");
        }
        assert!(matches!(unpack_bytes(Unpack::TarGz, b"junk", "tar-junk").1, Err(ModelError::Archive(_))));
    }

    #[test]
    fn cancel_stops_unpacking() {
        let bytes = make_zip(&[("a", b"a", None)], true);
        let dir = scratch("zip-cancel");
        let archive = dir.join("archive");
        fs::write(&archive, &bytes).expect("write");
        let result = unpack(Unpack::Zip, &archive, &dir, &AtomicBool::new(true));
        assert!(matches!(result, Err(ModelError::Cancelled)));
    }
}
