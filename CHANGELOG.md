# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added
- `rag-eval` binary: offline retrieval evaluation harness that reads a JSONL eval set, runs the production retrieval pipeline against an already-indexed Qdrant collection, and reports Recall@k, MRR and nDCG@k for both the pre-rerank and post-rerank candidate sets. Run with `cargo run --bin rag-eval --features ssr --no-default-features -- --queries eval/queries.jsonl`. A starter eval set against the demo corpus is included at `eval/queries.jsonl`.
- Per-sub-query, pre-rerank and post-rerank chunk-id logging in the RAG retrieval pipeline (filterable by `session_id`) for triaging individual chat retrievals.
- HyDE (Hypothetical Document Embeddings) in RAG chat: an LLM generates a synthetic answer document whose embedding is used in place of the raw query embedding, improving recall when query phrasing differs from documentation style. Enable with `rag.hyde_model`. Falls back to original query on error.
- `rag.analyzer_url` and `rag.hyde_url` allow routing analyzer/HyDE steps to a dedicated endpoint (e.g. local Ollama) independently from the main `chat_url`.
- Optional `infinity` service in `docker-compose.yml` serving `BAAI/bge-reranker-v2-m3` on port 7997, for the cross-encoder reranker in dev.
- Query decomposition in RAG chat: an LLM classifier detects multi-entity and multi-hop queries, splits them into atomic sub-queries, and runs parallel vector searches. Enable with `rag.analyzer_model`. Falls back to simple retrieval on error.
- Cross-encoder reranker in RAG chat: retrieved chunks are re-scored by a cross-encoder model (Jina/Infinity/Cohere-compatible API) before being passed to the LLM. Enable with `rag.reranker_url`. Falls back to retrieval order on error.
- Hybrid search in RAG chat: Meilisearch BM25 results are fused with Qdrant vector results using Reciprocal Rank Fusion (RRF). Enable with `rag.hybrid_search_enabled = true` (requires Meilisearch configured).
- `lekton-sync` now supports schema manifests (`lekton.schema.yml`) for OpenAPI, AsyncAPI, and JSON Schema artifacts, with delta sync via `POST /api/v1/schemas/sync`.
- `cargo-deny` configuration for license compliance (AGPL-3.0-compatible allowlist) and RustSec advisory auditing, with weekly CI workflow
- Clippy CI job enforcing zero warnings on both SSR and hydrate targets (`-D warnings`)
- `#[forbid(unsafe_code)]` crate-level attribute on both `lekton` and `lekton-sync`

### Changed
- README now documents the optional local setup for hybrid search, reranking, query decomposition, HyDE, and query rewriting in development.
- Update safe dependencies: async-openai 0.35, pulldown-cmark 0.13, rand 0.9, sha2 0.11, text-splitter 0.30, gloo-timers 0.4, gloo-net 0.7, mockall 0.14, axum-test 20

### Fixed
- Schema registry metadata now includes per-version `access_level`, RBAC filtering on list/detail/content reads, and archive-missing support for removed schema versions.
- Resolved all clippy warnings across SSR and hydrate targets (unused imports, deprecated APIs, non-idiomatic patterns)
- Replaced `unwrap()` calls in non-test code with safe alternatives (let-else, unwrap_or, if-let)

## [0.14.3] 2026-04-18

### Fixed
- MCP endpoint now supports configurable `allowed_hosts` (`[mcp] allowed_hosts`) to work behind reverse proxies with custom hostnames. Fixes `Forbidden: Host header is not allowed` caused by rmcp 1.5's default DNS rebinding protection.

## [0.14.2] 2026-04-18

### Added
- **Logged-in session cookie** (`lekton_logged_in`): A non-httpOnly indicator cookie is now set alongside the refresh token on login and refresh, enabling dual-mode endpoints (navigation, search, document pages) to distinguish "anonymous visitor" from "logged-in user with expired access token" and return 401 instead of silently falling back to public-only data.

### Changed
- Dual-mode server functions (`get_navigation`, `search_docs`, `get_doc_html`) now return the unauthorized sentinel when the logged-in cookie is present but the JWT is missing/expired, triggering the client-side token refresh flow.
- The refresh endpoint now clears all session cookies (access token, refresh token, logged-in indicator) when the refresh token is expired or revoked, preventing stale session state.

