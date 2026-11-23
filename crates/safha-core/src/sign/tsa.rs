//! Time-stamping authorities (RFC 3161 over HTTP, RFC 3161 section 3.4):
//! a request goes out as `application/timestamp-query`, the token comes back
//! as `application/timestamp-reply`. The authority is a setting the user
//! fills in; nothing is sent anywhere unless they do (ADR 0047).
//!
//! What goes out is a hash of the signature value and a random nonce: no
//! part of the document, nor who signed it.

use std::io::Read as _;
use std::sync::Arc;
use std::time::Duration;

use ureq::tls::{Certificate, RootCerts, TlsConfig};

use crate::error::{Error, Result};

/// The time-stamp could not be had: unreachable, refused, or not a token.
pub(crate) const TIMESTAMP_FAILED: &str = "timestamp_failed";
/// The authority's address is not an http(s) URL.
pub(crate) const TIMESTAMP_URL: &str = "timestamp_url";

/// The largest reply read, in bytes (tokens are a few kilobytes).
const MAX_REPLY: u64 = 1024 * 1024;

/// Something that answers time-stamp requests: the HTTP client, or a test's
/// own authority.
pub trait TimeStamper: Sync {
    /// The DER `TimeStampResp` for the DER `TimeStampReq` `request`.
    fn stamp(&self, request: &[u8]) -> Result<Vec<u8>>;
}

/// A time-stamping authority reached over HTTP or HTTPS. HTTPS is checked
/// against the operating system's trust store; plain HTTP is common for
/// authorities and safe here, since the token is signed and checked.
pub struct HttpTimeStamper {
    url: String,
    agent: ureq::Agent,
}

impl HttpTimeStamper {
    pub fn new(url: &str) -> Result<Self> {
        let url = url.trim();
        let lower = url.to_ascii_lowercase();
        let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"));
        if rest.is_none_or(|rest| rest.is_empty() || rest.starts_with('/')) || url.len() > 2048 {
            return Err(Error::Signing(TIMESTAMP_URL));
        }
        let tls = TlsConfig::builder()
            .root_certs(RootCerts::Specific(system_roots()))
            .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .build();
        let agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .max_redirects(3)
            .user_agent(format!("Mirsam/{}", env!("CARGO_PKG_VERSION")).as_str())
            .build()
            .new_agent();
        Ok(Self { url: url.to_string(), agent })
    }
}

impl TimeStamper for HttpTimeStamper {
    fn stamp(&self, request: &[u8]) -> Result<Vec<u8>> {
        let failed = |err: &dyn std::fmt::Display| {
            eprintln!("safha: time-stamp request failed: {err}");
            Error::Signing(TIMESTAMP_FAILED)
        };
        let response = self
            .agent
            .post(&self.url)
            .header("Content-Type", "application/timestamp-query")
            .header("Accept", "application/timestamp-reply")
            .send(request)
            .map_err(|err| failed(&err))?;
        if response.status() != 200 {
            return Err(failed(&format!("HTTP {}", response.status())));
        }
        let mut reply = Vec::new();
        response
            .into_body()
            .into_reader()
            .take(MAX_REPLY + 1)
            .read_to_end(&mut reply)
            .map_err(|err| failed(&err))?;
        if reply.len() as u64 > MAX_REPLY {
            return Err(failed(&"the reply is too large"));
        }
        Ok(reply)
    }
}

/// The operating system's trusted roots for HTTPS, read once.
fn system_roots() -> Arc<Vec<Certificate<'static>>> {
    static ROOTS: std::sync::OnceLock<Arc<Vec<Certificate<'static>>>> = std::sync::OnceLock::new();
    Arc::clone(ROOTS.get_or_init(|| {
        let found = rustls_native_certs::load_native_certs();
        Arc::new(found.certs.iter().map(|der| Certificate::from_der(der.as_ref()).to_owned()).collect())
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_addresses_are_authorities() {
        for good in ["http://timestamp.digicert.com", "https://freetsa.org/tsr", " HTTP://tsa.example/ "] {
            assert!(HttpTimeStamper::new(good).is_ok(), "{good}");
        }
        for bad in ["", "ftp://tsa.example", "timestamp.digicert.com", "https://", "file:///etc/passwd"] {
            assert!(matches!(HttpTimeStamper::new(bad), Err(Error::Signing(TIMESTAMP_URL))), "{bad}");
        }
    }
}
