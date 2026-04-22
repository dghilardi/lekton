use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use async_openai::types::chat::{
    ChatCompletionRequestAssistantMessage, ChatCompletionRequestAssistantMessageContent,
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
    ChatCompletionRequestSystemMessageContent, ChatCompletionRequestUserMessage,
    ChatCompletionRequestUserMessageContent, CreateChatCompletionRequest,
};
use chrono::Utc;
use futures::StreamExt;
use uuid::Uuid;

use crate::auth::models::UserContext;
use crate::config::RagConfig;
use crate::db::chat_models::{ChatMessage, ChatSession, SourceReference};
use crate::db::chat_repository::ChatRepository;
use crate::error::AppError;
use crate::rag::analyzer::{Complexity, QueryAnalyzer, QueryPlan};
use crate::rag::client::format_llm_error;
use crate::rag::embedding::EmbeddingService;
use crate::rag::hyde::HydeService;
use crate::rag::provider::LlmProvider;
use crate::rag::query_rewriter::QueryRewriter;
use crate::rag::reranker::Reranker;
use crate::rag::vectorstore::{VectorSearchResult, VectorStore};
use crate::search::client::SearchService;

/// Maximum number of context chunks returned to the LLM.
const MAX_CONTEXT_CHUNKS: usize = 5;
/// Maximum number of conversation history messages to include in the prompt.
const MAX_HISTORY_MESSAGES: usize = 20;
/// How many extra Qdrant candidates to fetch when hybrid search or reranking
/// is enabled (gives RRF/reranker room to reorder before truncating).
const CANDIDATE_MULTIPLIER: usize = 3;

/// Orchestrates RAG chat: retrieval, prompt building, and LLM streaming.
pub struct ChatService {
    embedding: Arc<dyn EmbeddingService>,
    vectorstore: Arc<dyn VectorStore>,
    expand_to_parent: bool,
    /// Optional Meilisearch service used for hybrid search (RRF). `None` when
    /// hybrid search is disabled or Meilisearch is not configured.
    search_service: Option<Arc<dyn SearchService>>,
    /// Optional cross-encoder reranker applied after retrieval. `None` when
    /// `reranker_url` is empty.
    reranker: Option<Arc<dyn Reranker>>,
    /// Optional query analyzer for complexity classification and decomposition.
    /// `None` when `analyzer_model` is empty.
    analyzer: Option<QueryAnalyzer>,
    /// Optional HyDE service. When present, each query string is replaced by a
    /// synthetically generated hypothetical document before embedding.
    hyde: Option<HydeService>,
    chat_repo: Arc<dyn ChatRepository>,
    llm_provider: Arc<LlmProvider>,
    chat_model: String,
    chat_headers: HashMap<String, String>,
    tera: tera::Tera,
    system_template_name: String,
    query_rewriter: Option<QueryRewriter>,
}

/// Result of a pure retrieval pass (no LLM generation, no chat persistence).
///
/// Returned by [`ChatService::retrieve_only`] and used both by
/// [`ChatService::stream_response`] (which forwards `post_rerank` to the LLM)
/// and by offline tooling such as the `rag-eval` binary, which needs both the
/// pre-rerank and post-rerank candidate sets to compute retrieval metrics.
#[derive(Debug, Clone)]
pub struct RetrievalOutput {
    /// The query string actually used for retrieval (after query rewriting).
    pub retrieval_query: String,
    /// Plan produced by the query analyzer (or [`QueryPlan::simple`] when
    /// the analyzer is disabled or fails).
    pub query_plan: QueryPlan,
    /// The exact set of query strings that was embedded — equal to
    /// `[retrieval_query]` for simple plans, or the (optionally HyDE-expanded)
    /// sub-queries for decomposed plans.
    pub queries_embedded: Vec<String>,
    /// Top-K candidates after vector search, dedup, and (optional) hybrid RRF
    /// — but **before** the cross-encoder reranker. Already truncated to
    /// `MAX_CONTEXT_CHUNKS`.
    pub pre_rerank: Vec<VectorSearchResult>,
    /// Final top-K used to build the LLM prompt. Equal to `pre_rerank` when
    /// the reranker is disabled or fails.
    pub post_rerank: Vec<VectorSearchResult>,
}

/// A token event yielded by the streaming chat response.
#[derive(Debug, serde::Serialize)]
#[serde(tag = "type")]
pub enum ChatEvent {
    /// First event — carries the session ID.
    #[serde(rename = "session")]
    Session { session_id: String },
    /// Retrieved source references for the in-progress assistant reply.
    #[serde(rename = "sources")]
    Sources { sources: Vec<SourceReference> },
    /// A content delta token.
    #[serde(rename = "delta")]
    Delta { content: String },
    /// Stream finished — carries the saved message ID so the client can
    /// attach feedback to the correct message.
    #[serde(rename = "done")]
    Done { message_id: Option<String> },
    /// An error occurred.
    #[serde(rename = "error")]
    Error { message: String },
}

