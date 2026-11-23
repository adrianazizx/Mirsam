//! A throwaway public-key infrastructure for signing tests (ADR 0047), made
//! fresh on every run so no key is ever committed: a root CA, signers (a
//! P-256 key with a Latin name, one with an Arabic name, an RSA key), and a
//! time-stamping authority that answers RFC 3161 requests in process.
//!
//! Shared by `tests/sign.rs` and `examples/sign_fixtures.rs` (which writes
//! the same things to disk for external validators).

#![allow(dead_code)]

use std::str::FromStr;
use std::time::{Duration, SystemTime};

use cms::builder::{SignedDataBuilder, SignerInfoBuilder};
use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::signed_data::{EncapsulatedContentInfo, SignerIdentifier};
use const_oid::ObjectIdentifier;
use const_oid::db::rfc5912::ID_SHA_256;
use der::asn1::{Int, OctetString, SetOfVec};
use der::{Any, Decode, Encode, Tag};
use p12_keystore::{EncryptionAlgorithm, KeyStore, KeyStoreEntry, MacAlgorithm, PrivateKey, PrivateKeyChain};
use pkcs8::EncodePrivateKey;
use sha2::{Digest, Sha256};
use spki::{EncodePublicKey, SubjectPublicKeyInfoOwned};
use x509_cert::Certificate;
use x509_cert::attr::{Attribute, AttributeValue};
use x509_cert::builder::profile::BuilderProfile;
use x509_cert::builder::{Builder, CertificateBuilder};
use x509_cert::certificate::TbsCertificate;
use x509_cert::ext::pkix::{BasicConstraints, ExtendedKeyUsage, KeyUsage, KeyUsages};
use x509_cert::ext::{Extension, ToExtension};
use x509_cert::name::Name;
use x509_cert::serial_number::SerialNumber;
use x509_cert::time::{Time, Validity};
use x509_tsp::cmpv2::status::{PkiStatus, PkiStatusInfo};
use x509_tsp::{GeneralizedTimeNanos, TimeStampReq, TimeStampResp, TspVersion, TstInfo};

use safha_core::Error;
use safha_core::sign::{Identity, TimeStamper, TrustStore};

/// A certificate with its key, as PKCS#8 DER.
pub struct Issued {
    pub certificate: Certificate,
    pub key: Vec<u8>,
}

impl Issued {
    pub fn der(&self) -> Vec<u8> {
        self.certificate.to_der().expect("encodes")
    }

    /// The certificate as PEM, for tools that want it.
    pub fn pem(&self) -> String {
        use base64::Engine as _;
        let text = base64::engine::general_purpose::STANDARD.encode(self.der());
        let lines: Vec<&str> =
            text.as_bytes().chunks(64).map(|c| std::str::from_utf8(c).expect("ascii")).collect();
        format!("-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n", lines.join("\n"))
    }
}

/// The whole test PKI.
pub struct Pki {
    pub ca: Issued,
    pub signer: Issued,
    pub arabic: Issued,
    pub rsa: Issued,
    pub tsa: Issued,
}

pub const PASSWORD: &str = "كلمة-سر test";

impl Pki {
    pub fn new() -> Self {
        let ca_key = p256_key();
        let ca_name = name("CN=Mirsam Test Root CA,O=Mirsam tests");
        let ca = issue(&ca_name, &ca_name, &spki_p256(&ca_key), &Signer::P256(&ca_key), Role::Ca, years(10));
        let ca = Issued { certificate: ca, key: pkcs8_p256(&ca_key) };
        let leaf = |subject: &str, role: Role| {
            let key = p256_key();
            let certificate = issue(
                &name(subject),
                &ca_name,
                &spki_p256(&key),
                &Signer::P256(&p256_from(&ca.key)),
                role,
                years(5),
            );
            Issued { certificate, key: pkcs8_p256(&key) }
        };
        let signer = leaf("CN=Test Signer,O=Mirsam tests,C=OM", Role::Signer);
        let arabic = leaf("CN=الموقّع التجريبي,O=مرسام", Role::Signer);
        let tsa = leaf("CN=Mirsam Test Time-Stamping,O=Mirsam tests", Role::TimeStamping);
        let rsa_key = rsa_key();
        let rsa_spki = SubjectPublicKeyInfoOwned::from_der(
            rsa_key.to_public_key().to_public_key_der().expect("encodes").as_bytes(),
        )
        .expect("parses");
        let rsa_certificate = issue(
            &name("CN=Test Signer (RSA),O=Mirsam tests"),
            &ca_name,
            &rsa_spki,
            &Signer::P256(&p256_from(&ca.key)),
            Role::Signer,
            years(5),
        );
        let rsa = Issued {
            certificate: rsa_certificate,
            key: rsa_key.to_pkcs8_der().expect("encodes").as_bytes().to_vec(),
        };
        Pki { ca, signer, arabic, rsa, tsa }
    }

