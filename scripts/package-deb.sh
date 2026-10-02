#!/usr/bin/env bash
#
# Build the Debian/Ubuntu packages of one architecture:
#   lekton_<version>_<arch>.deb       server, site assets and systemd unit
#   lekton-sync_<version>_<arch>.deb  sync CLI (static, no dependencies)
#
# The service is installed but not enabled: configure /etc/lekton/lekton.env,
# then `sudo systemctl enable --now lekton`.
#
# Usage: scripts/package-deb.sh VERSION ARCH SERVER_DIR SYNC_DIR SITE_DIR OUT_DIR
#   ARCH        Debian architecture (amd64, arm64)
#   SERVER_DIR  directory containing the glibc lekton binary
#   SYNC_DIR    directory containing the static (musl) lekton-sync binary
#   SITE_DIR    Leptos site root produced by `cargo leptos build` (target/site)
#
set -euo pipefail

VERSION="${1:?usage: $0 VERSION ARCH SERVER_DIR SYNC_DIR SITE_DIR OUT_DIR}"
ARCH="${2:?missing ARCH}"
SERVER_DIR="${3:?missing SERVER_DIR}"
SYNC_DIR="${4:?missing SYNC_DIR}"
SITE_DIR="${5:?missing SITE_DIR}"
OUT_DIR="${6:?missing OUT_DIR}"

MAINTAINER="Davide Ghilardi <ghilardi.davide@gmail.com>"
HOMEPAGE="https://github.com/dghilardi/lekton"

mkdir -p "${OUT_DIR}"
WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

control() {
  local root="$1" package="$2" depends="$3" description="$4"
  mkdir -p "${root}/DEBIAN"
  cat > "${root}/DEBIAN/control" <<EOF
Package: ${package}
Version: ${VERSION}
Architecture: ${ARCH}
Maintainer: ${MAINTAINER}
Installed-Size: $(du -sk --exclude=DEBIAN "${root}" | cut -f1)
${depends:+Depends: ${depends}
}Section: web
Priority: optional
Homepage: ${HOMEPAGE}
Description: ${description}
EOF
}

# ── lekton ────────────────────────────────────────────────────────────────────
ROOT="${WORK}/lekton"
install -Dm 0755 "${SERVER_DIR}/lekton" "${ROOT}/usr/bin/lekton"
mkdir -p "${ROOT}/usr/share/lekton"
cp -r "${SITE_DIR}" "${ROOT}/usr/share/lekton/site"
install -Dm 0644 packaging/deb/lekton.service "${ROOT}/usr/lib/systemd/system/lekton.service"
# Holds secrets: readable by root only (systemd reads it before dropping privileges).
install -Dm 0600 packaging/deb/lekton.env "${ROOT}/etc/lekton/lekton.env"
install -Dm 0644 LICENSE "${ROOT}/usr/share/doc/lekton/copyright"

# Built on Ubuntu 22.04, so it needs its glibc or newer.
control "${ROOT}" lekton "libc6 (>= 2.35), libgcc-s1, libssl3 | libssl3t64, ca-certificates" \
  "Internal developer portal for documentation and API schemas
 Lekton serves documentation, API schemas and prompts, with full-text and
 AI-assisted search. Requires MongoDB and S3-compatible storage."
echo "/etc/lekton/lekton.env" > "${ROOT}/DEBIAN/conffiles"

cat > "${ROOT}/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
if [ "$1" = "configure" ] && [ -d /run/systemd/system ]; then
  systemctl daemon-reload || true
  systemctl try-restart lekton.service || true
fi
EOF
cat > "${ROOT}/DEBIAN/prerm" <<'EOF'
#!/bin/sh
set -e
if [ "$1" = "remove" ] && [ -d /run/systemd/system ]; then
  systemctl stop lekton.service || true
fi
EOF
cat > "${ROOT}/DEBIAN/postrm" <<'EOF'
#!/bin/sh
set -e
if [ -d /run/systemd/system ]; then
  systemctl daemon-reload || true
fi
EOF
chmod 0755 "${ROOT}/DEBIAN/postinst" "${ROOT}/DEBIAN/prerm" "${ROOT}/DEBIAN/postrm"

dpkg-deb -Zxz --build --root-owner-group "${ROOT}" "${OUT_DIR}/lekton_${VERSION}_${ARCH}.deb"

# ── lekton-sync ───────────────────────────────────────────────────────────────
ROOT="${WORK}/lekton-sync"
install -Dm 0755 "${SYNC_DIR}/lekton-sync" "${ROOT}/usr/bin/lekton-sync"
install -Dm 0644 LICENSE "${ROOT}/usr/share/doc/lekton-sync/copyright"

control "${ROOT}" lekton-sync "" \
  "Sync markdown documents to a Lekton instance
 Command-line tool that uploads a tree of markdown documents, attachments,
 schemas and prompts to a Lekton server."

dpkg-deb -Zxz --build --root-owner-group "${ROOT}" "${OUT_DIR}/lekton-sync_${VERSION}_${ARCH}.deb"
