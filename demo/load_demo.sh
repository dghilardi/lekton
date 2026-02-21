#!/bin/sh
# load_demo.sh — Ingest demo documents into Lekton via the API.
#
# This script is run by the demo-loader container after Lekton is healthy.

set -e

LEKTON_URL="${LEKTON_URL:-http://lekton:3000}"
SERVICE_TOKEN="${SERVICE_TOKEN:-demo-ingest-token}"
DEMO_DIR="/demo/documents"

ingest_doc() {
    local slug="$1"
    local title="$2"
    local access_level="$3"
    local service_owner="$4"
    local tags="$5"
    local file="$DEMO_DIR/$slug.md"

    if [ ! -f "$file" ]; then
        echo "WARN: File not found: $file — skipping"
        return
    fi

    # Read the file content and escape for JSON
    local content
    content=$(cat "$file" | sed 's/\\/\\\\/g' | sed 's/"/\\"/g' | sed ':a;N;$!ba;s/\n/\\n/g')

    echo "→ Ingesting: $title ($slug) [${access_level}]"

    HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
        -X POST "${LEKTON_URL}/api/v1/ingest" \
        -H "Content-Type: application/json" \
        -d "{
            \"service_token\": \"${SERVICE_TOKEN}\",
            \"slug\": \"${slug}\",
            \"title\": \"${title}\",
            \"content\": \"${content}\",
            \"access_level\": \"${access_level}\",
            \"service_owner\": \"${service_owner}\",
            \"tags\": [${tags}]
        }")

    if [ "$HTTP_CODE" = "200" ]; then
        echo "  ✅ OK (HTTP $HTTP_CODE)"
    else
        echo "  ❌ FAILED (HTTP $HTTP_CODE)"
    fi
}

echo "============================================"
echo "  Lekton Demo Loader"
echo "  Target: $LEKTON_URL"
echo "============================================"
echo ""

# Wait a moment for Lekton to be fully ready
sleep 2

# Ingest all demo documents
ingest_doc "getting-started"   "Getting Started with Lekton"  "public"     "platform-team"  '"getting-started", "onboarding"'
ingest_doc "architecture"      "Architecture Overview"        "developer"  "platform-team"  '"architecture", "design"'
ingest_doc "deployment-guide"  "Deployment Guide"             "developer"  "devops-team"    '"deployment", "docker", "k8s"'
ingest_doc "api-reference"     "API Reference"                "developer"  "platform-team"  '"api", "rest", "reference"'
ingest_doc "security-rbac"     "Security & RBAC"              "architect"  "security-team"  '"security", "rbac", "auth"'

echo ""
echo "============================================"
echo "  Demo loading complete!"
echo "  Visit: http://localhost:3000"
echo "============================================"
