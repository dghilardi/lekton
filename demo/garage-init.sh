#!/bin/sh
# garage-init.sh — Bootstrap Garage via Admin API
# Uses curl (from curlimages/curl container)

# Note: Don't use 'set -e' to make the script more idempotent

echo "=== Garage Init: Starting bootstrap ==="

ADMIN_URL="http://garage:3902"
BEARER_TOKEN="demo-admin-token"

# The S3 key Lekton is configured with (docker-compose.yml). Garage only
# accepts its own key format: "GK" + 24 hex chars, and a 64-hex-char secret.
ACCESS_KEY_ID="${S3_ACCESS_KEY_ID:?S3_ACCESS_KEY_ID is required}"
SECRET_ACCESS_KEY="${S3_SECRET_ACCESS_KEY:?S3_SECRET_ACCESS_KEY is required}"

# 1. Get node status and extract node ID
echo "Getting node status..."
NODE_STATUS=$(curl -s -H "Authorization: Bearer $BEARER_TOKEN" "$ADMIN_URL/v1/status")

# Extract the node ID using grep - look for "node": followed by the hex string (with possible spaces)
NODE_ID=$(echo "$NODE_STATUS" | grep -o '"node"[[:space:]]*:[[:space:]]*"[a-f0-9]*"' | head -1 | grep -o '[a-f0-9]\{64\}')
echo "Node ID: $NODE_ID"

if [ -z "$NODE_ID" ]; then
    echo "ERROR: Could not extract node ID"
    echo "Status was: $NODE_STATUS"
    exit 1
fi

# 2. Stage layout - assign this node to zone with capacity
echo "Staging layout for node $NODE_ID..."

# Create the layout update payload as an array
LAYOUT_PAYLOAD="[{\"id\":\"$NODE_ID\",\"zone\":\"dc1\",\"capacity\":1073741824,\"tags\":[]}]"

echo "Layout payload: $LAYOUT_PAYLOAD"

LAYOUT_RESPONSE=$(curl -s -X POST \
    -H "Authorization: Bearer $BEARER_TOKEN" \
    -H "Content-Type: application/json" \
    -d "$LAYOUT_PAYLOAD" \
    "$ADMIN_URL/v1/layout")

echo "Layout staging response: $LAYOUT_RESPONSE"

# 3. Apply the layout
echo "Applying layout..."
sleep 2

# Get the new layout version
LAYOUT_INFO=$(curl -s -H "Authorization: Bearer $BEARER_TOKEN" "$ADMIN_URL/v1/layout")
VERSION=$(echo "$LAYOUT_INFO" | grep -o '"version"[[:space:]]*:[[:space:]]*[0-9]*' | grep -o '[0-9]*$')

echo "Current layout version: $VERSION"

# Check if there are staged changes to apply
HAS_STAGED=$(echo "$LAYOUT_INFO" | grep -c 'stagedRoleChanges' || true)

if [ "$HAS_STAGED" -gt 0 ]; then
    echo "Found staged role changes, applying layout..."
    # For Garage, we apply with version+1 when there are staged changes
    NEXT_VERSION=$((VERSION + 1))
    APPLY_PAYLOAD="{\"version\":$NEXT_VERSION}"

    APPLY_RESPONSE=$(curl -s -X POST \
        -H "Authorization: Bearer $BEARER_TOKEN" \
        -H "Content-Type: application/json" \
        -d "$APPLY_PAYLOAD" \
        "$ADMIN_URL/v1/layout/apply")

    echo "Layout apply response: $APPLY_RESPONSE"
    echo "Waiting for layout to propagate..."
    sleep 5
else
    echo "No staged changes to apply"
fi

# 4. Create bucket
echo "Creating bucket 'lekton-docs'..."
BUCKET_PAYLOAD='{"globalAlias":"lekton-docs"}'

BUCKET_RESPONSE=$(curl -s -X POST \
    -H "Authorization: Bearer $BEARER_TOKEN" \
    -H "Content-Type: application/json" \
    -d "$BUCKET_PAYLOAD" \
    "$ADMIN_URL/v1/bucket")

echo "Bucket creation response: $BUCKET_RESPONSE"

# Extract bucket ID from response, or get it if bucket already exists
BUCKET_ID=$(echo "$BUCKET_RESPONSE" | grep -o '"id"[[:space:]]*:[[:space:]]*"[a-f0-9]*"' | head -1 | grep -o '[a-f0-9]\{64\}')

if [ -z "$BUCKET_ID" ]; then
    echo "Bucket may already exist, fetching bucket info..."
    BUCKET_INFO=$(curl -s -H "Authorization: Bearer $BEARER_TOKEN" "$ADMIN_URL/v1/bucket?globalAlias=lekton-docs")
    BUCKET_ID=$(echo "$BUCKET_INFO" | grep -o '"id"[[:space:]]*:[[:space:]]*"[a-f0-9]*"' | head -1 | grep -o '[a-f0-9]\{64\}')
fi

echo "Bucket ID: $BUCKET_ID"

# 5. Import the API key Lekton uses (a fresh key would have a random ID).
echo "Importing API key $ACCESS_KEY_ID..."
KEY_PAYLOAD="{\"name\":\"lekton-demo\",\"accessKeyId\":\"$ACCESS_KEY_ID\",\"secretAccessKey\":\"$SECRET_ACCESS_KEY\"}"

KEY_RESPONSE=$(curl -s -X POST \
    -H "Authorization: Bearer $BEARER_TOKEN" \
    -H "Content-Type: application/json" \
    -d "$KEY_PAYLOAD" \
    "$ADMIN_URL/v1/key/import")

# On a re-run the key already exists and the import is rejected; that is fine.
echo "Key import response: $KEY_RESPONSE"

# 6. Grant bucket permissions to key
if [ -n "$BUCKET_ID" ] && [ -n "$ACCESS_KEY_ID" ]; then
    echo "Granting permissions on bucket to key..."
    PERM_PAYLOAD="{\"bucketId\":\"$BUCKET_ID\",\"accessKeyId\":\"$ACCESS_KEY_ID\",\"permissions\":{\"read\":true,\"write\":true,\"owner\":true}}"

    PERM_RESPONSE=$(curl -s -X POST \
        -H "Authorization: Bearer $BEARER_TOKEN" \
        -H "Content-Type: application/json" \
        -d "$PERM_PAYLOAD" \
        "$ADMIN_URL/v1/bucket/allow")

    echo "Permission grant response: $PERM_RESPONSE"
else
    echo "WARNING: Missing bucket ID or key ID, skipping permission grant"
    echo "  BUCKET_ID=$BUCKET_ID"
    echo "  ACCESS_KEY_ID=$ACCESS_KEY_ID"
fi

echo ""
echo "=== Garage Init: Bootstrap complete ==="
echo ""
echo "  Bucket: lekton-docs"
echo "  Bucket ID: $BUCKET_ID"
echo "  Access Key ID: $ACCESS_KEY_ID"
echo "  S3 Endpoint: http://garage:3900"
echo "  S3 Region: garage"
echo ""
