

**Advanced RAG Pipeline**

Architecture & Implementation Analysis

*Query Decomposition • Hybrid Retrieval • Configurable Multi-Provider Backend*

Version 1.0 — Internal Technical Document

# **1\. Overview**

This document describes the architecture and incremental implementation plan for an advanced Retrieval-Augmented Generation (RAG) system designed to handle complex, multi-intent queries that naive single-embedding approaches fail to resolve reliably.

The system introduces a query orchestration layer that classifies incoming queries and routes them through the appropriate retrieval strategy, while keeping the common “simple query” path at zero overhead compared to a naive RAG baseline.

**Key design principle:** every strategy, every model endpoint, and every feature flag is externally configurable. Local inference (CPU or GPU on-premises) and cloud API providers are treated as interchangeable backends, selectable per pipeline step.

## **1.1 Problem Statement**

A standard RAG pipeline embeds the raw user query and performs a single nearest-neighbour search against a vector store. This approach degrades significantly when the query carries multiple orthogonal information needs, for example:

* **Multi-entity comparison:** "Compare device class A and device class B under firmware version X"

* **Multi-hop reasoning:** "Which firmware introduced the capability that caused the anomaly reported in ticket \#4421?"

* **Abstract \+ specific mix:** "Explain the general backup policy and show me devices that violate it"

The single embedding of such queries lands in a vector-space region that is equidistant from all sub-topics, causing the retriever to return mediocre chunks for all of them rather than highly relevant chunks for each.

## **1.2 Goals**

* Resolve multi-intent queries with precision comparable to targeted single-intent queries.

* Introduce zero additional latency or cost for queries that do not require decomposition.

* Allow each pipeline step to be backed by a different model and/or provider endpoint (local Ollama, vLLM, OpenAI-compatible API, Anthropic, etc.).

* Make every strategy opt-in via configuration, enabling gradual rollout and A/B testing.

* Deliver the system in independently deployable incremental tasks.

# **2\. System Architecture**

## **2.1 High-Level Pipeline**

The pipeline is organized in four sequential stages. Each stage is independently configurable and can fall back to a no-op pass-through when its strategy is disabled.

| \# | Stage | Responsibility | Config key |
| :---- | :---- | :---- | :---- |
| 1 | **Query Analysis** | Classify complexity, emit sub-queries | analyzer.\* |
| 2 | **Retrieval** | Execute single or parallel vector searches | retrieval.\* |
| 3 | **Context Assembly** | Deduplicate, re-rank, merge chunks | assembly.\* |
| 4 | **Answer Generation** | Generate final response from context | generator.\* |

## **2.2 Query Complexity Classes**

The analyzer classifies every incoming query into one of three complexity classes. The classification drives all downstream routing decisions.

| Class | Definition | Example |
| :---- | :---- | :---- |
| **simple** | Single information need, one entity | "What is the default backup interval?" |
| **multi\_entity** | Multiple independent entities to retrieve in parallel | "Compare the MQTT config of device A and B" |
| **multi\_hop** | Sequential retrieval where each step depends on prior results | "Which firmware introduced the capability that caused anomaly X?" |

## **2.3 Retrieval Strategies**

Each complexity class maps to a retrieval strategy. Strategies are not mutually exclusive and can be combined. All are individually togglable.

### **2.3.1 Direct Retrieval (simple)**

The baseline strategy. A single embedding of the original query is used to perform one vector search. No additional LLM calls are made.

### **2.3.2 Query Decomposition (multi\_entity)**

A lightweight LLM call decomposes the original query into N atomic sub-queries. Each sub-query is embedded and searched independently. Results are merged and deduplicated before context assembly. Sub-queries run in parallel to minimise added latency.

### **2.3.3 Iterative / Agentic Retrieval (multi\_hop)**

Sub-queries are executed sequentially. After each retrieval step the system evaluates whether the accumulated context is sufficient to answer the original query. Iteration stops either when the sufficiency check passes or when the configured maximum number of hops is reached.

**Sufficiency check options:** (a) lightweight LLM call, (b) token-count threshold, (c) minimum retrieval score threshold. Selectable per deployment via configuration.