impl ChatService {
    pub fn from_rag_config(
        config: &RagConfig,
        llm_provider: Arc<LlmProvider>,
        chat_repo: Arc<dyn ChatRepository>,
        embedding: Arc<dyn EmbeddingService>,
        vectorstore: Arc<dyn VectorStore>,
        search_service: Option<Arc<dyn SearchService>>,
        reranker: Option<Arc<dyn Reranker>>,
    ) -> Result<Self, AppError> {
        if config.chat_model.is_empty() {
            return Err(AppError::Internal(
                "chat_model is required for RAG chat".into(),
            ));
        }

        let mut tera = tera::Tera::default();
        let template_name = "system_prompt";
        tera.add_raw_template(template_name, &config.system_prompt_template)
            .map_err(|e| AppError::Internal(format!("invalid system_prompt_template: {e}")))?;

        let hybrid_search_service = if config.hybrid_search_enabled {
            if search_service.is_some() {
                tracing::info!("RAG hybrid search (RRF) enabled");
            } else {
                tracing::warn!(
                    "hybrid_search_enabled = true but Meilisearch is not configured — \
                     falling back to vector-only retrieval"
                );
            }
            search_service
        } else {
            None
        };

        if reranker.is_some() {
            tracing::info!("RAG cross-encoder reranker enabled");
        }

        let analyzer_provider = if !config.analyzer_url.is_empty() {
            tracing::info!(url = %config.analyzer_url, "RAG query analyzer using dedicated endpoint");
            Arc::new(LlmProvider::new_openai_compatible(
                config.analyzer_url.clone(),
                String::new(),
            ))
        } else {
            llm_provider.clone()
        };
        let analyzer = QueryAnalyzer::from_rag_config(config, analyzer_provider);
        if analyzer.is_some() {
            tracing::info!(model = %config.analyzer_model, "RAG query analyzer enabled");
        }

        let hyde_provider = if !config.hyde_url.is_empty() {
            tracing::info!(url = %config.hyde_url, "RAG HyDE using dedicated endpoint");
            Arc::new(LlmProvider::new_openai_compatible(
                config.hyde_url.clone(),
                String::new(),
            ))
        } else {
            llm_provider.clone()
        };
        let hyde = HydeService::from_rag_config(config, hyde_provider);
        if hyde.is_some() {
            tracing::info!(model = %config.hyde_model, "RAG HyDE enabled");
        }

        Ok(Self {
            embedding,
            vectorstore,
            expand_to_parent: config.expand_to_parent,
            search_service: hybrid_search_service,
            reranker,
            analyzer,
            hyde,
            chat_repo,
            llm_provider: llm_provider.clone(),
            chat_model: config.chat_model.clone(),
            chat_headers: config.chat_headers.clone(),
            tera,
            system_template_name: template_name.to_string(),
            query_rewriter: QueryRewriter::from_rag_config(config, llm_provider),
        })
    }

