//! Downloading one pinned file: resumable, checked, cancellable.
//!
//! A download writes to a `.part` file named by the file's SHA-256, so what was
//! received survives a cancel, a lost connection or a restart of the app, and
//! the next attempt asks only for the rest (`Range: bytes=N-`). The bytes
//! already there are hashed again first, so the final SHA-256 covers the whole
//! file whatever happened in between. A file that does not match is deleted.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use ureq::tls::{Certificate, RootCerts, TlsConfig};

use super::{Asset, ModelError};

/// Read and write in pieces this large; the cancel flag is checked between them.
const CHUNK: usize = 256 * 1024;

/// How downloads behave. [`DownloadConfig::default`] is the app's.
#[derive(Clone, Debug)]
pub struct DownloadConfig {
    /// Giving up on connecting.
    pub connect_timeout: Duration,
    /// Giving up on a connection that sends nothing for this long. It also
    /// bounds how long a cancel can take on a stalled connection.
    pub read_timeout: Duration,
    /// Tries after a failure in a row (a try that received bytes resets the count).
    pub retries: u32,
    /// Wait before the first retry; doubled for each further one.
    pub retry_delay: Duration,
    pub user_agent: String,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(30),
            read_timeout: Duration::from_secs(30),
            retries: 4,
            retry_delay: Duration::from_secs(1),
            user_agent: format!("Mirsam/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

/// Fetches pinned files over HTTP(S).
///
/// TLS is rustls on ring, checked against the operating system's trust store
/// (rustls-native-certs), so a company's own root certificate works where the
/// OS trusts it. Redirects are followed (Hugging Face serves files from a CDN).
pub struct Downloader {
    agent: ureq::Agent,
    config: DownloadConfig,
}

impl Default for Downloader {
    fn default() -> Self {
        Self::new(DownloadConfig::default())
    }
}

/// Why one request did not finish the file.
enum Attempt {
    /// Worth trying again (connection lost, timed out, server busy).
    Transient(ModelError),
    /// Trying again cannot help.
    Fatal(ModelError),
}

impl Downloader {
    pub fn new(config: DownloadConfig) -> Self {
        let tls = TlsConfig::builder()
            .root_certs(RootCerts::Specific(system_roots()))
            .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .build();
        let agent = ureq::Agent::config_builder()
            .tls_config(tls)
            // Statuses are handled here: 206, 200, 416 and retries on 5xx.
            .http_status_as_error(false)
            .timeout_connect(Some(config.connect_timeout))
            .timeout_per_read(Some(config.read_timeout))
            .max_redirects(10)
            .user_agent(config.user_agent.as_str())
            .build()
            .new_agent();
        Self { agent, config }
    }

    /// Downloads `asset` into `part`, resuming what is there, and checks it.
    ///
    /// `on_bytes` gets the file's length so far, first with what was already
    /// there. On `Ok` the whole file is in `part` and matches its SHA-256. On
    /// [`ModelError::Cancelled`] or a network error the bytes stay for the next
    /// try; on [`ModelError::ChecksumMismatch`] the file is deleted.
    pub fn fetch(
        &self,
        asset: &Asset,
        part: &Path,
        cancel: &AtomicBool,
        on_bytes: &mut dyn FnMut(u64),
    ) -> Result<(), ModelError> {
        let mut file = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(part)?;
        let mut have = file.metadata()?.len();
        if have > asset.size {
            file.set_len(0)?;
            have = 0;
        }
        let mut hasher = Sha256::new();
        if have > 0 {
            hash_prefix(&mut file, have, &mut hasher, cancel)?;
        }
        file.seek(SeekFrom::Start(have))?;
        on_bytes(have);

        let mut failures = 0;
        while have < asset.size {
            if cancel.load(Ordering::Relaxed) {
                return Err(ModelError::Cancelled);
            }
            let before = have;
            match self.attempt(asset, &mut file, &mut have, &mut hasher, cancel, on_bytes) {
                Ok(()) => {}
                Err(Attempt::Fatal(err)) => {
                    file.flush()?;
                    if matches!(err, ModelError::ChecksumMismatch { .. }) {
                        drop(file);
                        std::fs::remove_file(part)?;
                    }
                    return Err(err);
                }
                Err(Attempt::Transient(err)) => {
                    file.flush()?;
                    failures = if have > before { 1 } else { failures + 1 };
                    if failures > self.config.retries {
                        return Err(err);
                    }
                    let wait = self.config.retry_delay.saturating_mul(1 << (failures - 1).min(6));
                    sleep_unless_cancelled(wait, cancel)?;
                }
            }
        }
        file.sync_all()?;
        drop(file);

        let digest = hex(&hasher.finalize());
        if digest != asset.sha256 {
            std::fs::remove_file(part)?;
            return Err(ModelError::ChecksumMismatch { name: asset.name.clone() });
        }
        Ok(())
    }

    /// One request for the rest of the file, written as it arrives.
    fn attempt(
        &self,
        asset: &Asset,
        file: &mut File,
        have: &mut u64,
        hasher: &mut Sha256,
        cancel: &AtomicBool,
        on_bytes: &mut dyn FnMut(u64),
    ) -> Result<(), Attempt> {
        let mut request = self.agent.get(&asset.url);
        if *have > 0 {
            request = request.header("Range", format!("bytes={}-", *have));
        }
        let response = request.call().map_err(request_error)?;
        let status = response.status().as_u16();
        let wrong_file = || Attempt::Fatal(ModelError::ChecksumMismatch { name: asset.name.clone() });
        match status {
            206 if *have > 0 => {
                let range = response
                    .headers()
                    .get("content-range")
                    .and_then(|value| value.to_str().ok())
                    .and_then(parse_content_range);
                match range {
                    Some((start, total)) if start == *have => {
                        if total.is_some_and(|total| total != asset.size) {
                            return Err(wrong_file());
                        }
                    }
                    // Some other part of the file: start again from nothing.
                    _ => {
                        restart(file, have, hasher, on_bytes).map_err(fatal_io)?;
                        return Err(Attempt::Transient(ModelError::Network(
                            "the server sent another part of the file".into(),
                        )));
                    }
                }
            }
            200 => {
                if let Some(length) = content_length(&response)
                    && length != asset.size
                {
                    return Err(wrong_file());
                }
                // The server sends the whole file (no ranges): start over with it.
                if *have > 0 {
                    restart(file, have, hasher, on_bytes).map_err(fatal_io)?;
                }
            }
            // What is here is not a prefix the server knows: start over.
            416 if *have > 0 => {
                restart(file, have, hasher, on_bytes).map_err(fatal_io)?;
                return Err(Attempt::Transient(ModelError::Network("range not satisfiable".into())));
            }
            408 | 425 | 429 | 500..=599 => {
                return Err(Attempt::Transient(ModelError::Http { status, url: asset.url.clone() }));
            }
            _ => return Err(Attempt::Fatal(ModelError::Http { status, url: asset.url.clone() })),
        }

        let mut body = response.into_body().into_reader();
        let mut buffer = vec![0; CHUNK];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(Attempt::Fatal(ModelError::Cancelled));
            }
            let read = match body.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(Attempt::Transient(ModelError::Network(err.to_string()))),
            };
            if *have + read as u64 > asset.size {
                return Err(wrong_file());
            }
            file.write_all(&buffer[..read]).map_err(fatal_io)?;
            hasher.update(&buffer[..read]);
            *have += read as u64;
            on_bytes(*have);
        }
        if *have < asset.size {
            return Err(Attempt::Transient(ModelError::Network("the connection closed early".into())));
        }
        Ok(())
    }
}

