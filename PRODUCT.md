# Lekton

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

Developers and platform teams evaluating and running an internal developer portal. The public website must help them understand the shipped capabilities and try the product.

## Product Purpose

Centralize documentation and API schemas without rebuilding a static documentation site for each content update. Content can be synchronized from service repositories or edited through the portal.

## Operating Context

Rust application using Leptos and Axum, MongoDB metadata, S3-compatible content storage, and the `lekton-sync` CI/CD client. Docker Compose provides a local evaluation environment. Production requires configured authentication and service credentials.

## Capabilities and Constraints

Document ingestion, access controls, Markdown and visual editing, OpenAPI/AsyncAPI/JSON Schema registry, prompt library, and PAT-authenticated MCP integration. Meilisearch search and Qdrant-backed retrieval-augmented chat are optional features requiring configuration; RAG also needs embedding and generation providers. The website is a static GitHub Pages surface in the same repository, separate from the application runtime.

## Brand Commitments

Preserve the Lekton name. The public website is available in English and Italian. The user requests a minimal, modern presentation with concrete capabilities and usage instructions, without inflated marketing claims. The website and application must share a recognizable visual identity. The existing default light and dark themes are the starting point; any proposed theme changes must be shown together with the landing page before implementation.

## Evidence on Hand

README.md, cli/README.md, docs/REQUIREMENTS.md, config/default.toml, docker-compose.yml, and implementation source. No customer testimonials or measured performance claims have been provided. Any demonstration content must be labeled as illustrative.

## Product Principles

- Explain the content workflow before optional AI features.
- Make evaluation and operational requirements visible.
- Ground capability claims in repository evidence.
- Keep the public page versioned alongside the product.