### **2.3.4 Optional Enhancements**

The following techniques can be layered on top of any strategy:

* **HyDE (Hypothetical Document Embeddings):** an LLM generates a synthetic answer to the query; that answer is embedded instead of the raw question. Improves recall for queries that are phrased very differently from the indexed documents.

* **RAG-Fusion / RRF:** N rephrasings of the query are retrieved independently; results are merged using Reciprocal Rank Fusion. Adds robustness against specific query formulations.

* **Step-Back Prompting:** the query is first generalised to a higher-level question, context is retrieved for both the abstract and concrete forms, and both are fed to the generator.

# **3\. Configuration Schema**

All runtime behaviour is controlled by a single configuration object (YAML/JSON/env-compatible). No code changes are required to switch providers, toggle strategies, or adjust model parameters.

## **3.1 Full Configuration Reference**

| \# rag\_config.yaml \# ── Analyzer ────────────────────────────────────────────────── analyzer:   enabled: true                  \# false → always treat query as 'simple'   endpoint: http://localhost:11434/v1   \# Ollama local   model: phi3:mini   temperature: 0.0   max\_tokens: 256   timeout\_ms: 4000 \# ── Embedding ───────────────────────────────────────────────── embedding:   endpoint: http://localhost:11434/v1   model: nomic-embed-text   batch\_size: 32 \# ── Vector Store ────────────────────────────────────────────── vector\_store:   provider: qdrant              \# qdrant | weaviate | pgvector   url: http://localhost:6333   collection: documents   top\_k: 5   score\_threshold: 0.65 \# ── Retrieval strategies ─────────────────────────────────────── retrieval:   decomposition:     enabled: true     max\_sub\_queries: 4   iterative:     enabled: true     max\_hops: 4     sufficiency\_mode: llm        \# llm | token\_count | score     sufficiency\_token\_threshold: 1200     sufficiency\_score\_threshold: 0.80 \# ── Optional enhancements ────────────────────────────────────── enhancements:   hyde:     enabled: false     endpoint: http://localhost:11434/v1     model: phi3:mini   rag\_fusion:     enabled: false     num\_variants: 3     endpoint: http://localhost:11434/v1     model: phi3:mini   step\_back:     enabled: false     endpoint: http://localhost:11434/v1     model: phi3:mini \# ── Answer Generator ─────────────────────────────────────────── generator:   endpoint: https://api.anthropic.com  \# external provider   model: claude-sonnet-4-20250514   api\_key\_env: ANTHROPIC\_API\_KEY   temperature: 0.3   max\_tokens: 2048   context\_window\_limit: 32000 \# ── Context Assembly ─────────────────────────────────────────── assembly:   deduplication:     enabled: true     similarity\_threshold: 0.92   \# cosine; chunks above this are merged   reranker:     enabled: false               \# enable in Task 4     endpoint: http://localhost:11434/v1     model: bge-reranker-v2-m3   max\_context\_chunks: 12 |
| :---- |

## **3.2 Provider Endpoint Abstraction**

Each configurable endpoint follows the OpenAI-compatible chat/completions and embeddings API contract. This means any of the following can be used interchangeably as a backend for any step:

* **Ollama:** http://localhost:11434/v1 — local CPU or GPU inference

* **vLLM:** any host:port — on-premises GPU server

* **LM Studio:** http://localhost:1234/v1 — developer workstation

* **OpenAI:** https://api.openai.com/v1

* **Anthropic:** via compatibility shim or native SDK

* **Azure OpenAI:** https://\<resource\>.openai.azure.com

The recommended split for a hybrid on-prem/cloud deployment is:

| Step | Recommended backend | Rationale |
| :---- | :---- | :---- |
| **analyzer()** | Local CPU (Ollama) | Short output, low latency acceptable, no cost |
| **embed()** | Local CPU (Ollama) | Encoder-only, \~5-20ms per call on CPU |
| **sufficiency\_check()** | Local CPU or heuristic | Can be replaced by token threshold at zero cost |
| **answer()** | Local GPU or Cloud API | Quality-critical; justifies GPU or API spend |
| **reranker() (optional)** | Local CPU or GPU | Cross-encoder, heavier than bi-encoder; GPU preferred |

