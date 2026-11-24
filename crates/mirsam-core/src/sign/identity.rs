//! Signing identities: a certificate, the chain that issued it, and the key
//! that signs with it (ADR 0048).
//!
//! The key is either held in memory (read from a PKCS#12 file, `.p12` or
//! `.pfx`, with its password) or stays where it lives and is asked to sign
//! (the macOS keychain, [`super::keychain`]): [`KeyOperation`]. Either way the
//! CMS builder sees one [`Signer`] whose algorithm follows the certificate's
//! public key: RSA PKCS#1 v1.5 with SHA-256, ECDSA P-256 with SHA-256, or
//! ECDSA P-384 with SHA-384.

use std::fmt;
use std::sync::Arc;

use const_oid::ObjectIdentifier;
use const_oid::db::rfc5912::{
    ECDSA_WITH_SHA_256, ECDSA_WITH_SHA_384, ID_EC_PUBLIC_KEY, RSA_ENCRYPTION, SECP_256_R_1, SECP_384_R_1,
    SHA_256_WITH_RSA_ENCRYPTION,
};
use der::asn1::BitString;
use der::{Any, Decode, Encode, Tagged};
use serde::Serialize;
use sha2::{Digest, Sha256};
use spki::{AlgorithmIdentifierOwned, EncodePublicKey, SignatureBitStringEncoding};
use x509_cert::Certificate;
use x509_cert::name::Name;

use crate::error::{Error, Result};

/// Why an identity cannot be used, as the UI's error codes.
pub(crate) const WRONG_PASSWORD: &str = "certificate_password";
pub(crate) const UNREADABLE: &str = "certificate_unreadable";
pub(crate) const KEY_UNSUPPORTED: &str = "key_unsupported";
pub(crate) const SIGNING_FAILED: &str = "signing_failed";

/// The largest PKCS#12 file read, in bytes.
pub const MAX_PKCS12_BYTES: usize = 1024 * 1024;

/// The public-key algorithm of a certificate, and so how it signs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyKind {
    Rsa,
    P256,
    P384,
}

