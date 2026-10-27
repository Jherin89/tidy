#!/usr/bin/env bash
# Render Formula/tidy.rb into a local clone of Jherin89/homebrew-tap.
#
# Uses your existing `gh` login when it downloads release checksums.
# This script has no tokens of its own. It does not commit or push unless
# you pass --commit, and it does not push unless you also pass --push.
#
# Usage:
#   scripts/bump-homebrew.sh /path/to/homebrew-tap
#   scripts/bump-homebrew.sh --version 0.3.0 /path/to/homebrew-tap
#   scripts/bump-homebrew.sh --sha-from-dir /tmp/shas /tmp/tap
#   scripts/bump-homebrew.sh --dry-run /tmp/tap
#   scripts/bump-homebrew.sh --commit /path/to/homebrew-tap
#   scripts/bump-homebrew.sh --commit --push /path/to/homebrew-tap
#
# Checksums are the first field of tidy-<version>-<target>.tar.gz.sha256
# (the `shasum -a 256` / `sha256sum` layout: "hash  filename").
# --dry-run skips `gh` and git. Without --sha-from-dir it fills in a
# placeholder hash so the formula can be syntax-checked.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION=""
SHA_DIR=""
DRY_RUN=0
DO_COMMIT=0
DO_PUSH=0
DEST=""

usage() {
  sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
  case "$1" in
    --version)
      VERSION="${2:?--version needs a value}"
      shift 2
      ;;
    --sha-from-dir)
      SHA_DIR="${2:?--sha-from-dir needs a directory}"
      shift 2
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --commit)
      DO_COMMIT=1
      shift
      ;;
    --push)
      DO_PUSH=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      break
      ;;
    -*)
      echo "unknown flag: $1" >&2
      exit 2
      ;;
    *)
      if [ -n "$DEST" ]; then
        echo "unexpected extra argument: $1" >&2
        exit 2
      fi
      DEST="$1"
      shift
      ;;
  esac
done

if [ -z "$DEST" ]; then
  usage >&2
  exit 2
fi

if [ -z "$VERSION" ]; then
  VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -n 1)"
fi
if [ -z "$VERSION" ]; then
  echo "could not read version from $ROOT/Cargo.toml" >&2
  exit 1
fi

if [ "$DO_PUSH" = 1 ] && [ "$DO_COMMIT" = 0 ]; then
  echo "--push requires --commit" >&2
  exit 2
fi
if [ "$DRY_RUN" = 1 ] && [ "$DO_COMMIT" = 1 ]; then
  echo "--dry-run does not commit; drop --commit" >&2
  exit 2
fi

TARGETS=(
  universal-apple-darwin
  x86_64-unknown-linux-musl
  aarch64-unknown-linux-musl
)

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

placeholder="0000000000000000000000000000000000000000000000000000000000000000"

if [ -n "$SHA_DIR" ]; then
  find "$SHA_DIR" -name '*.sha256' -exec cp {} "$workdir/" \;
elif [ "$DRY_RUN" = 1 ]; then
  echo "warning: --dry-run without checksums; using a placeholder sha256" >&2
  for target in "${TARGETS[@]}"; do
    name="tidy-${VERSION}-${target}.tar.gz"
    printf '%s  %s\n' "$placeholder" "$name" > "$workdir/${name}.sha256"
  done
else
  gh release download "v${VERSION}" --repo Jherin89/tidy --pattern '*.sha256' --dir "$workdir"
fi

sha_of() {
  local target="$1"
  local name="tidy-${VERSION}-${target}.tar.gz"
  local path="$workdir/${name}.sha256"
  if [ ! -f "$path" ]; then
    path="$(find "$workdir" -name "${name}.sha256" -print -quit)"
  fi
  if [ -z "$path" ] || [ ! -f "$path" ]; then
    echo "missing checksum for ${name}" >&2
    exit 1
  fi
  awk 'NF { print $1; exit }' "$path"
}

SHA_MACOS="$(sha_of universal-apple-darwin)"
SHA_LINUX_INTEL="$(sha_of x86_64-unknown-linux-musl)"
SHA_LINUX_ARM="$(sha_of aarch64-unknown-linux-musl)"

for hash in "$SHA_MACOS" "$SHA_LINUX_INTEL" "$SHA_LINUX_ARM"; do
  if ! printf '%s' "$hash" | grep -Eq '^[0-9a-fA-F]{64}$'; then
    echo "checksum is not 64 hex digits: $hash" >&2
    exit 1
  fi
done

mkdir -p "$DEST/Formula"
sed \
  -e "s/__VERSION__/${VERSION}/g" \
  -e "s/__SHA_MACOS__/${SHA_MACOS}/g" \
  -e "s/__SHA_LINUX_INTEL__/${SHA_LINUX_INTEL}/g" \
  -e "s/__SHA_LINUX_ARM__/${SHA_LINUX_ARM}/g" \
  "$ROOT/scripts/homebrew/tidy.rb.tmpl" > "$DEST/Formula/tidy.rb"

echo "wrote $DEST/Formula/tidy.rb (tidy ${VERSION})"

if command -v ruby >/dev/null 2>&1; then
  ruby -c "$DEST/Formula/tidy.rb"
else
  echo "ruby not found; skipped ruby -c" >&2
fi

if [ "$DO_COMMIT" = 1 ]; then
  git -C "$DEST" add Formula/tidy.rb
  git -C "$DEST" commit -m "tidy ${VERSION}"
  if [ "$DO_PUSH" = 1 ]; then
    git -C "$DEST" push
  else
    echo "committed in $DEST; not pushed" >&2
  fi
fi
