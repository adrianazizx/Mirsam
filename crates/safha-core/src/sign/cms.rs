//! The CMS half of a PAdES signature (RFC 5652; ETSI EN 319 122-1 and
//! 319 142-1): building a detached `SignedData` for baseline B-B (content
//! type, message digest and ESS signing-certificate-v2 as signed attributes,
//! the certificate chain) and B-T (an RFC 3161 time-stamp token over the
//! signature value, as an unsigned attribute); and checking one, ours or
//! another tool's.
//!
//! PAdES baseline leaves the CMS `signing-time` attribute out: the claimed
//! time is the signature dictionary's `/M` (EN 319 142-1, 5.4).

use std::time::{Duration, SystemTime};

use cms::builder::{SignedDataBuilder, SignerInfoBuilder};
use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::content_info::ContentInfo;
use cms::signed_data::{EncapsulatedContentInfo, SignedData, SignerIdentifier, SignerInfo};
use const_oid::ObjectIdentifier;
use const_oid::db::rfc5911::{ID_DATA, ID_MESSAGE_DIGEST, ID_SIGNED_DATA, ID_SIGNING_TIME};
use const_oid::db::rfc5912::{ID_SHA_256, ID_SHA_384, ID_SHA_512};
use der::asn1::{Int, OctetString, SetOfVec};
use der::{Any, Decode, Encode};
use sha2::{Digest, Sha256, Sha384, Sha512};
use spki::AlgorithmIdentifierOwned;
use x509_cert::Certificate;
use x509_cert::attr::{Attribute, AttributeValue};
use x509_tsp::{MessageImprint, TimeStampReq, TimeStampResp, TspVersion, TstInfo};

use super::identity::{Identity, RawSignature, SIGNING_FAILED};
use super::tsa::{TIMESTAMP_FAILED, TimeStamper};
use crate::error::{Error, Result};

/// id-aa-signingCertificateV2 (RFC 5035).
const ID_AA_SIGNING_CERTIFICATE_V2: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.47");
/// id-aa-signingCertificate (RFC 2634, SHA-1): older signers' binding.
const ID_AA_SIGNING_CERTIFICATE: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.12");
/// id-aa-signatureTimeStampToken (RFC 3161, appendix A).
const ID_AA_SIGNATURE_TIME_STAMP_TOKEN: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.14");
/// id-ct-TSTInfo.
const ID_CT_TST_INFO: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.1.4");

/// The largest CMS structure read back, in bytes.
const MAX_CMS: usize = 1024 * 1024;

/// A digest algorithm a signature's content is hashed with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DigestKind {
    Sha256,
    Sha384,
    Sha512,
}

impl DigestKind {
    fn oid(self) -> ObjectIdentifier {
        match self {
            DigestKind::Sha256 => ID_SHA_256,
            DigestKind::Sha384 => ID_SHA_384,
            DigestKind::Sha512 => ID_SHA_512,
        }
    }

    pub(crate) fn of(oid: ObjectIdentifier) -> Option<Self> {
        match oid {
            ID_SHA_256 => Some(DigestKind::Sha256),
            ID_SHA_384 => Some(DigestKind::Sha384),
            ID_SHA_512 => Some(DigestKind::Sha512),
            _ => None,
        }
    }

    fn algorithm(self) -> AlgorithmIdentifierOwned {
        AlgorithmIdentifierOwned { oid: self.oid(), parameters: None }
    }

    /// The digest of `parts` one after the other.
    pub(crate) fn digest(self, parts: &[&[u8]]) -> Vec<u8> {
        fn run<D: Digest>(parts: &[&[u8]]) -> Vec<u8> {
            let mut hasher = D::new();
            for part in parts {
                hasher.update(part);
            }
            hasher.finalize().to_vec()
        }
        match self {
            DigestKind::Sha256 => run::<Sha256>(parts),
            DigestKind::Sha384 => run::<Sha384>(parts),
            DigestKind::Sha512 => run::<Sha512>(parts),
        }
    }
}

// ------------------------------------------------------------ building

