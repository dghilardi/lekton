# Lekton Implementation Roadmap

This document outlines the strategic roadmap to bring Lekton to full feature parity with the current Nextra-based CloudDocs portal, while simultaneously fulfilling the advanced architectural and product requirements outlined in the `REQUIREMENTS.md`.

## Phase 1: Core Navigation & Layout Parity

To replace Nextra, the portal must be able to handle deeply nested and extensive documentation sets.

*   [x] **Nested Sidebar Navigation:**
    *   Transition from the current flat sidebar to a multi-level, collapsible tree structure.
    *   Backend: Ensure MongoDB document schemas support hierarchical nesting and ordering.
*   [x] **Table of Contents (On This Page):**
    *   Implement an AST parser to extract Markdown headers (H2, H3) to build a persistent right-side TOC.
    *   Add scrollspy functionality to highlight the currently active section.
*   [x] **Breadcrumbs Component:**
    *   Add top-level breadcrumbs (e.g., `Docs > Microservices > Auth`) to improve situational awareness.

## Phase 2: Search & Discovery

Users rely heavily on search to find specific technical documentation.

*   [x] **Fix Meilisearch Integration:**
    *   Debug and repair the backend connection that currently throws a "Search error".
    *   Ensure documents are properly indexed upon ingestion.
*   [x] **Interactive Search Modal (UI):**
    *   Implement a `CTRL+K` global search modal (similar to Algolia DocSearch).
    *   Provide live preview snippets and highlight matching terms in the search results.

## Phase 3: Advanced Schema Registry (From Requirements)

Move beyond static API reference pages to interactive, living API documentation.

*   [x] **Interactive Spec Viewers:**
    *   Integrate a component like **Scalar** or **Redoc** for OpenAPI 3.0/3.1 specifications.
    *   Integrate **AsyncAPI-React** for event-driven specifications.
*   [x] **Dynamic Version Selector:**
    *   Implement a dropdown in the API views to switch between different versions of a schema, powered by the MongoDB `schemas` collection.

## Phase 4: Theme, Polish & Accessibility

Ensure the portal looks and feels like a premium, modern developer tool.

*   [x] **Theme Toggle (Dark/Light Mode):**
    *   Implement a system/light/dark mode toggle switch in the UI.
    *   *Requirement Satisfaction:* Ensure this ties into the "Style Customizability" requirement, allowing runtime CSS injection to override themes.
*   [x] **Document Metadata Polish:**
    *   Add "Last Updated" timestamps at the bottom of pages.
    *   Add user-friendly tags handling.

## Phase 5: Authentication & Authoring Experience

Fulfill the core differentiator of Lekton: robust RBAC and a seamless authoring experience.

*   [x] **OIDC Authentication Integration:**
    *   Transition from the current Demo/Mock login to a real Identity Provider (e.g., Keycloak).
    *   Map incoming OIDC groups to internal Lekton roles (Public, Developer, Admin).
*   [x] **Integrated Web Editor (Tiptap):**
    *   Complete the implementation of the `/edit` route to provide a WYSIWYG/Markdown editing experience directly in the portal.
    *   Add pre-save link validation (AST parsing) to prevent broken links.
*   [x] **Strict RBAC Enforcement:**
    *   Ensure the Axum backend strictly checks the `min_access_level` before fetching an S3 document.
    *   Filter the generated sidebar navigation tree so users never see links to documents they cannot access.

## Phase 6: Nextra Migration & Decommissioning

*   [ ] **Migration Tooling:** Write a one-off script to parse the current Nextra markdown structure, extract frontmatter, and ingest it into Lekton via the `POST /api/v1/ingest` endpoint.
*   [ ] **Beta Rollout:** Run Lekton in parallel with Nextra for a testing period.
*   [ ] **Decommission Nextra:** Sunsetting the legacy portal once full parity and stability are confirmed.
