# Lekton

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-stable-brightgreen.svg)](https://www.rust-lang.org)

**Lekton** is a high-performance, dynamic Internal Developer Portal (IDP) designed to replace static documentation generators. Built with Rust, it prioritizes speed, granular security (RBAC), and a seamless developer experience.

## 🚀 Vision

Lekton decouples content from code, allowing microservices to push their documentation and API schemas (OpenAPI/AsyncAPI) to a centralized, searchable, and secure hub. No more full site rebuilds for a typo fix.

## ✨ Key Features

-   **Dynamic Ingestion:** CI/CD integration for live documentation updates.
-   **Granular RBAC:** Server-level Role-Based Access Control for sensitive documents.
-   **Unified Schema Registry:** Support for OpenAPI (Swagger), AsyncAPI, and JSON Schema with versioning.
-   **Blazing Fast:** SSR powered by Leptos and Axum. 
-   **Intelligent Search:** Powered by Meilisearch with tenancy protection.
-   **Interactive Editing:** Integrated WYSIWYG/Markdown editor with link validation.

## 🛠️ Technology Stack

-   **Frontend:** [Leptos](https://leptos.dev/) (Rust, SSR/Hydration)
-   **Backend:** [Axum](https://github.com/tokio-rs/axum) (Rust API)
-   **Database:** [MongoDB](https://www.mongodb.com/) (Metadata & RBAC)
-   **Storage:** S3 Compatible (Markdown & Schema artifacts)
-   **Search:** [Meilisearch](https://www.meilisearch.com/)

## 🏗️ Architecture

Lekton follows a Headless CMS architecture:
-   **Storage Layer:** S3 for content, MongoDB for metadata.
-   **Service Layer:** Axum handles auth, ingestion, and search scoped by user roles.
-   **Presentation Layer:** Leptos for high-performance rendering.

## 🚀 Phase Status

### Phase 1: Core [COMPLETED]
- [x] Axum + Leptos scaffold
- [x] OIDC Authentication
- [x] Ingestion API (`POST /ingest`)
- [x] Basic GFM rendering

### Phase 2: Editor & Search [COMPLETED]
- [x] Tiptap-based Web Editor
- [x] Meilisearch integration
- [x] Link validation engine

## 🏁 Getting Started

### 1. Prerequisites
- [Rust](https://www.rust-lang.org/tools/install)
- [Docker & Docker Compose](https://docs.docker.com/compose/install/)

### 2. Infrastructure Setup
Start the required infrastructure (MongoDB, Minio/S3, Meilisearch):
```bash
docker-compose up -d
```

### 3. Environment Configuration
Copy the example environment file and fill in your OIDC credentials:
```bash
cp .env.example .env
```
> [!IMPORTANT]
> You must provide valid `OIDC_CLIENT_ID` and `OIDC_CLIENT_SECRET` for the application to start.

### 4. Running the Project
You can run the project in two ways:

#### Option A: Using `cargo-leptos` (Recommended for Dev)
```bash
cargo install cargo-leptos
cargo leptos watch
```

#### Option B: Using standard `cargo`
```bash
cargo run --features ssr
```

## 🏗️ Architecture

We welcome contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for details on our code of conduct and the process for submitting pull requests.

## 📝 License

Distributed under the MIT License. See `LICENSE` for more information.
