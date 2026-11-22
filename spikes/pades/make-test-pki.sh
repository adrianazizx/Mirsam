#!/usr/bin/env bash
# Makes a throwaway PKI for the PAdES spike (ADR 0044) in $1: a CA, signing
# certificates (P-256, RSA-2048, and a P-256 one whose name is Arabic), and a
# time-stamping authority for `openssl ts`. Keys exist only in that folder.
set -euo pipefail
out=${1:?usage: make-test-pki.sh <out-dir>}
mkdir -p "$out"
cd "$out"
days=3650
ec_key() { openssl ecparam -name prime256v1 -genkey -noout | openssl pkcs8 -topk8 -nocrypt -out "$1"; }

ec_key ca.key
openssl req -x509 -new -key ca.key -sha256 -days $days -utf8 \
  -subj "/O=Mirsam spike/CN=Mirsam Spike Test CA" \
  -addext "basicConstraints=critical,CA:TRUE" \
  -addext "keyUsage=critical,keyCertSign,cRLSign" \
  -addext "subjectKeyIdentifier=hash" -out ca.pem

leaf() { # name key-spec subject
  local name=$1 subject=$3
  if [ "$2" = rsa ]; then openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$name.key" 2>/dev/null
  else ec_key "$name.key"; fi
  openssl req -new -key "$name.key" -utf8 -subj "$subject" -out "$name.csr"
  cat > "$name.ext" <<EXT
basicConstraints=critical,CA:FALSE
keyUsage=critical,digitalSignature,nonRepudiation
extendedKeyUsage=emailProtection,1.2.840.113583.1.1.5
subjectKeyIdentifier=hash
authorityKeyIdentifier=keyid
EXT
  openssl x509 -req -in "$name.csr" -CA ca.pem -CAkey ca.key -CAcreateserial -sha256 \
    -days $days -extfile "$name.ext" -out "$name.pem" 2>/dev/null
}
leaf signer-ec ec "/O=Mirsam spike/CN=Test Signer (P-256)"
leaf signer-rsa rsa "/O=Mirsam spike/CN=Test Signer (RSA-2048)"
leaf signer-ar ec "/O=مرسام/CN=الموقّع التجريبي"

ec_key tsa.key
openssl req -new -key tsa.key -utf8 -subj "/O=Mirsam spike/CN=Spike Time Stamping" -out tsa.csr
printf 'basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=critical,timeStamping\nsubjectKeyIdentifier=hash\nauthorityKeyIdentifier=keyid\n' > tsa.ext
openssl x509 -req -in tsa.csr -CA ca.pem -CAkey ca.key -CAcreateserial -sha256 -days $days \
  -extfile tsa.ext -out tsa.pem 2>/dev/null
cp ca.pem tsa-chain.pem
echo 1000 > tsa.serial
cat > tsa.cnf <<CNF
[ tsa ]
default_tsa = tsa_config
[ tsa_config ]
dir = .
serial = ./tsa.serial
crypto_device = builtin
signer_cert = ./tsa.pem
certs = ./tsa-chain.pem
signer_key = ./tsa.key
signer_digest = sha256
default_policy = 1.2.3.4.1
digests = sha256, sha384
accuracy = secs:1
ordering = yes
tsa_name = yes
ess_cert_id_chain = no
ess_cert_id_alg = sha256
CNF
echo "test PKI written to $out"
