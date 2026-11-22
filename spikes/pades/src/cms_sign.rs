//! CMS signing (RFC 5652) for PAdES baseline B-B and B-T (ETSI EN 319 142-1,
//! 319 122-1): signed attributes (content type, message digest, signing time,
//! ESS signing-certificate-v2), the certificate chain, an optional RFC 3161
//! time-stamp token as an unsigned attribute, and a verifier for all of it.

use cms::builder::{SignedDataBuilder, SignerInfoBuilder};
use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::content_info::ContentInfo;
use cms::signed_data::{EncapsulatedContentInfo, SignedData, SignerIdentifier, SignerInfo};
use const_oid::ObjectIdentifier;
use const_oid::db::rfc5911::{ID_DATA, ID_SIGNED_DATA};
use const_oid::db::rfc5912::ID_SHA_256;
use der::asn1::{OctetString, SetOfVec};
use der::{Any, Decode, Encode};
use sha2::{Digest, Sha256};
use signature::{Keypair, Signer};
use spki::{
    AlgorithmIdentifierOwned, DynSignatureAlgorithmIdentifier, EncodePublicKey, SignatureBitStringEncoding,
};
use x509_cert::Certificate;
use x509_cert::attr::{Attribute, AttributeValue};
use x509_tsp::{MessageImprint, TimeStampReq, TimeStampResp, TspVersion, TstInfo};

/// id-aa-signingCertificateV2 (RFC 5035).
const ID_AA_SIGNING_CERTIFICATE_V2: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.47");
/// id-aa-signatureTimeStampToken (RFC 3161 / CAdES-T).
const ID_AA_SIGNATURE_TIME_STAMP_TOKEN: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.14");
/// id-ct-TSTInfo.
const ID_CT_TST_INFO: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.1.4");

pub type Error = Box<dyn std::error::Error>;

fn sha256_alg() -> AlgorithmIdentifierOwned {
    AlgorithmIdentifierOwned { oid: ID_SHA_256, parameters: None }
}

/// SigningCertificateV2 ::= SEQUENCE { certs SEQUENCE OF ESSCertIDv2 }, with
/// ESSCertIDv2 ::= SEQUENCE { certHash OCTET STRING } (SHA-256 is the default
/// hash, so its AlgorithmIdentifier is left out, as DER requires).
fn signing_certificate_v2(cert: &Certificate) -> Result<Attribute, Error> {
    let hash = Sha256::digest(cert.to_der()?);
    let octets = OctetString::new(hash.to_vec())?.to_der()?;
    let ess_cert_id = wrap(0x30, &octets);
    let certs = wrap(0x30, &ess_cert_id);
    let value = wrap(0x30, &certs);
    let mut values = SetOfVec::new();
    values.insert(AttributeValue::from_der(&value)?)?;
    Ok(Attribute { oid: ID_AA_SIGNING_CERTIFICATE_V2, values })
}

/// A DER header (definite length) in front of `body`.
fn wrap(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    match body.len() {
        n if n < 128 => out.push(n as u8),
        n if n < 256 => out.extend([0x81, n as u8]),
        n => out.extend([0x82, (n >> 8) as u8, n as u8]),
    }
    out.extend_from_slice(body);
    out
}

/// What the time-stamp exchange needs: the request goes to a time-stamping
/// authority (HTTP POST, `application/timestamp-query`), the reply comes back.
pub type Tsa<'a> = &'a dyn Fn(&[u8]) -> Result<Vec<u8>, Error>;

/// An RFC 3161 request for the SHA-256 of `signature` (the signature value:
/// that is what a signature time-stamp covers).
pub fn timestamp_request(signature: &[u8]) -> Result<Vec<u8>, Error> {
    let digest = Sha256::digest(signature);
    let request = TimeStampReq {
        version: TspVersion::V1,
        message_imprint: MessageImprint {
            hash_algorithm: spki::AlgorithmIdentifier { oid: ID_SHA_256, parameters: Some(Any::null()) },
            hashed_message: OctetString::new(digest.to_vec())?,
        },
        req_policy: None,
        nonce: None,
        cert_req: true,
        extensions: None,
    };
    Ok(request.to_der()?)
}

/// The time-stamp token in a TimeStampResp, or why there is none.
pub fn timestamp_token(reply: &[u8]) -> Result<ContentInfo, Error> {
    let response = TimeStampResp::from_der(reply)?;
    response.time_stamp_token.ok_or_else(|| "the time-stamping authority refused".into())
}