# **4\. Core Orchestration Logic**

The following pseudocode defines the main entry point. It is deliberately language-agnostic; concrete implementation notes are provided in Section 6\.

| async def rag\_answer(query, conversation\_context, config) \-\> str:     \# ── Stage 1: Query Analysis ──────────────────────────────     if config.analyzer.enabled:         plan \= await analyzer.classify(query, config.analyzer)         \# plan \= { complexity: 'simple'|'multi\_entity'|'multi\_hop',         \#           sub\_queries: \[str\] }     else:         plan \= { complexity: 'simple', sub\_queries: \[query\] }     \# ── Stage 2: Retrieval ────────────────────────────────────     chunks \= await retrieve(query, plan, config)     \# ── Stage 3: Context Assembly ─────────────────────────────     context \= await assemble(chunks, config.assembly)     \# ── Stage 4: Answer Generation ───────────────────────────     return await generator.answer(query, context,                                   conversation\_context,                                   config.generator) async def retrieve(query, plan, config) \-\> \[Chunk\]:     match plan.complexity:         case 'simple':             embedding \= await embed(query, config.embedding)             if config.enhancements.hyde.enabled:                 embedding \= await hyde\_embed(query, embedding, config)             return await vector\_store.search(embedding,                                              config.vector\_store.top\_k,                                              config)         case 'multi\_entity':             \# parallel retrieval — no extra LLM calls after decomposition             tasks \= \[                 retrieve\_single(q, config)                 for q in plan.sub\_queries             \]             results \= await gather\_parallel(tasks)             return flatten(results)         case 'multi\_hop':             accumulated \= \[\]             for q in plan.sub\_queries:                 new\_chunks \= await retrieve\_single(q, config)                 accumulated.extend(new\_chunks)                 if await is\_sufficient(query, accumulated, config):                     break             return accumulated async def is\_sufficient(query, chunks, config) \-\> bool:     mode \= config.retrieval.iterative.sufficiency\_mode     if mode \== 'token\_count':         return token\_count(chunks) \>= config.retrieval.iterative                                            .sufficiency\_token\_threshold     elif mode \== 'score':         return max(c.score for c in chunks) \>=                config.retrieval.iterative.sufficiency\_score\_threshold     else:  \# 'llm'         return await llm\_sufficiency\_check(query, chunks,                                            config.analyzer) |
| :---- |

# **5\. Incremental Implementation Plan**

The implementation is broken into five self-contained tasks. Each task is independently deployable and testable. Later tasks depend only on the public interface (not the internals) of earlier ones.

## **Task 1 — Baseline RAG with Provider Abstraction**

| 🎯 Objective |
| :---- |
| Establish a working naive RAG pipeline backed by a configuration-driven provider abstraction. This is the zero-overhead baseline; all later tasks layer on top of it. |

### **Deliverables**

* ProviderClient class: wraps HTTP calls to any OpenAI-compatible /chat/completions and /embeddings endpoint, reading base\_url, model, api\_key\_env, timeout from config.

* EmbeddingService: calls the configured embedding endpoint, supports batching.

* VectorStoreClient: thin adapter for at least one vector DB (Qdrant recommended); abstracts search(embedding, top\_k) behind a common interface.

* GeneratorService: calls the configured generator endpoint with a system prompt \+ retrieved context.

* RagPipeline.answer(query) — naive single-embed flow wired together.

* Configuration loader: reads rag\_config.yaml, validates schema, exposes typed config object.

### **Acceptance Criteria**

1. Swapping the generator endpoint from Ollama to Anthropic requires only a config change, zero code changes.

2. Embedding and generation can point to different endpoints simultaneously.

3. Integration test: answer a simple factual query and assert a non-empty response.

## **Task 2 — Query Analyzer & Decomposition**

| 🎯 Objective |
| :---- |
| Add the analyzer stage. Simple queries must pass through with zero overhead. Multi-entity queries must trigger parallel retrieval. |

### **Deliverables**

* QueryAnalyzer.classify(query, config) — calls the analyzer LLM endpoint with a structured JSON prompt; returns a QueryPlan with complexity class and sub\_queries list.

