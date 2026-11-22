# PAdES signing spike (ADR 0044)

Throwaway prototype: signs a PDF as an incremental update with a CMS signature built from RustCrypto
crates, optionally time-stamped, and verifies it again in Rust. Not part of the default build.

```bash
spikes/pades/make-test-pki.sh /tmp/pades-pki        # a CA, signers (P-256, RSA-2048, Arabic name), a TSA
cargo run -p spike-pades -- sign   in.pdf out.pdf /tmp/pades-pki ec|rsa|ar [--tsa]
cargo run -p spike-pades -- verify out.pdf /tmp/pades-pki
cargo run --release -p spike-pades -- bench in.pdf /tmp/pades-pki
```

Other tools that check the result (what ADR 0044 recorded):

```bash
certutil -N -d sql:nss --empty-password && certutil -A -d sql:nss -n CA -t "CT,C,C" -i /tmp/pades-pki/ca.pem
pdfsig -nssdir sql:nss -no-ocsp out.pdf             # Poppler + NSS: signature and certificate
qpdf --check out.pdf                                # structure
openssl cms -verify -cades -inform DER -in sig.der -content signed.bin -binary -CAfile ca.pem -purpose smimesign
openssl ts -verify -in token.der -token_in -data sigvalue.bin -CAfile ca.pem   # the time-stamp token
```

The keys are made on the spot and never committed. `src/pdf.rs` is the PDF half (the field, `/ByteRange`,
the `/Contents` placeholder, the update); `src/cms_sign.rs` the CMS half (signing, time-stamp, verifier).
