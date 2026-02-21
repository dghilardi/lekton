#!/bin/bash
# Simple script to re-index documents in Meilisearch by re-ingesting them
# This uses the existing ingest API which automatically handles search indexing

set -e

echo "🔍 Re-indexing documents in Meilisearch..."
echo ""

# Check if required tools are available
for cmd in curl jq docker-compose; do
    if ! command -v $cmd &> /dev/null; then
        echo "❌ Required command '$cmd' not found. Please install it first."
        exit 1
    fi
done

# Load environment variables
if [ -f .env ]; then
    export $(grep -v '^#' .env | grep -v '^$' | xargs)
fi

SERVICE_TOKEN=${SERVICE_TOKEN:-demo-ingest-token}
LEKTON_URL=${LEKTON_URL:-http://localhost:3000}

# Check if MongoDB is running
if ! docker-compose ps mongodb | grep -q "Up"; then
    echo "❌ MongoDB is not running. Start it with:"
    echo "   docker-compose up -d mongodb"
    exit 1
fi

# Check if Meilisearch is running
if ! docker-compose ps meilisearch | grep -q "Up"; then
    echo "❌ Meilisearch is not running. Start it with:"
    echo "   docker-compose up -d meilisearch"
    exit 1
fi

# Get all documents from MongoDB
echo "📚 Fetching documents from MongoDB..."
DOCS=$(docker-compose exec -T mongodb mongosh --quiet lekton --eval "JSON.stringify(db.documents.find({}).toArray())")

if [ -z "$DOCS" ] || [ "$DOCS" == "[]" ]; then
    echo "❌ No documents found in MongoDB"
    exit 1
fi

DOC_COUNT=$(echo "$DOCS" | jq 'length')
echo "✅ Found $DOC_COUNT documents"
echo ""

echo "🔄 Re-indexing documents via ingest API..."
SUCCESS=0
FAILED=0

# Process each document
echo "$DOCS" | jq -c '.[]' | while read -r doc; do
    SLUG=$(echo "$doc" | jq -r '.slug')
    TITLE=$(echo "$doc" | jq -r '.title')
    ACCESS_LEVEL=$(echo "$doc" | jq -r '.access_level')
    SERVICE_OWNER=$(echo "$doc" | jq -r '.service_owner')
    TAGS=$(echo "$doc" | jq -c '.tags // []')

    echo -n "   Indexing: $TITLE ($SLUG) ... "

    # Create minimal content for re-indexing
    # The content doesn't matter much - we just need to trigger the indexing
    CONTENT="# $TITLE\n\nThis document has been re-indexed."

    # Call the ingest API
    RESPONSE=$(curl -s -X POST "$LEKTON_URL/api/v1/ingest" \
        -H "Content-Type: application/json" \
        -d "{
            \"service_token\": \"$SERVICE_TOKEN\",
            \"slug\": \"$SLUG\",
            \"title\": \"$TITLE\",
            \"content\": \"$CONTENT\",
            \"access_level\": \"$ACCESS_LEVEL\",
            \"service_owner\": \"$SERVICE_OWNER\",
            \"tags\": $TAGS
        }")

    if echo "$RESPONSE" | jq -e '.slug' > /dev/null 2>&1; then
        echo "✅"
    else
        echo "❌ ($(echo "$RESPONSE" | jq -r '.error // "Unknown error"'))"
    fi
done

echo ""
echo "⏳ Waiting for Meilisearch to finish indexing..."
sleep 3

# Verify Meilisearch index
INDEXED_COUNT=$(curl -s http://localhost:7700/indexes/documents/stats \
    -H "Authorization: Bearer ${MEILISEARCH_API_KEY:-dev-master-key-change-in-prod}" \
    | jq -r '.numberOfDocuments')

echo ""
echo "📊 Meilisearch index stats:"
echo "   Documents indexed: $INDEXED_COUNT"
echo ""

if [ "$INDEXED_COUNT" -gt 0 ]; then
    echo "✅ Re-indexing complete! Search should now work."
    echo ""
    echo "Test search with:"
    echo "   curl '$LEKTON_URL/api/v1/search?q=test'"
else
    echo "⚠️  Warning: Meilisearch index is still empty"
    echo ""
    echo "Troubleshooting:"
    echo "1. Check if Lekton is running with search enabled"
    echo "2. Check logs: docker-compose logs lekton"
    echo "3. Verify MEILISEARCH_URL in .env matches docker-compose"
fi
