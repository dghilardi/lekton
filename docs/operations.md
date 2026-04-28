# Operations Guide

Runtime procedures for backup, recovery, token rotation, and dependency auditing.

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

To run locally:

```bash
cargo deny check advisories
cargo deny check licenses
```

When a new advisory appears, either upgrade the affected crate or add a justified `ignore` entry in `deny.toml`.
