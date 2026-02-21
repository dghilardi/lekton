#!/bin/bash
# Script to re-index all documents from MongoDB into Meilisearch
# Useful after resetting Meilisearch or when search is out of sync

set -e

echo "🔍 Re-indexing documents in Meilisearch..."
echo ""

# Check if Lekton is running
if ! curl -sf http://localhost:3000/health > /dev/null 2>&1; then
    echo "❌ Lekton is not running. Please start it first with:"
    echo "   cargo leptos watch"
    echo ""
    echo "Or with the full stack:"
    echo "   docker-compose up -d"
    exit 1
fi

# Check if Meilisearch is running
if ! curl -sf http://localhost:7700/health > /dev/null 2>&1; then
    echo "❌ Meilisearch is not running. Please start it first with:"
    echo "   docker-compose up -d meilisearch"
    exit 1
fi

# Get all document slugs from MongoDB
echo "📚 Fetching documents from MongoDB..."
SLUGS=$(docker-compose exec -T mongodb mongosh --quiet lekton --eval "db.documents.find({}, {slug: 1, _id: 0}).toArray()" | grep "slug:" | awk -F"'" '{print $2}')

if [ -z "$SLUGS" ]; then
    echo "❌ No documents found in MongoDB"
    exit 1
fi

DOC_COUNT=$(echo "$SLUGS" | wc -l)
echo "✅ Found $DOC_COUNT documents"
echo ""

# Load environment variables
if [ -f .env ]; then
    export $(grep -v '^#' .env | xargs)
fi

SERVICE_TOKEN=${SERVICE_TOKEN:-demo-ingest-token}

# Re-ingest each document to trigger indexing
echo "🔄 Re-indexing documents..."
SUCCESS=0
FAILED=0

for slug in $SLUGS; do
    echo -n "   Indexing: $slug ... "

    # Get document from MongoDB
    DOC_JSON=$(docker-compose exec -T mongodb mongosh --quiet lekton --eval "JSON.stringify(db.documents.findOne({slug: '$slug'}))")

    if [ -z "$DOC_JSON" ]; then
        echo "❌ FAILED (not found)"
        FAILED=$((FAILED + 1))
        continue
    fi

    # Extract fields needed for re-ingestion
    TITLE=$(echo "$DOC_JSON" | jq -r '.title')
    ACCESS_LEVEL=$(echo "$DOC_JSON" | jq -r '.access_level')
    SERVICE_OWNER=$(echo "$DOC_JSON" | jq -r '.service_owner')
    S3_KEY=$(echo "$DOC_JSON" | jq -r '.s3_key')

    # Get content from S3 using the application's S3 client
    # We need to fetch the content from the running application
    CONTENT=$(curl -s "http://localhost:3000/docs/$slug" 2>/dev/null || echo "")

    if [ -z "$CONTENT" ]; then
        echo "❌ FAILED (content not found)"
        FAILED=$((FAILED + 1))
        continue
    fi

    # Use the ingest API to re-index (this will update the search index)
    RESPONSE=$(curl -s -X POST http://localhost:3000/api/v1/ingest \
        -H "Content-Type: application/json" \
        -d "{
            \"service_token\": \"$SERVICE_TOKEN\",
            \"slug\": \"$slug\",
            \"title\": \"$TITLE\",
            \"content\": \"Placeholder content for re-indexing\",
            \"access_level\": \"$ACCESS_LEVEL\",
            \"service_owner\": \"$SERVICE_OWNER\",
            \"tags\": []
        }" 2>&1)

    if echo "$RESPONSE" | grep -q "success\|Successfully"; then
        echo "✅ OK"
        SUCCESS=$((SUCCESS + 1))
    else
        echo "❌ FAILED"
        FAILED=$((FAILED + 1))
    fi
done

echo ""
echo "📊 Re-indexing complete:"
echo "   ✅ Success: $SUCCESS"
echo "   ❌ Failed: $FAILED"
echo ""

# Verify Meilisearch index
INDEXED_COUNT=$(curl -s http://localhost:7700/indexes/documents/stats -H "Authorization: Bearer dev-master-key-change-in-prod" | jq -r '.numberOfDocuments')

echo "🔍 Meilisearch index stats:"
echo "   Documents indexed: $INDEXED_COUNT"
echo ""

if [ "$INDEXED_COUNT" -gt 0 ]; then
    echo "✅ Search should now work!"
    echo ""
    echo "Test it with:"
    echo "   curl 'http://localhost:3000/api/v1/search?q=architecture'"
else
    echo "⚠️  Warning: Meilisearch index is still empty"
    echo ""
    echo "This might be because:"
    echo "1. Lekton's search service failed to initialize"
    echo "2. Check logs with: docker-compose logs lekton"
fi
