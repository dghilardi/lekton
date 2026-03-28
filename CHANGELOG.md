# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added — `lekton-sync` CLI

- **`lekton-sync` CLI** (`cli/`): standalone binary that acts as the CI-side client for the Lekton ingestion API. Accepts `LEKTON_TOKEN` and `LEKTON_URL` environment variables plus a root path argument. Scans all `.md` files in the tree, reads YAML front matter (`title`, `slug`, `access_level`, `service_owner`, `tags`, `order`, `is_hidden`), computes SHA-256 content hashes, calls `POST /api/v1/sync` to get the delta, then calls `POST /api/v1/ingest` only for documents that need uploading. Supports a `.lekton.yml` project config file (server URL, default access level, service owner, slug prefix, `archive_missing` flag). Flags: `--archive-missing`, `--dry-run`, `--verbose`, `--config`. Files without a `title` or `slug` in their front matter are silently skipped. 9 unit tests covering hashing, front matter parsing, path-to-slug derivation, and file scanning.

## [0.5.1] 2026-03-28

### Fixed

- **Direct document access enforces access control**: `get_doc_html` now checks the caller's permissions before returning document content. Previously a user who knew a document's slug could access restricted content directly by URL, bypassing the navigation and search filters. Unauthorized access returns `None` (→ 404) to avoid leaking the existence of restricted documents. Draft documents are also gated by `include_draft` permission.
- **Archived documents deindexed from search**: When the sync API archives a document (`archive_missing: true`), it now calls `delete_document` on the search service so the document is removed from the Meilisearch index immediately. Previously archived documents remained searchable indefinitely.

## [0.5.0] 2026-03-28

### Added — CI-Driven Document Sync

- **Scoped service tokens**: Per-pipeline API keys with `allowed_scopes` (exact slugs or prefix patterns like `protocols/*`). Scope overlap between tokens is rejected at creation time. Replaces the single global `SERVICE_TOKEN` for fine-grained access control while preserving backward compatibility via legacy token fallback.
- **Admin token management**: `POST /api/v1/admin/service-tokens` creates a scoped token (raw value returned once), `GET /api/v1/admin/service-tokens` lists all tokens (hash never exposed), `DELETE /api/v1/admin/service-tokens/{id}` deactivates a token. All endpoints require admin authentication.
- **Content hashing**: SHA-256 hash (`sha256:<base64url>`) computed and stored on every document. Ingest API skips S3 upload when content is unchanged, and skips DB update too when metadata also matches. `IngestResponse` gains a `changed` boolean field.
- **Sync API**: `POST /api/v1/sync` accepts a list of `{slug, content_hash}` entries from the CI client and returns `{to_upload, to_archive, unchanged}`. Supports `archive_missing: true` to automatically soft-archive documents removed from the source repository. Token scopes are validated for all slugs in the request.
- **Document versioning**: When content changes during ingest, the previous version is copied to `docs/history/{slug}/{version}.md` in S3 and a `DocumentVersion` record (slug, version number, content hash, updated_by) is stored in the `document_versions` MongoDB collection. Version numbers auto-increment per slug.
- **Document archival**: `is_archived` field on documents, used by the sync API for soft-deleting documents no longer present in the source repo.
- **Admin settings page**: New `/admin/settings` page (admin-only, with sidebar link) for managing service tokens via the UI. Token list table with scopes, permissions, status, and deactivate button. Create form with name, scopes (one per line), and write permission toggle. One-time raw token display modal with clipboard copy.
- **Tests**: 30+ new unit tests covering scope matching, scope overlap detection, scoped/legacy token validation, content hash diffing, sync scenarios (upload/unchanged/archive), and token lifecycle.

## [0.4.2] 2026-03-28

## [0.4.1] 2026-03-28
## [0.4.0] - 2026-02-21

### Added — Phase 4: Theme, Polish & Accessibility

- **Dark/Light/System theme toggle**: Three-mode theme switcher in the navbar cycling system → light → dark. Persists user preference in `localStorage`. Inline `<head>` script prevents flash of unstyled content (FOUC) by applying saved theme before first paint. System mode respects OS `prefers-color-scheme` media query. Icons: sun (light), moon (dark), monitor (system).
- **Runtime CSS injection**: `SettingsRepository` trait with `MongoSettingsRepository` storing application settings in a `settings` MongoDB collection. `GetCustomCss`/`SaveCustomCss` server functions enable reading and writing custom CSS at runtime. `RuntimeCustomCss` component injects stored CSS as a `<style>` tag in the layout, allowing theme overrides without recompilation.
- **Document metadata display**: Document pages now show "Last Updated" timestamps at the bottom with a clock icon and divider. Dates formatted as human-friendly strings (e.g., "February 21, 2026"). Document tags displayed as badge pills below the title.
- **DocPageData struct**: Replaced tuple-based return from `get_doc_html` with a proper `DocPageData` struct carrying title, HTML, TOC headings, last_updated, and tags.
- **Tests**: 78 unit tests (2 new for settings). 9 new integration tests covering settings CRUD (default, set/get, update, clear) and document metadata (tags storage, timestamp freshness, timestamp refresh, tag replacement).

## [0.3.0] - 2026-02-21

### Added — Phase 3: Advanced Schema Registry

- **Schema Repository**: `SchemaRepository` trait with `MongoSchemaRepository` implementation backed by the `schemas` MongoDB collection. Supports CRUD operations: create/update, find by name, list all, add version, and delete.
- **Schema Ingestion API**: `POST /api/v1/schemas` endpoint for CI/CD-driven schema registration. Validates service tokens, schema types (openapi, asyncapi, jsonschema), version status (stable, beta, deprecated). Auto-detects JSON vs YAML content for S3 storage. Supports adding new versions to existing schemas and updating existing versions.
- **Schema Retrieval APIs**: `GET /api/v1/schemas` lists all schemas with latest version info. `GET /api/v1/schemas/:name` returns schema details with all versions. `GET /api/v1/schemas/:name/:version` returns raw spec content from S3.
- **Interactive OpenAPI Viewer**: Schema viewer page renders OpenAPI specifications using Scalar (loaded from CDN) for interactive API reference documentation with try-it-out functionality.
- **AsyncAPI Viewer**: AsyncAPI specifications rendered using AsyncAPI-React standalone component for event-driven API documentation.
- **JSON Schema Viewer**: JSON Schema displayed as formatted, syntax-highlighted code blocks.
- **Dynamic Version Selector**: Dropdown component to switch between different versions of a schema. Auto-selects latest stable version on page load. Version status badges (stable/beta/deprecated) shown for all versions.
- **Schema Registry UI**: Grid-based schema list page with cards showing schema name, type badge, version count, and latest version. Schema viewer page with breadcrumbs, version selector, and spec viewer.
- **Navigation**: Added "API Schemas" section in the sidebar with link to Schema Registry. Added `/schemas` and `/schemas/:name` routes.
- **Tests**: 76 unit tests (13 new) covering schema ingestion, validation, listing, retrieval, and content fetching. 12 new integration tests using testcontainers covering the full schema lifecycle.

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
