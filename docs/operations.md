# Operations Guide

Runtime procedures for backup, recovery, token rotation, and dependency auditing.

---

## Reverse Proxy And Rate Limiting

Lekton rate-limits dynamic routes, not static fallback assets. By default the limiter uses the peer IP, but it can read `X-Forwarded-For`, `X-Real-IP`, and `Forwarded` only when the direct peer matches `LKN__SERVER__RATE_LIMIT_TRUSTED_PROXIES`.

For nginx on the same host, the default trusted proxies (`127.0.0.1/32,::1/128`) are enough. For container or load-balancer networks, set the proxy IP or CIDR explicitly:

```bash
LKN__SERVER__RATE_LIMIT_TRUSTED_PROXIES="10.0.0.0/8,172.16.0.0/12"
LKN__SERVER__RATE_LIMIT_PER_SECOND=5
LKN__SERVER__RATE_LIMIT_BURST=50
```

Recommended nginx forwarding headers:

```nginx
proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
proxy_set_header X-Real-IP $remote_addr;
proxy_set_header X-Forwarded-Proto $scheme;
proxy_set_header Host $host;
```

`rate_limit_per_second` is the replenish interval in seconds, not a rate: with the values above an IP may send a burst of 50 requests, then one more every 5 seconds. `llm_rate_limit_per_second` works the same way for the LLM endpoints, per user.

If multiple real users share the same public NAT IP, they still share the same IP quota; for that deployment profile raise `rate_limit_burst` and **lower** `rate_limit_per_second` (e.g. `1` for one request per second).

---

## Prometheus Metrics

Usage metrics are exposed at `GET /metrics` when the `metrics` feature is enabled (off by default):

```bash
LKN__FEATURES__METRICS=true
# Optional bearer token; when set, scrapers must send Authorization: Bearer <token>.
LKN__SERVER__METRICS_TOKEN="<random-secret>"
```

