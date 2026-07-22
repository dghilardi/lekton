# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added
- Learn mode (behind the `learn` feature flag, off by default, requires `rag`): a guided, stateful alternative to reading docs or RAG chat that generates short personalized lessons grounded strictly on the documentation the user can access. Semantic retrieval grounds each lesson on the relevant document sections (real section anchors, so citations deep-link to `#anchor`); a single-document scope uses the whole document when it fits the context budget and splits it into sections when it doesn't. The lesson is structured (sanitized HTML body, source citations, and a 3-question quiz); citations that don't resolve to a provided document are dropped and lessons with none are backfilled from the grounding sections. Native `/learn` pages let users start a path (by tag, topic, or document slug) and optionally state a mission — their own reason for learning it — which grounds every lesson toward that goal; generate lessons, and take quizzes with immediate feedback. Quizzes are graded server-side and the answer key never reaches the browser — persisted lessons grade by id, while ephemeral (nothing-stored) lessons carry a sealed, tamper-proof token that lets the server grade without persisting anything. Progress is persisted and lightly calibrated (advance vs reinforce from recent quiz scores); a per-user "Save my progress" toggle enables an ephemeral, nothing-stored mode, and learning data can be deleted. The lesson LLM is configured under `[learn]` with a fallback to `[rag.chat]`/`[rag.llm]`.

## [0.28.0] 2026-07-21

### Added
- Prometheus usage metrics (behind the `metrics` feature flag, off by default): a `GET /metrics` endpoint exposing HTTP request counts/latencies plus product-usage counters (searches and zero-result searches, document views, schema views, RAG chat messages, editor saves, guided uploads) for building Grafana dashboards. Labels are kept low-cardinality (matched route templates, no per-document ids). The endpoint can be guarded with an optional `server.metrics_token` bearer token in addition to proxy-layer protection.

## [0.27.1] 2026-07-21

### Fixed
- Top navbar "Altro" overflow dropdown could be clipped and unreachable on wide viewports (e.g. fullscreen), where the centered search bar (2xl+) halves the space available to the nav-links row.

## [0.27.0] 2026-07-21

### Added
- Hand-authored pages in the WYSIWYG editor (behind the `editor` feature flag): a "Create Page" admin entry point opens the editor in creation mode for a new slug, with page-metadata fields (slug, access level, parent, order) consistent with the upload form. Creating and editing manual pages is admin-only.
- Documentation-agent groundwork (all behind existing feature flags, off by default):
  - Per-source opt-in flag `review_enabled` on the source registry — a source must explicitly opt in before the documentation agent may touch it (admin Sources editor toggle + `list_sources` MCP tool).
  - Feedback lifecycle: new `in_progress` status with claim/delivery metadata; the `claim_documentation_feedback` MCP tool takes an item in charge and returns a per-claim commit trailer; `list_documentation_feedback` gains a `delivery_source_id` filter (the CI candidate query) and the `in_progress` status.
  - Source-scoped resolution: `POST /api/v1/feedback/resolve` (service-token auth) closes items a repo's merge addressed, only when they are `in_progress` and claimed for that source.
  - `reopen_stale_documentation_claims` MCP tool to return abandoned in-progress claims to the open queue.
  - Admin provisioning of user-less "machine" PATs (not tied to a user account) for machine-to-machine MCP access.

## [0.26.2] 2026-07-20

### Fixed
- `lekton-sync` now computes each document's source path relative to the git repository root instead of the CLI's scan root, so "View source" links resolve correctly even when the tool is invoked from (or pointed at) a subdirectory of the repo.

## [0.26.1] 2026-07-17

### Added
- Externally-managed (sync) documents now show a "View source" link when their source is registered with a recognized provider repository URL (GitHub, GitLab, or Bitbucket). The link points at the source file on the repository's mainline branch and is shown to all users.

## [0.26.0] 2026-07-17

### Added
- Admin source registry (behind the `sources` feature flag, off by default): a "Sources" admin page that lists the documentation import sources discovered from ingested documents (with per-source document counts) and lets admins attach repository metadata — display name, repository URL, mainline branch, description, and a list of maintainers (external contacts or linked Lekton users).
- MCP tools (admin only) to drive external tooling over the source registry and feedback queue: `list_sources` (repository metadata + document counts, gated by the `sources` feature) and `list_documentation_feedback` (the feedback queue, open by default, gated by `documentation_feedback`) — each feedback item carries resolved repository targets (repo URL, mainline branch, source file path) for the documents it references.

## [0.25.21] 2026-07-16

### Fixed
- The admin Personal Access Tokens pager and the Profile page's own PAT pager no longer trigger a runaway pagination request loop: a bare `>=` in the "next page" button's `disabled` attribute confused the `view!` macro's parser, reattaching the click handler as button content where it reran (and advanced the page) on every render.

## [0.25.20] 2026-07-15

## [0.25.19] 2026-07-15

## [0.25.18] 2026-07-15

### Changed
- On small screens the primary navigation (Documentation, Registry, Chat, Admin) is now a single labelled "Menu" dropdown with text items instead of a row of unlabelled icons whose tooltips never appear on touch.
- Admin pages no longer duplicate their heading: the section header now carries a section-specific icon and subtitle, and the redundant per-card title/description was removed (Service Tokens, Users, Access Levels, Documentation Feedback, Navigation, Custom CSS, Personal Access Tokens, Upload Document).
- The home page feature cards use themed SVG icons instead of emoji and are no longer styled as interactive (they are informational, not links).

### Fixed
- Navbar dropdown menus (the mobile "Menu" and the desktop overflow/"more" and group menus) are no longer clipped by the nav container's `overflow: hidden`; the container now clips only horizontally so menus render fully below the header.
- Global search now shows a friendly "temporarily unavailable" message instead of leaking the raw internal/backend error string; the underlying error is logged to the browser console.
- The admin "section not found" state is now a neutral empty state with a recovery link instead of a full-width warning banner.
- The Users admin panel now shows an empty state when no users are registered instead of a blank card.
- Destructive admin actions (deactivate token, delete access level) now use a clear outline button instead of a bare red text link.
- The Custom CSS editor textarea now fills the available width instead of collapsing to its intrinsic size.
- Header controls now meet a 44px touch-target minimum on touch devices.
- Document pages now cap the reading column width so long-form content stays within a comfortable line length instead of spanning the full content area.
- The global search modal footer now shows keyboard hints (navigate / select / close) instead of a lone, duplicated "ESC" affordance.
- Service token "active" status now uses the success colour instead of the brand accent, so it reads as a status rather than a highlight.
- The "Generate with AI" description action is now tinted so it is discoverable instead of blending into the form.
- Added a browser tab favicon that mirrors the navbar logo: the built-in Lekton mark by default, or a deployment's `logo.svg` / `logo-light.svg` / `logo-dark.svg` override (with light/dark variants keyed to the OS colour scheme) when present.
- The "Create service token" form now stacks labels above full-width fields, fixing the collapsed/misaligned scopes textarea.
- Admin form fields on Access Levels and the Documentation Feedback filters now fill their columns instead of collapsing to their intrinsic width.
- Deleting an access level now asks for confirmation, matching service-token deactivation.