## [0.14.1] 2026-04-18

### Fixed
- Service tokens created via admin API now work with asset endpoints (`check-hashes`, `upload`, `delete`) and schema ingestion. Previously these endpoints only accepted the legacy `LEKTON_SERVICE_TOKEN` env var.

## [0.14.0] 2026-04-18

### Added
- RAG chat responses now expose document source references in the SSE stream, persist them with assistant messages, and render them in session history with RBAC filtering reapplied on replay.
- Added a `lekton-sync-ci` Docker image target based on `debian:bookworm-slim` for Jenkins-style runners that require shell-capable containers, while keeping the default `lekton-sync` image distroless.

### Fixed
- RAG chat now uses the configured `rag.chat_url` for non-Vertex OpenAI-compatible providers and no longer requires `rag.chat_api_key` for local endpoints that do not use authentication.
## [0.13.8] 2026-04-18

### Changed
- Rust code formatting is now enforced across the workspace with a dedicated CI check, and the contributor/agent documentation now explicitly requires running `cargo fmt --all` (or `just fmt`) before merge.

### Fixed
- OAuth/OIDC sessions now perform a silent refresh on app bootstrap when the access-token cookie has expired but the refresh-token cookie is still valid, so reloading the page restores the logged-in state instead of showing the user as anonymous.

## [0.13.7] 2026-04-18

### Changed
- Authentication and API bearer tokens now use 43-character alphanumeric secrets generated from a CSPRNG instead of UUID v4 strings.
- Access JWTs now include `iss`, `aud`, and `nbf` claims, with matching issuer/audience validation driven by typed auth configuration.

## [0.13.6] 2026-04-18

### Fixed
- Tiptap browser assets now load as ES modules from the SSR shell, avoiding local editor boot failures before hydration.

### Added
- **Automatic token refresh with deduplication**: When an access token expires mid-session, the client now detects the 401 sentinel, calls `POST /auth/refresh` once (regardless of how many concurrent requests failed simultaneously), retries the original call, covers authenticated bootstrap and admin/profile/prompt-library data loads, and redirects to `/login` only if the refresh token is also expired or revoked.
- **`auth::refresh_client` module**: New client-only module exposing `with_auth_retry(f)` (retry wrapper), `try_refresh()` (deduplicated refresh), and `is_auth_error(err)` (sentinel detection). In SSR builds the same API compiles as a passthrough so page components need no `#[cfg]` guards.
- **`UNAUTHORIZED_SENTINEL` constant**: Shared constant in `auth::models` used by server helpers to signal "needs refresh" to the client, keeping the server-side emitter and client-side detector in sync.

## [0.13.5] 2026-04-17

### Fixed
- RAG chat SSR builds no longer fail because the streaming response generator captures `&self` while logging the configured chat model.

## [0.13.4] 2026-04-17

### Changed
- RAG chat now emits debug logs for query rewriting, vector-store retrieval, and the prompt/response exchanged with the chat LLM to make the full chain easier to inspect.

## [0.13.3] 2026-04-16

### Fixed
- Vertex AI chat and rewrite failures now surface the provider's actual error message instead of a misleading OpenAI response deserialization error.

## [0.13.2] 2026-04-16

### Fixed
- Install rustls `aws-lc-rs` CryptoProvider at startup to prevent a panic when both `aws-lc-rs` and `ring` are present in the dependency tree (introduced by `gcp_auth`).

### Changed
- `lekton-sync` now recognises document front matter field names written in `kebab-case`, `snake_case`, or `camelCase` for the supported metadata keys.

## [0.13.1] 2026-04-15

### Added
- **Minimal `lekton-sync` Docker image**: Added a dedicated multi-stage `cli/Dockerfile` that builds only the sync CLI and runs it from a small distroless runtime. Tagged releases now also publish a separate `lekton-sync` Docker image, and the CLI docs include CI usage examples.

## [0.13.0] 2026-04-15

### Changed
- **LLM provider factory for chat requests**: RAG chat now initializes a shared LLM provider once at startup from the typed `config-rs` configuration, falls back to OpenRouter for open source deployments, and builds `async-openai` clients per request so Google Cloud Vertex AI access tokens can be refreshed automatically.

## [0.12.1] 2026-04-12

