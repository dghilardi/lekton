#!/usr/bin/env bash
#
# Package the prebuilt binaries of one target into release archives:
#   lekton-<version>-<target>.(tar.gz|zip)       server binary + site assets
#   lekton-sync-<version>-<target>.(tar.gz|zip)  sync CLI
#
# Windows targets are zipped, every other target is a gzipped tarball.
#
# Usage: scripts/package-release.sh VERSION TARGET BIN_DIR SITE_DIR OUT_DIR
#   BIN_DIR   directory containing lekton[.exe] and lekton-sync[.exe]
#   SITE_DIR  Leptos site root produced by `cargo leptos build` (target/site)
#
set -euo pipefail

VERSION="${1:?usage: $0 VERSION TARGET BIN_DIR SITE_DIR OUT_DIR}"
TARGET="${2:?missing TARGET}"
BIN_DIR="${3:?missing BIN_DIR}"
SITE_DIR="${4:?missing SITE_DIR}"
OUT_DIR="${5:?missing OUT_DIR}"

EXE=""
case "${TARGET}" in
  *windows*) EXE=".exe" ;;
esac

mkdir -p "${OUT_DIR}"
OUT_DIR="$(cd "${OUT_DIR}" && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

archive() {
  local name="$1"
  if [ -n "${EXE}" ]; then
    (cd "${WORK}" && zip -qr "${OUT_DIR}/${name}.zip" "${name}")
  else
    tar -C "${WORK}" -czf "${OUT_DIR}/${name}.tar.gz" "${name}"
  fi
}

SERVER="lekton-${VERSION}-${TARGET}"
mkdir -p "${WORK}/${SERVER}"
install -m 0755 "${BIN_DIR}/lekton${EXE}" "${WORK}/${SERVER}/lekton${EXE}"
cp -r "${SITE_DIR}" "${WORK}/${SERVER}/site"
cp LICENSE README.md "${WORK}/${SERVER}/"
cp .env.example "${WORK}/${SERVER}/lekton.env.example"
archive "${SERVER}"

SYNC="lekton-sync-${VERSION}-${TARGET}"
mkdir -p "${WORK}/${SYNC}"
install -m 0755 "${BIN_DIR}/lekton-sync${EXE}" "${WORK}/${SYNC}/lekton-sync${EXE}"
cp LICENSE "${WORK}/${SYNC}/"
cp cli/README.md "${WORK}/${SYNC}/README.md"
archive "${SYNC}"

echo "Packaged ${SERVER} and ${SYNC} into ${OUT_DIR}"
