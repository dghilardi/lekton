# Product Requirements Document: Lekton

| **Project Name** | Lekton |
| --- | --- |
| **Status** | Draft / Planning |
| **Target Stack** | Rust (Leptos, Axum), MongoDB, S3, Meilisearch |
| **Primary Goal** | Replace the static Nextra fork with a dynamic, high-performance, RBAC-aware portal. |

---

## 1. Executive Summary

**Problem:** The current documentation portal (Nextra fork) suffers from high maintenance overhead, requires full site rebuilds for content updates, forces a monolithic repository structure, and lacks granular access control for sensitive documentation.

**Solution:** Build Lekton, a custom Server-Side Rendered (SSR) portal using Rust. This platform will decouple content from code, allowing microservices to inject their own documentation via CI/CD, enforce strict Role-Based Access Control (RBAC) at the server level, and provide a unified interface for API Schemas (OpenAPI/AsyncAPI).

---

## 2. User Personas

1. **The Viewer (Junior/Public):** Can only access public guides and onboarding docs. Needs fast search.
2. **The Developer (Internal):** Can access architecture decision records (ADRs), internal API schemas, and deployment guides.
3. **The Contributor:** Uses the Web UI or CI pipelines to update documentation. Needs confidence that their links aren't broken.
4. **The Admin:** Manages user roles and integration keys for microservices.

---

## 3. High-Level Architecture

The system follows a **Headless CMS** architecture where the Rust backend acts as the orchestrator between storage, search, and the frontend.

### Core Components

*   **Frontend:** **Leptos** (Rust). Handles SSR, hydration, and client-side interactivity.
*   **Backend API:** **Axum** (Rust). Handles auth, ingestion logic, and schema validation.
*   **Metadata Store:** **MongoDB**. Stores document hierarchy, permissions, versioning, and link graphs.
*   **Blob Store:** **S3**. Stores raw Markdown/MDX files and schema artifacts (JSON/YAML).
*   **Search Engine:** **Meilisearch**. Stores indexed content with protected tenant tokens.

---

## 4. Functional Requirements

### 4.1. Authentication & Authorization (RBAC)

*   **Auth Provider:** Integration with company OIDC/OAuth provider (e.g., Keycloak, Okta, Google Workspace).
*   **Role Mapping:** Map Lekton groups to internal roles: `Public`, `Developer`, `Architect`, `Admin`.
*   **Granularity:**
*   Every document and schema version must have a `min_access_level` field in MongoDB.
*   The Axum middleware must reject requests to restricted paths before fetching content from S3.
*   **Requirement:** Users must never see a link in the sidebar or search result for a document they cannot access.



### 4.2. Ingestion Pipeline (The "write" path)

The system supports two methods of ingestion:

**A. CI/CD Injection (API)**

* Microservices utilize a dedicated endpoint: `POST /api/v1/ingest`.
* **Payload:** Service Token, Markdown content, Metadata (Category, Version).
* **Behavior:** Atomic update. If the doc exists, create a new version; if not, create it.

**B. Web Editor (GUI)**

* **Technology:** **Tiptap** (via `leptos-tiptap`) or **Milkdown**.
* **Features:** WYSIWYG editing, Markdown shortcut support, Image upload (to S3).
* **Validation:** BEFORE saving, the backend parses the AST (Abstract Syntax Tree) to validate internal links.

### 4.3. Documentation Rendering

* **Format:** Support for GitHub Flavored Markdown (GFM) and MDX-lite (custom Rust components injected into the stream).
* **Performance:** Time to First Byte (TTFB) < 50ms.
* **Navigation:** Dynamic sidebar generation based on the user's role and the MongoDB hierarchy tree.

### 4.4. Schema Registry

* **Supported Formats:** OpenAPI (v3.0, v3.1), AsyncAPI (v2+), JSON Schema.
* **Versioning:**
* Store multiple versions of a schema (e.g., `1.0.0`, `1.1.0`, `2.0.0-beta`).
* UI allows switching between versions via a dropdown.


* **Visualization:** Embed **Scalar** or **Redoc** for OpenAPI; **AsyncAPI-React** for event driven architectures.

### 4.5. Intelligent Search

* **Engine:** Meilisearch.
* **Scoped Search:** When a user logs in, generate a temporary Meilisearch Tenant Token containing the filter: `filter = "role IN ['public', 'user_role']"`.
* **Vector Readiness:** Ingestion pipeline must be extensible to generate embeddings (via `rust-bert` or OpenAI API) and store them for future RAG (Chat with Docs) features.

---

## 5. Data Models (MongoDB Draft)

### Collection: `documents`

```json
{
  "_id": "ObjectId(...)",
  "slug": "engineering/deployment-guide",
  "title": "Deployment Guide",
  "s3_key": "docs/eng/deploy_v4.md",
  "access_level": "developer", // [public, developer, admin]
  "service_owner": "devops-team",
  "last_updated": "ISO8601",
  "tags": ["k8s", "cicd"],
  "links_out": ["/docs/setup", "/schemas/api-v1"], // For backlink tracking
  "backlinks": [] // Populated via trigger/logic
}

```

### Collection: `schemas`

```json
{
  "_id": "ObjectId(...)",
  "name": "payment-service-api",
  "type": "openapi",
  "versions": [
    {
      "version": "1.0.0",
      "s3_key": "schemas/payment/1.0.0.json",
      "status": "deprecated"
    },
    {
      "version": "2.0.0",
      "s3_key": "schemas/payment/2.0.0.json",
      "status": "stable"
    }
  ]
}

```

---

## 6. Non-Functional Requirements

* **Performance:** The Rust binary should consume <100MB RAM under normal load.
* **Availability:** Stateless backend allows for horizontal scaling (Kubernetes ReplicaSet).
* **Maintainability:** Codebase must be a standard Cargo workspace. No forked frameworks.
* **Style Customizability:** The application MUST allow users to inject custom CSS styles at runtime (e.g., via a settings UI or a specific CSS file) to override the default theme without requiring a project recompilation.
* **Observability:** Integrated `tracing` (OpenTelemetry) for all requests. Errors logged to stdout/Sentry.

---

## 7. Development Phases

### Phase 1: The Core (MVP)

* Setup Axum + Leptos boilerplate with Tailwind CSS and DaisyUI.
* Implement OIDC Authentication.
* Create MongoDB Schema + S3 connection.
* Build the `POST /ingest` API.
* Basic Markdown rendering (Read-only).

### Phase 2: The Editor & Search

* Implement Tiptap editor in Leptos.
* Implement Link Validator logic (AST parsing).
* Setup Meilisearch container and ingestion hooks.
* Implement Tenant Token generation for secure search.

### Phase 3: The Registry & Polish

* Add Schema Registry (OpenAPI/AsyncAPI viewing).
* Add Version Selector.
* Migrate existing content from Nextra (write a one-off migration script).
* Decommission Nextra.

### Phase 4: Future Tech (AI)

* Add Vector Database (Qdrant or Meilisearch Vectors).
* Implement "Ask the Docs" chat interface.