## [0.25.17] 2026-07-12

### Security
- Refresh tokens now belong to a rotation family: reusing an already-rotated token revokes the whole family (theft detection), and a TTL index prunes expired/revoked tokens instead of letting them accumulate forever.
- The OAuth/OIDC login flow state (CSRF token and OIDC nonce) is now carried in a signed, short-lived token instead of a plaintext cookie, so it can no longer be forged or tampered with by the browser.
- Startup now rejects a JWT signing secret shorter than 32 bytes instead of accepting any non-empty value, so a trivially brute-forceable HS256 secret can no longer be configured.
- OIDC `id_token`s are now cryptographically verified (JWKS signature selected by `kid`, issuer, audience, expiry and nonce) instead of decoded without verification; symmetric/`none` algorithms are rejected and the JWKS is refetched on key rotation. OIDC now always performs discovery to obtain the issuer and JWKS URI.

### Added
- A local-first accessibility test suite (`e2e/a11y.spec.ts`) runs axe over the home, search, chat and admin flows plus keyboard-operability checks, and the Playwright config now includes WebKit and a mobile profile for local multi-browser runs (both kept out of CI until pre-existing violations are triaged).

### Fixed
- `/health/ready` now actively probes the enabled RAG (Qdrant) and search (Meilisearch) backends instead of reporting them healthy merely because the service was constructed, so an initialised-but-unreachable dependency is reported as an error.
- `POST /api/v1/assets/check-hashes` now caps the number of entries per request and looks them up in a single batched `$in` query instead of one sequential query per entry.
- The MCP documentation-feedback tools (`report_missing_documentation`, `propose_documentation_improvement`) now cap free-text field lengths and array sizes, so a client cannot persist an unbounded blob to the feedback store.
- The MCP `list_schemas` and `get_index` tools now paginate (`limit`/`offset`, bounded page sizes) and wrap results with pagination metadata, instead of serialising the entire schema registry / document tree into a single response.
- Fixed accessibility violations surfaced by the axe audit: the search input's `aria-controls` now points to an always-present results container (no dangling reference), and the navbar search bar and sidebar drawer toggle have accessible names.
- Raised muted body text (`text-base-content/40` and `/50` → `/65`) and slightly deepened the light-theme primary amber so text and the active nav link meet the WCAG AA 4.5:1 contrast ratio; the axe audit over home/search/chat/admin is now free of contrast and label violations.
- Navbar dropdown triggers now advertise `aria-haspopup`, and icon-only navbar controls (Docs, Registry, Chat, Admin) carry accessible names, so screen-reader users can tell they open menus and what they do.
- Chat feedback (thumbs up/down, comment, remove) now rolls back its optimistic UI state and shows an inline error when the write to the server fails, and admin access-level deletion surfaces failures instead of silently swallowing them.
- Outbound HTTP clients for external dependencies (OIDC, embedding, reranker) are now built with bounded connect/total timeouts via a shared helper, so a stuck dependency can no longer hang a request indefinitely.
- Asset uploads now roll back the just-written S3 object when the metadata write fails for a brand-new asset, instead of leaving an orphaned blob with no record (updates keep the blob, still referenced by the surviving record).
- The RAG reindex records which documents/attachments failed and offers a "Retry failed items" action (admin UI + `trigger_rag_reindex_failed`) that re-embeds only those, instead of forcing a full re-embed of the whole corpus to recover from a partial failure.
- Reindex status (search, RAG, schema endpoints) now reports per-run `failed`/`skipped` counts and the last error, surfaced in the admin UI and the REST status endpoints, so a run that reaches 100% with failures is no longer indistinguishable from a clean one.
- Background reindex jobs (search, RAG, schema endpoints) now reset their `is_running` flag via an RAII guard even if the job panics or is cancelled, so a crashed reindex can no longer stay stuck "running" and permanently block every future reindex.
- Attachment extractions left `Pending` or `InProgress` by a previous run are now re-enqueued on startup, so work is no longer silently lost across a restart while the extraction queue is in-memory.
- The WYSIWYG editor now indexes the document into search before persisting it, records `needs_reindex` when indexing fails (mirroring the ingest API), and reports partial success in the save message instead of always claiming success while search or asset-reference reconciliation silently failed.
- Orphaned-attachment cleanup now deindexes RAG/search and deletes the blob before removing the canonical asset record, keeping the record (marked failed) for retry if any step fails, instead of stranding indexed chunks with stale ACLs and no anchor to reconcile them.

## [0.25.16] 2026-07-11

### Fixed
- Restored the migration duplicate-key regression test and made the full Clippy quality gate pass.
- Mark attachment extraction as retryable when keyword-search indexing fails instead of reporting a false success.
- Made chat feedback controls keyboard-visible, screen-reader-labelled, stateful, and touch-sized.

## [0.25.15] 2026-07-10

## [0.25.14] 2026-07-10

### Fixed
- Limit embedding batch size to 100 items.

## [0.25.13] 2026-07-05

## [0.25.12] 2026-07-05

### Fixed
- Derive the home "Get Started" link from the actual navigation and point the schemas CTA to the stable `/schemas` route to avoid fresh-install 404s.
- Made `DELETE /api/v1/assets/{key}` idempotent so repeated deletes of missing assets now succeed cleanly.
- Close the desktop search dropdown when focus leaves the search control.
- Show the actual demo login failure message instead of always rendering a generic invalid-credentials error.
- Surface chat session deletion failures in the UI instead of failing silently.
- Added accessible labels to icon-only navigation and chat controls so navbar, user menu, send, and session-delete actions remain screen-reader discoverable.
- The global search modal now supports keyboard result navigation: arrow keys move through results and `Enter` opens the currently highlighted match while the input retains focus.
- The top navbar now renders a stable SSR placeholder instead of appearing only after hydration, reducing the visible pop-in of the docs/system links area on first paint.
- Streaming chat responses now throttle markdown re-rendering while tokens arrive, avoiding full reprocessing of the accumulated assistant text on every delta.
- The global search modal now exposes proper dialog semantics (`role="dialog"`, `aria-modal`, labelled title) and reliably restores keyboard focus to the search field when opened.
- The global search modal and navbar search bar now debounce server-side search requests, so typing a query no longer fires a request for every keystroke.
- Asset serving and chat source filtering now batch document ACL lookups by slug instead of issuing one document query per referenced slug, reducing N+1 latency on assets and cited sources with many backlinks.
- RAG chat prompts now enforce bounded history and retrieved-context sizes before sending the final LLM request, keeping parent-expanded sections and long conversations from inflating context windows and cost unpredictably.
- `POST /api/v1/ingest` no longer waits for attachment ACL recomputes before responding; the recompute is now scheduled in background like the document-upload flow, avoiding long-lived service-to-service requests on large or numerous PDFs.
- Attachment extraction uploads no longer drop queued reprocessing silently when the bounded worker channel is full; full queues now retry asynchronously, and a closed worker marks the asset as failed instead of leaving it stuck in `Pending`.
- Attachment ACL recomputes now fail closed: if Qdrant or attachment-keyword-search ACL updates fail, the attachment is deindexed and marked for reprocessing instead of remaining searchable with stale permissions.
- Demo-mode sessions now store only the selected demo account identifier in `lekton_demo_user`; all demo privileges are re-derived server-side so a forged cookie JSON payload cannot self-assign admin access.

