//! Checking a document's signatures (ADR 0048): for each, whether the bytes
//! it covers are as signed, who signed and whether a trusted authority
//! vouches for them, when (a time-stamp, or the signer's own claim), and what
//! the file changed after it.
//!
//! The verdict is deliberately plain:
//! - **valid**: the signed bytes are intact and the signer's certificate
//!   leads to a trusted root, in force when it was checked;
//! - **unknown**: intact, but the signer cannot be vouched for (a
//!   self-signed or private certificate, an expired one without a
//!   time-stamp, or a key not meant for signing);
//! - **invalid**: the signed bytes or the signature do not check out.
//!
//! Revocation is not checked yet; the UI says so with every verdict.

use std::time::SystemTime;

use serde::Serialize;

use super::cms::{self, Checked, DigestKind, TimestampChecked, UNREADABLE_SIGNATURE};
use super::identity::{CertificateSummary, unix};
use super::pdf::{Changes, Found, changes_after, find_signatures, revision_at};
use super::trust::{PathCheck, TrustStore, check_path, is_timestamping, may_sign};
use crate::pdf::crypt::Crypt;
use crate::view::ViewRect;

/// The verdict on one signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Valid,
    Unknown,
    Invalid,
}

/// Whether the bytes a signature covers are as signed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Integrity {
    /// The digest matches and the signature verifies.
    Intact,
    /// Something it covers changed, or the signature does not verify.
    Altered,
    /// It could not be read, or uses an algorithm this app does not check.
    Unchecked,
}

/// Whether the signer's certificate is vouched for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Trust {
    /// It leads to a root the system trusts, in force at the time used.
    Trusted,
    /// It leads to a trusted root but was not in force (expired, or not yet
    /// valid) at the time used.
    Expired,
    /// No path to a trusted root.
    Untrusted,
    /// There is no certificate to check.
    Unknown,
}

/// A document time-stamp, or a signature's own time-stamp.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimestampReport {
    /// When the authority says the signature existed, in Unix seconds.
    pub time: i64,
    /// The authority's name, from its certificate.
    pub authority: Option<String>,
    /// The token is over this signature and its signature verifies.
    pub intact: bool,
    /// The authority's certificate leads to a trusted root and is a
    /// time-stamping certificate.
    pub trusted: bool,
}

/// One signature as checked.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureReport {
    /// Its field's name, when the field was found.
    pub field: Option<String>,
    /// The page its widget is on and its box there (page space), when it is
    /// visible; [`crate::Document::signatures`] fills in `view`.
    pub page: Option<usize>,
    pub rect: Option<[f32; 4]>,
    pub view: Option<ViewRect>,
    /// A document time-stamp (`/DocTimeStamp`), not a person's signature.
    pub document_timestamp: bool,
    /// `/SubFilter`: `ETSI.CAdES.detached`, `adbe.pkcs7.detached`, …
    pub sub_filter: Option<String>,
    pub status: Status,
    pub integrity: Integrity,
    pub trust: Trust,
    pub signer: Option<CertificateSummary>,
    /// `/Name`, `/Reason`, `/Location`, `/ContactInfo` as the signer wrote them.
    pub name: Option<String>,
    pub reason: Option<String>,
    pub location: Option<String>,
    pub contact: Option<String>,
    /// When the signer's computer says it was signed (`/M`, or the CMS
    /// signing-time), in Unix seconds: a claim, not proof.
    pub claimed_time: Option<i64>,
    pub timestamp: Option<TimestampReport>,
    /// Which revision of the file it signs (1 = the first), of how many.
    pub revision: usize,
    pub revisions: usize,
    /// It covers the whole file as it is now.
    pub covers_whole_file: bool,
    /// What later revisions changed, when it does not.
    pub changes: Option<Changes>,
    /// Why the verdict is not "valid", as stable codes for the UI.
    pub problems: Vec<&'static str>,
}

// Problem codes.
const BYTE_RANGE_INVALID: &str = "byte_range_invalid";
const DIGEST_MISMATCH: &str = "digest_mismatch";
const SIGNATURE_INVALID: &str = "signature_invalid";
const CERTIFICATE_NOT_BOUND: &str = "certificate_not_bound";
const SIGNER_UNTRUSTED: &str = "signer_untrusted";
const SIGNER_EXPIRED: &str = "signer_expired";
const KEY_NOT_FOR_SIGNING: &str = "key_not_for_signing";
const NO_CERTIFICATE: &str = "no_certificate";
const TIMESTAMP_INVALID: &str = "timestamp_invalid";
const TIMESTAMP_UNTRUSTED: &str = "timestamp_untrusted";

