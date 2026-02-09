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

## 📅 Roadmap

### Phase 1: Core (In Progress)
- [ ] Axum + Leptos scaffold
- [ ] OIDC Authentication
- [ ] Ingestion API (`POST /ingest`)
- [ ] Basic GFM rendering

### Phase 2: Editor & Search
- [ ] Tiptap-based Web Editor
- [ ] Meilisearch integration
- [ ] Link validation engine

### Phase 3: Registry & Versioning
- [ ] OpenAPI/AsyncAPI visualization (Scalar/Redoc)
- [ ] Versioning support for schemas and docs

## 🤝 Contributing

We welcome contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for details on our code of conduct and the process for submitting pull requests.

## 📝 License

Distributed under the MIT License. See `LICENSE` for more information.