## [0.25.11] 2026-07-05

## [0.25.10] 2026-07-04

### Fixed
- Schema version updates now avoid read-modify-write races: adding a version uses a guarded atomic `$push`, and archiving a version updates only the targeted array entry instead of replacing the whole schema document.
- MongoDB startup now uses explicit typed client options (`app_name`, server-selection timeout, connect timeout, pool sizing) instead of the driver defaults.
- Startup migrations now enforce a unique `__migrations.change_id` index and refuse to proceed when a migration is already marked `STARTED`, preventing concurrent instances from applying the same migration twice.
- Attachment search ACL refresh now paginates through every indexed page for a file instead of updating only the first 1000 Meilisearch documents.
- `GET /api/v1/assets` now requires an authenticated admin session instead of exposing the full asset inventory to anonymous callers.
- PDF keyword-search hits now open the underlying asset at `#page=N` like RAG attachment citations instead of routing through the document page.
- Restored the top navbar admin entry on narrower desktop widths by preventing the docs-link cluster from clipping the system links.
- Startup migrations now add the missing MongoDB unique/index coverage for `service_tokens`, `prompts`, chat history, feedback, version-history, settings, navigation ordering, and user prompt preferences, eliminating key collection scans and blocking duplicate logical keys before startup continues.
- RAG parent-expansion now skips attachment hits instead of treating every attachment chunk as the same empty `(document_slug, section_anchor)` parent, preventing unrelated attachment text from being merged into a single chat context block.
- The legacy image upload endpoint now requires an authenticated session, derives the served content type from the filename instead of client headers, and adds `Content-Disposition`/`nosniff` hardening when serving uploaded images.
- `GET /api/v1/search` now derives document visibility from the authenticated user context instead of trusting client-supplied `access_levels`, closing an ACL bypass for restricted document and attachment search results.

## [0.25.9] 2026-07-04

### Fixed
- Kept the navbar theme toggle reachable at narrower desktop widths and aligned the upload/PDF E2E checks with the dedicated PDF document layout so GitHub Actions no longer fail on the release workflows.

## [0.25.8] 2026-07-04

### Added
- Admins can now archive an uploaded PDF document from its page (button next to Edit, with a confirmation dialog). Archiving de-indexes it from Meilisearch and RAG and unlinks the PDF asset, without deleting the underlying file.
- PDF attachment content is now also searchable by keyword: extracted pages are indexed into a dedicated Meilisearch index alongside RAG, and matches show up in the existing search bar/modal with a "PDF · page N" badge linking to the owning document. Complements RAG's semantic search for exact terms (part numbers, error codes, acronyms) that a purely semantic match might miss.

### Changed
- Guided document upload no longer indexes its markdown stub into RAG: the stub only holds the AI summary and a link, while the linked PDF is already indexed as an attachment, so indexing the stub duplicated content. Documents now carry a `skip_rag` flag (default off) that excludes them from the RAG vector store while keeping them in Meilisearch keyword search, so the page stays discoverable.
- Uploaded PDF documents now render with a dedicated page layout — a prominent open/download card for the PDF plus the AI summary — instead of the bare markdown stub with an inline link.
- A full RAG reindex now also force-reprocesses every referenced PDF attachment (regardless of whether its content changed), so previously-uploaded PDFs and configuration changes to chunking/embedding/extraction are picked up without needing a no-op re-upload of each file.

### Fixed
- A PDF attachment dereferenced by a document (via a plain markdown edit, `lekton-sync`, or document archiving — not just the admin upload form's own edit flow) was left permanently orphaned in S3, MongoDB, and Qdrant instead of being cleaned up; it is now deleted as soon as no document references it anymore.
- Attachment indexing was marked fully `Failed` (forcing a wasteful full re-embed on retry) whenever stale-chunk cleanup failed after a successful upsert, even though the new content was already indexed correctly; that cleanup failure is now logged and treated as non-fatal.
- A full RAG reindex ignored the `skip_rag` flag and silently re-added excluded documents (e.g. PDF upload stubs) to the RAG index; it now removes them instead, matching normal ingest behavior.

## [0.25.7] 2026-07-03

### Fixed
- Chat source references from PDF attachments now link to the asset at the cited page (`/api/v1/assets/{key}#page={n}`, opened in a new tab) instead of a broken `/docs/` link showing "Document '' not found".
- PDF attachment references are no longer dropped when reopening a saved chat: the session-messages loader was filtering every source through a document-slug lookup, which discarded attachment sources (no slug); attachment visibility is now resolved through the referencing asset's ACL.

## [0.25.6] 2026-07-03

### Fixed
- AI summary generation reported "summary generation failed" even when the server streamed a valid summary. The SSE completion event carried empty data (which browsers drop, so the client never saw it) and the server's error event was named `error` (colliding with `EventSource`'s built-in connection-error event, which also fires on the normal end-of-stream close). The completion event now carries a payload, the server error event is renamed `summary_error`, and the client treats a normal stream close with received content as success.

## [0.25.5] 2026-07-02

### Changed
- Guided document upload now indexes the linked PDF into RAG when the document is **saved** rather than when the file is uploaded. The upload step no longer embeds the attachment, so extraction/embedding no longer competes with AI summary generation for LLM quota, and chunks are indexed with the document's access levels already known (no transient `access_levels=[]` state). Unchanged PDFs are not re-embedded on edit.

## [0.25.4] 2026-07-02

## [0.25.3] 2026-07-01

## [0.25.2] 2026-06-30

