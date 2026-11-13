#!/usr/bin/env bash
# Sign and notarize the universal macOS tidy binary.
#
# Called from the release workflow after lipo and the smoke test, before
# packaging. Secrets arrive only through the environment. This script never
# prints secret values, certificate bytes, or passwords.
#
# Outcomes:
#   * No signing secrets set: print a skip message and exit 0. The unsigned
#     binary is packaged as before.
#   * Some signing secrets set: exit 1 and name the missing secret names.
#   * All signing secrets set, but notarization auth is not a complete Apple
#     ID trio or App Store Connect API-key trio: exit 1 and name what is
#     missing. A signed-but-unnotarized binary is not packaged. This check
#     runs before the certificate is written to disk.
#   * Signing secrets and one notary auth set are complete: import the .p12
#     into a temporary keychain, codesign with the hardened runtime, submit
#     the zip to notarytool --wait, and leave the signed binary in place.
#     When both notary sets are complete, the API key is used so the Apple
#     ID password is not placed on the notarytool command line.
#
# A bare Mach-O inside a .tar.gz cannot be stapled (xcrun stapler wants an
# app, pkg, or dmg). Gatekeeper can still check the notarization ticket
# online. This script does not call stapler.
#
# No entitlements file: tidy is a CLI. It does not need JIT, a debugger, or
# unsigned executable memory, so the hardened runtime is enabled with no
# exceptions.
#
# Usage: macos-sign-and-notarize.sh <path-to-tidy>
set +x
set -euo pipefail

binary="${1:?usage: macos-sign-and-notarize.sh <path-to-tidy>}"

# Hardcoded names only. Values are read with printenv and never echoed.
SIGNING_NAMES=(
  APPLE_CERTIFICATE_P12_BASE64
  APPLE_CERTIFICATE_PASSWORD
  APPLE_SIGNING_IDENTITY
)
APPLE_ID_NAMES=(
  APPLE_ID
  APPLE_APP_PASSWORD
  APPLE_TEAM_ID
)
API_KEY_NAMES=(
  APPLE_API_ISSUER_ID
  APPLE_API_KEY_ID
  APPLE_API_KEY_P8
)

is_set() {
  local name="$1"
  local value
  value="$(printenv "$name" 2>/dev/null || true)"
  [ -n "$value" ]
}

missing_names() {
  local name
  local missing=""
  for name in "$@"; do
    if ! is_set "$name"; then
      missing="${missing}${missing:+ }${name}"
    fi
  done
  printf '%s' "$missing"
}

count_set() {
  local name
  local count=0
  for name in "$@"; do
    if is_set "$name"; then
      count=$((count + 1))
    fi
  done
  printf '%s' "$count"
}

signing_set="$(count_set "${SIGNING_NAMES[@]}")"
if [ "$signing_set" -eq 0 ]; then
  echo "signing secrets not configured; shipping an unsigned macOS binary"
  exit 0
fi

if [ "$signing_set" -ne "${#SIGNING_NAMES[@]}" ]; then
  echo "signing secrets are incomplete; refusing to ship a half-signed binary" >&2
  echo "missing: $(missing_names "${SIGNING_NAMES[@]}")" >&2
  exit 1
fi

apple_set="$(count_set "${APPLE_ID_NAMES[@]}")"
api_set="$(count_set "${API_KEY_NAMES[@]}")"
notary_mode=""
if [ "$api_set" -eq "${#API_KEY_NAMES[@]}" ]; then
  notary_mode="api"
elif [ "$apple_set" -eq "${#APPLE_ID_NAMES[@]}" ]; then
  notary_mode="appleid"