/// Checks every signature in `bytes` (opened with `password` when
/// encrypted, to read their texts), oldest first, against `store`.
pub fn verify(bytes: &[u8], password: Option<&str>, store: &TrustStore) -> Vec<SignatureReport> {
    verify_at(bytes, password, store, SystemTime::now())
}

/// [`verify`] as of `now` (tests check expiry with it).
pub fn verify_at(
    bytes: &[u8],
    password: Option<&str>,
    store: &TrustStore,
    now: SystemTime,
) -> Vec<SignatureReport> {
    let found = find_signatures(bytes);
    if found.is_empty() {
        return Vec::new();
    }
    let crypt = crate::pdf::File::new(bytes).and_then(|file| Crypt::of(&file, password).ok().flatten());
    let revisions = revision_at(bytes, bytes.len());
    found.iter().map(|signature| report(bytes, signature, crypt.as_ref(), store, now, revisions)).collect()
}

fn report(
    bytes: &[u8],
    found: &Found,
    crypt: Option<&Crypt>,
    store: &TrustStore,
    now: SystemTime,
    revisions: usize,
) -> SignatureReport {
    let sub_filter = found.name(b"SubFilter");
    let document_timestamp = found.name(b"Type").as_deref() == Some("DocTimeStamp")
        || sub_filter.as_deref() == Some("ETSI.RFC3161");
    let mut report = SignatureReport {
        field: found.field.clone(),
        page: found.page,
        rect: found.rect,
        view: None,
        document_timestamp,
        sub_filter,
        status: Status::Invalid,
        integrity: Integrity::Unchecked,
        trust: Trust::Unknown,
        signer: None,
        name: found.text(b"Name", crypt),
        reason: found.text(b"Reason", crypt),
        location: found.text(b"Location", crypt),
        contact: found.text(b"ContactInfo", crypt),
        claimed_time: found.text(b"M", crypt).and_then(|m| crate::annotate::parse_pdf_date(&m)),
        timestamp: None,
        revision: 1,
        revisions,
        covers_whole_file: false,
        changes: None,
        problems: Vec::new(),
    };

    // The byte range: from the start of the file, around exactly the
    // `/Contents` string, which must be this dictionary's own.
    let Some(range) = checked_range(bytes, found) else {
        report.problems.push(BYTE_RANGE_INVALID);
        report.integrity = Integrity::Altered;
        return report;
    };
    let end = range[2] + range[3];
    report.revision = revision_at(bytes, end);
    let rest = &bytes[end..];
    report.covers_whole_file = rest.iter().all(|b| crate::pdf::is_white(*b));
    if !report.covers_whole_file {
        report.changes = Some(changes_after(bytes, end));
    }
    let covered = [&bytes[range[0]..range[0] + range[1]], &bytes[range[2]..end]];
    let digest_of = |kind: DigestKind| kind.digest(&covered);
    let der = found.contents().unwrap_or_default();

    if report.document_timestamp {
        match cms::check_document_timestamp(der, &digest_of) {
            Ok(token) => {
                let timestamp = timestamp_report(&token, store);
                report.integrity = if timestamp.intact { Integrity::Intact } else { Integrity::Altered };
                report.trust = if timestamp.trusted { Trust::Trusted } else { Trust::Untrusted };
                report.signer = token.authority.as_ref().map(CertificateSummary::of);
                report.claimed_time = Some(timestamp.time);
                if !timestamp.intact {
                    report.problems.push(DIGEST_MISMATCH);
                }
                if !timestamp.trusted {
                    report.problems.push(TIMESTAMP_UNTRUSTED);
                }
                report.timestamp = Some(timestamp);
            }
            Err(code) => report.problems.push(code),
        }
        report.status = verdict(&report);
        return report;
    }

    let checked: Checked = match cms::check(der, &digest_of) {
        Ok(checked) => checked,
        Err(code) => {
            report.problems.push(code);
            return report;
        }
    };
    report.signer = checked.signer.as_ref().map(CertificateSummary::of);
    if report.claimed_time.is_none() {
        report.claimed_time = checked.signing_time.map(unix);
    }
    if let Some(problem) = checked.problem {
        report.problems.push(problem);
    }
    report.integrity = if checked.problem.is_some() && !(checked.digest_matches && checked.signature_valid) {
        Integrity::Unchecked
    } else if checked.digest_matches && checked.signature_valid {
        Integrity::Intact
    } else {
        if !checked.digest_matches {
            report.problems.push(DIGEST_MISMATCH);
        }
        if !checked.signature_valid {
            report.problems.push(SIGNATURE_INVALID);
        }
        Integrity::Altered
    };
    if checked.certificate_bound == Some(false) {
        report.problems.push(CERTIFICATE_NOT_BOUND);
    }

    // The time the signer's certificate must have been in force: a trusted
    // time-stamp's, else now (the signer's own clock proves nothing).
    let mut at = now;
    if let Some(token) = &checked.timestamp {
        let timestamp = timestamp_report(token, store);
        if !timestamp.intact {
            report.problems.push(TIMESTAMP_INVALID);
        } else if !timestamp.trusted {
            report.problems.push(TIMESTAMP_UNTRUSTED);
        } else {
            at = token.time;
        }
        report.timestamp = Some(timestamp);
    }
    report.trust = match &checked.signer {
        None => {
            report.problems.push(NO_CERTIFICATE);
            Trust::Unknown
        }
        Some(signer) => {
            if !may_sign(signer) {
                report.problems.push(KEY_NOT_FOR_SIGNING);
            }
            match check_path(signer, &checked.certificates, store, at) {
                PathCheck::Trusted => Trust::Trusted,
                PathCheck::OutsideValidity => {
                    report.problems.push(SIGNER_EXPIRED);
                    Trust::Expired
                }
                PathCheck::Untrusted => {
                    report.problems.push(SIGNER_UNTRUSTED);
                    Trust::Untrusted
                }
            }
        }
    };
    report.status = verdict(&report);
    report
}

