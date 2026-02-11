# Deployment Guide

This guide covers deploying Lekton in production environments using Docker and Kubernetes.

## Prerequisites

- Docker 24+ and Docker Compose v2
- MongoDB 7+
- S3-compatible storage (AWS S3, MinIO, Garage)
- OIDC identity provider (Keycloak, Auth0, Okta)

## Docker Compose Deployment

The simplest way to deploy Lekton:

```bash
# Clone the repository
git clone https://github.com/dghilardi/lekton.git
cd lekton

# Start all services
docker compose up -d

# Check service health
docker compose ps
```

### Environment Variables

| Variable            | Required | Description                        |
|---------------------|----------|------------------------------------|
| `MONGODB_URI`       | Yes      | MongoDB connection string          |
| `S3_BUCKET`         | Yes      | S3 bucket for content storage      |
| `S3_ENDPOINT`       | No       | Custom S3 endpoint (Garage, MinIO) |
| `AWS_ACCESS_KEY_ID` | Yes      | S3 access key                      |
| `AWS_SECRET_ACCESS_KEY` | Yes  | S3 secret key                      |
| `SERVICE_TOKEN`     | Yes      | Token for ingestion API auth       |
| `DEMO_MODE`         | No       | Enable demo auth (never in prod!)  |
| `RUST_LOG`          | No       | Log level filter                   |

## Kubernetes Deployment

### Namespace Setup

```bash
kubectl create namespace lekton
```

### ConfigMap

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: lekton-config
  namespace: lekton
data:
  MONGODB_URI: "mongodb://mongo-svc.lekton:27017"
  S3_BUCKET: "lekton-docs"
  S3_ENDPOINT: "http://garage-svc.lekton:3900"
  RUST_LOG: "lekton=info"
```

### Deployment

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: lekton
  namespace: lekton
spec:
  replicas: 2
  selector:
    matchLabels:
      app: lekton
  template:
    metadata:
      labels:
        app: lekton
    spec:
      containers:
        - name: lekton
          image: ghcr.io/dghilardi/lekton:latest
          ports:
            - containerPort: 3000
          envFrom:
            - configMapRef:
                name: lekton-config
            - secretRef:
                name: lekton-secrets
          readinessProbe:
            httpGet:
              path: /
              port: 3000
            initialDelaySeconds: 5
            periodSeconds: 10
```

## Health Checks

Lekton exposes the following endpoints for monitoring:

- `GET /` — Returns 200 when the application is ready
- `GET /api/auth/me` — Validates auth layer is functional

## Upgrading

1. Pull the latest image: `docker compose pull lekton`
2. Restart: `docker compose up -d lekton`
3. Verify health: `curl http://localhost:3000`

> **Note:** Database migrations are handled automatically on startup.

## Troubleshooting

### Common Issues

**Container fails to start:**
- Verify MongoDB is reachable: `mongosh $MONGODB_URI`
- Verify S3 is reachable: `curl $S3_ENDPOINT`
- Check logs: `docker compose logs lekton`

**Documents not appearing:**
- Verify the ingest API is working: check `docker compose logs demo-loader`
- Verify the service token matches: compare `SERVICE_TOKEN` env var