/// SigningCertificateV2 ::= SEQUENCE { certs SEQUENCE OF ESSCertIDv2 }, with
/// ESSCertIDv2 ::= SEQUENCE { certHash OCTET STRING }: SHA-256 is the
/// default hash, so its identifier is left out, as DER requires.
fn signing_certificate_v2(certificate: &Certificate) -> Result<Attribute> {
    let der = certificate.to_der().map_err(|_| Error::Signing(SIGNING_FAILED))?;
    let hash = OctetString::new(Sha256::digest(der).to_vec()).map_err(|_| Error::Signing(SIGNING_FAILED))?;
    let octets = hash.to_der().map_err(|_| Error::Signing(SIGNING_FAILED))?;
    let value = wrap(0x30, &wrap(0x30, &wrap(0x30, &octets)));
    attribute(ID_AA_SIGNING_CERTIFICATE_V2, &value)
}

fn attribute(oid: ObjectIdentifier, value_der: &[u8]) -> Result<Attribute> {
    let failed = |_| Error::Signing(SIGNING_FAILED);
    let mut values = SetOfVec::new();
    values.insert(AttributeValue::from_der(value_der).map_err(failed)?).map_err(failed)?;
    Ok(Attribute { oid, values })
}

/// A DER header (definite length) in front of `body`.
fn wrap(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let length = body.len();
    if length < 0x80 {
        out.push(length as u8);
    } else {
        let bytes = length.to_be_bytes();
        let skip = bytes.iter().take_while(|b| **b == 0).count();
        out.push(0x80 | (bytes.len() - skip) as u8);
        out.extend_from_slice(&bytes[skip..]);
    }
    out.extend_from_slice(body);
    out
}

/// Signs `content_digest` (the digest of the bytes the signature covers,
/// with the identity's digest) and returns the DER of the CMS `ContentInfo`
/// for `/Contents`. With `timestamper`, the signature value is time-stamped
/// (B-T); a time-stamp that cannot be had fails the signing.
pub(crate) fn build(
    identity: &Identity,
    content_digest: &[u8],
    timestamper: Option<&dyn TimeStamper>,
) -> Result<Vec<u8>> {
    let failed = |_| Error::Signing(SIGNING_FAILED);
    let signer = identity.signer()?;
    let digest = identity.kind().digest();
    let certificate = &identity.certificate;
    let encapsulated = EncapsulatedContentInfo { econtent_type: ID_DATA, econtent: None };
    let sid = SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
        issuer: certificate.tbs_certificate().issuer().clone(),
        serial_number: certificate.tbs_certificate().serial_number().clone(),
    });
    let mut info = SignerInfoBuilder::new(sid, digest.algorithm(), &encapsulated, Some(content_digest))
        .map_err(failed)?;
    info.add_signed_attribute(signing_certificate_v2(certificate)?).map_err(failed)?;

    let mut builder = SignedDataBuilder::new(&encapsulated);
    builder.add_digest_algorithm(digest.algorithm()).map_err(failed)?;
    builder.add_certificate(CertificateChoices::Certificate(certificate.clone())).map_err(failed)?;
    for issuer in &identity.chain {
        builder.add_certificate(CertificateChoices::Certificate(issuer.clone())).map_err(failed)?;
    }
    let mut failure: Option<Error> = None;
    builder
        .add_signer_info_cb::<_, RawSignature, _>(info, &signer, |signer_info| {
            let Some(timestamper) = timestamper else { return Ok(()) };
            // The token covers the signature value, so it comes after signing.
            match timestamp_attribute(signer_info.signature.as_bytes(), timestamper) {
                Ok(attribute) => {
                    let mut unsigned = SetOfVec::new();
                    unsigned
                        .insert(attribute)
                        .map_err(|_| cms::builder::Error::Asn1(der::ErrorKind::Failed.into()))?;
                    signer_info.unsigned_attrs = Some(unsigned);
                }
                Err(err) => failure = Some(err),
            }
            Ok(())
        })
        .map_err(|_| failure.take().unwrap_or(Error::Signing(SIGNING_FAILED)))?;
    if let Some(err) = failure {
        return Err(err);
    }
    builder.build().map_err(failed)?.to_der().map_err(|_| Error::Signing(SIGNING_FAILED))
}