The endpoint is **not** meant to be public. Protect it at the proxy layer (block `/metrics` on the public vhost, expose it only to the scraper's network) and/or set `metrics_token`.

Exposed series (labels kept low-cardinality — matched route templates, no per-document ids):

- `http_requests_total{method,path,status}` and `http_request_duration_seconds{...}` — HTTP traffic and latency histogram.
- `lekton_search_queries_total`, `lekton_search_zero_results_total` — search volume and content gaps.
- `lekton_document_views_total{kind}` — document reads (`kind` = `markdown`/`upload`/`sync`).
- `lekton_schema_views_total` — schema detail views.
- `lekton_rag_chat_messages_total` — RAG chat usage.
- `lekton_editor_saves_total`, `lekton_document_uploads_total` — content production.

Example Prometheus scrape config (token via a `Bearer` credentials file or `authorization`):

```yaml
scrape_configs:
  - job_name: lekton
    metrics_path: /metrics
    authorization:
      type: Bearer
      credentials: "<random-secret>"   # omit if metrics_token is unset
    static_configs:
      - targets: ["lekton:3000"]
```

---

## MongoDB

### Backup

```bash
# Point-in-time dump (mongodump)
mongodump \
  --uri "$LKN__DATABASE__URI" \
  --db lekton \
  --out /backups/mongo/$(date +%Y-%m-%d)

# Compress
tar -czf /backups/mongo/$(date +%Y-%m-%d).tar.gz /backups/mongo/$(date +%Y-%m-%d)
```

Recommended: daily dump + weekly upload to S3. Keep at least 30 daily and 12 weekly snapshots.

### Restore

```bash
mongorestore \
  --uri "$LKN__DATABASE__URI" \
  --db lekton \
  /backups/mongo/<date>/<db>/
```

**Note:** restoring drops the existing collection if `--drop` is passed. Verify with a staging instance first.

---

## S3 / Garage / MinIO

Document content, assets, and prompt files are stored in the bucket configured via `LKN__STORAGE__BUCKET`.

### Backup

Enable versioning on the bucket and replicate to a secondary region or separate cluster. With Garage:

```bash
# Mirror to secondary cluster (rclone)
rclone sync garage:lekton-bucket garage-secondary:lekton-bucket-backup \
  --config /etc/rclone.conf
```

### Recovery

```bash
# Restore a single object
rclone copy garage:lekton-bucket/docs/my-doc.md ./

# Full restore
rclone sync garage:lekton-bucket-backup garage:lekton-bucket \
  --config /etc/rclone.conf
```

---

## Qdrant

Vector embeddings are stored in the collection configured via `LKN__RAG__QDRANT_COLLECTION` (default: `lekton`).

### Snapshot (backup)

```bash
# Create a snapshot via Qdrant REST API
curl -X POST "http://localhost:6333/collections/lekton/snapshots"
# → returns { "result": { "name": "lekton-<timestamp>.snapshot", ... } }

# Download the snapshot
curl -o /backups/qdrant/lekton-$(date +%Y-%m-%d).snapshot \
  "http://localhost:6333/collections/lekton/snapshots/lekton-<timestamp>.snapshot"
```

### Recovery

```bash
# Upload and restore a snapshot
curl -X POST "http://localhost:6333/collections/lekton/snapshots/upload" \
  -H "Content-Type: multipart/form-data" \
  -F "snapshot=@/backups/qdrant/lekton-<date>.snapshot"
```

If the collection is corrupt or missing, trigger a full re-index via the admin panel
(`/admin/settings` → *RAG* → *Re-index*) or via the API:

```bash
curl -X POST http://localhost/api/v1/admin/rag/reindex \
  -H "Authorization: Bearer $SERVICE_TOKEN"
```

### Upgrading

Qdrant only guarantees compatibility between consecutive minor versions, for both the storage and the client. Lekton uses `qdrant-client` 1.19 and is tested against Qdrant 1.19. To upgrade a server running 1.17:

1. Take a snapshot (see above).
2. Upgrade Qdrant to the latest 1.18.x and wait until every collection reports `green` (`GET /collections/lekton`). Do not skip 1.18: Qdrant does not support upgrading across more than one minor version.
3. Deploy the new Lekton version.
4. Upgrade Qdrant to 1.19.x.

Re-indexing instead of upgrading recomputes every embedding, which costs embedding-provider credits.

---

## Meilisearch

The search index is derived from MongoDB and can always be rebuilt with a full re-index (`/admin/settings` → *Search* → *Re-index*, or `POST /api/v1/admin/search/reindex` as an administrator).

### Upgrading

Lekton is tested against Meilisearch 1.54. A Meilisearch database only opens with the version that created it, so a new image needs one of:

- **In-place upgrade**: take a snapshot (`POST /snapshots`), then start the new version with `MEILI_UPGRADE_DB=true` (`docker-compose.yml` sets it). Works for databases created by Meilisearch 1.12 or later; the upgrade is not atomic, so keep the snapshot until it succeeds.
- **Fresh index**: start the new version on an empty data directory and run a full re-index. Use this for databases older than 1.12 or if the in-place upgrade fails.

---

## Service token rotation

Service tokens are stored hashed in MongoDB (`service_tokens` collection). The raw token is shown only once at creation.

**Rotation procedure:**

1. Create a new token via the admin panel or API, noting the raw value.
2. Update all consumers (CI pipelines, sync agents) to use the new token.
3. Deactivate the old token via the admin panel (`/admin/settings` → *Service Tokens* → *Deactivate*).
4. Verify consumers work, then optionally delete the old token from the DB.

**Emergency revocation** (token compromised):

```bash
# Direct MongoDB — deactivate immediately without waiting for UI
mongosh "$LKN__DATABASE__URI" --eval '
  db.getSiblingDB("lekton").service_tokens.updateOne(
    { _id: "<token-id>" },
    { $set: { is_active: false } }
  )
'
```

---

## JWT secret rotation

The JWT secret (`LKN__AUTH__JWT_SECRET`) signs all access and refresh tokens. Rotating it invalidates all existing sessions immediately.

1. Generate a new secret: `openssl rand -base64 64`
2. Update the secret in your secrets manager / environment.
3. Restart all Lekton instances. All users will be logged out on next request and must re-authenticate.

---

## Dependency security auditing

`cargo deny` (configured in `deny.toml`) checks RustSec advisories, license compatibility, and crate provenance. It runs:

- On every push to `main` and `feat/*` branches.
- Weekly on Monday at 06:00 UTC (`.github/workflows/deny.yml`).

The `[advisories]` section in `deny.toml` already covers both advisories and licenses — no separate `cargo audit` step is needed.
The dependency graph is resolved with all features enabled (`[graph] all-features = true`), so the server (`ssr`) and browser (`hydrate`) dependencies are both audited.

To run locally:

```bash
cargo deny check advisories
cargo deny check licenses
```

When a new advisory appears, either upgrade the affected crate or add a justified `ignore` entry in `deny.toml`.

Dependabot (`.github/dependabot.yml`) opens weekly update PRs for GitHub Actions, Cargo and npm, grouping minor and patch updates. It skips the `wasm-bindgen` crates, which must match the `wasm-bindgen-cli` pinned in CI and the Dockerfile, and minor `qdrant-client` updates, which must follow the Qdrant server (see [Qdrant upgrading](#upgrading)).

The npm packages are only used at build time: Mermaid and the schema viewers are copied as prebuilt bundles, so `npm audit` reports on build tooling and on the versions of those bundles.

### Accepted advisories

Advisories without a patched release are accepted only after weighing their impact:

| Advisory | Introduced by | Impact | Options |
|----------|---------------|--------|---------|
| RUSTSEC-2023-0071 (`rsa` timing side channel) | `openidconnect` 4 | Lekton only verifies RS256 ID-token signatures with public keys; the attack needs private-key operations. | Move to `rsa` 0.10 once it is stable and `openidconnect` adopts it. |
| GHSA-866g-f22w-33x8 (low, `@ai-sdk/provider-utils` resource consumption) | `@scalar/api-reference` → `@scalar/agent-chat`, which pins `ai` 6.0.33 | The Scalar agent chat is not configured by Lekton; the shipped viewer is Scalar's prebuilt bundle, so an npm override would not change it. | Wait for a Scalar release that bumps the pin, or pass `agent: { disabled: true }` to the viewer. |
