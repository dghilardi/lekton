#!/bin/sh
# garage-init.sh — Bootstrap Garage: assign layout, create bucket, create API key
#
# This runs as a one-shot init container after Garage is healthy.

set -e

ADMIN_URL="http://garage:3902"
ADMIN_TOKEN="demo-admin-token"

echo "=== Garage Init: Starting bootstrap ==="

# 1. Get the node ID
NODE_ID=$(garage -c /etc/garage.toml node id 2>/dev/null | head -1 | cut -d'@' -f1)
echo "Node ID: $NODE_ID"

# 2. Assign layout (zone=dc1, capacity=1GB)
garage -c /etc/garage.toml layout assign "$NODE_ID" -z dc1 -c 1G 2>/dev/null || true

# 3. Apply layout
layout_version=$(garage -c /etc/garage.toml layout show 2>/dev/null | grep "apply --version" | grep -oE '[0-9]+' || echo "")
if [ -n "$layout_version" ]; then
    garage -c /etc/garage.toml layout apply --version "$layout_version" 2>/dev/null || true
    echo "Layout applied (version $layout_version)"
else
    echo "Layout already applied or no changes needed"
fi

# 4. Create the bucket
garage -c /etc/garage.toml bucket create lekton-docs 2>/dev/null || echo "Bucket may already exist, continuing..."

# 5. Create an API key
garage -c /etc/garage.toml key create lekton-key 2>/dev/null || echo "Key may already exist, continuing..."

# 6. Import a specific key (so docker-compose env vars match)
garage -c /etc/garage.toml key import --yes lekton-demo-key lekton-demo-secret --name lekton-key 2>/dev/null || echo "Key import skipped (may already exist)"

# 7. Grant the key access to the bucket
garage -c /etc/garage.toml bucket allow --read --write --owner lekton-docs --key lekton-key 2>/dev/null || echo "Permission grant skipped"

echo "=== Garage Init: Bootstrap complete ==="
echo "  Bucket: lekton-docs"
echo "  Key ID: lekton-demo-key"
echo "  Secret: lekton-demo-secret"
echo "  S3 Endpoint: http://garage:3900"