/// Signs `content_digest` (SHA-256 of the signed byte ranges) and returns the
/// DER of the CMS ContentInfo to put in /Contents.
pub fn sign<S, Sig>(
    signer: &S,
    certificate: &Certificate,
    chain: &[Certificate],
    content_digest: &[u8],
    tsa: Option<Tsa<'_>>,
) -> Result<Vec<u8>, Error>
where
    S: Keypair + DynSignatureAlgorithmIdentifier + Signer<Sig>,
    S::VerifyingKey: EncodePublicKey,
    Sig: SignatureBitStringEncoding,
{
    let encapsulated = EncapsulatedContentInfo { econtent_type: ID_DATA, econtent: None };
    let sid = SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
        issuer: certificate.tbs_certificate().issuer().clone(),
        serial_number: certificate.tbs_certificate().serial_number().clone(),
    });
    let mut info = SignerInfoBuilder::new(sid, sha256_alg(), &encapsulated, Some(content_digest))?;
    info.add_signed_attribute(cms::builder::create_signing_time_attribute()?)?;
    info.add_signed_attribute(signing_certificate_v2(certificate)?)?;

    let mut builder = SignedDataBuilder::new(&encapsulated);
    builder.add_digest_algorithm(sha256_alg())?;
    builder.add_certificate(CertificateChoices::Certificate(certificate.clone()))?;
    for extra in chain {
        builder.add_certificate(CertificateChoices::Certificate(extra.clone()))?;
    }
    let mut failure: Option<Error> = None;
    builder.add_signer_info_cb::<S, Sig, _>(info, signer, |signer_info| {
        if let Some(tsa) = tsa {
            // The token covers the signature value, so it is added after signing.
            let attempt = (|| -> Result<Attribute, Error> {
                let request = timestamp_request(signer_info.signature.as_bytes())?;
                let token = timestamp_token(&tsa(&request)?)?;
                let mut values = SetOfVec::new();
                values.insert(AttributeValue::from_der(&token.to_der()?)?)?;
                Ok(Attribute { oid: ID_AA_SIGNATURE_TIME_STAMP_TOKEN, values })
            })();
            match attempt {
                Ok(attribute) => {
                    let mut unsigned = SetOfVec::new();
                    unsigned
                        .insert(attribute)
                        .map_err(|_| cms::builder::Error::Asn1(der::ErrorKind::Failed.into()))?;
                    signer_info.unsigned_attrs = Some(unsigned);
                }
                Err(err) => failure = Some(err),
            }
        }
        Ok(())
    })?;
    if let Some(err) = failure {
        return Err(err);
    }
    Ok(builder.build()?.to_der()?)
}

// ------------------------------------------------------------ verification