impl KeyKind {
    /// The key algorithm a certificate's public key is for.
    pub(crate) fn of(certificate: &Certificate) -> Option<Self> {
        let info = certificate.tbs_certificate().subject_public_key_info();
        match info.algorithm.oid {
            RSA_ENCRYPTION => Some(KeyKind::Rsa),
            ID_EC_PUBLIC_KEY => {
                let curve = info.algorithm.parameters.as_ref()?.decode_as::<ObjectIdentifier>().ok()?;
                match curve {
                    SECP_256_R_1 => Some(KeyKind::P256),
                    SECP_384_R_1 => Some(KeyKind::P384),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The digest the content and the signed attributes are hashed with.
    pub(crate) fn digest(self) -> super::cms::DigestKind {
        match self {
            KeyKind::P384 => super::cms::DigestKind::Sha384,
            _ => super::cms::DigestKind::Sha256,
        }
    }

    fn signature_algorithm(self) -> AlgorithmIdentifierOwned {
        match self {
            // PKCS#1 v1.5 algorithm identifiers carry an explicit NULL.
            KeyKind::Rsa => {
                AlgorithmIdentifierOwned { oid: SHA_256_WITH_RSA_ENCRYPTION, parameters: Some(Any::null()) }
            }
            KeyKind::P256 => AlgorithmIdentifierOwned { oid: ECDSA_WITH_SHA_256, parameters: None },
            KeyKind::P384 => AlgorithmIdentifierOwned { oid: ECDSA_WITH_SHA_384, parameters: None },
        }
    }
}

/// A key that signs where it lives (a keychain, a token): asked for one
/// signature at a time and never read out.
pub trait KeyOperation: Send + Sync {
    /// Signs `message`, hashing it first with the digest of the
    /// certificate's [`KeyKind`] (SHA-256, or SHA-384 for P-384). Returns a
    /// PKCS#1 v1.5 signature for RSA, a DER `Ecdsa-Sig-Value` for ECDSA. The
    /// error is for logs; the UI says signing failed.
    fn sign(&self, message: &[u8]) -> std::result::Result<Vec<u8>, String>;
}

/// The private half of an identity.
#[derive(Clone)]
enum Key {
    Rsa(rsa::PrivateKey),
    P256(p256::ecdsa::SigningKey),
    P384(p384::ecdsa::SigningKey),
    External(Arc<dyn KeyOperation>),
}

/// A certificate with the key that signs for it.
#[derive(Clone)]
pub struct Identity {
    pub(crate) certificate: Certificate,
    /// Certificates that issued it, nearest first, as the source had them
    /// (put in the signature so a validator can build the path).
    pub(crate) chain: Vec<Certificate>,
    kind: KeyKind,
    key: Key,
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never the key.
        f.debug_struct("Identity").field("subject", &self.summary().name).field("kind", &self.kind).finish()
    }
}

impl Identity {
    /// Reads a PKCS#12 file (`.p12`, `.pfx`) with its password: the first
    /// private key, the certificate whose public key matches it, and the
    /// other certificates in the file that issued it.
    pub fn from_pkcs12(data: &[u8], password: &str) -> Result<Identity> {
        if data.len() > MAX_PKCS12_BYTES {
            return Err(Error::Signing(UNREADABLE));
        }
        let archive = p12_keystore::Pkcs12Archive::from_pkcs12(data, password).map_err(|err| match err {
            p12_keystore::error::Error::MacError(_) => Error::Signing(WRONG_PASSWORD),
            // A wrong password on a file without a MAC shows as bad padding
            // or a key that does not parse.
            p12_keystore::error::Error::UnpadError | p12_keystore::error::Error::Pkcs5Error(_) => {
                Error::Signing(WRONG_PASSWORD)
            }
            _ => Error::Signing(UNREADABLE),
        })?;
        let certificates: Vec<Certificate> =
            archive.certs.iter().filter_map(|bag| Certificate::from_der(bag.cert.as_der()).ok()).collect();
        let mut unsupported = false;
        for bag in &archive.keys {
            let Some((key, public)) = software_key(bag.key.as_der(), &mut unsupported) else { continue };
            let Some(leaf) = certificates.iter().find(|certificate| {
                certificate.tbs_certificate().subject_public_key_info().to_der().ok().as_deref()
                    == Some(public.as_slice())
            }) else {
                continue;
            };
            let kind = KeyKind::of(leaf).ok_or(Error::Signing(KEY_UNSUPPORTED))?;
            let chain = issuers(leaf, &certificates);
            return Ok(Identity { certificate: leaf.clone(), chain, kind, key });
        }
        Err(Error::Signing(if unsupported { KEY_UNSUPPORTED } else { UNREADABLE }))
    }

    /// An identity whose key signs elsewhere (`operation`), for
    /// `certificate` (DER) and the `chain` (DER) that issued it.
    pub fn external(
        certificate: &[u8],
        chain: &[Vec<u8>],
        operation: Arc<dyn KeyOperation>,
    ) -> Result<Identity> {
        let certificate = Certificate::from_der(certificate).map_err(|_| Error::Signing(UNREADABLE))?;
        let kind = KeyKind::of(&certificate).ok_or(Error::Signing(KEY_UNSUPPORTED))?;
        let pool: Vec<Certificate> = chain.iter().filter_map(|der| Certificate::from_der(der).ok()).collect();
        let chain = issuers(&certificate, &pool);
        Ok(Identity { certificate, chain, kind, key: Key::External(operation) })
    }

    pub fn kind(&self) -> KeyKind {
        self.kind
    }

    /// What the UI shows of it.
    pub fn summary(&self) -> CertificateSummary {
        CertificateSummary::of(&self.certificate)
    }

    /// The signer the CMS builder calls.
    pub(crate) fn signer(&self) -> Result<Signer<'_>> {
        let spki = self
            .certificate
            .tbs_certificate()
            .subject_public_key_info()
            .to_der()
            .map_err(|_| Error::Signing(UNREADABLE))?;
        Ok(Signer { identity: self, spki: PublicKeyDer(spki) })
    }
}

/// The PKCS#8 key in `der`, and its public key as SPKI DER, for the
/// algorithms we sign with.
fn software_key(der: &[u8], unsupported: &mut bool) -> Option<(Key, Vec<u8>)> {
    use pkcs8::DecodePrivateKey;
    let info = pkcs8::PrivateKeyInfoRef::try_from(der).ok()?;
    match info.algorithm.oid {
        RSA_ENCRYPTION => {
            let key = rsa::PrivateKey::from_pkcs8_der(der).ok()?;
            let public = key.to_public_key().to_public_key_der().ok()?.into_vec();
            Some((Key::Rsa(key), public))
        }
        ID_EC_PUBLIC_KEY => {
            if let Ok(key) = p256::ecdsa::SigningKey::from_pkcs8_der(der) {
                let public = key.verifying_key().to_public_key_der().ok()?.into_vec();
                return Some((Key::P256(key), public));
            }
            if let Ok(key) = p384::ecdsa::SigningKey::from_pkcs8_der(der) {
                let public = key.verifying_key().to_public_key_der().ok()?.into_vec();
                return Some((Key::P384(key), public));
            }
            *unsupported = true;
            None
        }
        _ => {
            *unsupported = true;
            None
        }
    }
}

/// The certificates in `pool` that issued `leaf`, nearest first, by name
/// (the validator checks the signatures); stops at a self-issued one.
pub(crate) fn issuers(leaf: &Certificate, pool: &[Certificate]) -> Vec<Certificate> {
    let mut chain: Vec<Certificate> = Vec::new();
    let mut current = leaf;
    for _ in 0..8 {
        let tbs = current.tbs_certificate();
        if tbs.issuer() == tbs.subject() {
            break;
        }
        let Some(issuer) = pool
            .iter()
            .find(|candidate| candidate.tbs_certificate().subject() == tbs.issuer() && *candidate != current)
        else {
            break;
        };
        if issuer == leaf || chain.contains(issuer) {
            break;
        }
        chain.push(issuer.clone());
        current = issuer;
    }
    chain
}

// ------------------------------------------------------------ the CMS signer

/// A public key as SPKI DER: what the CMS builder's signer bound asks for.
#[derive(Clone, Debug)]
pub(crate) struct PublicKeyDer(Vec<u8>);

impl EncodePublicKey for PublicKeyDer {
    fn to_public_key_der(&self) -> spki::Result<der::Document> {
        Ok(der::Document::try_from(self.0.as_slice())?)
    }
}

/// A signature value as the key produced it.
pub(crate) struct RawSignature(Vec<u8>);

impl SignatureBitStringEncoding for RawSignature {
    fn to_bitstring(&self) -> der::Result<BitString> {
        BitString::from_bytes(&self.0)
    }
}

/// One identity, as the CMS builder's signer.
pub(crate) struct Signer<'a> {
    identity: &'a Identity,
    spki: PublicKeyDer,
}

impl signature::Keypair for Signer<'_> {
    type VerifyingKey = PublicKeyDer;

