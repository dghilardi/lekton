#!/usr/bin/env bash
#
# Stage all served static JS/CSS assets into the Leptos site root.
#
# Why this exists:
#   The server serves static files only from the Leptos site root (target/site).
#   cargo-leptos is supposed to sync public/ -> target/site/ during the build, and
#   build.rs generates the viewer/mermaid bundles into public/js from node_modules.
#   BOTH steps are skipped when cargo considers the build up-to-date (cached builds
#   in CI and Docker), which silently leaves assets missing from the served root and
#   produces 404s in production.
#
#   This script makes the served root complete and IDENTICAL across local, CI (e2e)
#   and Docker builds, with node_modules as the source of truth for generated bundles
#   (always present after `npm ci`, independent of cargo's build cache). It is the
#   single source of truth for asset staging — invoked from the Dockerfile and the
#   e2e workflow so the two environments can never drift apart again.
#
# Usage: scripts/stage-site-assets.sh [SITE_ROOT]   (default: target/site)
#
set -euo pipefail

SITE_ROOT="${1:-target/site}"
SITE_JS="${SITE_ROOT}/js"

mkdir -p "${SITE_JS}/chunks/mermaid.esm.min"

# 1. Committed static JS (mermaid-loader, tiptap, code-blocks, login, editor-assets).
#    Guarantees they reach the served root even when the cargo-leptos public/ sync is
#    skipped on a cached build.
cp -r public/js/. "${SITE_JS}/"

# 2. Generated bundles, copied straight from node_modules so they are present even
#    when build.rs was skipped on a cached build.
cp node_modules/mermaid/dist/mermaid.esm.min.mjs "${SITE_JS}/"
find node_modules/mermaid/dist/chunks/mermaid.esm.min -name '*.mjs' \
  -exec cp {} "${SITE_JS}/chunks/mermaid.esm.min/" \;
cp node_modules/@scalar/api-reference/dist/browser/standalone.js "${SITE_JS}/scalar-standalone.js"
cp node_modules/@scalar/api-reference/dist/style.css "${SITE_JS}/scalar-style.css"
cp node_modules/@asyncapi/react-component/browser/standalone/index.js "${SITE_JS}/asyncapi-standalone.js"
cp node_modules/@asyncapi/react-component/styles/default.min.css "${SITE_JS}/asyncapi-default.min.css"

echo "Staged static assets into ${SITE_JS}"