else
  echo "signing secrets are set, but notarization auth is incomplete" >&2
  echo "a signed binary that is not notarized is worse for Gatekeeper, so this job will not ship one" >&2
  if [ "$apple_set" -gt 0 ] && [ "$api_set" -eq 0 ]; then
    echo "Apple ID auth is incomplete. Missing: $(missing_names "${APPLE_ID_NAMES[@]}")" >&2
    echo "Alternatively set the App Store Connect API key secrets: ${API_KEY_NAMES[*]}" >&2
  elif [ "$api_set" -gt 0 ] && [ "$apple_set" -eq 0 ]; then
    echo "App Store Connect API key auth is incomplete. Missing: $(missing_names "${API_KEY_NAMES[@]}")" >&2
    echo "Alternatively set Apple ID auth: ${APPLE_ID_NAMES[*]}" >&2
  elif [ "$apple_set" -eq 0 ] && [ "$api_set" -eq 0 ]; then
    echo "No notarization auth is configured. Set either:" >&2
    echo "  Apple ID: ${APPLE_ID_NAMES[*]}" >&2
    echo "  or App Store Connect API key: ${API_KEY_NAMES[*]}" >&2
  else
    echo "Apple ID auth is incomplete. Missing: $(missing_names "${APPLE_ID_NAMES[@]}")" >&2
    echo "App Store Connect API key auth is incomplete. Missing: $(missing_names "${API_KEY_NAMES[@]}")" >&2
    echo "Provide one complete set." >&2
  fi
  exit 1
fi

if [ ! -f "$binary" ]; then
  echo "binary not found: $binary" >&2
  exit 1
fi

WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/tidy-sign.XXXXXX")"
KEYCHAIN="$WORKDIR/tidy.keychain-db"
P12_PATH="$WORKDIR/developer-id.p12"
P8_PATH="$WORKDIR/AuthKey.p8"
ZIP_PATH="$WORKDIR/tidy.zip"
KEYCHAIN_PASSWORD="$(openssl rand -base64 24)"
CHANGED_KEYCHAINS=0
ORIGINAL_KEYCHAINS=()

cleanup() {
  local status=$?
  set +e
  if [ "$CHANGED_KEYCHAINS" -eq 1 ]; then
    if [ "${#ORIGINAL_KEYCHAINS[@]}" -gt 0 ]; then
      security list-keychains -d user -s "${ORIGINAL_KEYCHAINS[@]}" >/dev/null 2>&1
    fi
  fi
  if [ -n "${KEYCHAIN:-}" ]; then
    security delete-keychain "$KEYCHAIN" >/dev/null 2>&1
  fi
  rm -rf "$WORKDIR"
  exit "$status"
}
trap cleanup EXIT

chmod 700 "$WORKDIR"

printf '%s' "$APPLE_CERTIFICATE_P12_BASE64" | base64 -D >"$P12_PATH"
chmod 600 "$P12_PATH"

while IFS= read -r line; do
  line="${line#\"}"
  line="${line%\"}"
  line="${line#"${line%%[![:space:]]*}"}"
  line="${line%"${line##*[![:space:]]}"}"
  if [ -n "$line" ]; then
    ORIGINAL_KEYCHAINS+=("$line")
  fi
done < <(security list-keychains -d user)

security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
security set-keychain-settings -lut 21600 "$KEYCHAIN"
security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
security import "$P12_PATH" -k "$KEYCHAIN" -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign -T /usr/bin/security
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN" >/dev/null
if [ "${#ORIGINAL_KEYCHAINS[@]}" -gt 0 ]; then
  security list-keychains -d user -s "$KEYCHAIN" "${ORIGINAL_KEYCHAINS[@]}"
else
  security list-keychains -d user -s "$KEYCHAIN"
fi
CHANGED_KEYCHAINS=1

# Hardened runtime, stable identifier, no entitlements (see header).
codesign --force --options runtime --timestamp \
  --identifier "com.github.jherin89.tidy" \
  --sign "$APPLE_SIGNING_IDENTITY" \
  "$binary"
codesign -dv --verbose=2 "$binary"

ditto -c -k --keepParent "$binary" "$ZIP_PATH"

if [ "$notary_mode" = "api" ]; then
  printf '%s' "$APPLE_API_KEY_P8" >"$P8_PATH"
  chmod 600 "$P8_PATH"
  xcrun notarytool submit "$ZIP_PATH" \
    --key "$P8_PATH" \
    --key-id "$APPLE_API_KEY_ID" \
    --issuer "$APPLE_API_ISSUER_ID" \
    --wait
else
  xcrun notarytool submit "$ZIP_PATH" \
    --apple-id "$APPLE_ID" \
    --password "$APPLE_APP_PASSWORD" \
    --team-id "$APPLE_TEAM_ID" \
    --wait
fi

echo "signed and notarized ${binary}"
