#!/bin/sh
# load_demo.sh — Ingest demo documents and schemas into Lekton via the API.
#
# This script is run by the demo-loader container after Lekton is healthy.

set -e

LEKTON_URL="${LEKTON_URL:-http://lekton:3000}"
SERVICE_TOKEN="${SERVICE_TOKEN:-demo-ingest-token}"
DEMO_DIR="/demo/documents"
SCHEMA_DIR="/demo/schemas"

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

ingest_schema() {
    local name="$1"
    local schema_type="$2"
    local version="$3"
    local status="$4"
    local file="$5"

    if [ ! -f "$file" ]; then
        echo "WARN: Schema file not found: $file — skipping"
        return
    fi

    # Read the file content and escape for JSON embedding
    local content
    content=$(cat "$file" | sed 's/\\/\\\\/g' | sed 's/"/\\"/g' | sed ':a;N;$!ba;s/\n/\\n/g')

    echo "→ Ingesting schema: $name v$version ($schema_type) [$status]"

    HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
        -X POST "${LEKTON_URL}/api/v1/schemas" \
        -H "Content-Type: application/json" \
        -d "{
            \"service_token\": \"${SERVICE_TOKEN}\",
            \"name\": \"${name}\",
            \"schema_type\": \"${schema_type}\",
            \"version\": \"${version}\",
            \"status\": \"${status}\",
            \"content\": \"${content}\"
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

# --- Documents ---
echo "--- Loading documents ---"
echo ""

ingest_doc "getting-started"   "Getting Started with Lekton"  "public"     "platform-team"  '"getting-started", "onboarding"'
ingest_doc "architecture"      "Architecture Overview"        "developer"  "platform-team"  '"architecture", "design"'
ingest_doc "deployment-guide"  "Deployment Guide"             "developer"  "devops-team"    '"deployment", "docker", "k8s"'
ingest_doc "api-reference"     "API Reference"                "developer"  "platform-team"  '"api", "rest", "reference"'
ingest_doc "security-rbac"     "Security & RBAC"              "architect"  "security-team"  '"security", "rbac", "auth"'

echo ""

# --- Schemas ---
echo "--- Loading schemas ---"
echo ""

# Payment Service API — two versions (v1 deprecated, v2 stable)
ingest_schema "payment-service-api" "openapi" "1.0.0" "deprecated" "$SCHEMA_DIR/payment-service-api-v1.json"
ingest_schema "payment-service-api" "openapi" "2.0.0" "stable"     "$SCHEMA_DIR/payment-service-api-v2.json"

# Inventory Service API — single stable version
ingest_schema "inventory-service-api" "openapi" "1.0.0" "stable" "$SCHEMA_DIR/inventory-service-api-v1.json"

# Order Events — AsyncAPI
ingest_schema "order-events" "asyncapi" "1.0.0" "stable" "$SCHEMA_DIR/order-events-v1.json"

echo ""
echo "============================================"
echo "  Demo loading complete!"
echo "  Visit: http://localhost:3000"
echo "============================================"