/// The operating system's trusted root certificates, read once. Certificates
/// it cannot parse are skipped; with none at all, HTTPS fails with a TLS error.
fn system_roots() -> Arc<Vec<Certificate<'static>>> {
    static ROOTS: OnceLock<Arc<Vec<Certificate<'static>>>> = OnceLock::new();
    Arc::clone(ROOTS.get_or_init(|| {
        let found = rustls_native_certs::load_native_certs();
        Arc::new(found.certs.iter().map(|der| Certificate::from_der(der.as_ref()).to_owned()).collect())
    }))
}

/// Empties the part file, to fetch the whole file again.
fn restart(
    file: &mut File,
    have: &mut u64,
    hasher: &mut Sha256,
    on_bytes: &mut dyn FnMut(u64),
) -> io::Result<()> {
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    *have = 0;
    *hasher = Sha256::new();
    on_bytes(0);
    Ok(())
}

/// Hashes the first `len` bytes of `file` (a download being resumed).
fn hash_prefix(
    file: &mut File,
    len: u64,
    hasher: &mut Sha256,
    cancel: &AtomicBool,
) -> Result<(), ModelError> {
    file.seek(SeekFrom::Start(0))?;
    let mut reader = file.take(len);
    let mut buffer = vec![0; CHUNK * 4];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(ModelError::Cancelled);
        }
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(());
        }
        hasher.update(&buffer[..read]);
    }
}

fn sleep_unless_cancelled(wait: Duration, cancel: &AtomicBool) -> Result<(), ModelError> {
    let until = Instant::now() + wait;
    while Instant::now() < until {
        if cancel.load(Ordering::Relaxed) {
            return Err(ModelError::Cancelled);
        }
        std::thread::sleep(Duration::from_millis(50).min(until - Instant::now()));
    }
    Ok(())
}

fn request_error(err: ureq::Error) -> Attempt {
    match err {
        // Certificates and protocol violations do not fix themselves.
        ureq::Error::Tls(_)
        | ureq::Error::Rustls(_)
        | ureq::Error::BadUri(_)
        | ureq::Error::TooManyRedirects
        | ureq::Error::RedirectFailed => Attempt::Fatal(ModelError::Network(err.to_string())),
        _ => Attempt::Transient(ModelError::Network(err.to_string())),
    }
}

fn fatal_io(err: io::Error) -> Attempt {
    Attempt::Fatal(err.into())
}

fn content_length(response: &ureq::http::Response<ureq::Body>) -> Option<u64> {
    response.headers().get("content-length")?.to_str().ok()?.trim().parse().ok()
}

/// `bytes 100-199/1000` → (100, Some(1000)); `bytes 100-199/*` → (100, None).
fn parse_content_range(value: &str) -> Option<(u64, Option<u64>)> {
    let rest = value.trim().strip_prefix("bytes")?.trim_start();
    let (range, total) = rest.split_once('/')?;
    let (start, _end) = range.split_once('-')?;
    let total = match total.trim() {
        "*" => None,
        total => Some(total.parse().ok()?),
    };
    Some((start.trim().parse().ok()?, total))
}

/// Lower-case hex.
pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range() {
        assert_eq!(parse_content_range("bytes 100-199/1000"), Some((100, Some(1000))));
        assert_eq!(parse_content_range("bytes 0-0/*"), Some((0, None)));
        assert_eq!(parse_content_range(" bytes  5-9/10 "), Some((5, Some(10))));
        assert_eq!(parse_content_range("items 1-2/3"), None);
        assert_eq!(parse_content_range("bytes */1000"), None);
        assert_eq!(parse_content_range("bytes x-2/3"), None);
    }

    #[test]
    fn hex_is_lower_case() {
        assert_eq!(hex(&[0x00, 0xab, 0x7f]), "00ab7f");
        assert_eq!(
            hex(&Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
