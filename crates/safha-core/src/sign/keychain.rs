//! Signing identities in the macOS keychain (ADR 0047): certificates with a
//! private key, such as one a company issued or a `.p12` the user imported
//! with Keychain Access. The key never leaves the keychain: it signs there,
//! and macOS asks the user to allow it the first time.
//!
//! Only identities meant for signing documents are offered: a supported key,
//! a key usage that allows signing, in force now, and no extended key usage
//! that limits it to something else (code signing, TLS clients).

use std::sync::Arc;

use const_oid::ObjectIdentifier;
use der::Decode;
use security_framework::identity::SecIdentity;
use security_framework::item::{ItemClass, ItemSearchOptions, Limit, Reference, SearchResult};
use security_framework::key::{Algorithm, SecKey};
use x509_cert::Certificate;

use super::identity::{CertificateSummary, Identity, KeyKind, KeyOperation, SIGNING_FAILED};
use super::trust::may_sign;
use crate::error::{Error, Result};

/// The keychain could not be searched.
pub(crate) const KEYCHAIN_UNAVAILABLE: &str = "keychain_unavailable";

/// Extended key usages that allow signing documents.
const DOCUMENT_USAGES: [ObjectIdentifier; 5] = [
    ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.4"), // e-mail protection (S/MIME)
    ObjectIdentifier::new_unwrap("1.2.840.113583.1.1.5"), // Adobe Authentic Documents Trust
    ObjectIdentifier::new_unwrap("1.3.6.1.4.1.311.10.3.12"), // Microsoft document signing
    ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.36"), // document signing (RFC 9336)
    ObjectIdentifier::new_unwrap("2.5.29.37.0"),       // any
];

/// The keychain's identities that can sign documents.
pub fn identities() -> Result<Vec<CertificateSummary>> {
    Ok(signing_identities()?
        .into_iter()
        .map(|(_, certificate)| CertificateSummary::of(&certificate))
        .collect())
}

/// The keychain identity whose certificate has SHA-256 `fingerprint` (as
/// [`CertificateSummary::fingerprint`]), ready to sign.
pub fn identity(fingerprint: &str) -> Result<Identity> {
    let (identity, certificate) = signing_identities()?
        .into_iter()
        .find(|(_, certificate)| CertificateSummary::of(certificate).fingerprint == fingerprint)
        .ok_or(Error::Signing(super::identity::UNREADABLE))?;
    let key = identity.private_key().map_err(|_| Error::Signing(KEYCHAIN_UNAVAILABLE))?;
    let kind = KeyKind::of(&certificate).ok_or(Error::Signing(super::identity::KEY_UNSUPPORTED))?;
    let der = der::Encode::to_der(&certificate).map_err(|_| Error::Signing(SIGNING_FAILED))?;
    Identity::external(&der, &chain_candidates(), Arc::new(KeychainKey { key, kind }))
}

fn signing_identities() -> Result<Vec<(SecIdentity, Certificate)>> {
    let results =
        ItemSearchOptions::new().class(ItemClass::identity()).load_refs(true).limit(Limit::All).search();
    let results = match results {
        Ok(results) => results,
        // errSecItemNotFound: none at all.
        Err(err) if err.code() == -25300 => return Ok(Vec::new()),
        Err(_) => return Err(Error::Signing(KEYCHAIN_UNAVAILABLE)),
    };
    let now = std::time::SystemTime::now();
    let mut identities = Vec::new();
    for result in results {
        let SearchResult::Ref(Reference::Identity(identity)) = result else { continue };
        let Ok(certificate) = identity.certificate() else { continue };
        let Ok(certificate) = Certificate::from_der(&certificate.to_der()) else { continue };
        let (not_before, not_after) = super::cms::validity(&certificate);
        if KeyKind::of(&certificate).is_none()
            || !may_sign(&certificate)
            || !for_documents(&certificate)
            || now < not_before
            || now > not_after
        {
            continue;
        }
        identities.push((identity, certificate));
    }
    Ok(identities)
}

/// No extended key usage, or one that includes document signing.
fn for_documents(certificate: &Certificate) -> bool {
    use x509_cert::ext::pkix::ExtendedKeyUsage;
    match certificate.tbs_certificate().get_extension::<ExtendedKeyUsage>() {
        Ok(Some((_, usages))) => usages.0.iter().any(|usage| DOCUMENT_USAGES.contains(usage)),
        Ok(None) => true,
        Err(_) => false,
    }
}

/// The keychains' certificates, as DER: where an identity's issuers are.
fn chain_candidates() -> Vec<Vec<u8>> {
    let results = ItemSearchOptions::new()
        .class(ItemClass::certificate())
        .load_refs(true)
        .limit(Limit::Max(2000))
        .search()
        .unwrap_or_default();
    results
        .into_iter()
        .filter_map(|result| match result {
            SearchResult::Ref(Reference::Certificate(certificate)) => Some(certificate.to_der()),
            _ => None,
        })
        .collect()
}

/// A keychain key, asked to sign.
struct KeychainKey {
    key: SecKey,
    kind: KeyKind,
}

impl KeyOperation for KeychainKey {
    fn sign(&self, message: &[u8]) -> std::result::Result<Vec<u8>, String> {
        let algorithm = match self.kind {
            KeyKind::Rsa => Algorithm::RSASignatureMessagePKCS1v15SHA256,
            KeyKind::P256 => Algorithm::ECDSASignatureMessageX962SHA256,
            KeyKind::P384 => Algorithm::ECDSASignatureMessageX962SHA384,
        };
        self.key.create_signature(algorithm, message).map_err(|err| err.to_string())
    }
}
