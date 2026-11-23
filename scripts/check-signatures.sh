#!/usr/bin/env bash
# Checks the app's certificate signatures with validators that are not ours
# (ADR 0048). Signs fixtures with a throwaway PKI (examples/sign_fixtures.rs),
# then for every signed PDF:
#   - Poppler `pdfsig` (NSS), trusting only the test root: every signature
#     "Valid" and its certificate "Trusted";
#   - OpenSSL `cms -verify -cades` on each signature and the bytes its
#     /ByteRange covers (extracted here, independently of our reader), and
#     `ts -verify` on a signature's time-stamp token;
#   - `qpdf --check`.
# Needs pdfsig and certutil (Homebrew: poppler, nss), openssl 3+, qpdf, python3.
#
#   scripts/check-signatures.sh <out-dir>
set -euo pipefail
out=${1:?usage: scripts/check-signatures.sh <out-dir>}
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
for tool in pdfsig certutil openssl qpdf python3; do
  command -v "$tool" >/dev/null || { echo "missing: $tool" >&2; exit 2; }
done

(cd "$root" && cargo run -q -p mirsam-core --example sign_fixtures -- "$out")

nss="$out/nss"
rm -rf "$nss" && mkdir -p "$nss"
certutil -N -d "sql:$nss" --empty-password
certutil -A -d "sql:$nss" -n "Mirsam Test Root CA" -t "CT,C,C" -i "$out/ca.pem"

failures=0
fail() { echo "FAIL: $*" >&2; failures=$((failures + 1)); }

for pdf in "$out"/*.pdf; do
  name="$(basename "$pdf")"
  password=""
  if [[ -f "$out/${name%.pdf}.password" ]]; then
    password="$(cat "$out/${name%.pdf}.password")"
  fi

  # Poppler / NSS.
  if [[ -n "$password" ]]; then
    report="$(pdfsig -nssdir "sql:$nss" -no-ocsp -upw "$password" "$pdf" 2>&1 || true)"
  else
    report="$(pdfsig -nssdir "sql:$nss" -no-ocsp "$pdf" 2>&1 || true)"
  fi
  echo "== $name (pdfsig)"
  echo "$report" | grep -E "^Signature #|Common Name:|Signature Validation:|Certificate Validation:|Signature Type:|document signed" || true
  expected=$(grep -c . "$pdf.expect" || true)
  valid=$(echo "$report" | grep -c "Signature Validation: Signature is Valid." || true)
  trusted=$(echo "$report" | grep -c "Certificate Validation: Certificate is Trusted." || true)
  [[ "$valid" == "$expected" ]] || fail "$name: pdfsig found $valid valid signatures, expected $expected"
  [[ "$trusted" == "$expected" ]] || fail "$name: pdfsig trusted $trusted certificates, expected $expected"
  while IFS= read -r signer; do
    [[ -z "$signer" ]] && continue
    echo "$report" | grep -qF "Common Name: $signer" || fail "$name: pdfsig does not name the signer $signer"
  done < "$pdf.expect"

  # qpdf (exit 3: warnings only).
  status=0
  qpdf "--password=$password" --check "$pdf" >/dev/null 2>&1 || status=$?
  [[ $status == 0 || $status == 3 ]] || fail "$name: qpdf --check (exit $status)"

  # OpenSSL, on each signature as /ByteRange delimits it.
  dir="$out/${name%.pdf}.parts"
  rm -rf "$dir" && mkdir -p "$dir"
  python3 -I - "$pdf" "$dir" <<'PY'
import re, sys
pdf, out = sys.argv[1], sys.argv[2]
data = open(pdf, "rb").read()
for i, m in enumerate(re.finditer(rb"/ByteRange\s*\[\s*(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s*\]", data)):
    a, b, c, d = map(int, m.groups())
    signed = data[a:a + b] + data[c:c + d]
    hexed = data[a + b + 1:c - 1]
    der = bytes.fromhex(hexed.decode())
    # The DER structure's own length (the placeholder is zero padded).
    n = der[1]
    length = 2 + n if n < 0x80 else 2 + (n & 0x7F) + int.from_bytes(der[2:2 + (n & 0x7F)], "big")
    open(f"{out}/{i}.der", "wb").write(der[:length])
    open(f"{out}/{i}.signed", "wb").write(signed)
PY
  for der in "$dir"/*.der; do
    i="$(basename "$der" .der)"
    if openssl cms -verify -cades -inform DER -in "$der" -content "$dir/$i.signed" -binary \
        -CAfile "$out/ca.pem" -purpose any -out /dev/null 2>"$dir/$i.cms.log"; then
      echo "   signature $i: OpenSSL CMS and CAdES verification successful"
    else
      fail "$name signature $i: openssl cms -verify -cades: $(cat "$dir/$i.cms.log")"
    fi
    # A time-stamp token, when the signature has one: over the signature value.
    if openssl cms -cmsout -inform DER -in "$der" -print -noout 2>/dev/null | grep -q "id-smime-aa-timeStampToken"; then
      python3 -I - "$der" "$dir/$i" <<'PY'
import subprocess, sys
der, prefix = sys.argv[1], sys.argv[2]
# Let OpenSSL parse the structure; pull the token and the signature value out.
text = subprocess.run(["openssl", "asn1parse", "-inform", "DER", "-in", der], capture_output=True, text=True).stdout
lines = text.splitlines()
data = open(der, "rb").read()
def element(line):
    off = int(line.split(":")[0]); hl = int(line.split("hl=")[1].split()[0]); l = int(line.split(" l=")[1].split()[0])
    return off, hl, l
token = None; value = None
for j, line in enumerate(lines):
    if "id-smime-aa-timeStampToken" in line:
        # The SET after the OID holds the token (a SEQUENCE).
        off, hl, l = element(lines[j + 2])
        token = data[off:off + hl + l]
for j, line in enumerate(lines):
    if "OCTET STRING" in line and "[HEX DUMP]" in line:
        off, hl, l = element(line)
        value = data[off + hl:off + hl + l]   # the last one before the unsigned attributes
        last = value
    if "id-smime-aa-timeStampToken" in line:
        break
open(prefix + ".token", "wb").write(token)
open(prefix + ".value", "wb").write(last)
PY
      if openssl ts -verify -in "$dir/$i.token" -token_in -data "$dir/$i.value" -CAfile "$out/ca.pem" \
          >"$dir/$i.ts.log" 2>&1; then
        echo "   signature $i: OpenSSL time-stamp verification OK"
      else
        fail "$name signature $i: openssl ts -verify: $(cat "$dir/$i.ts.log")"
      fi
    fi
  done
done

if [[ $failures -gt 0 ]]; then
  echo "$failures check(s) failed" >&2
  exit 1
fi
echo "All signatures check out with pdfsig, OpenSSL and qpdf."
