# Google Ranking API reranking

Google reranking is optional and disabled by default. It applies after retrieval
in RAG chat and `rag-eval`; `rag-bench` accepts the same reranker fields in its
benchmark TOML. MCP `search_documents` continues to search Qdrant directly.

```sh
LKN__RAG__RERANKER_VERTEX_PROJECT_ID=my-project
# Optional; empty uses this Google ranking model:
LKN__RAG__RERANKER_VERTEX_MODEL=semantic-ranker-default@latest
```

These map to `rag.reranker_vertex_project_id` and `rag.reranker_vertex_model` in the
typed application configuration. A nonempty Google project takes precedence over
`reranker_url`, `reranker_api_key`, `reranker_model`, and `reranker_headers`; those
settings are ignored for Google. The dedicated Google model field leaves a pinned
cross-encoder model intact when switching providers. Clearing the
Google project restores the existing Jina/Infinity/Cohere-compatible provider;
with both project and URL empty, reranking remains disabled. Chat and embedding
models and their configuration are independent and unchanged.

## Authentication and project setup

Reuse the same service account / Application Default Credentials supported by
Lekton's Vertex chat and embedding providers (`gcp_auth`), for example an existing
`GOOGLE_APPLICATION_CREDENTIALS` service account file or an attached runtime
service account. No static bearer token or reranker API key is needed. Each call
asks the token provider for a valid token; the library caches and refreshes it.
The requested OAuth scope is `https://www.googleapis.com/auth/cloud-platform`,
which the [rank method accepts](https://docs.cloud.google.com/generative-ai-app-builder/docs/reference/rest/v1/projects.locations.rankingConfigs/rank).

Before opting in, a project administrator must:

- Enable billing and the **Discovery Engine API** (`discoveryengine.googleapis.com`)
  on the configured ranking project. Enabling Vertex AI alone is insufficient.
- Grant the runtime identity `discoveryengine.rankingConfigs.rank` on that
  project's ranking config, through an appropriate role or a custom role. Consult
  the [Discovery Engine roles](https://docs.cloud.google.com/iam/docs/roles-permissions/discoveryengine).
  Existing chat/embedding permissions do not imply ranking permission.
- Ensure the identity can consume services on the configured project
  (`serviceusage.services.use`, for example via Service Usage Consumer). This
  permission is required for every call because Lekton always sends the configured
  project in `x-goog-user-project` for quota/billing. A 403 can indicate this
  permission is missing even when ranking and Vertex chat permissions are present.

The implementation always calls
`https://discoveryengine.googleapis.com/v1/projects/PROJECT/locations/global/rankingConfigs/default_ranking_config:rank`.
It does not inherit a regional Vertex endpoint, enable APIs, or change IAM.
Use a project ID or project number; for legacy domain-scoped IDs such as
`example.com:project`, configure the project number.

## Limits and fallback

All reranker providers use separate retrieval and final-context budgets:

```sh
LKN__RAG__RETRIEVAL_CANDIDATE_LIMIT=50
LKN__RAG__CONTEXT_CHUNK_LIMIT=5
```

These map to `rag.retrieval_candidate_limit` and `rag.context_chunk_limit`.
Both are positive; the final context limit must not exceed the candidate limit,
and the candidate limit must not exceed **100**. Defaults are 50 candidates and
5 final chunks. The same fields work in `rag-bench` TOML. Invalid budgets are
rejected at startup and in retrieval-only evaluation initialization.

Each vector/subquery search fetches up to the candidate limit when reranking is
active. Results are merged with RRF, deduplicated by vector point (or source/chunk
identity for legacy hits without IDs), and fused with the optional lexical signal
**before** the merged candidate cap. Subquery top hits are reserved within that
pool, subject to the configured cap. The reranker chooses the final context by
relevance; subquery diversity is not forced again after reranking. Provider errors
fall back to the first context-sized slice of the candidate pool. Attachments are
filtered before fusion/reranking when attachment indexing is disabled, and all
search calls retain caller access-level and draft filters.

**Hybrid limitation:** the existing Meilisearch integration returns document slugs
and short previews, not indexed chunk payloads. It boosts matching vector-derived
candidates through RRF; lexical-only hits cannot enter this pool. Fetching their
full chunks requires an additional chunk-retrieval interface. Keyword search still
uses the original query when HyDE replaces embedding inputs. HyDE generates query
embeddings only; its synthetic answers never become context chunks.

Without a reranker, the pre/post lists stay final-context sized. Vector-only
retrieval requests only that many hits. Hybrid retrieval retains its existing
small overfetch (three times the final limit, capped by the candidate budget).
Thrifty mode skips analyzer, HyDE, rewriting, lexical search and reranking, and
requests only the final vector count. The existing prompt-character budget and
optional parent-section expansion still apply after chunk selection.

`rag-eval` uses this same pipeline: `pre_rerank` exposes the bounded candidate pool
and `post_rerank` the bounded final context. Choose a comparison `--top-k` no larger
than the final context limit, or raise the context budget for larger-k evaluations.
No live Google quality evaluation is implied by the mock regression tests.

The [ranking guide](https://docs.cloud.google.com/generative-ai-app-builder/docs/ranking)
currently documents up to **1000 records** per request. Lekton rejects larger
candidate sets and uses chat's original retrieval order; it does not split batches
and compare independently ranked scores. Older client references still mention
200 records; deployments subject to that limit also fall back on API rejection.
Google's per-record context window depends on the selected model (512 tokens for
older models, 1024 for version 004 and later); Google handles oversized content
through segmentation and may truncate very long records. Lekton sends the chunk
text and document title without its own token truncation.

Empty candidate lists or `top_n = 0` perform no auth or HTTP work. Otherwise,
`topN` is clamped to the candidate count. Per-request opaque IDs map results back
to the original chunks, preserving source metadata and retrieval scores, as the
URL provider does. The returned scores determine order only. Unknown, malformed,
or duplicate IDs, invalid scores, missing records, HTTP/auth failures, and
30-second timeouts all return an error to chat's retrieval-order fallback. A
missing scalar score is treated as zero per protobuf JSON defaults. The timeout
covers token refresh and HTTP response processing. ADC initialization also has a
30-second bound; if it fails at server startup, chat continues without reranking
and logs a warning. Evaluation initialization reports the error instead.

Tests use local mock HTTP servers and a fake `TokenProvider`. They verify token
requests and refreshed token use without credentials. They do not validate live
Google IAM, API enablement, quotas, or retrieval quality for a particular corpus.