* Parallel retrieval path in RagPipeline for multi\_entity: runs N vector searches concurrently.

* Deduplication in assembly stage: removes chunks with cosine similarity above configured threshold.

* analyzer.enabled flag: when false, QueryPlan defaults to simple with no LLM call.

### **Acceptance Criteria**

4. A simple query incurs exactly one embedding call and zero analyzer LLM calls when analyzer.enabled \= false.

5. A multi-entity query returns chunks covering all mentioned entities.

6. Unit test: mock analyzer response, assert parallel retrieval is invoked N times.

## **Task 3 — Iterative / Multi-Hop Retrieval**

| 🎯 Objective |
| :---- |
| Implement the multi\_hop path with configurable sufficiency check. Validate that the iteration stops early when context is already sufficient. |

### **Deliverables**

* SufficiencyChecker interface with three concrete implementations: LlmSufficiencyChecker, TokenCountChecker, ScoreThresholdChecker.

* Iterative retrieval loop in RagPipeline with max\_hops guard.

* retrieval.iterative.enabled flag: when false, multi\_hop queries fall back to multi\_entity parallel retrieval.

* Telemetry hooks: emit a structured log event per retrieval step (hop index, chunks retrieved, sufficiency result).

### **Acceptance Criteria**

7. With sufficiency\_mode \= token\_count and a low threshold, the loop exits after the first hop.

8. With max\_hops \= 1, only one retrieval call is made regardless of sufficiency.

9. Integration test: multi-hop query returns chunks from at least two distinct sub-topics.

## **Task 4 — Optional Enhancement Strategies**

| 🎯 Objective |
| :---- |
| Add HyDE, RAG-Fusion, and Step-Back as opt-in enhancements. Each must be independently togglable with no impact when disabled. |

### **Deliverables**

* **HyDE:** HydeEmbedder generates a synthetic document, embeds it, and uses that embedding in place of the query embedding. Activated by enhancements.hyde.enabled.

* **RAG-Fusion:** RagFusion generates N query variants, retrieves for each, and merges results with Reciprocal Rank Fusion. Activated by enhancements.rag\_fusion.enabled.

* **Step-Back:** StepBackPrompter generates an abstract version of the query, retrieves context for both abstract and concrete queries, concatenates results. Activated by enhancements.step\_back.enabled.

* **Cross-encoder reranker (optional):** RerankerService re-scores assembled chunks using a cross-encoder model. Activated by assembly.reranker.enabled.

### **Acceptance Criteria**

10. Disabling all enhancements produces identical output to Task 1 baseline (same code path).

11. HyDE and RAG-Fusion can both be enabled simultaneously without conflict.

12. Benchmark: HyDE improves mean retrieval score by a measurable margin on a held-out query set.

## **Task 5 — Observability & Production Hardening**

| 🎯 Objective |
| :---- |
| Instrument the full pipeline with structured logs, metrics, and traces. Add resilience patterns: circuit breaker, per-step timeouts, fallback strategies. |

### **Deliverables**

* OpenTelemetry spans for each pipeline stage with attributes: complexity\_class, num\_sub\_queries, hops\_executed, chunks\_assembled, generator\_model.

* Prometheus metrics: rag\_request\_total (by complexity), rag\_latency\_seconds (by stage), rag\_retrieval\_score\_avg.

* Circuit breaker on every external provider call; fallback to direct retrieval on analyzer/enhancer failure.

* Per-step timeout enforcement from config (analyzer.timeout\_ms, etc.).

* Graceful degradation: if decomposition fails, log a warning and fall through to simple retrieval.

### **Acceptance Criteria**

13. A Grafana dashboard (or equivalent) shows per-stage latency percentiles.

14. Simulating analyzer endpoint downtime causes zero failed user requests (fallback to simple).

15. Traces correlate retrieval sub-spans to their parent query span end-to-end.

# **6\. Implementation Notes**

## **6.1 Recommended Model Choices**