    /// Stream a chat response as a series of [`ChatEvent`]s.
    ///
    /// Returns a stream that the caller can forward as SSE.
    pub async fn stream_response(
        &self,
        user_ctx: &UserContext,
        session_id: Option<String>,
        user_message: String,
    ) -> Result<std::pin::Pin<Box<dyn futures::Stream<Item = ChatEvent> + Send>>, AppError> {
        // 1. Resolve or create session
        let session_id = match session_id {
            Some(id) => {
                // Verify the session exists and belongs to this user
                let session = self
                    .chat_repo
                    .get_session(&id)
                    .await?
                    .ok_or_else(|| AppError::NotFound("Chat session not found".into()))?;
                if session.user_id != user_ctx.user.user_id {
                    return Err(AppError::NotFound("Chat session not found".into()));
                }
                id
            }
            None => {
                let id = Uuid::new_v4().to_string();
                let title = truncate_title(&user_message);
                let now = Utc::now();
                self.chat_repo
                    .create_session(ChatSession {
                        id: id.clone(),
                        user_id: user_ctx.user.user_id.clone(),
                        title,
                        created_at: now,
                        updated_at: now,
                    })
                    .await?;
                id
            }
        };

        // 2. Fetch conversation history (needed both for query rewriting and prompt building)
        let history = self
            .chat_repo
            .get_messages(&session_id, MAX_HISTORY_MESSAGES)
            .await?;

        // 3. Save user message
        self.chat_repo
            .add_message(ChatMessage {
                id: Uuid::new_v4().to_string(),
                session_id: session_id.clone(),
                role: "user".into(),
                content: user_message.clone(),
                sources: None,
                created_at: Utc::now(),
            })
            .await?;

        // 4-7. Pure retrieval (analyzer + HyDE + decomposition + vector search +
        //      hybrid RRF + reranker). Side-effect-free w.r.t. chat persistence.
        let retrieval = self
            .retrieve_only(user_ctx, &user_message, &history, &session_id)
            .await?;

        let search_results = self
            .expand_results_to_parent(user_ctx, retrieval.post_rerank, &session_id)
            .await?;
        let source_references = build_source_references(&search_results);

        // 7. Build context string from search results
        let context = search_results
            .iter()
            .map(|r| {
                format!(
                    "[{}] ({})\n{}",
                    r.document_title, r.document_slug, r.chunk_text
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");

        // 8. Render system prompt via Tera
        let mut tera_ctx = tera::Context::new();
        tera_ctx.insert("context", &context);
        tera_ctx.insert("question", &user_message);
        let system_prompt = self
            .tera
            .render(&self.system_template_name, &tera_ctx)
            .map_err(|e| AppError::Internal(format!("tera render failed: {e}")))?;

        // 9. Build message array: system prompt + history + current user message
        let mut messages: Vec<ChatCompletionRequestMessage> = Vec::new();
        messages.push(ChatCompletionRequestMessage::System(
            ChatCompletionRequestSystemMessage {
                content: ChatCompletionRequestSystemMessageContent::Text(system_prompt),
                name: None,
            },
        ));

        // Add history (skip the user message we just saved — it's the last one)
        for msg in &history[..history.len().saturating_sub(1)] {
            match msg.role.as_str() {
                "user" => {
                    messages.push(ChatCompletionRequestMessage::User(
                        ChatCompletionRequestUserMessage {
                            content: ChatCompletionRequestUserMessageContent::Text(
                                msg.content.clone(),
                            ),
                            name: None,
                        },
                    ));
                }
                "assistant" => {
                    messages.push(ChatCompletionRequestMessage::Assistant(
                        ChatCompletionRequestAssistantMessage {
                            content: Some(ChatCompletionRequestAssistantMessageContent::Text(
                                msg.content.clone(),
                            )),
                            name: None,
                            tool_calls: None,
                            refusal: None,
                            audio: None,
                            ..Default::default()
                        },
                    ));
                }
                _ => {}
            }
        }

        // Add current user message
        messages.push(ChatCompletionRequestMessage::User(
            ChatCompletionRequestUserMessage {
                content: ChatCompletionRequestUserMessageContent::Text(user_message),
                name: None,
            },
        ));

        let llm_messages = summarize_messages(&messages);
        let chat_model = self.chat_model.clone();
        tracing::debug!(
            session_id = %session_id,
            model = %chat_model,
            messages = ?llm_messages,
            "RAG: sending chat request to LLM"
        );

        // 10. Create streaming LLM request
        let request = CreateChatCompletionRequest {
            messages,
            model: chat_model.clone(),
            stream: Some(true),
            ..Default::default()
        };

        let llm_client = self
            .llm_provider
            .get_client_with_headers(&self.chat_headers)
            .await?;

        let mut stream = llm_client
            .chat()
            .create_stream(request)
            .await
            .map_err(|e| {
                AppError::Internal(format!(
                    "LLM stream creation failed: {}",
                    format_llm_error(&e)
                ))
            })?;

        // 11. Build SSE event stream
        let chat_repo = self.chat_repo.clone();
        let sid = session_id.clone();
        let sources = source_references.clone();

        let event_stream = async_stream::stream! {
            // First event: session ID
            yield ChatEvent::Session { session_id: sid.clone() };
            yield ChatEvent::Sources {
                sources: sources.clone(),
            };

            let mut full_response = String::new();

            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(chunk) => {
                        for choice in &chunk.choices {
                            if let Some(content) = &choice.delta.content {
                                if !content.is_empty() {
                                    full_response.push_str(content);
                                    yield ChatEvent::Delta {
                                        content: content.clone(),
                                    };
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let error_message = format_llm_error(&e);
                        tracing::error!("LLM stream error: {error_message}");
                        yield ChatEvent::Error {
                            message: format!("LLM error: {error_message}"),
                        };
                        return;
                    }
                }
            }

            // Save the full assistant response
            let saved_message_id = if !full_response.is_empty() {
                tracing::debug!(
                    session_id = %sid,
                    model = %chat_model,
                    response = %preview_text(&full_response, 4_000),
                    "RAG: received chat response from LLM"
                );
                let msg_id = Uuid::new_v4().to_string();
                let msg = ChatMessage {
                    id: msg_id.clone(),
                    session_id: sid.clone(),
                    role: "assistant".into(),
                    content: full_response,
                    sources: Some(sources.clone()),
                    created_at: Utc::now(),
                };
                if let Err(e) = chat_repo.add_message(msg).await {
                    tracing::error!("Failed to save assistant message: {e}");
                }
                if let Err(e) = chat_repo.touch_session(&sid).await {
                    tracing::error!("Failed to touch session: {e}");
                }
                Some(msg_id)
            } else {
                None
            };

            yield ChatEvent::Done { message_id: saved_message_id };
        };

        Ok(Box::pin(event_stream))
    }

    async fn expand_results_to_parent(
        &self,
        user_ctx: &UserContext,
        results: Vec<VectorSearchResult>,
        session_id: &str,
    ) -> Result<Vec<VectorSearchResult>, AppError> {
        if !self.expand_to_parent || results.is_empty() {
            return Ok(results);
        }

        let (allowed_levels, include_draft) = user_ctx.document_visibility();
        let parents = unique_parents_in_order(&results);
        tracing::debug!(
            session_id = %session_id,
            parents = parents.len(),
            "RAG: expanding reranked chunks to parent sections"
        );

        let section_fetches = parents.iter().map(|(slug, anchor)| {
            self.vectorstore.get_section_chunks(
                slug,
                anchor,
                allowed_levels.as_deref(),
                include_draft,
            )
        });
        let fetched = futures::future::join_all(section_fetches).await;
        let merged = merge_parent_sections(results, parents, fetched)?;

        tracing::debug!(
            session_id = %session_id,
            expanded = ?summarize_search_results(&merged),
            "RAG: parent expansion complete"
        );

        Ok(merged)
    }

    /// Run the retrieval pipeline (analyzer + HyDE + decomposition + vector
    /// search + hybrid RRF + reranker) without invoking the LLM and without
    /// touching chat persistence.
    ///
    /// `history` is consulted only by the optional query rewriter; pass an
    /// empty slice for headless / single-turn evaluation. `session_id` is used
    /// only as a tracing field so that retrieval logs can be correlated with a
    /// caller-defined identifier (a chat session id, an eval run id, etc.).
    pub async fn retrieve_only(
        &self,
        user_ctx: &UserContext,
        user_message: &str,
        history: &[ChatMessage],
        session_id: &str,
    ) -> Result<RetrievalOutput, AppError> {
        // Rewrite the query into a standalone question when history is non-empty.
        // Falls back to the original message when rewriting is disabled or history is empty.
        let retrieval_query = match &self.query_rewriter {
            Some(rewriter) => rewriter.rewrite(user_message, history).await?,
            None => user_message.to_string(),
        };

        tracing::debug!(
            session_id = %session_id,
            original_query = %user_message,
            retrieval_query = %retrieval_query,
            history_messages = history.len(),
            "RAG: retrieval query ready"
        );

        // Analyze query complexity when the analyzer is configured. Falls back
        // to simple on any error so the pipeline is never blocked.
        let query_plan: QueryPlan = if let Some(ref analyzer) = self.analyzer {
            match analyzer.classify(&retrieval_query).await {
                Ok(plan) => {
                    tracing::debug!(
                        session_id = %session_id,
                        complexity = ?plan.complexity,
                        sub_queries = plan.sub_queries.len(),
                        "RAG: query plan"
                    );
                    plan
                }
                Err(e) => {
                    tracing::warn!(
                        session_id = %session_id,
                        error = %e,
                        "query analyzer failed — using simple retrieval"
                    );
                    QueryPlan::simple()
                }
            }
        } else {
            QueryPlan::simple()
        };

        // The set of strings to embed: original query for simple plans,
        // sub-queries for multi-entity / multi-hop decomposition.
        let queries_to_embed: Vec<String> = match query_plan.complexity {
            Complexity::Simple => vec![retrieval_query.clone()],
            _ => query_plan.sub_queries.clone(),
        };

        // HyDE: replace each query string with a synthetically generated
        // hypothetical answer document before embedding. The Meilisearch text
        // search (if enabled) still uses the original retrieval_query so that
        // keyword recall is not degraded by the generative expansion.
        let queries_to_embed = if let Some(ref hyde) = self.hyde {
            tracing::debug!(
                session_id = %session_id,
                queries = queries_to_embed.len(),
                "RAG: generating HyDE hypothetical documents"
            );
            hyde.expand_queries(queries_to_embed).await
        } else {
            queries_to_embed
        };

        // Embed all queries in a single batched call.
        let (allowed_levels, include_draft) = user_ctx.document_visibility();
        let all_vectors = self.embedding.embed(&queries_to_embed).await?;
        if all_vectors.is_empty() {
            return Err(AppError::Internal("embedding returned no vectors".into()));
        }

        let vector_limit = if self.search_service.is_some() || self.reranker.is_some() {
            MAX_CONTEXT_CHUNKS * CANDIDATE_MULTIPLIER
        } else {
            MAX_CONTEXT_CHUNKS
        };
        tracing::debug!(
            session_id = %session_id,
            retrieval_query = %retrieval_query,
            sub_queries = queries_to_embed.len(),
            vector_limit,
            hybrid = self.search_service.is_some(),
            reranker = self.reranker.is_some(),
            allowed_levels = ?allowed_levels,
            include_draft,
            "RAG: searching vector store"
        );

        // Run one vector search per query in parallel, plus an optional single
        // Meilisearch query (for the hybrid RRF signal) at the same time.
        let vector_searches: Vec<_> = all_vectors
            .into_iter()
            .map(|vector| {
                self.vectorstore.search(
                    vector,
                    vector_limit,
                    allowed_levels.as_deref(),
                    include_draft,
                )
            })
            .collect();

        let (vector_results_nested, text_slugs) = if let Some(ref svc) = self.search_service {
            let text_future =
                svc.search(&retrieval_query, allowed_levels.as_deref(), include_draft);
            let (vector_list, text_result) =
                tokio::join!(futures::future::join_all(vector_searches), text_future);
            let slugs: Vec<String> = text_result
                .unwrap_or_else(|e| {
                    tracing::warn!(
                        session_id = %session_id,
                        error = %e,
                        "hybrid search: Meilisearch query failed, falling back to vector-only"
                    );
                    vec![]
                })
                .into_iter()
                .map(|h| h.slug)
                .collect();
            (vector_list, slugs)
        } else {
            (futures::future::join_all(vector_searches).await, vec![])
        };

        // Collect per-sub-query results, preserving the list structure needed
        // for RRF-based merging with per-sub-query diversity guarantees.
        let mut chunks_per_subquery: Vec<Vec<VectorSearchResult>> = Vec::new();
        for (sub_query, result) in queries_to_embed.iter().zip(vector_results_nested) {
            let chunks = result?;
            tracing::debug!(
                session_id = %session_id,
                sub_query = %preview_text(sub_query, 200),
                hits = chunks.len(),
                chunk_ids = ?chunks.iter().map(|c| c.point_id.as_str()).collect::<Vec<_>>(),
                scores = ?chunks.iter().map(|c| c.score).collect::<Vec<_>>(),
                "RAG: sub-query hits"
            );
            chunks_per_subquery.push(chunks);
        }

        let (merged_chunks, guaranteed_ids) = if chunks_per_subquery.len() > 1 {
            merge_subquery_chunks(chunks_per_subquery)
        } else {
            (
                chunks_per_subquery.into_iter().flatten().collect(),
                Vec::new(),
            )
        };

        // Apply RRF if hybrid is enabled; otherwise keep retrieval order.
        // `take_with_guarantee` is used in both paths so that the per-sub-query
        // diversity contract survives hybrid reordering.
        let pre_rerank: Vec<VectorSearchResult> = if !text_slugs.is_empty() {
            tracing::debug!(
                session_id = %session_id,
                text_hits = text_slugs.len(),
                "RAG: applying hybrid RRF"
            );
            let fused = crate::rag::rrf::fuse(merged_chunks, &text_slugs);
            take_with_guarantee(fused, &guaranteed_ids, MAX_CONTEXT_CHUNKS)
        } else {
            take_with_guarantee(merged_chunks, &guaranteed_ids, MAX_CONTEXT_CHUNKS)
        };

        tracing::debug!(
            session_id = %session_id,
            pre_rerank_ids = ?pre_rerank.iter().map(|c| c.point_id.as_str()).collect::<Vec<_>>(),
            pre_rerank_scores = ?pre_rerank.iter().map(|c| c.score).collect::<Vec<_>>(),
            "RAG: pre-rerank candidates"
        );

        // Cross-encoder reranking (optional): re-score retrieved chunks jointly
        // against the query and keep only the top MAX_CONTEXT_CHUNKS.
        let post_rerank = if let Some(ref reranker) = self.reranker {
            tracing::debug!(
                session_id = %session_id,
                candidates = pre_rerank.len(),
                "RAG: reranking chunks"
            );
            // Keep a truncated fallback in case the reranker call fails.
            let fallback: Vec<_> = pre_rerank
                .iter()
                .take(MAX_CONTEXT_CHUNKS)
                .cloned()
                .collect();
            match reranker
                .rerank(&retrieval_query, pre_rerank.clone(), MAX_CONTEXT_CHUNKS)
                .await
            {
                Ok(reranked) => reranked,
                Err(e) => {
                    tracing::warn!(
                        session_id = %session_id,
                        error = %e,
                        "reranker failed, falling back to retrieval order"
                    );
                    fallback
                }
            }
        } else {
            pre_rerank.clone()
        };

        let post_rerank_summary = summarize_search_results(&post_rerank);
        tracing::debug!(
            session_id = %session_id,
            retrieval_query = %retrieval_query,
            results = ?post_rerank_summary,
            "RAG: retrieval complete"
        );

        Ok(RetrievalOutput {
            retrieval_query,
            query_plan,
            queries_embedded: queries_to_embed,
            pre_rerank,
            post_rerank,
        })
    }
}

/// Merge and deduplicate chunks from multiple sub-query searches.
///
/// Applies RRF (Reciprocal Rank Fusion) across sub-queries — each chunk
/// accumulates `1/(K + rank + 1)` for every sub-query where it appears —
/// and guarantees the top-ranked chunk from each sub-query is present in
/// the output. This prevents a single high-scoring topic from claiming all
/// context slots in a multi-hop query (e.g. "compare X and Y" where X and Y
/// come from different documents with different cosine similarity ranges).
///
/// Output order: guaranteed chunks first (one per sub-query, in sub-query
/// order), then remaining candidates sorted by RRF score descending.
/// Returns `(rrf_sorted_candidates, guaranteed_ids)`.
///
/// `guaranteed_ids` contains the `point_id` of the top-ranked chunk from each
/// unique sub-query (in sub-query order, deduplicated). Callers should pass
/// this to [`take_with_guarantee`] so the diversity contract survives any
/// subsequent reordering (e.g. hybrid `rrf::fuse`).
fn merge_subquery_chunks(
    chunks_per_subquery: Vec<Vec<VectorSearchResult>>,
) -> (Vec<VectorSearchResult>, Vec<String>) {
    const K: usize = 60;

    let mut rrf: HashMap<String, (VectorSearchResult, f64)> = HashMap::new();
    let mut guaranteed_ids: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for sub_query_chunks in &chunks_per_subquery {
        if let Some(chunk) = sub_query_chunks.first() {
            if seen.insert(chunk.point_id.clone()) {
                guaranteed_ids.push(chunk.point_id.clone());
            }
        }
        for (rank, chunk) in sub_query_chunks.iter().enumerate() {
            let contrib = 1.0 / (K + rank + 1) as f64;
            let entry = rrf
                .entry(chunk.point_id.clone())
                .or_insert_with(|| (chunk.clone(), 0.0));
            entry.1 += contrib;
            if chunk.score > entry.0.score {
                entry.0 = chunk.clone();
            }
        }
    }

    let mut sorted: Vec<(String, VectorSearchResult, f64)> = rrf
        .into_iter()
        .map(|(id, (chunk, score))| (id, chunk, score))
        .collect();
    sorted.sort_by(|a, b| b.2.total_cmp(&a.2));

    let sorted_idx: HashMap<&str, usize> = sorted
        .iter()
        .enumerate()
        .map(|(i, (id, _, _))| (id.as_str(), i))
        .collect();

    let guaranteed_set: HashSet<&str> = guaranteed_ids.iter().map(|s| s.as_str()).collect();

    let mut result: Vec<VectorSearchResult> = guaranteed_ids
        .iter()
        .filter_map(|id| sorted_idx.get(id.as_str()).map(|&i| sorted[i].1.clone()))
        .collect();

    for (id, chunk, _) in &sorted {
        if !guaranteed_set.contains(id.as_str()) {
            result.push(chunk.clone());
        }
    }

    (result, guaranteed_ids)
}

/// Truncate `candidates` to `limit`, ensuring every `guaranteed_ids` entry
/// appears in the output.
///
/// Guaranteed chunks are placed first (in the order given), then remaining
/// slots are filled from `candidates` in their existing order. This preserves
/// the diversity guarantee even after `candidates` has been reordered by an
/// external step such as hybrid `rrf::fuse`.
fn take_with_guarantee(
    candidates: Vec<VectorSearchResult>,
    guaranteed_ids: &[String],
    limit: usize,
) -> Vec<VectorSearchResult> {
    let candidate_map: HashMap<&str, VectorSearchResult> = candidates
        .iter()
        .map(|c| (c.point_id.as_str(), c.clone()))
        .collect();

    let mut result: Vec<VectorSearchResult> = Vec::with_capacity(limit);
    let mut included: HashSet<String> = HashSet::new();

    for id in guaranteed_ids {
        if result.len() >= limit {
            break;
        }
        if let Some(chunk) = candidate_map.get(id.as_str()) {
            if included.insert(id.clone()) {
                result.push(chunk.clone());
            }
        }
    }

    for chunk in candidates {
        if result.len() >= limit {
            break;
        }
        if !included.contains(&chunk.point_id) {
            included.insert(chunk.point_id.clone());
            result.push(chunk);
        }
    }

    result
}

fn unique_parents_in_order(results: &[VectorSearchResult]) -> Vec<(String, String)> {
    let mut seen: HashMap<(String, String), ()> = HashMap::new();
    let mut parents = Vec::new();

    for result in results {
        let key = (result.document_slug.clone(), result.section_anchor.clone());
        if seen.insert(key.clone(), ()).is_none() {
            parents.push(key);
        }
    }

    parents
}

fn merge_parent_sections(
    reranked_results: Vec<VectorSearchResult>,
    parents: Vec<(String, String)>,
    fetched_sections: Vec<Result<Vec<VectorSearchResult>, AppError>>,
) -> Result<Vec<VectorSearchResult>, AppError> {
    let mut parent_scores: HashMap<(String, String), VectorSearchResult> = HashMap::new();
    for result in reranked_results {
        let key = (result.document_slug.clone(), result.section_anchor.clone());
        match parent_scores.get(&key) {
            Some(existing) if existing.score >= result.score => {}
            _ => {
                parent_scores.insert(key, result);
            }
        }
    }

    let mut expanded = Vec::new();
    for ((document_slug, section_anchor), fetched) in parents.into_iter().zip(fetched_sections) {
        let siblings = fetched?;
        let Some(best_parent_hit) =
            parent_scores.get(&(document_slug.clone(), section_anchor.clone()))
        else {
            continue;
        };

        if siblings.is_empty() {
            expanded.push(best_parent_hit.clone());
            continue;
        }

        let merged_text = merge_chunk_texts(
            &siblings
                .iter()
                .map(|chunk| chunk.chunk_text.as_str())
                .collect::<Vec<_>>(),
        );

        let mut parent_result = best_parent_hit.clone();
        parent_result.chunk_text = merged_text;
        parent_result.chunk_index = siblings.first().map(|chunk| chunk.chunk_index).unwrap_or(0);
        if let Some(first) = siblings.first() {
            parent_result.section_path = first.section_path.clone();
            parent_result.document_title = first.document_title.clone();
        }

        expanded.push(parent_result);
    }

    Ok(expanded)
}

fn merge_chunk_texts(chunks: &[&str]) -> String {
    let mut merged = String::new();

    for chunk in chunks {
        if chunk.is_empty() {
            continue;
        }

        if merged.is_empty() {
            merged.push_str(chunk);
            continue;
        }

        if merged.contains(chunk) {
            continue;
        }

        let mut overlap_start = 0usize;
        for (idx, _) in chunk.char_indices() {
            if merged.ends_with(&chunk[..idx]) {
                overlap_start = idx;
            }
        }

        if overlap_start == 0 && !merged.ends_with('\n') {
            merged.push_str("\n\n");
        }
        merged.push_str(&chunk[overlap_start..]);
    }

    merged
}

fn summarize_messages(messages: &[ChatCompletionRequestMessage]) -> Vec<String> {
    messages.iter().map(summarize_message).collect()
}

fn summarize_message(message: &ChatCompletionRequestMessage) -> String {
    match message {
        ChatCompletionRequestMessage::System(msg) => {
            format!("system: {}", preview_system_content(&msg.content))
        }
        ChatCompletionRequestMessage::User(msg) => {
            format!("user: {}", preview_user_content(&msg.content))
        }
        ChatCompletionRequestMessage::Assistant(msg) => {
            let content = msg
                .content
                .as_ref()
                .map(preview_assistant_content)
                .unwrap_or_else(|| "<empty>".to_string());
            format!("assistant: {content}")
        }
        other => format!("{other:?}"),
    }
}

fn preview_system_content(content: &ChatCompletionRequestSystemMessageContent) -> String {
    match content {
        ChatCompletionRequestSystemMessageContent::Text(text) => preview_text(text, 1_500),
        other => format!("{other:?}"),
    }
}

fn preview_user_content(content: &ChatCompletionRequestUserMessageContent) -> String {
    match content {
        ChatCompletionRequestUserMessageContent::Text(text) => preview_text(text, 1_500),
        other => format!("{other:?}"),
    }
}

fn preview_assistant_content(content: &ChatCompletionRequestAssistantMessageContent) -> String {
    match content {
        ChatCompletionRequestAssistantMessageContent::Text(text) => preview_text(text, 1_500),
        other => format!("{other:?}"),
    }
}

fn summarize_search_results(
    results: &[crate::rag::vectorstore::VectorSearchResult],
) -> Vec<String> {
    results
        .iter()
        .map(|result| {
            format!(
                "id={} score={:.4} slug={} anchor={} title={} chunk={}",
                result.point_id,
                result.score,
                result.document_slug,
                result.section_anchor,
                result.document_title,
                preview_text(&result.chunk_text, 240)
            )
        })
        .collect()
}

fn build_source_references(
    results: &[crate::rag::vectorstore::VectorSearchResult],
) -> Vec<SourceReference> {
    let mut deduped: HashMap<(String, String), SourceReference> = HashMap::new();

    for result in results {
        let snippet = preview_text(&result.chunk_text, 180);
        let section_title = result.section_path.last().cloned();
        let section_anchor = if result.section_anchor.is_empty() {
            None
        } else {
            Some(result.section_anchor.clone())
        };
        let candidate = SourceReference {
            document_slug: result.document_slug.clone(),
            document_title: result.document_title.clone(),
            section_title,
            section_anchor: section_anchor.clone(),
            score: result.score,
            snippet: if snippet.is_empty() {
                None
            } else {
                Some(snippet)
            },
        };
        let dedupe_key = (
            result.document_slug.clone(),
            section_anchor.clone().unwrap_or_default(),
        );

        match deduped.get(&dedupe_key) {
            Some(existing) if existing.score >= candidate.score => {}
            _ => {
                deduped.insert(dedupe_key, candidate);
            }
        }
    }

    let mut sources: Vec<SourceReference> = deduped.into_values().collect();
    sources.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.document_slug.cmp(&b.document_slug))
            .then_with(|| a.section_anchor.cmp(&b.section_anchor))
    });
    sources
}

fn preview_text(text: &str, max_chars: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = normalized.chars();
    let preview: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{preview}...")
    } else {
        preview
    }
}

