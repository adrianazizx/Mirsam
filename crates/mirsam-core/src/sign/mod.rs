//! Certificate signatures (PAdES; ADRs 0044 and 0048): signing a document
//! with a certificate, and checking the signatures a document carries.
//!
//! **Signing** appends one incremental update to the file ([`pdf`]): a
//! signature field with an invisible widget, or a visible one showing a kept
//! signature and a caption ([`appearance`]), and a signature dictionary
//! whose `/Contents` holds a CMS signature ([`cms`]) over every byte of the
//! file but itself. Nothing before the update changes, so signatures already
//! in the file stay valid. The signature is PAdES baseline B-B, or B-T with
//! an RFC 3161 time-stamp from the authority the user configured ([`tsa`]).
//! Keys come from a PKCS#12 file or, on macOS, the keychain ([`identity`]).
//!
//! **Checking** ([`verify`]) finds every signature, checks the bytes it
//! covers and its signature, the signer's certificate path to the system's
//! trusted roots ([`trust`]), its time-stamp, and what the file changed
//! after it.

mod appearance;
mod cms;
mod identity;
#[cfg(target_os = "macos")]
pub mod keychain;
mod pdf;
mod trust;
mod tsa;
mod verify;

use std::time::SystemTime;

pub use appearance::{MAX_CAPTION, VisibleSignature};
pub use identity::{CertificateSummary, Identity, KeyKind, KeyOperation, MAX_PKCS12_BYTES};
pub use pdf::Changes;
pub use trust::TrustStore;
pub use tsa::{HttpTimeStamper, TimeStamper};
pub use verify::{Integrity, SignatureReport, Status, TimestampReport, Trust, verify, verify_at};

use crate::error::{Error, Result};

/// The signer's certificate is not in force now, or its key may not sign.
pub(crate) const CERTIFICATE_CANNOT_SIGN: &str = "certificate_cannot_sign";
/// The signature would not fit any reasonable placeholder.
pub(crate) const SIGNATURE_TOO_LARGE: &str = "signature_too_large";

/// The largest signature placeholder, in bytes of DER.
const MAX_RESERVED: usize = 256 * 1024;

/// What to say in a signature besides who signed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SignOptions {
    pub reason: Option<String>,
    pub location: Option<String>,
    pub contact: Option<String>,
    /// Shown on a page, or `None` for an invisible signature.
    pub visible: Option<VisibleSignature>,
}

impl SignOptions {
    fn check(&self) -> Result<()> {
        for text in [&self.reason, &self.location, &self.contact].into_iter().flatten() {
            if text.chars().count() > 500 {
                return Err(Error::InvalidEdit(
                    "a signature's reason, place or contact is at most 500 characters",
                ));
            }
        }
        Ok(())
    }
}

/// `base` signed by `identity`: `base` followed by one incremental update.
///
/// `password` unlocks an encrypted `base` (the update's strings are
/// encrypted like the file's). With `timestamper` the signature is
/// time-stamped (PAdES B-T); without, it is B-B. The result is checked
/// before it is returned: the new signature must verify, and every
/// signature that verified before must still.
pub fn sign_bytes(
    base: &[u8],
    password: Option<&str>,
    identity: &Identity,
    options: &SignOptions,
    timestamper: Option<&dyn TimeStamper>,
) -> Result<Vec<u8>> {
    sign_bytes_at(base, password, identity, options, timestamper, SystemTime::now())
}

/// [`sign_bytes`] with the signing time given.
pub fn sign_bytes_at(
    base: &[u8],
    password: Option<&str>,
    identity: &Identity,
    options: &SignOptions,
    timestamper: Option<&dyn TimeStamper>,
    now: SystemTime,
) -> Result<Vec<u8>> {
    options.check()?;
    let (not_before, not_after) = cms::validity(&identity.certificate);
    if now + cms::CLOCK_SLACK < not_before
        || now > not_after + cms::CLOCK_SLACK
        || !trust::may_sign(&identity.certificate)
    {
        return Err(Error::Signing(CERTIFICATE_CANNOT_SIGN));
    }
    let summary = identity.summary();
    let entries = pdf::SigEntries {
        name: Some(summary.name.as_str()),
        reason: options.reason.as_deref(),
        location: options.location.as_deref(),
        contact: options.contact.as_deref(),
        time: now,
    };
    let field = field_name(base, password);
    let before: Vec<Integrity> = verify::verify(base, password, &TrustStore::default())
        .iter()
        .map(|report| report.integrity)
        .collect();

    let mut reserved = estimate(identity, timestamper.is_some());
    loop {
        let mut prepared =
            pdf::prepare(base, password, &entries, options.visible.as_ref(), &field, reserved)?;
        prepared.fill_byte_range();
        let digest = identity.kind().digest().digest(&prepared.covered());
        let der = cms::build(identity, &digest, timestamper)?;
        if der.len() > prepared.room() {
            // Only a surprisingly large time-stamp gets here: once more, with room.
            let needed = der.len() + 4096;
            if needed > MAX_RESERVED || reserved >= needed {
                return Err(Error::Signing(SIGNATURE_TOO_LARGE));
            }
            reserved = needed;
            continue;
        }
        prepared.insert(&der);
        let signed = prepared.bytes;
        // Checked as a validator would: ours is intact, the others are as they were.
        let after = verify::verify(&signed, password, &TrustStore::default());
        let ours_intact = after.last().is_some_and(|report| report.integrity == Integrity::Intact);
        let others_kept = before
            .iter()
            .zip(&after)
            .all(|(was, now)| *was != Integrity::Intact || now.integrity == Integrity::Intact);
        if !ours_intact || after.len() != before.len() + 1 || !others_kept {
            return Err(Error::Signing(identity::SIGNING_FAILED));
        }
        return Ok(signed);
    }
}

/// Room for the CMS: the certificates, the signature and its attributes,
/// and a time-stamp token (which carries the authority's certificates).
fn estimate(identity: &Identity, timestamp: bool) -> usize {
    use der::Encode;
    let certificates: usize = std::iter::once(&identity.certificate)
        .chain(&identity.chain)
        .map(|certificate| {
            certificate.encoded_len().ok().and_then(|len| usize::try_from(len).ok()).unwrap_or(2048)
        })
        .sum();
    let estimate = certificates + 4096 + if timestamp { 16 * 1024 } else { 0 };
    estimate.div_ceil(1024) * 1024
}

/// "Signature1", "Signature2", …: the first name no field of the form has.
fn field_name(base: &[u8], password: Option<&str>) -> String {
    let taken: Vec<String> = crate::forms::read(base, password)
        .map(|form| form.fields.into_iter().map(|field| field.name).collect())
        .unwrap_or_default();
    (1..)
        .map(|n| format!("Signature{n}"))
        .find(|name| !taken.iter().any(|taken| taken == name || taken.starts_with(&format!("{name}."))))
        .unwrap_or_else(|| "Signature".to_string())
}