### Fixed
- **Integration test harness aligned with documentation feedback registry**: Updated shared `AppState` test wiring to provide the new `documentation_feedback_repo`, preventing GitHub Actions integration builds from failing after the documentation feedback subsystem was introduced.

## [0.12.0] 2026-04-12

### Changed
- **MCP documentation access model**: The MCP server now exposes documentation as native read-only `docs://...` resources with discovery and direct reads, while semantic search returns matching resource URIs instead of relying on a full-document read tool.
- **MCP documentation tools simplified**: Removed the legacy `search_docs` alias and clarified `get_index` as a compatibility helper rather than the primary discovery path.
- **Documentation feedback registry**: Added a lightweight documentation-feedback subsystem with three MCP tools (`search_documentation_feedback`, `report_missing_documentation`, `propose_documentation_improvement`), MongoDB persistence, and an admin-only UI to review, resolve, and mark duplicate feedback without introducing full ticket management. The admin view now handles sparse records, multiline queries, long identifiers/URIs, and filter/action alignment more robustly on smaller layouts.

## [0.11.0] 2026-04-11

### Added
- **Prompt Library foundations**: Added backend domain models and repository traits for prompts, prompt version history, and per-user prompt preferences. The new prompt model includes MCP publication metadata (`publish_to_mcp`, `default_primary`, `context_cost`) to support a future split between prompt library discovery and directly published context prompts.
- **Prompt ingest and sync API**: Added `POST /api/v1/prompts/ingest` and `POST /api/v1/prompts/sync` with scoped service-token validation, content/metadata hashing, YAML blob storage in S3, version archiving on body changes, and archive-missing sync behavior aligned with document ingestion.
- **Prompt MCP tools**: Extended the MCP server with `list_prompts`, `get_prompt`, `search_prompts`, and `get_context_prompts`. The context tool combines published primary prompts with per-user favorites, excludes hidden defaults, applies RBAC, and emits warnings when the estimated prompt context cost grows too large. The effective context prompt set is also published as native MCP prompts for prompt-aware clients.
- **Prompt Library UI**: Added the `/prompts` page with per-user favorites and hidden-primary toggles, shared context-cost warnings, and a navbar/user-menu entry to manage published prompt context preferences.
- **Demo prompt content**: Demo mode now loads a small prompt library so the UI and MCP features can be exercised end-to-end without extra setup. The demo dataset includes prompts for code review, architecture analysis, and git history sanitization.
- **`lekton-sync` prompt support**: The CLI now scans prompt YAML files, computes prompt content/metadata hashes, calls the prompt sync API, and uploads changed prompts alongside markdown documents. New `.lekton.yml` options (`prompts_dir`, `prompt_slug_prefix`) control prompt discovery and slug generation.

## [0.10.0] 2026-04-10
### Added
- **Embedding cache**: chunk embeddings are now cached in a new MongoDB `embedding_cache` collection, keyed on `(sha256(normalised_text), model)`. Only missing embeddings are forwarded to the embedding service; hits are returned directly. Two optional config flags (default `false`): `rag.embedding_cache_store_text` persists the original chunk text alongside the vector for debugging, `rag.embedding_cache_query` extends caching to chat-query embeddings in addition to chunk embeddings.
- **Custom LLM headers**: `rag.chat_headers` and `rag.embedding_headers` config maps allow injecting arbitrary HTTP headers into every chat/rewrite and embedding request respectively. Keys are normalised at request time: underscores (`_`) are replaced with hyphens (`-`), enabling hyphenated header names (e.g. `x-producer`) to be set via environment variables (`LKN__RAG__CHAT_HEADERS__X_PRODUCER=LEKTON`). TOML files can use quoted keys to set hyphens directly.
- **AI response feedback**: Users can give a thumbs-up or thumbs-down on each assistant message in the chat. Negative feedback shows an optional free-text comment box. The selected rating is persisted immediately; clicking the active button removes the feedback. A small badge below each rated message shows the current rating with an × to remove it.
- **Feedback history in `/profile`**: New section at the bottom of the profile page lists all feedback the user has submitted, newest first, with pagination (10 per page). Each item shows the rating badge, date, optional comment, a "View session" link, and a delete button.
- **Admin feedback export API**: `GET /api/v1/admin/rag/feedback` — paginated, filterable list of all feedback across users. Supports query parameters: `rating` (`positive` | `negative`), `date_from` / `date_to` (RFC 3339), `user_id`, `page` (0-based), `per_page` (max 200, default 50). Callable via Bearer PAT with admin scope.
- New REST endpoints: `POST /api/v1/rag/messages/{id}/feedback` (create/update), `DELETE /api/v1/rag/messages/{id}/feedback` (remove).
- `GET /api/v1/rag/sessions/{id}/messages` now includes `id` and `feedback` fields per message so the chat UI can restore feedback state when loading a previous session.
- `ChatEvent::Done` now carries an optional `message_id` so the client immediately knows the server-assigned ID of the saved assistant message and can attach feedback without reloading the session.
- New `src/db/feedback_repository.rs`: `FeedbackRepository` trait and `MongoFeedbackRepository` implementation. Feedback is stored in the `message_feedback` collection with upsert semantics (one entry per user + message pair). Supports paginated queries with rating, date-range, and user filters.
- `MessageFeedback` model and `FeedbackRating` enum added to `src/db/chat_models.rs`.
- `feedback_repo: Option<Arc<dyn FeedbackRepository>>` added to `AppState`; initialised alongside `chat_repo` when RAG is enabled.
- `ChatRepository::get_message_by_id` added for ownership validation in the feedback submit endpoint.
- `list_user_feedback` and `delete_user_feedback` server functions (Leptos `#[server]`) for the profile history page.