/// Truncate a message to use as a session title.
fn truncate_title(message: &str) -> String {
    let first_line = message.lines().next().unwrap_or(message);
    if first_line.len() > 80 {
        format!("{}…", &first_line[..77])
    } else {
        first_line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rag::vectorstore::VectorSearchResult;

    #[test]
    fn truncate_title_short() {
        assert_eq!(truncate_title("Hello world"), "Hello world");
    }

    #[test]
    fn truncate_title_long() {
        let long = "a".repeat(100);
        let title = truncate_title(&long);
        assert!(title.len() <= 81); // 77 + "…" (3 bytes)
        assert!(title.ends_with('…'));
    }

    #[test]
    fn truncate_title_multiline() {
        assert_eq!(truncate_title("First line\nSecond line"), "First line");
    }

    #[test]
    fn build_source_references_deduplicates_by_slug_and_keeps_best_score() {
        let sources = build_source_references(&[
            VectorSearchResult {
                point_id: "p1".into(),
                chunk_text: "First chunk".into(),
                document_slug: "docs/a".into(),
                document_title: "Doc A".into(),
                chunk_index: 0,
                section_path: vec!["Intro".into()],
                section_anchor: "intro".into(),
                score: 0.42,
            },
            VectorSearchResult {
                point_id: "p2".into(),
                chunk_text: "Better chunk".into(),
                document_slug: "docs/a".into(),
                document_title: "Doc A".into(),
                chunk_index: 1,
                section_path: vec!["Intro".into()],
                section_anchor: "intro".into(),
                score: 0.81,
            },
            VectorSearchResult {
                point_id: "p3".into(),
                chunk_text: "Other chunk".into(),
                document_slug: "docs/b".into(),
                document_title: "Doc B".into(),
                chunk_index: 0,
                section_path: vec!["Usage".into()],
                section_anchor: "usage".into(),
                score: 0.65,
            },
        ]);

        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].document_slug, "docs/a");
        assert_eq!(sources[0].section_title.as_deref(), Some("Intro"));
        assert_eq!(sources[0].section_anchor.as_deref(), Some("intro"));
        assert_eq!(sources[0].score, 0.81);
        assert_eq!(sources[0].snippet.as_deref(), Some("Better chunk"));
        assert_eq!(sources[1].document_slug, "docs/b");
    }

    #[test]
    fn build_source_references_keeps_distinct_sections_from_same_document() {
        let sources = build_source_references(&[
            VectorSearchResult {
                point_id: "p1".into(),
                chunk_text: "Storage chunk".into(),
                document_slug: "docs/a".into(),
                document_title: "Doc A".into(),
                chunk_index: 0,
                section_path: vec!["Architecture".into(), "Storage".into()],
                section_anchor: "architecture-storage".into(),
                score: 0.77,
            },
            VectorSearchResult {
                point_id: "p2".into(),
                chunk_text: "Deployment chunk".into(),
                document_slug: "docs/a".into(),
                document_title: "Doc A".into(),
                chunk_index: 1,
                section_path: vec!["Architecture".into(), "Deployment".into()],
                section_anchor: "architecture-deployment".into(),
                score: 0.71,
            },
        ]);

        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].section_title.as_deref(), Some("Storage"));
        assert_eq!(
            sources[1].section_anchor.as_deref(),
            Some("architecture-deployment")
        );
    }

    #[test]
    fn merge_chunk_texts_trims_overlapping_prefix() {
        let merged = merge_chunk_texts(&[
            "## Storage\n\nFirst half of the section.",
            "section.\n\nSecond half of the section.",
        ]);

        assert_eq!(
            merged,
            "## Storage\n\nFirst half of the section.\n\nSecond half of the section."
        );
    }

    #[test]
    fn merge_parent_sections_replaces_top_hits_with_full_parent_text() {
        let reranked = vec![
            VectorSearchResult {
                point_id: "p1".into(),
                chunk_text: "chunk-a".into(),
                document_slug: "docs/a".into(),
                document_title: "Doc A".into(),
                chunk_index: 0,
                section_path: vec!["Architecture".into(), "Storage".into()],
                section_anchor: "architecture-storage".into(),
                score: 0.91,
            },
            VectorSearchResult {
                point_id: "p2".into(),
                chunk_text: "chunk-b".into(),
                document_slug: "docs/b".into(),
                document_title: "Doc B".into(),
                chunk_index: 0,
                section_path: vec!["Usage".into()],
                section_anchor: "usage".into(),
                score: 0.77,
            },
        ];
        let parents = unique_parents_in_order(&reranked);
        let fetched = vec![
            Ok(vec![
                VectorSearchResult {
                    point_id: "f1".into(),
                    chunk_text: "## Storage\n\nPart one.".into(),
                    document_slug: "docs/a".into(),
                    document_title: "Doc A".into(),
                    chunk_index: 0,
                    section_path: vec!["Architecture".into(), "Storage".into()],
                    section_anchor: "architecture-storage".into(),
                    score: 0.0,
                },
                VectorSearchResult {
                    point_id: "f2".into(),
                    chunk_text: "Part one.\n\nPart two.".into(),
                    document_slug: "docs/a".into(),
                    document_title: "Doc A".into(),
                    chunk_index: 1,
                    section_path: vec!["Architecture".into(), "Storage".into()],
                    section_anchor: "architecture-storage".into(),
                    score: 0.0,
                },
            ]),
            Ok(vec![VectorSearchResult {
                point_id: "f3".into(),
                chunk_text: "## Usage\n\nOnly chunk.".into(),
                document_slug: "docs/b".into(),
                document_title: "Doc B".into(),
                chunk_index: 0,
                section_path: vec!["Usage".into()],
                section_anchor: "usage".into(),
                score: 0.0,
            }]),
        ];

        let expanded = merge_parent_sections(reranked, parents, fetched).unwrap();

        assert_eq!(expanded.len(), 2);
        assert_eq!(expanded[0].score, 0.91);
        assert_eq!(
            expanded[0].chunk_text,
            "## Storage\n\nPart one.\n\nPart two."
        );
        assert_eq!(expanded[1].chunk_text, "## Usage\n\nOnly chunk.");
    }

    fn make_chunk(id: &str, score: f32) -> VectorSearchResult {
        VectorSearchResult {
            point_id: id.to_string(),
            chunk_text: format!("text for {id}"),
            document_slug: id.to_string(),
            document_title: id.to_string(),
            chunk_index: 0,
            section_path: Vec::new(),
            section_anchor: String::new(),
            score,
        }
    }

    #[test]
    fn merge_subquery_guarantees_top1_per_subquery() {
        let sq_a = vec![
            make_chunk("a1", 0.69),
            make_chunk("a2", 0.68),
            make_chunk("a3", 0.67),
        ];
        let sq_b = vec![
            make_chunk("b1", 0.76),
            make_chunk("b2", 0.75),
            make_chunk("b3", 0.74),
            make_chunk("b4", 0.73),
            make_chunk("b5", 0.72),
        ];

        let (merged, guaranteed) = merge_subquery_chunks(vec![sq_a, sq_b]);
        let ids: Vec<&str> = merged.iter().map(|c| c.point_id.as_str()).collect();
        assert!(
            ids.contains(&"a1"),
            "top chunk from sub-query A must be present: {ids:?}"
        );
        assert!(
            ids.contains(&"b1"),
            "top chunk from sub-query B must be present: {ids:?}"
        );
        assert_eq!(guaranteed, vec!["a1", "b1"]);
    }

    #[test]
    fn merge_subquery_deduplicates_cross_subquery() {
        let sq_a = vec![make_chunk("shared", 0.80), make_chunk("a1", 0.70)];
        let sq_b = vec![make_chunk("shared", 0.75), make_chunk("b1", 0.65)];

        let (merged, _) = merge_subquery_chunks(vec![sq_a, sq_b]);
        let count = merged.iter().filter(|c| c.point_id == "shared").count();
        assert_eq!(
            count, 1,
            "shared chunk must appear exactly once: {merged:?}"
        );
    }

    #[test]
    fn merge_subquery_rrf_boosts_cross_subquery_chunks() {
        let sq_a = vec![make_chunk("unique", 0.90), make_chunk("cross", 0.80)];
        let sq_b = vec![make_chunk("cross", 0.75), make_chunk("other", 0.70)];

        let (merged, _) = merge_subquery_chunks(vec![sq_a, sq_b]);

        // "unique" is guaranteed (top of SQ-A), so it comes first.
        // After guaranteed slots, "cross" must outrank "other" due to multi-subquery RRF.
        let pos_cross = merged.iter().position(|c| c.point_id == "cross").unwrap();
        let pos_other = merged.iter().position(|c| c.point_id == "other").unwrap();
        assert!(
            pos_cross < pos_other,
            "cross should rank above other: {merged:?}"
        );
    }

    #[test]
    fn take_with_guarantee_keeps_guaranteed_after_reorder() {
        // Simulate hybrid rrf::fuse pushing guaranteed chunk "a1" to position 5
        // (beyond the limit of 3), which plain .take(3) would drop.
        let candidates = vec![
            make_chunk("b1", 0.80),
            make_chunk("b2", 0.78),
            make_chunk("b3", 0.76),
            make_chunk("b4", 0.74),
            make_chunk("a1", 0.69), // guaranteed but pushed to index 4
        ];
        let guaranteed = vec!["a1".to_string(), "b1".to_string()];

        let result = take_with_guarantee(candidates, &guaranteed, 3);
        let ids: Vec<&str> = result.iter().map(|c| c.point_id.as_str()).collect();

        assert_eq!(result.len(), 3);
        assert!(
            ids.contains(&"a1"),
            "guaranteed a1 must survive truncation: {ids:?}"
        );
        assert!(
            ids.contains(&"b1"),
            "guaranteed b1 must survive truncation: {ids:?}"
        );
    }

    #[test]
    fn take_with_guarantee_single_subquery_behaves_like_take() {
        let candidates = vec![
            make_chunk("c1", 0.90),
            make_chunk("c2", 0.80),
            make_chunk("c3", 0.70),
        ];

        // No guaranteed ids (single sub-query path)
        let result = take_with_guarantee(candidates.clone(), &[], 2);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].point_id, "c1");
        assert_eq!(result[1].point_id, "c2");
    }
}