| Step | Recommended model | Hardware | Typical latency |
| :---- | :---- | :---- | :---- |
| **analyzer()** | Phi-3-mini / Qwen2.5-1.5B Q4 | CPU (Ollama) | 1–4 s |
| **embed()** | nomic-embed-text / bge-m3 | CPU (Ollama) | 5–20 ms |
| **sufficiency()** | heuristic or same as analyzer | CPU / none | \~0 ms / 1–4 s |
| **answer()** | Qwen2.5-14B Q4 / Cloud API | GPU or Cloud | 3–15 s |
| **reranker()** | bge-reranker-v2-m3 | CPU or GPU | 100–500 ms |

## **6.2 Analyzer Prompt Contract**

The analyzer must be prompted to return a strictly typed JSON object with no surrounding prose. The schema must be validated before use; on parse failure the system falls back to the simple class.

| SYSTEM: You are a query classifier. Respond ONLY with a JSON object. No preamble, no explanation, no markdown fences. Schema: {   "complexity": "simple" | "multi\_entity" | "multi\_hop",   "sub\_queries": \["string"\]   // empty if complexity \== "simple" } Rules: \- simple: single entity or concept, one retrieval pass is sufficient \- multi\_entity: multiple independent entities that can be retrieved in parallel \- multi\_hop: answer to one sub-question is needed to form the next USER: {query} |
| :---- |

## **6.3 Deduplication Strategy**

When multiple sub-queries return overlapping chunks, deduplication is applied before context assembly. The recommended approach is cosine similarity comparison between chunk embeddings. Chunks with similarity above the configured threshold are merged into the highest-scored representative. This avoids inflating the context window with near-duplicate content, which wastes generator tokens and can cause repetitive answers.

## **6.4 Context Window Budget**

The assembled context must respect the generator model’s context window limit (assembly.context\_window\_limit). The assembly stage trims the chunk list to fit, prioritising by retrieval score. A max\_context\_chunks hard cap is applied first as a fast guard, followed by a token count check.

## **6.5 Testing Strategy**

* Unit tests for the analyzer: mock LLM responses and assert correct QueryPlan construction.

* Unit tests for each SufficiencyChecker implementation with known inputs.

* Integration tests with a local vector store (Qdrant in Docker) and a local Ollama instance.

* Golden-set evaluation: a fixed set of 20–50 queries with expected entity coverage, scored automatically against retrieved chunk metadata.

* Regression guard: no Task N commit may degrade the Task 1 baseline latency by more than 5% on simple queries (measured via CI benchmark).

# **7\. Risks & Mitigations**

| Risk | Likelihood | Mitigation |
| :---- | :---- | :---- |
| Analyzer misclassifies query, inflating cost | Medium | Monitor complexity distribution; tune prompt; fall back to simple on parse error |
| Local model too slow for analyzer step | Low–Medium | Switch to heuristic classification or larger cloud model; latency is bounded by timeout\_ms |
| Parallel retrieval increases vector DB load | Low | Cap max\_sub\_queries; add connection pooling; monitor QPS |
| Context window overflow on multi-hop queries | Medium | Enforce max\_context\_chunks and token budget; reranker reduces chunk count |
| Cloud API latency degrades answer step | Low | Local GPU fallback via config swap; streaming response to reduce perceived latency |

# **8\. Glossary**

| Term | Definition |
| :---- | :---- |
| **RAG** | Retrieval-Augmented Generation. Combines a retriever (vector search) with a generative LLM. |
| **HyDE** | Hypothetical Document Embeddings. Embeds a generated synthetic answer rather than the raw query. |
| **RRF** | Reciprocal Rank Fusion. Score merging algorithm for combining ranked lists from multiple retrievals. |
| **Query Decomposition** | Breaking a complex query into atomic sub-queries, each targeting a single information need. |
| **Multi-hop** | Retrieval pattern where each step may depend on the output of the previous one. |
| **Cross-encoder** | A model that scores query–document pairs jointly; more accurate but slower than bi-encoders. |
| **Sufficiency check** | A condition evaluated after each retrieval hop to decide whether to stop iteration. |
| **Provider abstraction** | A configuration-driven layer that routes LLM/embedding calls to any OpenAI-compatible endpoint. |

*End of Document*

