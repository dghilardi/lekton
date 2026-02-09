# Agent Guidelines (AGENT.md)

Welcome, fellow Agent. This document provides context and standards for AI coding assistants working on **Lekton**.

## 🎯 Project Context
Lekton is a dynamic IDP built with **Leptos (frontend)** and **Axum (backend)**. It aims to solve the maintenance pain of static docs by providing a dynamic, RBAC-aware, and searchable portal.

## 🛠️ Tech Stack & Patterns
-   **Rust:** Use stable toolchain. Prefer `tokio` for async.
-   **Frontend:** Leptos (Hydration/SSR). Use `leptos-router`. Follow idiomatic component structure.
-   **Backend:** Axum. Use `tower` middleware.
-   **DB:** MongoDB. Use `mongodb` crate with typed models.
-   **Error Handling:** Use `thiserror` for library errors and `anyhow` for application-level logic.

## 🏗️ Architecture Standards
1.  **Strict Typing:** Ensure all API boundaries are strictly typed.
2.  **RBAC First:** Every new endpoint must verify `access_level`.
3.  **Documentation:** Keep `docs/REQUIREMENTS.md` and `docs/ADRs/` updated with major design changes.
4.  **Trunk-Based Development:** Prefer small, frequent commits to the main branch. Use feature flags for long-running changes.

## 📝 Maintenance
-   **Changelog:** Update `CHANGELOG.md` for every significant change.
-   **Tests:** Every feature requires unit tests. Inegration tests for API endpoints.
-   **Documentation:** If you change an interface, update the relevant Markdown documentation.

## 🔗 Useful Links
- [Requirements](docs/REQUIREMENTS.md)
- [Contributing Guidelines](CONTRIBUTING.md)
