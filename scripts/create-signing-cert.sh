#!/usr/bin/env bash
# Creates a self-signed code signing certificate "Power IDEA Local" in the login
# keychain, so every build of the app has the same signature and macOS keeps
# its privacy permissions across rebuilds. Local use only; it is not a
# Developer ID and does not make the app trusted on other Macs.
#
# macOS will ask for your login password to trust the certificate.
set -euo pipefail

NAME="Power IDEA Local"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"

if security find-certificate -c "$NAME" "$KEYCHAIN" >/dev/null 2>&1; then
  echo "Certificate \"$NAME\" already exists."
  exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

cat > "$WORK/cert.cnf" <<CNF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $NAME
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
CNF

openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
  -config "$WORK/cert.cnf" -keyout "$WORK/key.pem" -out "$WORK/cert.pem" 2>/dev/null
# Random one-time password: the .p12 only exists inside $WORK for the import.
P12_PASS="$(openssl rand -hex 16)"
openssl pkcs12 -export -inkey "$WORK/key.pem" -in "$WORK/cert.pem" \
  -name "$NAME" -out "$WORK/identity.p12" -passout "pass:$P12_PASS"

security import "$WORK/identity.p12" -k "$KEYCHAIN" -P "$P12_PASS" -T /usr/bin/codesign
security add-trusted-cert -r trustRoot -p codeSign -k "$KEYCHAIN" "$WORK/cert.pem"

echo "Created \"$NAME\". Rebuild with scripts/bundle-macos.sh."