/// Asks `timestamper` for a token over `signature` and checks it answers
/// that request: the imprint and the nonce.
fn timestamp_attribute(signature: &[u8], timestamper: &dyn TimeStamper) -> Result<Attribute> {
    let invalid = |_| Error::Signing(TIMESTAMP_FAILED);
    let imprint = Sha256::digest(signature).to_vec();
    let mut nonce = [0u8; 8];
    getrandom::fill(&mut nonce).map_err(|_| Error::Signing(TIMESTAMP_FAILED))?;
    nonce[0] &= 0x7F; // positive
    nonce[0] |= 0x01; // and no leading zero byte
    let nonce = Int::new(&nonce).map_err(invalid)?;
    let request = TimeStampReq {
        version: TspVersion::V1,
        message_imprint: MessageImprint {
            hash_algorithm: spki::AlgorithmIdentifier { oid: ID_SHA_256, parameters: Some(Any::null()) },
            hashed_message: OctetString::new(imprint.clone()).map_err(invalid)?,
        },
        req_policy: None,
        nonce: Some(nonce.clone()),
        cert_req: true,
        extensions: None,
    };
    let reply = timestamper.stamp(&request.to_der().map_err(invalid)?)?;
    let response = TimeStampResp::from_der(&reply).map_err(invalid)?;
    use x509_tsp::cmpv2::status::PkiStatus;
    if !matches!(response.status.status, PkiStatus::Accepted | PkiStatus::GrantedWithMods) {
        return Err(Error::Signing(TIMESTAMP_FAILED));
    }
    let token = response.time_stamp_token.ok_or(Error::Signing(TIMESTAMP_FAILED))?;
    let info = tst_info(&token).ok_or(Error::Signing(TIMESTAMP_FAILED))?;
    if info.message_imprint.hashed_message.as_bytes() != imprint.as_slice()
        || info.nonce.as_ref() != Some(&nonce)
    {
        return Err(Error::Signing(TIMESTAMP_FAILED));
    }
    attribute(ID_AA_SIGNATURE_TIME_STAMP_TOKEN, &token.to_der().map_err(invalid)?)
}

// ------------------------------------------------------------ checking

/// What checking a CMS signature found. The certificate path is checked
/// apart ([`super::trust`]).
#[derive(Debug, Default)]
pub(crate) struct Checked {
    /// The signer's certificate, found in the signature.
    pub(crate) signer: Option<Certificate>,
    /// Every certificate the signature carries.
    pub(crate) certificates: Vec<Certificate>,
    /// The message-digest attribute equals the digest of the covered bytes.
    pub(crate) digest_matches: bool,
    /// The signature over the signed attributes verifies with the signer's key.
    pub(crate) signature_valid: bool,
    /// The signed attributes bind the signer's certificate by hash (ESS);
    /// `None` when they do not name one (older, non-PAdES signatures).
    pub(crate) certificate_bound: Option<bool>,
    /// The CMS `signing-time` attribute, when present (non-PAdES signers).
    pub(crate) signing_time: Option<SystemTime>,
    pub(crate) timestamp: Option<TimestampChecked>,
    /// Why it could not be checked fully (an unsupported algorithm).
    pub(crate) problem: Option<&'static str>,
}

/// An RFC 3161 time-stamp token's checks.
#[derive(Debug)]
pub(crate) struct TimestampChecked {
    pub(crate) time: SystemTime,
    /// It is over what it should be (the signature value, or the document).
    pub(crate) imprint_matches: bool,
    /// The authority's signature over the token verifies.
    pub(crate) signature_valid: bool,
    pub(crate) authority: Option<Certificate>,
    pub(crate) certificates: Vec<Certificate>,
}

/// Why a signature could not be read at all.
pub(crate) const UNREADABLE_SIGNATURE: &str = "signature_unreadable";
/// It uses an algorithm this app does not check (SHA-1, DSA, …).
pub(crate) const UNSUPPORTED_ALGORITHM: &str = "algorithm_unsupported";

