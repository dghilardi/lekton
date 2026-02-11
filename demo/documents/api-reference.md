# API Reference

Lekton exposes a REST API for programmatic document management and authentication.

## Base URL

```
https://lekton.yourcompany.com/api
```

## Authentication

### Service Token (API)

For CI/CD integration, include the service token in the request body:

```json
{
  "service_token": "your-token-here"
}
```

### Demo Mode (Web UI)

When `DEMO_MODE=true`, use the web login:

```bash
POST /api/auth/login
Content-Type: application/json

{
  "username": "demo",
  "password": "demo"
}
```

---

## Endpoints

### `POST /api/v1/ingest`

Ingest a document into Lekton. This creates or updates a document by slug.

**Request Body:**

| Field          | Type     | Required | Description                          |
|----------------|----------|----------|--------------------------------------|
| `service_token`| string   | Yes      | Service authentication token         |
| `slug`         | string   | Yes      | URL-safe path (e.g., `team/doc`)     |
| `title`        | string   | Yes      | Human-readable title                 |
| `content`      | string   | Yes      | Raw Markdown content                 |
| `access_level` | string   | Yes      | `public`, `developer`, `architect`, or `admin` |
| `service_owner`| string   | Yes      | Team/service that owns this doc      |
| `tags`         | string[] | No       | Tags for categorization              |

**Example Request:**

```bash
curl -X POST http://localhost:3000/api/v1/ingest \
  -H "Content-Type: application/json" \
  -d '{
    "service_token": "demo-ingest-token",
    "slug": "backend/auth-guide",
    "title": "Authentication Guide",
    "content": "# Authentication\n\nThis guide covers...",
    "access_level": "developer",
    "service_owner": "backend-team",
    "tags": ["auth", "security"]
  }'
```

**Success Response (200):**

```json
{
  "message": "Document ingested successfully",
  "slug": "backend/auth-guide",
  "s3_key": "docs/backend_auth-guide.md"
}
```

**Error Responses:**

| Status | Condition                        |
|--------|----------------------------------|
| 400    | Invalid access level or empty slug |
| 401    | Invalid service token            |
| 500    | Database or storage error        |

---

### `POST /api/auth/login` *(Demo Mode Only)*

Authenticate with built-in credentials.

**Request Body:**

```json
{
  "username": "demo",
  "password": "demo"
}
```

**Response (200):**

```json
{
  "message": "Login successful",
  "user": {
    "user_id": "demo-demo",
    "email": "demo@demo.lekton.dev",
    "access_level": "Developer"
  }
}
```

---

### `GET /api/auth/me` *(Demo Mode Only)*

Returns the currently authenticated user from the session cookie.

**Response (200):**

```json
{
  "user_id": "demo-demo",
  "email": "demo@demo.lekton.dev",
  "access_level": "Developer"
}
```

**Response (401):** Not logged in.

---

### `POST /api/auth/logout` *(Demo Mode Only)*

Clears the session cookie.

**Response (200):** Empty body, cookie cleared.

## Rate Limits

There are no built-in rate limits in the current release. For production deployments, use a reverse proxy (nginx, Caddy, etc.) to enforce rate limiting.