## [0.9.1] 2026-04-09

### Fixed
- **E2E CI timeout**: Run pre-built binary directly in CI instead of `cargo leptos serve --release`, which redundantly recompiled the entire project and exceeded the Playwright 180s timeout.

## [0.9.0] 2026-04-08

### Added
- **PAT self-service management**: Users can create, toggle, and delete their own Personal Access Tokens from the new `/profile` page. The raw token is shown once after creation, with a ready-to-use `claude mcp add-json` command snippet. "Profile & Tokens" link added to the user menu dropdown.
- **Admin PAT overview**: New admin section at `/admin/pats` — paginated table of all PATs across users with user email resolution, last-used timestamp, and per-token activate/deactivate. Accessible from the admin sidebar.
- New REST endpoints: `GET/POST /api/v1/user/pats`, `PATCH/DELETE /api/v1/user/pats/{id}`, `GET /api/v1/admin/pats`, `PATCH /api/v1/admin/pats/{id}`.
- `ServiceTokenRepository` extended with `set_active`, `list_by_user_id`, `list_pats_paginated`, and `delete_pat` (ownership-checked hard delete).
- Admin-PAT support: PATs with `user_id = null` are treated as admin tokens with full access, enabling machine-to-machine integrations without requiring a linked user account (useful in demo mode).
- **MCP server (Model Context Protocol)**: Expose Lekton documentation to IDE agents (Claude Code, Cursor, RooCode) via the Streamable HTTP transport (`POST /mcp`). Authenticated with Personal Access Tokens (PAT) stored in the `service_tokens` collection. Three tools are available:
  - `get_index`: Returns the document tree with slugs, titles, hierarchy, and tags visible to the authenticated user.
  - `search_docs`: Semantic search via Qdrant vector store with access-level filtering, returns text fragments with source document slugs.
  - `read_document`: Retrieves the full Markdown content of a document by slug, with access control enforcement.
- New `src/mcp/` module: `auth.rs` (PAT middleware), `server.rs` (MCP tool definitions using `rmcp`).
- `ServiceToken` model extended with `token_type` (`"service"` | `"pat"`) and `user_id` fields. PAT tokens inherit the linked user's RBAC permissions. Backwards-compatible with existing service tokens via `serde(default)`.
- New dependencies: `rmcp` (MCP Rust SDK with streamable HTTP transport), `schemars` (JSON Schema generation for tool parameters).
- **RAG query rewriting**: Conditional standalone-question generation for multi-turn conversations. When `rewrite_model` is configured, follow-up questions are rewritten by an LLM into self-contained queries before computing embeddings, improving vector-search relevance for elliptic or anaphoric inputs. Falls back transparently to the original message on the first turn or when the feature is disabled (`rewrite_model = ""`).
- New `RagConfig` fields: `rewrite_model` (empty = disabled) and `rewrite_max_tokens` (default 80). Both configurable via `LKN__RAG__REWRITE_MODEL` / `LKN__RAG__REWRITE_MAX_TOKENS` environment variables.
- `src/rag/query_rewriter.rs`: `QueryRewriter` struct with unit-tested `format_history` windowing (last 6 messages) and graceful degradation on empty LLM response.

