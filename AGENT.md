# Agent Guidelines (AGENT.md)

Welcome, fellow Agent. This document provides context and standards for AI coding assistants working on **Lekton**.

## 🎯 Project Context
Lekton is a dynamic IDP built with **Leptos (frontend)** and **Axum (backend)**. It aims to solve the maintenance pain of static docs by providing a dynamic, RBAC-aware, and searchable portal.

## 🛠️ Tech Stack & Patterns
-   **Rust:** Use stable toolchain (edition 2021). Prefer `tokio` for async.
-   **Frontend:** Leptos 0.8 (Hydration/SSR). Use `leptos-router`. Follow idiomatic component structure.
-   **Backend:** Axum 0.8. Use `tower` middleware.
-   **Styling:** Tailwind CSS v4 (CSS-first config) + DaisyUI 5. No `tailwind.config.js` — configuration is in `style/tailwind.css`.
-   **Build:** `cargo-leptos` for coordinated SSR + WASM builds with Tailwind integration.
-   **DB:** MongoDB. Use `mongodb` crate with typed models. Business logic behind traits for testability.
-   **Storage:** S3-compatible blob storage via `aws-sdk-s3`. Behind `StorageClient` trait.
-   **Error Handling:** Use `thiserror` for library errors and `anyhow` for application-level logic.
-   **Customizability:** Users can inject custom CSS via `public/custom.css` without recompilation. Design tokens use CSS custom properties (`--lekton-*`).

## 🏗️ Architecture Standards
1.  **Strict Typing:** Ensure all API boundaries are strictly typed.
2.  **RBAC First:** Every new endpoint must verify `access_level` using the `AccessLevel` enum.
3.  **Trait-Based Services:** Database and storage access must be behind traits (`DocumentRepository`, `StorageClient`) to enable mock testing.
4.  **Documentation:** Keep `docs/REQUIREMENTS.md` and `docs/ADRs/` updated with major design changes.
5.  **Trunk-Based Development:** Prefer small, frequent commits to the main branch. Use feature flags for long-running changes.

## 📁 Project Structure
```
src/
├── app.rs          # Leptos root component, router, layout
├── lib.rs          # Library root, module re-exports, WASM hydrate
├── main.rs         # Axum server entry point (SSR feature only)
├── error.rs        # AppError enum (thiserror)
├── auth/           # OIDC authentication & RBAC
│   ├── config.rs   # OidcConfig from env vars
│   ├── middleware.rs # Claims mapping, user extraction
│   └── models.rs   # AccessLevel enum, AuthenticatedUser
├── api/            # REST API handlers
│   ├── errors.rs   # AppError → HTTP response mapping
│   └── ingest.rs   # POST /api/v1/ingest handler
├── db/             # MongoDB models & repository
│   ├── models.rs   # Document, Schema, IngestRequest/Response
│   └── repository.rs # DocumentRepository trait + MongoDocumentRepository
├── storage/        # S3 blob storage
│   └── client.rs   # StorageClient trait + S3StorageClient
└── rendering/      # Content rendering
    └── markdown.rs # GFM markdown → HTML renderer
```

## 📝 Maintenance
-   **Changelog:** Update `CHANGELOG.md` for every significant change.
-   **Tests:** Every feature requires unit tests. Integration tests for API endpoints.
-   **Documentation:** If you change an interface, update the relevant Markdown documentation.

## 🔗 Useful Links
- [Requirements](docs/REQUIREMENTS.md)
- [Contributing Guidelines](CONTRIBUTING.md)
