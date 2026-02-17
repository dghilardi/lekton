# Changelog

All notable changes to this project will be documented in this file.

## [0.2.0] - 2026-02-17

### Added — Phase 2: The Editor & Search

- **Link extraction & validation**: AST-based internal link extraction from markdown using `pulldown-cmark`. `extract_internal_links()` parses documents and returns normalized slugs. `validate_links()` checks extracted links against the document repository.
- **Backlink tracking**: `DocumentRepository::update_backlinks()` maintains bidirectional link graphs in MongoDB. The ingestion pipeline now populates `links_out` and updates `backlinks` on referenced documents automatically.
- **Meilisearch integration**: Full-text search via `meilisearch-sdk`. `SearchService` trait with `MeilisearchService` implementation. Documents are indexed on ingestion with searchable attributes (title, content preview, slug, tags) and filterable attributes (access level, service owner).
- **Tenant token generation**: RBAC-scoped Meilisearch tenant tokens via `jsonwebtoken`. Tokens embed `searchRules` filters based on user access level.
- **Search API**: `GET /api/v1/search?q=<query>&access_level=<level>` endpoint with RBAC filtering.
- **Tiptap WYSIWYG editor**: Rich text editor via `leptos-tiptap` with toolbar (bold, italic, strike, headings, lists, blockquote, highlight). Editor loads existing document content from S3, converts markdown to HTML, and saves back via server functions.
- **Image upload**: `POST /api/v1/upload-image` endpoint for multipart image uploads to S3. `GET /api/v1/image/:filename` serves uploaded images.
- **Functional DocPage**: Document viewer now fetches real content from S3 and renders markdown. Includes an "Edit" button linking to the editor.
- **Search UI**: Reactive search bar in the navbar with live dropdown results from Meilisearch.
- **Docker Compose**: Added Meilisearch service with health check, persistent volume, and environment configuration.
- **Tests**: 56 unit tests (25 new) covering link extraction, markdown preview stripping, search document building, and tenant token generation.

## [0.1.0] - 2026-02-10

### Added — Phase 1: The Core (MVP)

- **Project scaffold**: Leptos 0.8 + Axum SSR application with `cargo-leptos` build system.
- **Design system**: Tailwind CSS v4 and DaisyUI 5 integration with CSS-first configuration.
- **Runtime customizability**: `public/custom.css` allows users to inject custom styles without recompiling. CSS custom properties (`--lekton-*`) provide override hooks for fonts, spacing, and layout.
- **Application shell**: DaisyUI-styled layout with responsive navbar, collapsible sidebar, and content area.
- **OIDC Authentication**: Configuration and middleware for OIDC-based authentication with role mapping.
- **RBAC model**: `AccessLevel` enum (`Public`, `Developer`, `Architect`, `Admin`) with ordered comparisons for granular access control.
- **MongoDB integration**: Document and Schema data models matching the requirements. `DocumentRepository` trait with MongoDB implementation supporting upsert, slug lookup, and RBAC-filtered listing.
- **S3 storage**: `StorageClient` trait with S3 implementation for blob storage. Supports custom endpoints (MinIO, LocalStack).
- **Ingestion API**: `POST /api/v1/ingest` endpoint for CI/CD-driven documentation updates. Validates service tokens, parses access levels, uploads to S3, and upserts MongoDB metadata.
- **Markdown rendering**: GFM-compatible renderer using `pulldown-cmark` with support for tables, task lists, strikethrough, footnotes, and code blocks.
- **Error handling**: Centralized `AppError` type with HTTP status code mapping.
- **Tests**: 31 unit tests covering auth models, RBAC logic, data model serialization, ingestion workflows (success, auth failure, validation, upsert), and markdown rendering.
- **Documentation**: Updated README with getting started guide, configuration table, and customizability section.