/// Checks the CMS `der` (from `/Contents`, zero padding allowed after it)
/// against the bytes it covers; `digest_of` hashes them with an algorithm.
pub(crate) fn check(
    der: &[u8],
    digest_of: &dyn Fn(DigestKind) -> Vec<u8>,
) -> std::result::Result<Checked, &'static str> {
    let der = trim_padding(der).ok_or(UNREADABLE_SIGNATURE)?;
    let data = signed_data(der).ok_or(UNREADABLE_SIGNATURE)?;
    let certificates = certificates_of(&data);
    let info = data.signer_infos.0.iter().next().ok_or(UNREADABLE_SIGNATURE)?;
    let raw_attributes = raw_signed_attributes(der, 0);
    let mut checked = check_signer(info, &certificates, raw_attributes.as_deref(), Some(digest_of))?;
    checked.certificates = certificates;
    if let Some(unsigned) = &info.unsigned_attrs
        && let Some(attribute) = unsigned.iter().find(|a| a.oid == ID_AA_SIGNATURE_TIME_STAMP_TOKEN)
        && let Some(value) = attribute.values.iter().next()
        && let Ok(token_der) = value.to_der()
    {
        let signature = info.signature.as_bytes().to_vec();
        checked.timestamp = check_token(&token_der, &|kind| kind.digest(&[&signature])).ok();
    }
    Ok(checked)
}

/// Checks a document time-stamp (`/SubFilter /ETSI.RFC3161`): `/Contents` is
/// a time-stamp token whose imprint is the digest of the covered bytes.
pub(crate) fn check_document_timestamp(
    der: &[u8],
    digest_of: &dyn Fn(DigestKind) -> Vec<u8>,
) -> std::result::Result<TimestampChecked, &'static str> {
    check_token(trim_padding(der).ok_or(UNREADABLE_SIGNATURE)?, digest_of)
}

/// A time-stamp token (a CMS `SignedData` over a `TSTInfo`): its time, its
/// imprint against `digest_of` of what it should cover, and its signature.
fn check_token(
    der: &[u8],
    digest_of: &dyn Fn(DigestKind) -> Vec<u8>,
) -> std::result::Result<TimestampChecked, &'static str> {
    let token = ContentInfo::from_der(der).map_err(|_| UNREADABLE_SIGNATURE)?;
    let data = signed_data_of(&token).ok_or(UNREADABLE_SIGNATURE)?;
    let tst = tst_info(&token).ok_or(UNREADABLE_SIGNATURE)?;
    let econtent = tst_bytes(&data).ok_or(UNREADABLE_SIGNATURE)?;
    let certificates = certificates_of(&data);
    let info = data.signer_infos.0.iter().next().ok_or(UNREADABLE_SIGNATURE)?;
    let raw_attributes = raw_signed_attributes(der, 0);
    let econtent_digest = |kind: DigestKind| kind.digest(&[&econtent]);
    let signer = check_signer(info, &certificates, raw_attributes.as_deref(), Some(&econtent_digest))?;
    let imprint_matches = DigestKind::of(tst.message_imprint.hash_algorithm.oid)
        .is_some_and(|kind| digest_of(kind) == tst.message_imprint.hashed_message.as_bytes());
    Ok(TimestampChecked {
        time: SystemTime::UNIX_EPOCH + tst.gen_time.to_unix_duration(),
        imprint_matches,
        signature_valid: signer.signature_valid && signer.digest_matches,
        authority: signer.signer,
        certificates,
    })
}

fn signed_data(der: &[u8]) -> Option<SignedData> {
    if der.len() > MAX_CMS {
        return None;
    }
    signed_data_of(&ContentInfo::from_der(der).ok()?)
}

fn signed_data_of(content: &ContentInfo) -> Option<SignedData> {
    (content.content_type == ID_SIGNED_DATA).then(|| content.content.decode_as().ok()).flatten()
}

fn tst_bytes(data: &SignedData) -> Option<Vec<u8>> {
    if data.encap_content_info.econtent_type != ID_CT_TST_INFO {
        return None;
    }
    let econtent = data.encap_content_info.econtent.as_ref()?;
    Some(econtent.decode_as::<OctetString>().ok()?.as_bytes().to_vec())
}

fn tst_info(token: &ContentInfo) -> Option<TstInfo> {
    TstInfo::from_der(&tst_bytes(&signed_data_of(token)?)?).ok()
}

fn certificates_of(data: &SignedData) -> Vec<Certificate> {
    data.certificates
        .iter()
        .flat_map(|set| set.0.iter())
        .filter_map(|choice| match choice {
            CertificateChoices::Certificate(certificate) => Some(certificate.clone()),
            _ => None,
        })
        .collect()
}