    /// Trust in this PKI's root only.
    pub fn trust(&self) -> TrustStore {
        TrustStore::from_der([self.ca.der().as_slice()])
    }

    /// `issued` and the root, as a PKCS#12 file (PBES2 with AES-256, or
    /// the legacy 3DES scheme older exports use) under `password`.
    pub fn pkcs12(&self, issued: &Issued, password: &str, legacy: bool) -> Vec<u8> {
        let key = PrivateKey::from_der(&issued.key).expect("PKCS#8");
        let chain = [
            p12_keystore::Certificate::from_der(&issued.der()).expect("parses"),
            p12_keystore::Certificate::from_der(&self.ca.der()).expect("parses"),
        ];
        let mut store = KeyStore::new();
        store.add_entry(
            "signer",
            KeyStoreEntry::PrivateKeyChain(PrivateKeyChain::new(vec![1, 2, 3, 4], key, chain)),
        );
        let writer = store.writer(password);
        let writer = if legacy {
            writer
                .encryption_algorithm(EncryptionAlgorithm::PbeWithShaAnd3KeyTripleDesCbc)
                .mac_algorithm(MacAlgorithm::HmacSha1)
        } else {
            writer
        };
        writer.write().expect("writes a PKCS#12 file")
    }

    /// `issued` as a signing identity, through a PKCS#12 file.
    pub fn identity(&self, issued: &Issued) -> Identity {
        Identity::from_pkcs12(&self.pkcs12(issued, PASSWORD, false), PASSWORD).expect("reads the identity")
    }

    /// This PKI's time-stamping authority.
    pub fn authority(&self) -> TestAuthority {
        TestAuthority { certificate: self.tsa.certificate.clone(), key: p256_from(&self.tsa.key) }
    }
}

/// An RFC 3161 authority answering in process (what a server would send).
pub struct TestAuthority {
    certificate: Certificate,
    key: p256::ecdsa::SigningKey,
}

impl TimeStamper for TestAuthority {
    fn stamp(&self, request: &[u8]) -> safha_core::Result<Vec<u8>> {
        let request = TimeStampReq::from_der(request).map_err(|_| Error::Signing("timestamp_failed"))?;
        let info = TstInfo {
            version: TspVersion::V1,
            policy: ObjectIdentifier::new_unwrap("1.2.3.4.1"),
            message_imprint: request.message_imprint,
            serial_number: Int::new(&[0x01, 0x23]).expect("int"),
            gen_time: GeneralizedTimeNanos::from_unix_duration(
                SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).expect("after 1970"),
            )
            .expect("time"),
            accuracy: None,
            ordering: false,
            nonce: request.nonce,
            tsa: None,
            extensions: None,
        };
        let content = OctetString::new(info.to_der().expect("encodes")).expect("octets");
        let encapsulated = EncapsulatedContentInfo {
            econtent_type: ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.1.4"),
            econtent: Some(Any::from_der(&content.to_der().expect("encodes")).expect("any")),
        };
        let sid = SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
            issuer: self.certificate.tbs_certificate().issuer().clone(),
            serial_number: self.certificate.tbs_certificate().serial_number().clone(),
        });
        let sha256 = spki::AlgorithmIdentifierOwned { oid: ID_SHA_256, parameters: None };
        let mut signer = SignerInfoBuilder::new(sid, sha256.clone(), &encapsulated, None).expect("builder");
        signer.add_signed_attribute(ess_v2(&self.certificate)).expect("attribute");
        let mut builder = SignedDataBuilder::new(&encapsulated);
        builder.add_digest_algorithm(sha256).expect("digest");
        builder
            .add_certificate(CertificateChoices::Certificate(self.certificate.clone()))
            .expect("certificate");
        builder.add_signer_info::<_, p256::ecdsa::DerSignature>(signer, &self.key).expect("signs");
        let token = builder.build().expect("token");
        let response = TimeStampResp {
            status: PkiStatusInfo { status: PkiStatus::Accepted, status_string: None, fail_info: None },
            time_stamp_token: Some(token),
        };
        Ok(response.to_der().expect("encodes"))
    }
}

/// An authority that is down.
pub struct DownAuthority;

impl TimeStamper for DownAuthority {
    fn stamp(&self, _request: &[u8]) -> safha_core::Result<Vec<u8>> {
        Err(Error::Signing("timestamp_failed"))
    }
}

// ------------------------------------------------------------ making certificates

enum Signer<'a> {
    P256(&'a p256::ecdsa::SigningKey),
}

#[derive(Clone, Copy)]
enum Role {
    Ca,
    Signer,
    TimeStamping,
}

struct Profile {
    subject: Name,
    issuer: Name,
    role: Role,
}

impl BuilderProfile for Profile {
    fn get_issuer(&self, _subject: &Name) -> Name {
        self.issuer.clone()
    }

    fn get_subject(&self) -> Name {
        self.subject.clone()
    }

