#!/usr/bin/env bash
# Package one release binary.
# Usage: package-archive.sh <target-name> <path-to-tidy>
# Writes dist/tidy-<version>-<target>.tar.gz and a sibling .sha256 file.
# The archive contains a top-level directory of the same name.
set -euo pipefail

target="${1:?target name}"
binary="${2:?binary path}"
version="${GITHUB_REF_NAME#v}"
if [ -z "${version}" ] || [ "${version}" = "${GITHUB_REF_NAME}" ]; then
  echo "GITHUB_REF_NAME must be a v* tag" >&2
  exit 1
fi

name="tidy-${version}-${target}"
stage="dist/${name}"
rm -rf "${stage}"
mkdir -p "${stage}"
cp "${binary}" "${stage}/tidy"
chmod +x "${stage}/tidy"
cp LICENSE README.md CHANGELOG.md "${stage}/"
tar -C dist -czf "dist/${name}.tar.gz" "${name}"

checksum="dist/${name}.tar.gz.sha256"
if command -v shasum >/dev/null 2>&1; then
  (cd dist && shasum -a 256 "${name}.tar.gz" > "${name}.tar.gz.sha256")
else
  (cd dist && sha256sum "${name}.tar.gz" > "${name}.tar.gz.sha256")
fi
# Both tools write "hash<two spaces>filename".
awk 'NF != 2 { exit 1 }' "${checksum}"
echo "wrote dist/${name}.tar.gz"