fn check_signer(
    info: &SignerInfo,
    certificates: &[Certificate],
    raw_attributes: Option<&[u8]>,
    digest_of: Option<&dyn Fn(DigestKind) -> Vec<u8>>,
) -> std::result::Result<Checked, &'static str> {
    let signer = certificates
        .iter()
        .find(|certificate| match &info.sid {
            SignerIdentifier::IssuerAndSerialNumber(id) => {
                certificate.tbs_certificate().issuer() == &id.issuer
                    && certificate.tbs_certificate().serial_number() == &id.serial_number
            }
            SignerIdentifier::SubjectKeyIdentifier(id) => {
                use x509_cert::ext::pkix::SubjectKeyIdentifier;
                matches!(
                    certificate.tbs_certificate().get_extension::<SubjectKeyIdentifier>(),
                    Ok(Some((_, own))) if own == *id
                )
            }
        })
        .cloned();
    let mut checked = Checked { signer: signer.clone(), ..Checked::default() };
    let Some(attributes) = &info.signed_attrs else {
        // Without signed attributes the signature is over the content
        // itself: allowed by CMS, unused by PDF signers.
        checked.problem = Some(UNSUPPORTED_ALGORITHM);
        return Ok(checked);
    };
    let Some(digest) = DigestKind::of(info.digest_alg.oid) else {
        checked.problem = Some(UNSUPPORTED_ALGORITHM);
        return Ok(checked);
    };

    // The digest of the content is a signed attribute.
    let claimed = attributes
        .iter()
        .find(|a| a.oid == ID_MESSAGE_DIGEST)
        .and_then(|a| a.values.iter().next())
        .and_then(|value| value.decode_as::<OctetString>().ok());
    checked.digest_matches = match (claimed, digest_of) {
        (Some(claimed), Some(digest_of)) => claimed.as_bytes() == digest_of(digest).as_slice(),
        _ => false,
    };

    // The signature is over the attributes as a SET (tag 0x31): the bytes as
    // the signer encoded them, else re-encoded.
    let signed = match raw_attributes {
        Some(raw) => raw.to_vec(),
        None => attributes.to_der().map_err(|_| UNREADABLE_SIGNATURE)?,
    };
    if let Some(signer) = &signer {
        match verify_with(signer, &info.signature_algorithm, &signed, info.signature.as_bytes()) {
            Some(valid) => checked.signature_valid = valid,
            None => checked.problem = Some(UNSUPPORTED_ALGORITHM),
        }
        let bound = |oid: ObjectIdentifier, hash: Vec<u8>| {
            attributes.iter().find(|a| a.oid == oid).map(|attribute| {
                attribute
                    .values
                    .iter()
                    .next()
                    .and_then(|value| value.to_der().ok())
                    .is_some_and(|value| value.windows(hash.len()).any(|w| w == hash.as_slice()))
            })
        };
        let der = signer.to_der().unwrap_or_default();
        checked.certificate_bound = bound(ID_AA_SIGNING_CERTIFICATE_V2, Sha256::digest(&der).to_vec())
            .or_else(|| bound(ID_AA_SIGNING_CERTIFICATE, sha1_of(&der)));
    }
    checked.signing_time = attributes
        .iter()
        .find(|a| a.oid == ID_SIGNING_TIME)
        .and_then(|a| a.values.iter().next())
        .and_then(|value| value.decode_as::<x509_cert::time::Time>().ok())
        .map(|time| time.to_system_time());
    Ok(checked)
}

/// SHA-1, only to compare an older signer's ESS v1 certificate binding;
/// nothing is signed or trusted on it.
fn sha1_of(data: &[u8]) -> Vec<u8> {
    sha1::Sha1::digest(data).to_vec()
}

/// Whether `signature` over `message` verifies with `certificate`'s key;
/// `None` for an algorithm this app does not check.
pub(crate) fn verify_with(
    certificate: &Certificate,
    algorithm: &AlgorithmIdentifierOwned,
    message: &[u8],
    signature: &[u8],
) -> Option<bool> {
    use x509_verify::{Message, Signature, VerifyInfo, VerifyingKey};
    let key = VerifyingKey::try_from(certificate).ok()?;
    match key.verify(VerifyInfo::new(Message::new(message), Signature::new(algorithm, signature))) {
        Ok(()) => Some(true),
        Err(x509_verify::Error::Verification) => Some(false),
        Err(_) => None,
    }
}

