# Getting Started with Lekton

Welcome to **Lekton** — your Internal Developer Portal. This guide will get you up and running in minutes.

## What is Lekton?

Lekton is a dynamic documentation platform designed for engineering teams. Unlike static site generators, Lekton allows you to push documentation updates directly from your CI/CD pipeline — no rebuilds required.

## Quick Start

### 1. Push Documentation

Use the Lekton Ingest API to publish docs from your CI/CD pipeline:

```bash
curl -X POST https://lekton.yourcompany.com/api/v1/ingest \
  -H "Content-Type: application/json" \
  -d '{
    "service_token": "your-token",
    "slug": "my-service/setup",
    "title": "My Service Setup Guide",
    "content": "# Setup\n\nFollow these steps...",
    "access_level": "developer",
    "service_owner": "backend-team",
    "tags": ["setup", "my-service"]
  }'
```

### 2. Browse Documentation

Navigate the sidebar to explore documentation organized by team and topic. Use the search bar to find specific content.

### 3. Access Control

Documents are protected by **Role-Based Access Control (RBAC)**:

| Role        | Access                                               |
|-------------|------------------------------------------------------|
| **Public**      | Publicly accessible documentation                    |
| **Developer**   | Internal engineering docs                            |
| **Architect**   | Architecture decision records, system design         |
| **Admin**       | Full access including sensitive operational docs     |

## GitHub Actions Integration

Add this step to your workflow to auto-publish docs:

```yaml
- name: Publish to Lekton
  run: |
    curl -sf -X POST $LEKTON_URL/api/v1/ingest \
      -H "Content-Type: application/json" \
      -d @docs/lekton-payload.json
  env:
    LEKTON_URL: ${{ secrets.LEKTON_URL }}
```

## Next Steps

- Read the [Architecture Overview](/docs/architecture) to understand how Lekton works
- Check the [API Reference](/docs/api-reference) for full API documentation
- Review [Security & RBAC](/docs/security-rbac) for access control details