### Fixed
- Document upload: `recompute_access_levels` is now fire-and-forget (`tokio::spawn`) in the Leptos server function path so large PDFs with many RAG chunks no longer cause a GCP Load Balancer 502 timeout. The HTTP ingest handler (server-to-server) still awaits synchronously.
- Document upload: `access_levels=[]` on attachment chunks after upload is resolved as a consequence of the above fix (the recompute no longer times out before completing).
- Document upload: "Generate with AI" now streams the summary via SSE (`GET /api/v1/document-upload/summary`) instead of a blocking Leptos server function call, keeping the connection alive even when the LLM takes longer than 30 s.

## [0.25.1] 2026-06-28

## [0.25.0] 2026-06-28

### Added
- Guided document upload (`LKN__FEATURES__DOCUMENT_UPLOAD=true`, off by default): an admin-only form (Admin → Upload Document) to publish an existing PDF as a portal page. Upload the PDF, set title, access level, tree position and a description, and a page with that description plus a download link is created and indexed for search and RAG (the linked PDF inherits the page's access level). A "Generate with AI" button drafts the description from the document's first pages (requires `rag`). Upload-origin pages get an Edit button that reopens the form. Independent of the `editor` feature, so it works in a read-only portal.
- Runtime feature flags (`[features]` config section, `LKN__FEATURES__*`) to enable/disable functional areas at startup: `mcp`, `rag`, `editor`, `schema_registry`, `search`, `prompt_library`, `documentation_feedback`. A disabled feature is hidden end-to-end (no backend services, no routes, no UI). Enabling a feature without its prerequisites fails fast at startup with an actionable error.
- Attachment RAG indexing (`LKN__FEATURES__ATTACHMENT_INDEXING=true`, requires `rag`, off by default): uploaded text and PDF attachments are extracted and indexed into the vector store so chat can draw on them. PDFs use the native text layer via libpdfium; image-heavy pages (diagrams, tables, screenshots, scans) are rendered and transcribed by a vision LLM configured under `[rag.vlm]` (controlled by `rag.attachment_page_text_threshold`). Extraction runs on a background worker; each attachment's status (queued / indexing / indexed / failed) is shown in the editor asset panel. Attachment chunks inherit the access levels of the published documents that reference them, are cited in chat answers (filename + page, linking to the asset), and are removed from the index when the asset is deleted.

### Changed
- **Breaking:** documents imported from an external source (ingest API / lekton-sync — anything with a `source_id`) are now read-only in the portal. Their Edit button is hidden and the markdown editor refuses to open or save them, since lekton-sync is their source of truth and in-portal edits would be overwritten on the next sync. Hand-made pages (created in the editor, no `source_id`) and upload-form pages remain editable through their respective flows. Previously an admin could edit any document in the WYSIWYG editor.
- Documents now maintain `Asset.referenced_by`: when a document links to or embeds an asset (`/api/v1/assets/{key}`), it is recorded on that asset. This activates the existing asset access model — assets referenced by a document become accessible to that document's audience (previously every asset was accessible only to its uploader or an admin) — and drives attachment access levels for RAG.
- **Breaking:** RAG chunk access levels are now stored as a list (`access_levels`) instead of a single value, so a chunk can be visible to several audiences. Existing vector-store points must be migrated by running the admin RAG re-index once after upgrading (embeddings are served from cache, so this re-embeds nothing); until then previously-indexed documents are not retrievable in chat.
- E2E tests now enable `features.search` explicitly and treat `/chat` returning the feature-disabled 404 page as valid when RAG is off, keeping CI aligned with the new runtime feature-flag defaults.
- **Breaking:** RAG and full-text search are now off by default and must be enabled explicitly via `LKN__FEATURES__RAG=true` / `LKN__FEATURES__SEARCH=true` (in addition to their existing `[rag]` / `[search]` configuration). Previously they auto-enabled whenever their connection URLs were set.
- The MCP server is no longer coupled to RAG: it mounts whenever `features.mcp` is enabled and advertises only the tools whose feature is on (`search_documents` needs RAG; the schema, prompt, and documentation-feedback tools follow their respective flags). Previously `/mcp` was mounted only when RAG was configured.
- Disabling the editor (`LKN__FEATURES__EDITOR=false`) puts the portal in read-only mode: the `/edit` route and asset-upload endpoints are gone, edit affordances are hidden, and the WYSIWYG editor bundle is not loaded. Documents are still created via the ingest API.

## [0.24.34] 2026-06-23

## [0.24.33] 2026-06-23

### Fixed
- Logout no longer leaves the browser in a stale "logged in" state: the refresh-token cookie is scoped to `/auth`, OIDC logout has a migration-safe `/auth/refresh/logout` alias for pre-update sessions, and the session indicator cookie is cleared to prevent post-logout auth refresh/redirect loops.
- Document links to uploaded assets such as PDFs now force a full browser navigation instead of being intercepted by the client router, so `/api/v1/assets/...` links open correctly from the docs view.

## [0.24.32] 2026-06-14

### Changed
- `lekton-sync`: all sync and ingest HTTP calls now share a single retry helper, so the document/prompt/schema *sync* requests also retry with exponential backoff on HTTP 429 (previously only the ingest/upload calls did).

## [0.24.31] 2026-06-14

## [0.24.30] 2026-06-14

### Fixed
- RAG embedding cache: the cache key now includes the embedding dimensions and a hash of the backend endpoint, not just the model name. Repointing the embedding URL / Vertex project, or changing dimensions for a Matryoshka model while keeping the model name, no longer returns stale vectors from the old backend (which caused dimension mismatches against Qdrant or semantically wrong results). The composite namespace doubles as a bumpable cache-version key.
- Ingest: when a document is persisted to MongoDB but fails to (re)index in Meilisearch/RAG, the failure is no longer silent. The document is flagged `needs_reindex`, the ingest response reports `indexed: false`, and `lekton-sync` warns the operator to run an admin re-index. A flagged document is also re-processed on its next ingest (instead of being skipped as unchanged), so the index is retried automatically.
- `docker-compose.yml`: environment variables migrated to `LKN__` prefix (e.g. `LKN__DATABASE__URI`, `LKN__AUTH__DEMO_MODE`). The previous legacy names (`MONGODB_URI`, `DEMO_MODE`, etc.) were silently ignored by the config loader, making `docker-compose up` non-functional.
- README: Configuration table, `.env` snippet, e2e test command, and Demo Mode section updated to use `LKN__*` variable names. License section corrected from "GNU GPL v3" to "GNU AGPL v3".
- RAG indexing: a document is no longer dropped from the vector store when its embedding fails. Indexing now computes embeddings before any destructive write and upserts the new chunks before deleting the stale ones (upsert-then-delete-stale); previously the old chunks were deleted first, so a failed/timed-out embedding left the document unsearchable in chat while still present in MongoDB. RAG re-index also now reports indexed/skipped/failed counts instead of silently swallowing per-document failures.

### Changed
- `.gitignore`: added `*.err` and `.codex` patterns; removed stray tracked files (`build_ssr.err`, `.codex`, `cli/.codex`).
- Documentation: README now lists RAG chat, the MCP server, and the Prompt Library as features, links the `lekton-sync` CLI, and documents the full `v1` API surface (RAG, prompts, PATs, schemas, reindex). `AGENTS.md` project tree, `IMPLEMENTATION_ROADMAP.md`, and `REQUIREMENTS.md` updated to reflect the shipped subsystems (RAG/MCP/Prompt Library/PAT) and the dynamic access-level RBAC model.

## [0.24.29] 2026-06-13

### Fixed
- Schema registry: schema format (and the stored object extension / served content-type) is now detected by attempting a strict JSON parse rather than checking for a leading `{`. Array-rooted or BOM-prefixed JSON schemas are no longer mis-stored and served as YAML.
- Chat: the "View session" deep-link from the profile feedback list (`/chat?session=<id>`) now loads that session's messages instead of opening an empty chat.

## [0.24.28] 2026-06-13

### Fixed
- Asset serving: access is now derived from the documents that reference each asset. Unreferenced assets are restricted to the uploader or admins; assets referenced by at least one document inherit the visibility of those documents via the standard `doc_is_accessible` check.
- Editor: navigating between `/edit/*` routes no longer resets in-progress edits. Title and content signals are now seeded in an `Effect` that only fires when the loaded slug changes, not on every resource refetch.
- Schema registry: "latest version" is now determined by semver ordering (numerically descending, lexical fallback) rather than insertion order. REST API, MCP, and UI now all use the same `latest_schema_version` helper, so they agree on which version is "latest" regardless of ingest order.
- Schema REST API: `GET /api/v1/schemas/{name}?version={ver}` is now the canonical way to fetch a schema artifact. Replaces the `rsplit_once('/')` heuristic that was ambiguous for scoped names like `payments/api`. Detail is returned when `?version` is absent.
- Added shared wire-vector tests (CLI ↔ server) for document, prompt, and schema metadata hashes, so any future canonical-format drift is caught immediately.

### Changed
- Documents collection now has fail-fast indexes on `slug` (unique), `source_path`, and `source_id`, created via the migration plan instead of a best-effort startup call. Indexes for `documentation_feedback` and `embedding_cache` are likewise moved to migrations, making all index creation versioned, ordered, and fatal on failure.

### Security
- Asset serving now sets `X-Content-Type-Options: nosniff` and a `Content-Disposition` header (images and PDFs `inline`, all other types — including SVG — `attachment`), preventing stored-XSS from SVG or other uploaded content being rendered as a top-level document in the origin.

## [0.24.27] 2026-06-13

## [0.24.26] 2026-06-12

### Fixed
- RAG chat: mid-stream LLM errors now persist the partial response (with a truncation notice) and cleanly dismiss the loading state instead of leaving the UI hanging.
- RAG chat: empty query vectors are skipped before Qdrant search, preventing spurious retrieval errors on empty or whitespace-only queries.
- Admin: Navigation ordering "Discard" button now correctly restores the original order instead of doing nothing.
- Chat: switching between sessions no longer carries over stale feedback button state from the previous session's messages.
- `lekton-sync`: prompt metadata hash now matches the server wire format; prompts with unchanged metadata are no longer re-uploaded on every run.
- `lekton-sync`: `is_hidden`, `order`, and `parent_slug` can now be cleared via sync (removing the field or setting it to the zero value is now authoritative).
- `lekton-sync`: documents whose attachments failed to upload are now skipped with a clear per-file error instead of being ingested with broken links.
- `lekton-sync`: negative `order` values in front-matter now produce a descriptive error (with file path and value) instead of an opaque HTTP 400.
- Schemas REST API: authenticated non-admin users can now see schemas with `public` access level.
- MCP `search_documentation_feedback`: non-admin users now only see their own feedback reports.
- Static asset caching: the 1-year `immutable` cache is now applied only when an actual `v` fingerprint query parameter is present, not for any query string merely containing the substring `v=` (e.g. `?nav=1`).
- Startup: removed debug `println!` lines that wrote the demo-mode environment variable to stdout before logging was initialized; replaced with a gated `tracing::debug!`.

### Security
- Editor asset upload endpoint now requires an authenticated session.
- Uploaded asset content-type is now derived from the file extension rather than the client-supplied header, preventing XSS via crafted HTML or JavaScript uploads.

## [0.24.25] 2026-06-12

### Fixed
- Integration Tests CI now frees GitHub runner disk space before installing npm dependencies and pulling testcontainers images, avoiding `No space left on device` failures during the workflow.
- `lekton-sync`: slug renames (changing the `slug` front-matter field after first sync) are now handled correctly. The server detects the rename via `source_path + source_id`, performs an in-place slug rename in MongoDB, preserves the existing S3 key when content is unchanged, and no longer silently ignores the new slug or creates a duplicate archived document.
- `lekton-sync`: metadata hash mismatch between client and server is fixed. Both sides now use a canonical BTreeMap serialised to JSON (alphabetically sorted keys, without `source_id`), so hashes agree and unchanged documents are no longer re-uploaded on every run.
- Sync now rejects a slug rename when the target slug is already in use by a different document, preventing silent overwrites.

## [0.24.24] 2026-06-08

### Fixed
- Manual in-page anchor targets in markdown (e.g. `<a id="git_repo"></a>`) are no longer stripped by HTML sanitization, so intra-document links pointing at them now resolve. `id`/`name` are allowed on `<a>` while event handlers and other unsafe attributes remain stripped.
- Mermaid diagrams no longer fail to render in production: static asset staging is now unified in a single `scripts/stage-site-assets.sh` used by both the Docker and the e2e builds, so the served site root can no longer be missing the mermaid (or schema-viewer) bundles in production while passing in CI.
- Ingest now rejects invalid `parent_slug` values server-side, including self-referential parents and malformed absolute or traversal-like parent paths, so bad hierarchy metadata from old or custom clients cannot corrupt navigation.

## [0.24.23] 2026-06-08

## [0.24.22] 2026-06-08

## [0.24.21] 2026-06-08

### Fixed
- E2E/Publish CI no longer downloads the MinIO server binary at runtime; it now starts MinIO from a container to avoid external download failures during workflow startup.
- E2E CI now uses the local npm `tailwindcss` CLI instead of relying on `cargo-leptos` to download the standalone Tailwind binary from GitHub Releases during the build.
- CI dependency audit now ignores `RUSTSEC-2026-0173` for `proc-macro-error2`, which is still a transitive dependency of the current Leptos stack and has no stable upstream replacement path yet.
- Archived documents are now consistently hidden from document listings/navigation after sync, and `archive_missing` now removes archived documents from both Meilisearch and the RAG index.
- `lekton-sync`: `README.md` and `index.md` files now correctly map to their parent directory slug (e.g. `docs/README.md` → `docs`), matching the documented behavior. Previously the slug was derived from the file name or title, ignoring the index-file convention.

### Changed
- `lekton-sync`: split monolithic `main.rs` into focused modules (`config`, `models`, `api`, `hash`, `slug`, `front_matter`, `attachment`, `scan`, `http`); tests moved to their respective modules and new unit tests added for `slug_from_title`, `apply_prefix`, `normalize_summary`, and `compute_metadata_hash`.

## [0.24.20] 2026-06-05

### Fixed
- `lekton-sync`: when a document has an explicit front-matter `slug` with fewer path segments than its filesystem path, and a `slug_prefix` is configured, `parent_slug` was incorrectly computed from the filesystem path instead of the effective slug. This caused `parent_slug` to equal the document's own slug (self-referential), making the document disappear from the navigation menu.

## [0.24.19] 2026-06-05

### Fixed
- `lekton-sync`: `.md` files starting with a UTF-8 BOM (saved by Notepad and some other Windows editors) were silently skipped because the `---` front-matter delimiter was not found.

### Changed
- `lekton-sync --verbose`: log each skipped `.md` file with the reason it was excluded (e.g. missing `lekton-import: true`).

## [0.24.18] 2026-05-26

### Fixed
- RAG chat: conversation history was fetched before saving the current user message, so the `saturating_sub(1)` slice intended to exclude the current message was instead discarding the last assistant response. The LLM never saw its own previous answers, causing it to repeat itself every turn.

### Changed
- RAG chat system prompt: instruct the model to answer only the specific question asked and skip context already covered in previous conversation turns, reducing repetition in multi-turn conversations.

## [0.24.17] 2026-05-22

## [0.24.16] 2026-05-19

### Fixed
- Ignore RUSTSEC-2026-0145 (`astral-tokio-tar` PAX header desynchronization) in `deny.toml`; transitive dev-dependency only via `testcontainers`, no production exposure.
- Syntax highlighting: switched syntect from `ClassStyle::Spaced` to `ClassStyle::SpacedPrefixed { prefix: "hl-" }` to prevent Tailwind utility classes (`.block`, `.meta`, `.storage`, etc.) from accidentally applying `display: block` to highlight spans and breaking code layout. Updated token CSS selectors to use `hl-` prefix.

## [0.24.15] 2026-05-19

### Added
- Code blocks in documentation now include server-side syntax highlighting (via `syntect`), a copy button, and a language badge.

## [0.24.14] 2026-05-18

### Fixed
- lekton-sync no longer treats relative links to `.md`/`.mdx`/`.markdown` files as binary attachments; those links are now left untouched for server-side resolution by `link_transform.rs`.

## [0.24.13] 2026-05-17

### Changed
- Rate limiting now ignores static fallback assets, replenishes faster by default, and can use forwarded client IP headers from configured trusted proxies.
- Auto-archiving during sync is now scoped to `source_id` instead of token permission scopes: only documents from the same source are candidates for archiving, so multiple sources can share overlapping token permissions without interfering with each other.
- `SyncRequest` now requires a `source_id` field (matches the `.lekton.yml` `id`).
- Ingest rejects writes to a document that is already owned by a different (non-archived) source, returning `403 Forbidden`.

## [0.24.12] 2026-05-16

### Added
- `source_id` field on `Document` (optional, backward-compatible) and `IngestRequest` (required): stable identifier for the import source, taken from the `id` field in `.lekton.yml`. Used to group documents from the same repository for relative link resolution.
- `find_all_by_source_id` on `DocumentRepository`: returns all non-archived documents sharing a `source_id`, used to build the sibling map at render time.
- `src/rendering/link_transform.rs`: link transformation module with `transform_url`, `rewrite_links_in_html`, and `rewrite_links_in_markdown` — resolves relative (`./foo`, `../bar`), repo-absolute (`/path`), and `lekton://docs/` cross-repo links at render time for both the web UI and MCP resources.
- lekton-sync now requires `id` in `.lekton.yml` and fails with a descriptive error if missing.

## [0.24.11] 2026-05-11

### Fixed
- Vertex AI embedding calls now use the native text embeddings `:predict` API with provider batch limits and configured output dimensionality.

## [0.24.10] 2026-05-11

## [0.24.9] 2026-05-11

### Added
- Vertex AI support for the embedding service: set `rag.embedding_vertex_project_id` (and optionally `rag.embedding_vertex_location`) to use GCP auth with automatic token refresh instead of a static `embedding_url`/`embedding_api_key`.

## [0.24.8] 2026-05-11

## [0.24.7] 2026-05-10

### Fixed
- Token expiry is now handled correctly in the schema registry and RAG chat: expired tokens trigger a silent refresh and retry instead of showing an error to the user.

## [0.24.6] 2026-05-10

### Fixed
- Schema viewer bundles (`scalar-standalone.js`, `asyncapi-standalone.js`) are now guaranteed to be present in the Docker image via an explicit copy step, preventing 404s when cargo considers the assets sync up-to-date.
- `loadScriptOnce` no longer caches rejected promises: a failed viewer script load no longer causes all subsequent schema views to instantly show raw JSON instead of retrying.

## [0.24.5] 2026-05-09

### Fixed
- Schema viewer pages now include the required Leptos meta head marker, avoid unsafe schema/sidebar resource reads during hydration, and load Scalar/AsyncAPI assets without dynamic `leptos_meta` head injection, preventing OpenAPI viewer panics from breaking site navigation.
- Anonymous browser sessions no longer attempt `POST /auth/refresh` without the `lekton_logged_in` indicator cookie, avoiding spurious auth refresh errors in the console.
- Sidebar active-item styling no longer adds the left border indicator, including the bundled Comelit custom CSS theme.

### Tests
- Added an OpenAPI schema e2e guard that fails on viewer HTTP errors, browser console/page errors, and broken hydrated navigation after rendering Scalar.

## [0.24.4] 2026-05-09

### Fixed
- Default `cargo check` no longer compiles browser-only splash animation code without the `hydrate` feature.

### Added
- Entrance animation on portal load: the navbar slides in from the top and page content fades up. For authenticated users (session cookie present) a full-screen spinner covers the layout while the access token is validated server-side; the animation starts once validation completes. Anonymous users see the animation immediately with full SSR content preserved.

## [0.24.3] 2026-05-09
### Fixed
- Docs sidebar no longer shows empty on leaf document pages (e.g. `/docs/getting-started`): falls back to showing all top-level navigation items when the current page has no section children.
- Sidebar active-item highlighting now uses `aria-current="page"` set reactively via `use_location()` in all sidebar components (docs, admin, schemas), replacing a broken CSS selector that matched all items instead of only the active one.

### Changed
- Portal layout now hides the contextual sidebar on routes that do not use it, keeps navbar search resilient when custom fonts widen navigation labels, refreshes section landing cards in documentation, and makes the admin Custom CSS editor more usable with shared theme/layout token guidance.
- `example.custom.css` and custom CSS active-item selectors updated to use `a[aria-current="page"]` for compatibility with the new active-state mechanism.
- Documentation cleanup: removed outdated `docs/ACTION_PLAN.md` (superseded by CHANGELOG), updated `ENH-005` status to Implemented, added Phase 6 deferred note to `IMPLEMENTATION_ROADMAP.md`, fixed stale `docs/ADRs/` reference in `AGENTS.md`.

## [0.24.2] 2026-05-08

### Fixed
- `justfile` `npm-deps` recipe now compares `package-lock.json` modification time against `node_modules/.package-lock.json` instead of checking for a single sentinel directory (`node_modules/mermaid`). This ensures `npm ci` is re-run whenever the lock file changes, preventing missing bundles (Scalar, AsyncAPI) after new npm dependencies are added.

### Tests
- Added e2e tests verifying that `scalar-standalone.js` and `asyncapi-standalone.js` are served with HTTP 200 and a JavaScript content-type, catching regressions where the build step did not copy the bundles to `public/js/`.
- Added e2e tests for the AsyncAPI viewer: schema appears in the registry list, detail page shows the version, and `AsyncApiStandalone.render()` produces visible spec content (seeded `event-api` schema with title "Event API").

### Added
- RAG chunking is now Mermaid-aware: Mermaid code fences are detected as diagram blocks and oversized diagrams are split into valid Mermaid fenced chunks by diagram family, with repeated declarations/context and structural guards for containers, schema blocks, interaction blocks, hierarchies, and chart axes. Reindex RAG vectors after deployment.

## [0.24.1] 2026-05-03

### Fixed
- Mermaid diagrams now render reliably in CI: `mermaid-loader.js` gains a MutationObserver that triggers rendering when `pre.mermaid` elements are injected into the DOM (e.g. after Leptos hydration applies `inner_html`), and render errors are now surfaced via `console.error` instead of silently swallowed.
- E2E test `waitForMermaidSvg` now passes `{ timeout }` as the correct third argument to `page.waitForFunction` (was incorrectly passed as `arg`), so the 30 s timeout is enforced; browser console errors are also captured and included in failure output.

### Performance
- Added MongoDB indexes (via migrations 005-007) on `schemas.name`, `users.id`, `users.email`, `users.provider_sub+provider_type`, and `refresh_tokens.token_hash`, eliminating full collection scans on registry page loads and authenticated requests.
- Schema list page now uses a projected query that excludes per-version endpoint arrays, reducing data transfer when loading the registry overview.
- Schema version content fetch now uses `$elemMatch` projection to load only the requested version's minimal fields instead of the full schema document.
- Schema viewer JS libraries (Scalar for OpenAPI, AsyncAPI React for AsyncAPI) are now bundled locally via npm and served from `/js/` instead of loaded from CDN at runtime, eliminating the 1–2 s first-render delay and removing the external network dependency.
- Schema detail page pre-fetches the default version's content in parallel with schema metadata, removing the sequential wait between the two calls.
- Schema detail handler now uses a projected MongoDB query (`find_by_name_summary`) that excludes endpoint arrays, reducing unnecessary data transfer for detail page loads.
- Schema content REST endpoints now emit `Cache-Control: private, max-age=3600` headers; version content is immutable so repeat fetches are served from browser cache.
- Static JS/CSS assets under `/js/` are now served with `Cache-Control: public, max-age=31536000, immutable` when requested with a `?v=` fingerprint query parameter (1-hour fallback without it); fingerprints are derived from file modification times at startup.
- Scalar and AsyncAPI viewer bundles now use versioned URLs (`?v=<mtime>`) in the injected script tags, matching the cache policy and avoiding redundant revalidation on repeat visits.
- Added `<link rel="preload">` hints for Scalar/AsyncAPI JS and CSS resources so the browser starts fetching them while the page HTML is still parsing.

## [0.24.0] 2026-04-30

### Changed
- Document ingestion now accepts an optional `summary`, `lekton-sync` reads it from front matter with recommended-length warnings, and MCP resource descriptions prefer the summary when present.
- MCP documentation resources now use the Lekton-specific `lekton://docs/...` URI scheme instead of the previous generic scheme.

## [0.23.1] 2026-04-30

### Added
- MCP Streamable HTTP session settings for stateful/stateless mode, JSON responses, inactivity timeout, and completed-stream resume cache lifetime.

## [0.23.0] 2026-04-30

### Added
- `source_path` field on `Document` and `IngestRequest`: stable file-identity for each document (relative path within the repository, e.g. `docs/guide.md`). Enables slug stability across title renames and drives migration lookup for pre-existing documents.
- `legacy_slug` field on `SyncDocumentEntry`: path-derived slug sent when the desired slug differs, allowing the server to locate documents indexed before `source_path` was introduced.
- `SyncUploadEntry` in the sync response: each entry in `to_upload` now carries both `source_path` and `actual_slug`, so the CLI always uses the server-resolved canonical slug when calling the ingest endpoint.
- `find_by_source_path` method on `DocumentRepository` for server-side source-path lookup.

### Changed
- `lekton-sync` CLI: slug derivation now follows the priority `front-matter slug → title-derived → path-derived`. Documents without an explicit `order` in front matter receive an implicit order based on alphabetical filename position within their parent group.
- Sync response `to_upload` changed from `Vec<String>` (slugs) to `Vec<SyncUploadEntry>` — **breaking change** for clients using the sync API directly.
- `IngestRequest.source_path` is now a required field.

## [0.22.1] 2026-04-28

## [0.22.0] 2026-04-27

### Changed
- RAG Markdown chunking now detects GFM tables with the Markdown parser and splits oversized tables by row groups with repeated headers. Reindex RAG vectors after deployment.

### Added
- `rag-bench` binary: multi-config RAG benchmark with automated document ingest. Reads `.toml` config files from `eval/configs/`, Markdown documents from `eval/docs/`, and a JSONL query set; for each config it creates an isolated Qdrant collection, ingests documents, runs queries, and produces per-config JSON reports plus a comparative Markdown report in `eval/reports/`.
- `expected_text_fragments` field in eval query records: case-insensitive substring match on retrieved chunk text, as an alternative to `expected_doc_slugs` for paragraph-level precision testing.
- `QdrantVectorStore::delete_collection()` method for clean-state benchmark lifecycle management.
- Both eval binaries (`rag-eval`, `rag-bench`) now use `clap` for argument parsing with `--help` support.
- Reranker now supports custom HTTP headers via `reranker_headers` config (env: `LKN__RAG__RERANKER_HEADERS__<NAME>`), enabling authenticated proxy scenarios.

### Fixed
- Doc page no longer shows duplicate title: removed standalone H1 header from `DocPage`; the markdown H1 serves as the only page title. Edit button moved inline with breadcrumbs.
- OpenAPI viewer (Scalar) Configure/Share/Deploy panel background was transparent due to stale DaisyUI 4 CSS variable references (`--bc`, `--b2`, `--b3`). Updated to DaisyUI 5 variables (`--color-base-*`, `--color-primary`) and set `--scalar-background-1` to an opaque value, fixing content bleed-through on overlay panels.

## [0.21.0] 2026-04-26
### Added
- Mermaid diagram rendering for Markdown documents and chat responses.

### Fixed
- Mermaid diagrams now re-render correctly when the user switches theme. The loader saves the original diagram source before mermaid replaces the element with SVG, and a `MutationObserver` on `data-theme` triggers a full re-initialize + re-render.

### Security
- Markdown renderer now sanitizes HTML via `ammonia` to prevent stored XSS from raw HTML in document sources and LLM chat responses.

### Changed
- Mermaid support is now opt-out via a `mermaid` Cargo feature (default-on). Disabling it removes the `npm ci` prerequisite, allowing backend-only `cargo check --features ssr` without Node.js installed.

### Tests
- RAG integration test: covers the full index_document → Qdrant vector search pipeline using a testcontainer and a deterministic in-process mock embedding service.
- Added `QdrantVectorStore::new(url, collection)` public constructor to support direct instantiation in tests without a full `RagConfig`.
- Playwright e2e specs for Mermaid rendering (`e2e/mermaid.spec.ts`) and chat page (`e2e/chat.spec.ts`).

## [0.20.0] 2026-04-25
### Added
- MCP schema registry tools: `list_schemas`, `search_schemas`, `get_schema_detail`, `get_schema_content`, `search_schema_operations` — expose the schema registry to MCP clients with user-level access control.
- Schema endpoint indexing: API operations (path, HTTP method, summary) are extracted from OpenAPI and AsyncAPI specs at ingest time and stored on `SchemaVersion`, enabling `search_schema_operations` without S3 round-trips.
- Admin panel "Schema Endpoint Re-index" card: backfills endpoint data for schema versions ingested before this feature, with progress bar and REST endpoints (`POST /api/v1/admin/schemas/reindex-endpoints`, `GET …/status`).

### Fixed
- Navigating to a folder that contains only sub-folders (no direct document children) now renders a section index page instead of "Document not found."

## [0.19.3] 2026-04-25
### Fixed
- Search reindex now succeeds for documents with `/` in their slug (e.g. `incidents/2025-12-13`): slugs are encoded to a valid Meilisearch document ID using base64 URL-safe encoding.

## [0.19.2] 2026-04-25

## [0.19.1] 2026-04-25

## [0.19.0] 2026-04-24
### Added
- Admin-triggered Meilisearch reindex: rebuild the full-text search index from stored documents, with REST endpoints and an admin panel button.

## [0.18.0] 2026-04-24
### Added
- Database migration framework: idempotent startup migrations tracked in `__migrations` collection; failed migrations block startup until resolved. First migration backfills `created_at` on existing `AccessLevelEntity` documents.
- Access-level inheritance: levels form a DAG via a new `inherits_from` field; a user assigned `cloud-developer` automatically inherits access to `developer`, `internal`, and so on. Cycle detection prevents invalid hierarchies.
- Implicit system levels `public` (every request) and `loggeduser` (every authenticated request) — injected at query time, never stored on users.
- Pre-computed `effective_access_levels` on the `User` document, kept in sync by a background recompute job when the inheritance graph changes.
- Admin UI: new "Access Levels" panel (list, create, edit inheritance, delete non-system levels) and "Users" panel (assign access levels, toggle write/draft permissions) under `/admin/access-levels` and `/admin/users`.

### Changed
- Permission model simplified: removed per-level `UserPermission` collection and granular `can_read/can_write/can_read_draft/can_write_draft` per level. Replaced with user-global `can_write`, `can_read_draft`, `can_write_draft` flags plus the assigned-levels set.
- Admin API: `PUT /api/v1/admin/users/{user_id}/access-levels` replaces the old `/permissions` endpoints.

## [0.17.0] 2026-04-23

### Changed
- RAG LLM configuration refactored from a flat `[rag]` block to a structured hierarchy: `[rag.llm]` holds shared defaults (url, api_key, model, headers, vertex settings); `[rag.chat]` configures the main chat step; optional steps (`[rag.analyzer]`, `[rag.hyde]`, `[rag.rewriter]`) are enabled by presence and disabled by absence, with each step able to override any LLM field independently (endpoint, auth, model, headers, Vertex project/location).

### Fixed
- Multi-hop RAG retrieval now uses RRF across sub-queries with a per-sub-query diversity guarantee: the top-ranked chunk from each sub-query is always included in the context window, preventing high-scoring topics from monopolising all context slots. The guarantee is enforced via `take_with_guarantee` and survives hybrid `rrf::fuse` reordering.

## [0.16.0] 2026-04-21

### Added
- Optional parent-section expansion for RAG chat context (`rag.expand_to_parent`): after reranking on small chunks, the final prompt can now expand each top hit to its full section by fetching and merging sibling chunks in order.
- RAG chat source references now cite sections, not just whole documents: retrieval results propagate `section_path`/`section_anchor`, source references are deduplicated per `slug#anchor`, and the chat UI links directly to section anchors when available.
- RAG chunking overhaul (Tier 1): token-aware splitting via `tiktoken-rs` cl100k_base (`chunk_size_tokens = 256`, `chunk_overlap_tokens = 64` in `RagConfig`); two-pass heading-aware splitter that splits by H1/H2 and merges tiny sections forward; atomic code fences and tables (oversize rather than torn); `SplitChunk` struct with `section_path`, `section_anchor`, and byte/char offsets; enriched `embedding_text` prefixed with `Title > Section` while `display_text` stays clean for prompt injection.
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