/// The DER structure at the start of `bytes` without the zero padding a
/// `/Contents` placeholder leaves after it.
pub(crate) fn trim_padding(bytes: &[u8]) -> Option<&[u8]> {
    let length = tlv(bytes, 0)?.2;
    bytes.get(..length)
}

/// (tag, header length, total length) of the TLV at `at`; definite lengths only.
fn tlv(bytes: &[u8], at: usize) -> Option<(u8, usize, usize)> {
    let tag = *bytes.get(at)?;
    let first = *bytes.get(at + 1)?;
    let (header, length) = if first < 0x80 {
        (2, usize::from(first))
    } else {
        let count = usize::from(first & 0x7F);
        if count == 0 || count > 4 {
            return None;
        }
        let length =
            bytes.get(at + 2..at + 2 + count)?.iter().fold(0usize, |n, b| (n << 8) | usize::from(*b));
        (2 + count, length)
    };
    let total = header.checked_add(length)?;
    (at.checked_add(total)? <= bytes.len()).then_some((tag, header, total))
}

/// The signer's signed attributes as the signer encoded them, re-tagged as a
/// SET (what the signature covers, RFC 5652, 5.4), for signer `index`.
fn raw_signed_attributes(der: &[u8], index: usize) -> Option<Vec<u8>> {
    // ContentInfo ::= SEQUENCE { contentType, [0] EXPLICIT SignedData }
    let (_, header, _) = tlv(der, 0)?;
    let oid = tlv(der, header)?;
    let explicit_at = header + oid.2;
    let (_, explicit_header, _) = tlv(der, explicit_at)?;
    // SignedData ::= SEQUENCE { version, digestAlgorithms, encapContentInfo,
    //   [0] certificates OPTIONAL, [1] crls OPTIONAL, signerInfos }
    let signed_at = explicit_at + explicit_header;
    let (_, signed_header, signed_total) = tlv(der, signed_at)?;
    let mut at = signed_at + signed_header;
    let end = signed_at + signed_total;
    let mut signer_infos = None;
    while at < end {
        let (tag, _, total) = tlv(der, at)?;
        if tag == 0x31 {
            signer_infos = Some(at); // the last SET is signerInfos
        }
        at += total;
    }
    let set_at = signer_infos?;
    let (_, set_header, set_total) = tlv(der, set_at)?;
    let mut at = set_at + set_header;
    for _ in 0..index {
        at += tlv(der, at)?.2;
    }
    if at >= set_at + set_total {
        return None;
    }
    // SignerInfo ::= SEQUENCE { version, sid, digestAlgorithm,
    //   [0] IMPLICIT signedAttrs OPTIONAL, … }
    let (_, info_header, _) = tlv(der, at)?;
    let mut field = at + info_header;
    for _ in 0..3 {
        field += tlv(der, field)?.2;
    }
    let (tag, _, total) = tlv(der, field)?;
    if tag != 0xA0 {
        return None;
    }
    let mut raw = der.get(field..field + total)?.to_vec();
    raw[0] = 0x31;
    Some(raw)
}

/// When a certificate is in force: for showing, and for path checks.
pub(crate) fn validity(certificate: &Certificate) -> (SystemTime, SystemTime) {
    let validity = certificate.tbs_certificate().validity();
    (validity.not_before.to_system_time(), validity.not_after.to_system_time())
}

/// A time a second either side of which the check still passes: CA clocks
/// and ours disagree a little.
pub(crate) const CLOCK_SLACK: Duration = Duration::from_secs(60);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lengths_get_long_form_headers() {
        assert_eq!(wrap(0x04, &[1, 2]), [0x04, 2, 1, 2]);
        let long = wrap(0x04, &[0; 300]);
        assert_eq!(&long[..4], &[0x04, 0x82, 0x01, 0x2C]);
        assert_eq!(tlv(&long, 0), Some((0x04, 4, 304)));
    }

    #[test]
    fn padding_after_the_structure_is_trimmed() {
        let mut bytes = wrap(0x30, &[0x05, 0x00]);
        bytes.extend_from_slice(&[0; 10]);
        assert_eq!(trim_padding(&bytes), Some(&bytes[..4]));
        assert_eq!(trim_padding(&[0x30, 0x85, 0, 0, 0, 0, 1]), None);
    }
}
