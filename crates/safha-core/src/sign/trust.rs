//! Whom to trust, and whether a certificate leads there (ADR 0048).
//!
//! Trust anchors are the operating system's trusted roots (as
//! `rustls-native-certs` reads them: the macOS keychains' trust settings,
//! Windows' root store, the Linux CA bundle), so a company's own root that
//! the system trusts works here too. No list is bundled.
//!
//! The path check is ours and short: from the signer's certificate, each
//! certificate must be issued (by name and by signature) by the next, every
//! one in force at the validation time, and every issuer a CA, until one is
//! issued by an anchor (or is one). Revocation (OCSP, CRLs) is not checked
//! yet; the UI says so.

use std::sync::OnceLock;
use std::time::SystemTime;

use der::{Decode, Encode};
use x509_cert::Certificate;
use x509_cert::ext::pkix::{BasicConstraints, KeyUsage, KeyUsages};

use super::cms::{CLOCK_SLACK, validity};

/// Certificates trusted as roots.
#[derive(Debug, Clone, Default)]
pub struct TrustStore {
    anchors: Vec<Certificate>,
}

impl TrustStore {
    /// The operating system's trusted roots, read once.
    pub fn system() -> &'static TrustStore {
        static SYSTEM: OnceLock<TrustStore> = OnceLock::new();
        SYSTEM.get_or_init(|| {
            let found = rustls_native_certs::load_native_certs();
            TrustStore::from_der(found.certs.iter().map(|der| der.as_ref()))
        })
    }

    /// Anchors from DER certificates; ones that do not parse are skipped.
    pub fn from_der<'a>(certificates: impl IntoIterator<Item = &'a [u8]>) -> Self {
        Self { anchors: certificates.into_iter().filter_map(|der| Certificate::from_der(der).ok()).collect() }
    }

    pub fn len(&self) -> usize {
        self.anchors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.anchors.is_empty()
    }

    fn is_anchor(&self, certificate: &Certificate) -> bool {
        let der = certificate.to_der().ok();
        self.anchors.iter().any(|anchor| anchor.to_der().ok() == der)
    }
}

/// How a certificate's path to a trust anchor came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathCheck {
    /// It leads to an anchor, everything in force at the time asked about.
    Trusted,
    /// It leads to an anchor, but a certificate on the way was not in force
    /// at that time (expired, or not yet valid).
    OutsideValidity,
    /// No path to an anchor (self-signed, a private CA the system does not
    /// trust, or a missing intermediate).
    Untrusted,
}

/// Checks the path from `leaf`, through certificates in `pool`, to an anchor
/// in `store`, at time `at`.
pub(crate) fn check_path(
    leaf: &Certificate,
    pool: &[Certificate],
    store: &TrustStore,
    at: SystemTime,
) -> PathCheck {
    let mut best = PathCheck::Untrusted;
    walk(leaf, pool, store, at, 0, true, &mut best);
    best
}

fn walk(
    certificate: &Certificate,
    pool: &[Certificate],
    store: &TrustStore,
    at: SystemTime,
    depth: usize,
    in_force_so_far: bool,
    best: &mut PathCheck,
) {
    if depth > 8 || *best == PathCheck::Trusted {
        return;
    }
    let in_force = in_force_so_far && in_force_at(certificate, at);
    let reached = |best: &mut PathCheck, in_force: bool| {
        *best = if in_force { PathCheck::Trusted } else { PathCheck::OutsideValidity };
    };
    // A certificate the system trusts itself (a trusted self-signed one).
    if store.is_anchor(certificate) {
        reached(best, in_force);
        return;
    }
    let issuer_name = certificate.tbs_certificate().issuer();
    if store.anchors.iter().any(|anchor| {
        anchor.tbs_certificate().subject() == issuer_name && issued_by(certificate, anchor) && is_ca(anchor)
    }) {
        reached(best, in_force);
        return;
    }
    for candidate in pool {
        if candidate == certificate
            || candidate.tbs_certificate().subject() != issuer_name
            || !is_ca(candidate)
            || !issued_by(certificate, candidate)
        {
            continue;
        }
        walk(candidate, pool, store, at, depth + 1, in_force, best);
    }
}

fn in_force_at(certificate: &Certificate, at: SystemTime) -> bool {
    let (not_before, not_after) = validity(certificate);
    not_before <= at + CLOCK_SLACK && at <= not_after + CLOCK_SLACK
}

/// `certificate`'s signature verifies with `issuer`'s key.
fn issued_by(certificate: &Certificate, issuer: &Certificate) -> bool {
    use x509_verify::{VerifyInfo, VerifyingKey};
    let Ok(key) = VerifyingKey::try_from(issuer) else { return false };
    VerifyInfo::try_from(certificate).is_ok_and(|info| key.verify(info).is_ok())
}

/// A certificate that may issue others: basic constraints say CA (version 1
/// certificates, which have no extensions, are taken as they come), and a
/// key usage, when stated, allows signing certificates.
fn is_ca(certificate: &Certificate) -> bool {
    let tbs = certificate.tbs_certificate();
    if tbs.extensions().is_none() {
        return true;
    }
    let ca = matches!(tbs.get_extension::<BasicConstraints>(), Ok(Some((_, constraints))) if constraints.ca);
    let usage_allows = match tbs.get_extension::<KeyUsage>() {
        Ok(Some((_, usage))) => usage.0.contains(KeyUsages::KeyCertSign),
        _ => true,
    };
    ca && usage_allows
}

/// Whether a signer's certificate may sign documents: its key usage, when
/// stated, includes digital signature or non-repudiation.
pub(crate) fn may_sign(certificate: &Certificate) -> bool {
    match certificate.tbs_certificate().get_extension::<KeyUsage>() {
        Ok(Some((_, usage))) => {
            usage.0.contains(KeyUsages::DigitalSignature) || usage.0.contains(KeyUsages::NonRepudiation)
        }
        _ => true,
    }
}

/// Whether a time-stamping authority's certificate is one: RFC 3161 2.3
/// asks for the time-stamping extended key usage.
pub(crate) fn is_timestamping(certificate: &Certificate) -> bool {
    use x509_cert::ext::pkix::ExtendedKeyUsage;
    const ID_KP_TIME_STAMPING: const_oid::ObjectIdentifier =
        const_oid::ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.8");
    matches!(
        certificate.tbs_certificate().get_extension::<ExtendedKeyUsage>(),
        Ok(Some((_, usages))) if usages.0.contains(&ID_KP_TIME_STAMPING)
    )
}