    fn verifying_key(&self) -> PublicKeyDer {
        self.spki.clone()
    }
}

impl spki::DynSignatureAlgorithmIdentifier for Signer<'_> {
    fn signature_algorithm_identifier(&self) -> spki::Result<AlgorithmIdentifierOwned> {
        Ok(self.identity.kind.signature_algorithm())
    }
}

impl signature::Signer<RawSignature> for Signer<'_> {
    fn try_sign(&self, message: &[u8]) -> std::result::Result<RawSignature, signature::Error> {
        use signature::SignatureEncoding;
        Ok(RawSignature(match &self.identity.key {
            Key::Rsa(key) => {
                let signer = rsa::pkcs1v15::SigningKey::<Sha256>::new(key.clone());
                let signature: rsa::pkcs1v15::Signature = signature::Signer::try_sign(&signer, message)?;
                signature.to_vec()
            }
            Key::P256(key) => {
                let signature: p256::ecdsa::DerSignature = signature::Signer::try_sign(key, message)?;
                signature.to_vec()
            }
            Key::P384(key) => {
                let signature: p384::ecdsa::DerSignature = signature::Signer::try_sign(key, message)?;
                signature.to_vec()
            }
            Key::External(operation) => operation.sign(message).map_err(|err| {
                eprintln!("mirsam: the key refused to sign: {err}");
                signature::Error::new()
            })?,
        }))
    }
}

// ------------------------------------------------------------ summaries

/// A certificate as the UI shows it. Names are as the certificate spells
/// them (Arabic stays Arabic); times are Unix seconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CertificateSummary {
    /// The common name, else the organisation, else the whole name.
    pub name: String,
    pub organization: Option<String>,
    pub email: Option<String>,
    /// The issuer's common name (or organisation).
    pub issuer: String,
    pub not_before: i64,
    pub not_after: i64,
    /// SHA-256 of the certificate, upper-case hex: what to compare by hand.
    pub fingerprint: String,
    pub self_signed: bool,
    pub key: Option<KeyKind>,
}