## [0.8.1] 2026-04-07

## [0.8.0] 2026-04-07

### Added
- **RAG (Retrieval-Augmented Generation) integration**: Optional feature that connects to external embedding and chat providers (Ollama, OpenRouter, etc.) and Qdrant vector database. When configured, documents are automatically chunked, embedded and indexed during ingestion. Disabled by default — enable via `[rag]` config section with `qdrant_url` and `embedding_url`.
- **RAG Chat**: Streaming multi-turn chat API (`POST /api/v1/rag/chat`) with SSE, filtered by user's access levels. Conversations are persisted in MongoDB (`chat_sessions` / `chat_messages` collections) with session management endpoints (`GET /api/v1/rag/sessions`, `DELETE /api/v1/rag/sessions/{id}`).
- **RAG Admin Re-index**: Background re-embedding of all documents via `POST /api/v1/admin/rag/reindex` with progress tracking (`GET /api/v1/admin/rag/reindex/status`). Prevents concurrent runs via CAS.
- **Chat page** (`/chat`): Leptos chat UI with DaisyUI chat bubbles, streaming token display, session sidebar. Visible only when RAG is enabled and user is authenticated.
- **Admin re-index panel**: Progress bar and trigger button in admin settings page, with auto-polling during re-index.
- **Configurable system prompt**: Tera-templated system prompt for the RAG chat, with `{{context}}` and `{{question}}` variables.
- **New dependencies**: `qdrant-client`, `async-openai` (embedding + chat-completion), `text-splitter` (markdown), `tera`, `async-stream`, `serde-wasm-bindgen`, `gloo-timers`.
- **Centralised configuration via `config` crate**: All runtime settings are now loaded in priority order — embedded `config/default.toml` defaults, optional `config/lekton.toml` local override (git-ignored), and `LKN_*` environment variables (e.g. `LKN_DATABASE__URI`, `LKN_AUTH__JWT_SECRET`). Replaces the previous ad-hoc `std::env::var` calls scattered across modules.
- **`AppConfig` struct** (`src/config.rs`): Typed configuration with sections `server`, `database`, `storage`, `search`, and `auth`.
- **`insecure_cookies` and `max_attachment_size_bytes` fields on `AppState`**: cookie security and upload limits are now driven by config rather than per-request env reads.

### Changed
- `auth::config::AuthProviderConfig::from_env()` replaced by `from_app_config(&AuthConfig)`.
- `auth::token_service::TokenService::from_env()` replaced by `from_app_config(&AuthConfig)`.
- `auth::provider::build_provider_from_env()` renamed to `build_provider(&AuthConfig)`.
- `storage::client::S3StorageClient::from_env()` replaced by `from_app_config(&StorageConfig)`.
- `search::client::MeilisearchService::from_env()` replaced by `from_app_config(&SearchConfig)`.
- `api::assets::process_upload_asset` now accepts an explicit `max_size: u64` parameter instead of reading `MAX_ATTACHMENT_SIZE_MB` from the environment at call time.
- Cookie builder functions (`access_token_cookie`, `refresh_token_cookie`, `auth_state_cookie`) now accept an explicit `secure: bool` parameter instead of reading `INSECURE_COOKIES` from the environment internally.
- `.env.example` updated to use `LKN_*` prefix for all application settings.

### Fixed
- **lekton-sync: attachment changes always detected**: Attachment hashes are now checked for every document on each sync run, not only those already flagged for upload. Replacing a PDF or image with new content while leaving the markdown body unchanged is now correctly detected and re-uploaded.
- **lekton-sync: metadata-only changes trigger re-upload**: Changing front-matter fields (`access_level`, `title`, `service_owner`, `tags`, `parent_slug`, `order`, `is_hidden`) previously left the content hash identical, causing the document to be silently skipped. A separate `metadata_hash` is now computed from the canonical metadata and compared during sync. Documents stored before this version are treated as having no metadata hash and are re-uploaded once so their metadata hash gets populated.

## [0.7.2] 2026-04-04

## [0.7.1] 2026-04-04

### Fixed

