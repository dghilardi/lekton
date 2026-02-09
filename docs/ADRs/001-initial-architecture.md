# ADR 001: Initial Architecture - Headless CMS with Rust

## Status
Accepted

## Context
The goal is to build a high-performance IDP that decouples content from code. Static generators (like Nextra) require full rebuilds and lack granular RBAC.

## Decision
Use a Headless CMS architecture with a Rust-based backend (Axum) and a Rust-based frontend (Leptos).
- **Metadata:** MongoDB for flexible schema and RBAC mapping.
- **Content:** S3 for infinitely scalable blob storage of Markdown/Schema files.
- **Search:** Meilisearch for high-performance, developer-friendly search.

## Consequences
- **Pros:** Blazing fast performance, modular ingestion, granular security at the server level.
- **Cons:** Higher initial development complexity compared to static generators.
- **Impact:** Requires a more complex infrastructure but provides significant long-term scalability and security benefits.