    fn build_extensions(
        &self,
        _spk: spki::SubjectPublicKeyInfoRef<'_>,
        _issuer_spk: spki::SubjectPublicKeyInfoRef<'_>,
        tbs: &TbsCertificate,
    ) -> x509_cert::builder::Result<Vec<Extension>> {
        let subject = tbs.subject();
        let mut extensions = Vec::new();
        let mut push = |extension: Extension| extensions.push(extension);
        match self.role {
            Role::Ca => {
                push((&BasicConstraints { ca: true, path_len_constraint: None }).to_extension(subject, &[])?);
                push((&KeyUsage(KeyUsages::KeyCertSign | KeyUsages::CRLSign)).to_extension(subject, &[])?);
            }
            Role::Signer => {
                push(
                    (&BasicConstraints { ca: false, path_len_constraint: None })
                        .to_extension(subject, &[])?,
                );
                push(
                    (&KeyUsage(KeyUsages::DigitalSignature | KeyUsages::NonRepudiation))
                        .to_extension(subject, &[])?,
                );
                // NSS (pdfsig) wants e-mail protection; Adobe its own OID.
                push(
                    (&ExtendedKeyUsage(vec![
                        ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.4"),
                        ObjectIdentifier::new_unwrap("1.2.840.113583.1.1.5"),
                    ]))
                        .to_extension(subject, &[])?,
                );
            }
            Role::TimeStamping => {
                push(
                    (&BasicConstraints { ca: false, path_len_constraint: None })
                        .to_extension(subject, &[])?,
                );
                push((&KeyUsage(KeyUsages::DigitalSignature.into())).to_extension(subject, &[])?);
                let mut eku = (&ExtendedKeyUsage(vec![ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.8")]))
                    .to_extension(subject, &[])?;
                eku.critical = true;
                push(eku);
            }
        }
        Ok(extensions)
    }
}

fn issue(
    subject: &Name,
    issuer: &Name,
    spki: &SubjectPublicKeyInfoOwned,
    signer: &Signer<'_>,
    role: Role,
    validity: Validity,
) -> Certificate {
    let mut serial = [0u8; 16];
    getrandom::fill(&mut serial).expect("random");
    serial[0] &= 0x7F;
    serial[0] |= 0x01;
    let profile = Profile { subject: subject.clone(), issuer: issuer.clone(), role };
    let builder =
        CertificateBuilder::new(profile, SerialNumber::new(&serial).expect("serial"), validity, spki.clone())
            .expect("builder");
    match signer {
        Signer::P256(key) => builder.build::<_, p256::ecdsa::DerSignature>(*key).expect("signs"),
    }
}

fn years(count: u64) -> Validity {
    let now = SystemTime::now();
    let start = now - Duration::from_secs(3600);
    let end = now + Duration::from_secs(count * 365 * 86_400);
    Validity::new(Time::try_from(start).expect("time"), Time::try_from(end).expect("time"))
}

fn name(text: &str) -> Name {
    Name::from_str(text).expect("a distinguished name")
}

fn p256_key() -> p256::ecdsa::SigningKey {
    loop {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).expect("random");
        if let Ok(key) = p256::ecdsa::SigningKey::from_slice(&bytes) {
            return key;
        }
    }
}

fn p256_from(pkcs8: &[u8]) -> p256::ecdsa::SigningKey {
    use pkcs8::DecodePrivateKey;
    p256::ecdsa::SigningKey::from_pkcs8_der(pkcs8).expect("PKCS#8")
}

fn pkcs8_p256(key: &p256::ecdsa::SigningKey) -> Vec<u8> {
    key.to_pkcs8_der().expect("encodes").as_bytes().to_vec()
}

fn spki_p256(key: &p256::ecdsa::SigningKey) -> SubjectPublicKeyInfoOwned {
    SubjectPublicKeyInfoOwned::from_der(key.verifying_key().to_public_key_der().expect("encodes").as_bytes())
        .expect("parses")
}

fn rsa_key() -> rsa::PrivateKey {
    use rsa::Generate;
    rsa::PrivateKey::generate()
}

/// ESS signing-certificate-v2 for `certificate` (RFC 5035), as time-stamping
/// authorities put in their tokens (RFC 5816).
fn ess_v2(certificate: &Certificate) -> Attribute {
    let hash =
        OctetString::new(Sha256::digest(certificate.to_der().expect("encodes")).to_vec()).expect("octets");
    let mut inner = hash.to_der().expect("encodes");
    for _ in 0..3 {
        inner = sequence(&inner);
    }
    let mut values = SetOfVec::new();
    values.insert(AttributeValue::from_der(&inner).expect("value")).expect("set");
    Attribute { oid: ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.47"), values }
}

fn sequence(body: &[u8]) -> Vec<u8> {
    Any::new(Tag::Sequence, body).expect("sequence").to_der().expect("encodes")
}
