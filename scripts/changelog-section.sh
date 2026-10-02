#!/usr/bin/env bash
#
# Print the CHANGELOG.md section of a released version (without its heading),
# used as the body of the GitHub release.
#
# Usage: scripts/changelog-section.sh VERSION [CHANGELOG]
#
set -euo pipefail

VERSION="${1:?usage: $0 VERSION [CHANGELOG]}"
CHANGELOG="${2:-CHANGELOG.md}"

SECTION="$(awk -v version="${VERSION}" '
  /^## \[/ {
    if (found) exit
    if (index($0, "## [" version "]") == 1) { found = 1; next }
  }
  found { print }
' "${CHANGELOG}")"

# Trim leading/trailing blank lines.
SECTION="$(printf '%s\n' "${SECTION}" | sed -e '/./,$!d' | sed -e ':a' -e '/^\n*$/{$d;N;ba' -e '}')"

if [ -z "${SECTION}" ]; then
  echo "No CHANGELOG entry found for version ${VERSION}" >&2
  exit 1
fi

printf '%s\n' "${SECTION}"
