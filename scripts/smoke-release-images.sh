#!/usr/bin/env bash
# Build the release Docker targets from downloaded build-binaries artifacts.
set -euo pipefail

ARTIFACTS="$(realpath "${1:?Usage: smoke-release-images.sh ARTIFACTS_DIR}")"
PREFIX="lekton-release-smoke-$$"

cleanup() {
  status=$?
  if [ "$status" -ne 0 ]; then
    docker logs "$PREFIX-server" 2>/dev/null || true
  fi
  docker rm -f "$PREFIX-server" "$PREFIX-mongo" >/dev/null 2>&1 || true
  docker network rm "$PREFIX" >/dev/null 2>&1 || true
  docker image rm "$PREFIX:server" "$PREFIX:cli" "$PREFIX:ci" >/dev/null 2>&1 || true
  exit "$status"
}
trap cleanup EXIT

docker buildx build --load --platform linux/amd64 --target release \
  --build-context "release-artifacts=$ARTIFACTS" -t "$PREFIX:server" .
docker buildx build --load --platform linux/amd64 -f cli/Dockerfile --target release-runtime \
  --build-context "release-artifacts=$ARTIFACTS" -t "$PREFIX:cli" .
docker buildx build --load --platform linux/amd64 -f cli/Dockerfile --target release-ci \
  --build-context "release-artifacts=$ARTIFACTS" -t "$PREFIX:ci" .

# The distroless image must run as non-root; the CI image must retain its shell.
case "$(docker image inspect --format '{{.Config.User}}' "$PREFIX:cli")" in
  nonroot|nonroot:nonroot|65532|65532:65532) ;;
  *) echo "Expected the distroless nonroot user" >&2; exit 1 ;;
esac
docker run --rm "$PREFIX:cli" --help >/dev/null
docker run --rm "$PREFIX:ci" --help >/dev/null
docker run --rm --entrypoint sh "$PREFIX:ci" -c 'lekton-sync --help >/dev/null'

docker network create "$PREFIX" >/dev/null
docker run -d --name "$PREFIX-mongo" --network "$PREFIX" mongo:7 >/dev/null
docker run -d --name "$PREFIX-server" --network "$PREFIX" \
  -e "LKN__DATABASE__URI=mongodb://$PREFIX-mongo:27017" \
  -e LKN__DATABASE__NAME=lekton_smoke -e LKN__AUTH__DEMO_MODE=true \
  -e LKN__STORAGE__BUCKET=lekton-smoke \
  -e AWS_REGION=us-east-1 -e AWS_ACCESS_KEY_ID=smoke -e AWS_SECRET_ACCESS_KEY=smoke \
  "$PREFIX:server" >/dev/null

for attempt in $(seq 1 60); do
  if docker exec "$PREFIX-server" curl -sf -o /dev/null http://localhost:3000/; then
    break
  fi
  sleep 1
done
for path in / /pkg/lekton.wasm /pkg/lekton.js /pkg/lekton.css \
  /js/mermaid.esm.min.mjs /js/tiptap.js /js/scalar-standalone.js /js/asyncapi-standalone.js; do
  docker exec "$PREFIX-server" curl -sf -o /dev/null "http://localhost:3000$path"
done
docker exec "$PREFIX-server" test -r /usr/lib/libpdfium.so
echo "Release images serve SSR pages and static assets; both CLI images run successfully."
