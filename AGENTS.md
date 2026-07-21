# Agent Guidelines (AGENTS.md)

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
4.  **Auth Refresh Contract:** For client-side calls to authenticated server functions that return the shared unauthorized sentinel, wrap the call with `auth::refresh_client::with_auth_retry(...)`. Do not duplicate ad hoc auth-refresh, retry, or login-redirect logic in page components.
5.  **Centralized Configuration:** New application configuration must be modeled in the typed `config-rs` configuration (`src/config.rs` + `config/default.toml`) and loaded through `AppConfig`. Avoid introducing new scattered `std::env::var` reads in runtime code unless there is a strong, explicit reason.
6.  **Documentation:** Keep `docs/REQUIREMENTS.md` updated with major design changes.
7.  **Trunk-Based Development:** Prefer small, frequent commits to the main branch. Use feature flags for long-running changes.

## 📁 Project Structure
```
src/
├── app.rs          # Leptos root component, router, AppState (derives FromRef)
├── lib.rs          # Library root, module re-exports, WASM hydrate entry
├── main.rs         # Axum server entry point + router assembly (ssr only)
├── config.rs       # Typed config-rs configuration (AppConfig, LKN__ env vars)
├── error.rs        # AppError enum (thiserror)
├── api/            # REST API handlers (ingest, sync, schemas, prompts,
│                   #   assets, upload, search, rag, admin, pat, auth, health)
├── auth/           # OIDC/OAuth2 authentication & RBAC
│   ├── config.rs   # AuthConfig (from AppConfig)
│   ├── provider.rs # OIDC + generic OAuth2 providers
│   ├── token_service.rs / refresh_client.rs  # JWT issue/verify, refresh rotation
│   ├── middleware.rs / extractor.rs           # Auth extraction & guards
│   ├── demo_auth.rs # Built-in demo users (DEMO_MODE)
│   └── models.rs   # AccessLevel, AuthenticatedUser, UserContext
├── db/             # MongoDB models, repositories (trait + Mongo impl) and
│                   #   the versioned migration plan (migrations.rs)
├── server/         # Leptos #[server] functions (docs, search, nav, prompts,
│                   #   pats, users, feedback, access_levels, custom_css, reindex)
├── pages/          # Leptos page components (home, doc, chat, login, profile,
│                   #   prompts, admin_settings, not_found)
├── components/     # Shared UI components (layout, navigation, search, theme,
│                   #   markdown_content, contextual_sidebars, user_menu, …)
├── editor/         # Tiptap WYSIWYG editor + asset panel
├── schema/         # Schema registry UI component + endpoint reindex
├── rag/            # RAG pipeline: service, chat, embedding, vectorstore (Qdrant),
│                   #   splitters, reranker, hyde, query_rewriter, rrf, eval, reindex
├── mcp/            # Model Context Protocol server (server.rs, auth.rs) at /mcp
├── search/         # Meilisearch client, tenant tokens, reindex
├── storage/        # S3 blob storage (StorageClient trait + S3StorageClient)
├── rendering/      # GFM markdown → HTML, link extraction & transformation
├── jobs/           # Background jobs (e.g. recompute_access_levels)
└── bin/            # rag-eval, rag-bench (retrieval quality tooling)

cli/                # `lekton-sync`: CI/CD ingestion CLI (separate workspace member)
config/default.toml # Embedded default configuration (overridden by LKN__ env vars)
```

## 📝 Maintenance
-   **Changelog:** Update `CHANGELOG.md` for every significant change.
-   **DCO:** Every commit pushed to GitHub must include a `Signed-off-by:` trailer. Prefer `git commit -s` and `git commit --amend -s` so PRs pass the DCO check.
-   **Formatting:** Run `cargo fmt --all` before finishing Rust changes. Prefer `just fmt` locally and keep the tree passing `cargo fmt --all --check`, because CI enforces formatting on pushes and pull requests.
-   **Tests:** Every feature requires unit tests. Integration tests for API endpoints.
-   **Documentation:** If you change an interface, update the relevant Markdown documentation.

## 🔗 Useful Links
- [Requirements](docs/REQUIREMENTS.md)
- [Contributing Guidelines](CONTRIBUTING.md)

## Agent skills

### Issue tracker

Issues live in GitHub Issues (`dghilardi/lekton`), managed via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default five canonical roles (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context — one `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.