impl CertificateSummary {
    pub(crate) fn of(certificate: &Certificate) -> Self {
        let tbs = certificate.tbs_certificate();
        let subject = tbs.subject();
        let issuer = tbs.issuer();
        let name = attribute(subject, COMMON_NAME)
            .or_else(|| attribute(subject, ORGANIZATION))
            .unwrap_or_else(|| subject.to_string());
        let fingerprint = Sha256::digest(certificate.to_der().unwrap_or_default())
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect();
        let validity = tbs.validity();
        CertificateSummary {
            name,
            organization: attribute(subject, ORGANIZATION),
            email: attribute(subject, EMAIL).or_else(|| rfc822_name(certificate)),
            issuer: attribute(issuer, COMMON_NAME)
                .or_else(|| attribute(issuer, ORGANIZATION))
                .unwrap_or_else(|| issuer.to_string()),
            not_before: unix(validity.not_before.to_system_time()),
            not_after: unix(validity.not_after.to_system_time()),
            fingerprint,
            self_signed: subject == issuer,
            key: KeyKind::of(certificate),
        }
    }
}

pub(crate) fn unix(time: std::time::SystemTime) -> i64 {
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        Err(before) => -i64::try_from(before.duration().as_secs()).unwrap_or(i64::MAX),
    }
}

const COMMON_NAME: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.3");
const ORGANIZATION: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.10");
const EMAIL: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.1");

/// The first value of attribute `oid` in `name`, decoded.
fn attribute(name: &Name, oid: ObjectIdentifier) -> Option<String> {
    name.iter()
        .find(|pair| pair.oid == oid)
        .and_then(|pair| directory_string(&pair.value))
        .filter(|text| !text.trim().is_empty())
}

/// A DirectoryString (or IA5String) as text.
pub(crate) fn directory_string(value: &Any) -> Option<String> {
    let bytes = value.value();
    match value.tag().number().value() {
        // UTF8String, PrintableString, IA5String, VisibleString.
        12 | 19 | 22 | 26 => String::from_utf8(bytes.to_vec()).ok(),
        // TeletexString: Latin-1 in practice.
        20 => Some(bytes.iter().map(|&b| char::from(b)).collect()),
        // BMPString: UTF-16BE.
        30 => {
            let units: Vec<u16> =
                bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_be_bytes(*pair)).collect();
            String::from_utf16(&units).ok()
        }
        // UniversalString: UTF-32BE.
        28 => bytes.as_chunks::<4>().0.iter().map(|quad| char::from_u32(u32::from_be_bytes(*quad))).collect(),
        _ => None,
    }
}

/// The first e-mail address in the certificate's subject alternative names.
fn rfc822_name(certificate: &Certificate) -> Option<String> {
    use x509_cert::ext::pkix::SubjectAltName;
    use x509_cert::ext::pkix::name::GeneralName;
    let (_, names) = certificate.tbs_certificate().get_extension::<SubjectAltName>().ok()??;
    names.0.iter().find_map(|name| match name {
        GeneralName::Rfc822Name(email) => Some(email.to_string()),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_strings_decode_in_every_encoding() {
        let utf8 = Any::new(der::Tag::Utf8String, "الموقّع".as_bytes()).expect("valid");
        assert_eq!(directory_string(&utf8).as_deref(), Some("الموقّع"));
        let bmp: Vec<u8> = "سارة".encode_utf16().flat_map(u16::to_be_bytes).collect();
        let bmp = Any::new(der::Tag::BmpString, bmp).expect("valid");
        assert_eq!(directory_string(&bmp).as_deref(), Some("سارة"));
        let printable = Any::new(der::Tag::PrintableString, b"Test CA".as_slice()).expect("valid");
        assert_eq!(directory_string(&printable).as_deref(), Some("Test CA"));
    }

    #[test]
    fn a_file_that_is_not_pkcs12_is_unreadable() {
        assert!(matches!(Identity::from_pkcs12(b"not a pfx", "x"), Err(Error::Signing(UNREADABLE))));
    }
}