fn verdict(report: &SignatureReport) -> Status {
    let broken = report.integrity != Integrity::Intact
        || report.problems.iter().any(|p| {
            matches!(
                *p,
                CERTIFICATE_NOT_BOUND | TIMESTAMP_INVALID | UNREADABLE_SIGNATURE | BYTE_RANGE_INVALID
            )
        });
    if broken {
        Status::Invalid
    } else if report.trust == Trust::Trusted && !report.problems.contains(&KEY_NOT_FOR_SIGNING) {
        Status::Valid
    } else {
        Status::Unknown
    }
}

fn timestamp_report(token: &TimestampChecked, store: &TrustStore) -> TimestampReport {
    let trusted = token.authority.as_ref().is_some_and(|authority| {
        is_timestamping(authority)
            && check_path(authority, &token.certificates, store, token.time) == PathCheck::Trusted
    });
    TimestampReport {
        time: unix(token.time),
        authority: token.authority.as_ref().map(|authority| CertificateSummary::of(authority).name),
        intact: token.imprint_matches && token.signature_valid,
        trusted,
    }
}

/// `/ByteRange` when it is sound: it starts the file, its gap is exactly a
/// hex string equal to this dictionary's `/Contents`, and the dictionary
/// itself starts inside the covered bytes (so its entries are signed).
fn checked_range(bytes: &[u8], found: &Found) -> Option<[usize; 4]> {
    let range = found.byte_range()?;
    let [start, first, gap_end, second] = range;
    if start != 0 || first >= gap_end || gap_end.checked_add(second)? > bytes.len() {
        return None;
    }
    let gap = &bytes[first..gap_end];
    if gap.first() != Some(&b'<') || gap.last() != Some(&b'>') {
        return None;
    }
    let hex = &gap[1..gap.len() - 1];
    if !hex.len().is_multiple_of(2) || !hex.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let decoded: Vec<u8> = hex
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let digit = |b: u8| (b as char).to_digit(16).unwrap_or(0) as u8;
            digit(pair[0]) << 4 | digit(pair[1])
        })
        .collect();
    if found.contents() != Some(decoded.as_slice()) {
        return None;
    }
    if found.offset.is_some_and(|offset| offset >= first) {
        return None;
    }
    Some(range)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sign::cms::UNSUPPORTED_ALGORITHM;

    #[test]
    fn an_unsigned_file_has_no_signatures() {
        let bytes = b"%PDF-1.7\n1 0 obj\n<</Type/Catalog>>\nendobj\nxref\n0 2\n0000000000 65535 f\r\n0000000009 00000 n\r\ntrailer\n<</Size 2/Root 1 0 R>>\nstartxref\n43\n%%EOF\n";
        assert!(verify(bytes, None, &TrustStore::default()).is_empty());
    }

    #[test]
    fn unreadable_signatures_are_never_valid() {
        let mut report = SignatureReport {
            field: None,
            page: None,
            rect: None,
            view: None,
            document_timestamp: false,
            sub_filter: None,
            status: Status::Invalid,
            integrity: Integrity::Unchecked,
            trust: Trust::Trusted,
            signer: None,
            name: None,
            reason: None,
            location: None,
            contact: None,
            claimed_time: None,
            timestamp: None,
            revision: 1,
            revisions: 1,
            covers_whole_file: true,
            changes: None,
            problems: vec![UNSUPPORTED_ALGORITHM],
        };
        assert_eq!(verdict(&report), Status::Invalid);
        report.integrity = Integrity::Intact;
        report.problems.clear();
        assert_eq!(verdict(&report), Status::Valid);
        report.trust = Trust::Untrusted;
        assert_eq!(verdict(&report), Status::Unknown);
    }
}