/// What verifying a signature found.
#[derive(Debug, Default)]
#[allow(dead_code)]
pub struct Report {
    pub signer: String,
    pub digest_matches: bool,
    pub signature_valid: bool,
    pub signing_certificate_bound: bool,
    pub chain_valid: bool,
    pub signing_time: Option<String>,
    pub timestamp: Option<TimestampReport>,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct TimestampReport {
    pub gen_time: String,
    pub imprint_matches: bool,
    pub signature_valid: bool,
    pub chain_valid: bool,
}

/// Verifies a CMS signature over `content_digest`: the digest attribute, the
/// signature over the signed attributes, the ESS binding of the signer
/// certificate, the chain up to `anchors`, and the time-stamp if present.
pub fn verify(der: &[u8], content_digest: &[u8], anchors: &[Certificate]) -> Result<Report, Error> {
    let content = ContentInfo::from_der(der)?;
    if content.content_type != ID_SIGNED_DATA {
        return Err("not SignedData".into());
    }
    let data: SignedData = content.content.decode_as()?;
    let certificates: Vec<Certificate> = data
        .certificates
        .iter()
        .flat_map(|set| set.0.iter())
        .filter_map(|choice| match choice {
            CertificateChoices::Certificate(c) => Some(c.clone()),
            _ => None,
        })
        .collect();
    let info = data.signer_infos.0.iter().next().ok_or("no signer")?;
    let mut report = check_signer(info, &certificates, anchors, Some(content_digest))?;

    if let Some(unsigned) = &info.unsigned_attrs {
        for attribute in unsigned.iter().filter(|a| a.oid == ID_AA_SIGNATURE_TIME_STAMP_TOKEN) {
            let token: ContentInfo = attribute.values.iter().next().ok_or("empty")?.decode_as()?;
            report.timestamp = Some(check_timestamp(&token, info.signature.as_bytes(), anchors)?);
        }
    }
    Ok(report)
}

fn check_timestamp(
    token: &ContentInfo,
    signature: &[u8],
    anchors: &[Certificate],
) -> Result<TimestampReport, Error> {
    let data: SignedData = token.content.decode_as()?;
    if data.encap_content_info.econtent_type != ID_CT_TST_INFO {
        return Err("not a TSTInfo".into());
    }
    let econtent = data.encap_content_info.econtent.as_ref().ok_or("no TSTInfo")?;
    let tst_bytes = econtent.decode_as::<OctetString>()?;
    let tst = TstInfo::from_der(tst_bytes.as_bytes())?;
    let certificates: Vec<Certificate> = data
        .certificates
        .iter()
        .flat_map(|set| set.0.iter())
        .filter_map(|choice| match choice {
            CertificateChoices::Certificate(c) => Some(c.clone()),
            _ => None,
        })
        .collect();
    let info = data.signer_infos.0.iter().next().ok_or("no TSA signer")?;
    let tsa_digest = Sha256::digest(tst_bytes.as_bytes());
    let checked = check_signer(info, &certificates, anchors, Some(&tsa_digest))?;
    Ok(TimestampReport {
        gen_time: format!("{:?}", tst.gen_time),
        imprint_matches: tst.message_imprint.hashed_message.as_bytes()
            == Sha256::digest(signature).as_slice(),
        signature_valid: checked.signature_valid && checked.digest_matches,
        chain_valid: checked.chain_valid,
    })
}

fn check_signer(
    info: &SignerInfo,
    certificates: &[Certificate],
    anchors: &[Certificate],
    content_digest: Option<&[u8]>,
) -> Result<Report, Error> {
    let SignerIdentifier::IssuerAndSerialNumber(id) = &info.sid else {
        return Err("subjectKeyIdentifier signers: not in the spike".into());
    };
    let signer = certificates
        .iter()
        .find(|c| {
            c.tbs_certificate().issuer() == &id.issuer
                && c.tbs_certificate().serial_number() == &id.serial_number
        })
        .ok_or("the signer's certificate is not in the signature")?;
    let attributes = info.signed_attrs.as_ref().ok_or("no signed attributes")?;
    let mut report = Report { signer: signer.tbs_certificate().subject().to_string(), ..Report::default() };

    // The digest of the content is in a signed attribute.
    let digest_attribute = attributes
        .iter()
        .find(|a| a.oid == const_oid::db::rfc5911::ID_MESSAGE_DIGEST)
        .ok_or("no message-digest")?;
    let claimed = digest_attribute.values.iter().next().ok_or("empty")?.decode_as::<OctetString>()?;
    report.digest_matches = content_digest.is_some_and(|d| claimed.as_bytes() == d);

    // The signature is over the DER of the attributes as a SET (tag 0x31).
    let signed = attributes.to_der()?;
    report.signature_valid =
        verify_signature(signer, &info.signature_algorithm, &signed, info.signature.as_bytes());

    // ESS: the signer's certificate is bound by hash (stops certificate substitution).
    if let Some(attribute) = attributes.iter().find(|a| a.oid == ID_AA_SIGNING_CERTIFICATE_V2) {
        let value = attribute.values.iter().next().ok_or("empty")?.to_der()?;
        let hash = Sha256::digest(signer.to_der()?);
        report.signing_certificate_bound = value.windows(32).any(|w| w == hash.as_slice());
    }
    report.signing_time = attributes
        .iter()
        .find(|a| a.oid == const_oid::db::rfc5911::ID_SIGNING_TIME)
        .and_then(|a| a.values.iter().next())
        .map(|v| format!("{v:?}"));
    report.chain_valid = chain_valid(signer, certificates, anchors, 0);
    Ok(report)
}

fn verify_signature(
    signer: &Certificate,
    algorithm: &AlgorithmIdentifierOwned,
    message: &[u8],
    signature: &[u8],
) -> bool {
    use x509_verify::{Message, Signature, VerifyingKey};
    let Ok(key) = VerifyingKey::try_from(signer) else { return false };
    key.verify(x509_verify::VerifyInfo::new(Message::new(message), Signature::new(algorithm, signature)))
        .is_ok()
}

/// A leaf-to-anchor path: each certificate signed by the next, all within
/// their validity, the issuers CA certificates. (No revocation: see the ADR.)
fn chain_valid(cert: &Certificate, pool: &[Certificate], anchors: &[Certificate], depth: usize) -> bool {
    use x509_verify::VerifyingKey;
    if depth > 8 {
        return false;
    }
    let now = std::time::SystemTime::now();
    let validity = cert.tbs_certificate().validity();
    if validity.not_before.to_system_time() > now || validity.not_after.to_system_time() < now {
        return false;
    }
    let issued_by = |issuer: &Certificate| -> bool {
        issuer.tbs_certificate().subject() == cert.tbs_certificate().issuer()
            && VerifyingKey::try_from(issuer).ok().is_some_and(|key| {
                x509_verify::VerifyInfo::try_from(cert).ok().is_some_and(|info| key.verify(info).is_ok())
            })
    };
    if anchors.iter().any(issued_by) {
        return true;
    }
    pool.iter()
        .filter(|c| *c != cert)
        .any(|issuer| issued_by(issuer) && chain_valid(issuer, pool, anchors, depth + 1))
}
