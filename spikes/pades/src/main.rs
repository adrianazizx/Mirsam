//! PAdES signing spike (ADR 0044). Throwaway: signs a PDF as an incremental
//! update with a CMS signature built from RustCrypto crates, optionally
//! time-stamped, and verifies it again. Other tools check the result
//! (scripts in the README).
//!
//! ```text
//! spike-pades sign   <in.pdf> <out.pdf> <pki-dir> ec|rsa|ar [--tsa]
//! spike-pades verify <signed.pdf> <pki-dir>
//! spike-pades bench  <in.pdf> <pki-dir>
//! ```

mod cms_sign;
mod pdf;

use std::path::Path;
use std::process::Command;
use std::time::Instant;

use der::DecodePem;
use pkcs8::DecodePrivateKey;
use sha2::{Digest, Sha256};
use x509_cert::Certificate;

type Error = Box<dyn std::error::Error>;

fn cert(path: &Path) -> Result<Certificate, Error> {
    Ok(Certificate::from_pem(std::fs::read(path)?)?)
}

/// Stand-in for the HTTP call to a time-stamping authority: `openssl ts`
/// answers a request the way a TSA would.
fn local_tsa(pki: &Path) -> impl Fn(&[u8]) -> Result<Vec<u8>, Error> + '_ {
    move |request| {
        let (query, reply) = (pki.join("tsa.tsq"), pki.join("tsa.tsr"));
        std::fs::write(&query, request)?;
        let status = Command::new("openssl")
            .current_dir(pki)
            .args(["ts", "-reply", "-queryfile", "tsa.tsq", "-config", "tsa.cnf", "-out", "tsa.tsr"])
            .output()?;
        if !status.status.success() {
            return Err(String::from_utf8_lossy(&status.stderr).into_owned().into());
        }
        Ok(std::fs::read(reply)?)
    }
}

fn now() -> String {
    let output = Command::new("date").args(["-u", "+%Y%m%d%H%M%S"]).output().expect("date runs");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

struct Timings {
    hash: f64,
    sign: f64,
    total: f64,
    der: usize,
}

fn sign(
    input: &[u8],
    pki: &Path,
    which: &str,
    tsa: bool,
    reserved: usize,
) -> Result<(Vec<u8>, Timings), Error> {
    let started = Instant::now();
    let name = match which {
        "ar" => "الموقّع التجريبي",
        "rsa" => "Test Signer (RSA-2048)",
        _ => "Test Signer (P-256)",
    };
    let appearance = pdf::Appearance { name, reason: "Approved — موافق", location: "Muscat" };
    let mut prepared = pdf::prepare(input, reserved, &appearance, &now());
    prepared.byte_range();

    let hashing = Instant::now();
    let digest = Sha256::digest(prepared.signed_content());
    let hash = hashing.elapsed().as_secs_f64();

    let chain = vec![cert(&pki.join("ca.pem"))?];
    let leaf = cert(&pki.join(format!("signer-{}.pem", if which == "ar" { "ar" } else { which })))?;
    let key_pem = std::fs::read_to_string(pki.join(format!("signer-{which}.key")))?;
    let tsa_call = local_tsa(pki);
    let tsa_ref: Option<cms_sign::Tsa<'_>> = if tsa { Some(&tsa_call) } else { None };

    let signing = Instant::now();
    let der = if which == "rsa" {
        let key = rsa::PrivateKey::from_pkcs8_pem(&key_pem)?;
        let signer = rsa::pkcs1v15::SigningKey::<Sha256>::new(key);
        cms_sign::sign::<_, rsa::pkcs1v15::Signature>(&signer, &leaf, &chain, &digest, tsa_ref)?
    } else {
        let signer = p256::ecdsa::SigningKey::from_pkcs8_pem(&key_pem)?;
        cms_sign::sign::<_, p256::ecdsa::DerSignature>(&signer, &leaf, &chain, &digest, tsa_ref)?
    };
    let sign_time = signing.elapsed().as_secs_f64();
    prepared.insert(&der).map_err(|needed| format!("{needed} bytes do not fit in {reserved}"))?;
    let timings = Timings { hash, sign: sign_time, total: started.elapsed().as_secs_f64(), der: der.len() };
    Ok((prepared.bytes, timings))
}

fn verify(bytes: &[u8], pki: &Path) -> Result<cms_sign::Report, Error> {
    let (range, der) = pdf::extract(bytes).ok_or("no signature")?;
    let digest = Sha256::digest(pdf::covered(bytes, range));
    // DER from the placeholder keeps its zero padding: the parser stops at the structure's end.
    cms_sign::verify(&der, &digest, &[cert(&pki.join("ca.pem"))?])
}

fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("sign") => {
            let (input, output, pki, which) = (&args[1], &args[2], Path::new(&args[3]), args[4].as_str());
            let tsa = args.iter().any(|a| a == "--tsa");
            let (bytes, timings) = sign(&std::fs::read(input)?, pki, which, tsa, pdf::DEFAULT_RESERVED)?;
            std::fs::write(output, &bytes)?;
            println!(
                "signed {} bytes -> {} bytes; CMS {} bytes; hash {:.1} ms, sign {:.1} ms, total {:.1} ms",
                std::fs::metadata(input)?.len(),
                bytes.len(),
                timings.der,
                timings.hash * 1e3,
                timings.sign * 1e3,
                timings.total * 1e3
            );
        }
        Some("verify") => {
            let report = verify(&std::fs::read(&args[1])?, Path::new(&args[2]))?;
            println!("{report:#?}");
            let ok = report.digest_matches && report.signature_valid && report.chain_valid;
            std::process::exit(i32::from(!ok));
        }
        Some("bench") => {
            let input = std::fs::read(&args[1])?;
            let pki = Path::new(&args[2]);
            for (which, tsa) in [("ec", false), ("rsa", false), ("ar", false), ("ec", true)] {
                let (bytes, t) = sign(&input, pki, which, tsa, pdf::DEFAULT_RESERVED)?;
                let started = Instant::now();
                let report = verify(&bytes, pki)?;
                println!(
                    "{which:3} tsa={tsa:5} file {:>9} B  cms {:>5} B  hash {:6.1} ms  sign {:6.1} ms  total {:6.1} ms  verify {:5.1} ms  ok={}",
                    bytes.len(),
                    t.der,
                    t.hash * 1e3,
                    t.sign * 1e3,
                    t.total * 1e3,
                    started.elapsed().as_secs_f64() * 1e3,
                    report.digest_matches && report.signature_valid && report.chain_valid
                );
            }
        }
        _ => eprintln!("usage: spike-pades sign|verify|bench (see the file header)"),
    }
    Ok(())
}