- **E2E tests aligned with navigation redesign**: Updated all Playwright specs to match the new navbar/sidebar architecture introduced in navigation-ordering. Tests no longer rely on `<details>` elements on the home page or click-navigation through WASM-rendered links. Replaced with direct URL navigation and increased timeouts for WASM hydration in CI release builds.
- **CI wasm-bindgen version mismatch**: Pinned `wasm-bindgen-cli` installation in CI workflow to match the project's dependency version (0.2.117), preventing build failures from version drift.

## [0.7.0] 2026-04-04

### Added

- **Configurable OAuth2 userinfo field mapping**: New environment variables (`AUTH_USERINFO_SUB_FIELD`, `AUTH_USERINFO_EMAIL_FIELD`, `AUTH_USERINFO_NAME_FIELD`) allow dot-notation paths to extract user identity from non-standard OAuth2 provider responses. Supports nested fields (e.g. `data.loginEmail`) and comma-separated paths for name concatenation (e.g. `data.firstName,data.lastName`). Falls back to standard OIDC fields (`sub`, `email`, `name`) when unset.

### Fixed

- **OAuth2/OIDC login not shown in frontend**: Login page and user menu now detect whether the app is in demo mode or OAuth mode. In OAuth mode, clicking "Log In" redirects directly to the external identity provider instead of showing the demo username/password form.

### Changed

- **Updated `.env.example`**: Auth configuration section now reflects the actual environment variables (`AUTH_PROVIDER_TYPE`, `AUTH_CLIENT_ID`, etc.) instead of the stale `OIDC_*` placeholders.

### Added

- **Navigation ordering**: Sections and categories in the sidebar and navbar are now sorted deterministically — alphabetically by title by default, with support for custom ordering via a dedicated `navigation_order` MongoDB collection. Documents (leaves) continue to sort by their `order` field, then alphabetically.
- **Navigation ordering admin UI**: New "Navigation Ordering" section in Admin Settings with drag-and-drop reordering of sections and categories. Includes up/down arrow buttons as an alternative, per-level indentation, and save/discard controls.
- **`navigation_order` collection**: New MongoDB collection storing per-slug weights for custom section/category ordering. Managed via `get_navigation_order` / `save_navigation_order` admin-only server functions.

### Fixed

- **Non-deterministic navigation order**: Sections and categories no longer shuffle on page refresh. The root cause was `HashMap::into_iter()` returning items in arbitrary order during navigation tree construction.

### Added

- **Local attachment sync**: `lekton-sync` now detects local file references in markdown (`![](path)`, `[](path)`, `<img src="path">`) and automatically uploads them as assets before ingesting the document. Paths are rewritten in the uploaded content to server URLs (`/api/v1/assets/attachments/{slug}/{filename}`), while local files remain untouched. Supports all relative paths including `../`. Configurable via `max_attachment_size_mb` in `.lekton.yml` (default: 10 MB). Dry-run mode shows attachment upload plan.
- **Asset content hash deduplication**: `POST /api/v1/assets/check-hashes` endpoint accepts a list of asset keys with their SHA-256 hashes and returns which ones need uploading. Used by `lekton-sync` to skip unchanged attachments.
- **Server-side attachment size limit**: `MAX_ATTACHMENT_SIZE_MB` environment variable (default: 25 MB) rejects oversized asset uploads with a clear error message.
- **`content_hash` field on Asset model**: SHA-256 hash stored on every asset upload for deduplication support.

### Changed

- **`lekton-sync` requires `lekton-import: true`**: only files with this flag in their YAML front matter are synced. Prevents accidental ingestion of READMEs, dependency docs, or other non-portal markdown files.

## [0.6.2] 2026-04-01

### Added

- **`MONGODB_USERNAME` / `MONGODB_PASSWORD` env vars**: MongoDB credentials can now be provided as separate environment variables in addition to (or instead of) embedding them in `MONGODB_URI`. When both are set, they are percent-encoded and injected into the URI after the scheme, replacing any existing inline credentials.

## [0.6.1] 2026-03-28

## [0.6.0] 2026-03-28

### Added — `lekton-sync` CLI

- **`lekton-sync` publish workflow**: `docker-publish.yml` now has a `publish-cli` job (requires `needs: publish`) that runs `cargo publish -p lekton-sync` after a successful Docker Hub push. Requires a `CARGO_REGISTRY_TOKEN` secret in the repository settings.
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
