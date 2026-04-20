use std::{collections::HashMap, sync::Arc};

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
    /// Optional Meilisearch service used for hybrid search (RRF). `None` when
    /// hybrid search is disabled or Meilisearch is not configured.
    search_service: Option<Arc<dyn SearchService>>,
    /// Optional cross-encoder reranker applied after retrieval. `None` when
    /// `reranker_url` is empty.
    reranker: Option<Arc<dyn Reranker>>,
    /// Optional query analyzer for complexity classification and decomposition.
    /// `None` when `analyzer_model` is empty.
    analyzer: Option<QueryAnalyzer>,
    chat_repo: Arc<dyn ChatRepository>,
    llm_provider: Arc<LlmProvider>,
    chat_model: String,
    chat_headers: HashMap<String, String>,
    tera: tera::Tera,
    system_template_name: String,
    query_rewriter: Option<QueryRewriter>,
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

        let analyzer = QueryAnalyzer::from_rag_config(config, llm_provider.clone());
        if analyzer.is_some() {
            tracing::info!(model = %config.analyzer_model, "RAG query analyzer enabled");
        }

        Ok(Self {
            embedding,
            vectorstore,
            search_service: hybrid_search_service,
            reranker,
            analyzer,
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

        // 4. Rewrite the query into a standalone question when history is non-empty.
        //    This improves vector-search relevance for follow-up / elliptic questions.
        //    Falls back to the original message when rewriting is disabled or history is empty.
        let retrieval_query = match &self.query_rewriter {
            Some(rewriter) => rewriter.rewrite(&user_message, &history).await?,
            None => user_message.clone(),
        };

        tracing::debug!(
            session_id = %session_id,
            original_query = %user_message,
            retrieval_query = %retrieval_query,
            history_messages = history.len(),
            "RAG: retrieval query ready"
        );

        // 4.5 Analyze query complexity when the analyzer is configured.
        //     Falls back to simple on any error so the pipeline is never blocked.
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

        // The set of strings to embed: original query for simple plans, sub-queries
        // for multi-entity / multi-hop decomposition.
        let queries_to_embed: Vec<String> = match query_plan.complexity {
            Complexity::Simple => vec![retrieval_query.clone()],
            _ => query_plan.sub_queries.clone(),
        };

        // 5. Embed all queries in a single batched call.
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

        // 6. Run one vector search per query in parallel, plus an optional single
        //    Meilisearch query (for the hybrid RRF signal) at the same time.
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

        // Flatten results from all sub-query searches, propagating errors.
        let mut merged_chunks: Vec<VectorSearchResult> = Vec::new();
        for result in vector_results_nested {
            merged_chunks.extend(result?);
        }

        // Deduplicate when multiple sub-queries were used (same chunk can appear
        // in results for several sub-queries).
        if queries_to_embed.len() > 1 {
            merged_chunks = dedup_chunks(merged_chunks);
        }

        // Apply RRF if hybrid is enabled; otherwise keep retrieval order.
        let search_results: Vec<VectorSearchResult> = if !text_slugs.is_empty() {
            tracing::debug!(
                session_id = %session_id,
                text_hits = text_slugs.len(),
                "RAG: applying hybrid RRF"
            );
            let fused = crate::rag::rrf::fuse(merged_chunks, &text_slugs);
            fused.into_iter().take(MAX_CONTEXT_CHUNKS).collect()
        } else {
            merged_chunks.into_iter().take(MAX_CONTEXT_CHUNKS).collect()
        };

        // 7. Cross-encoder reranking (optional): re-score retrieved chunks jointly
        //    against the query and keep only the top MAX_CONTEXT_CHUNKS.
        let search_results = if let Some(ref reranker) = self.reranker {
            tracing::debug!(
                session_id = %session_id,
                candidates = search_results.len(),
                "RAG: reranking chunks"
            );
            // Keep a truncated fallback in case the reranker call fails.
            let fallback: Vec<_> = search_results
                .iter()
                .take(MAX_CONTEXT_CHUNKS)
                .cloned()
                .collect();
            match reranker
                .rerank(&retrieval_query, search_results, MAX_CONTEXT_CHUNKS)
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
            search_results
        };

        let search_results_summary = summarize_search_results(&search_results);
        let source_references = build_source_references(&search_results);
        tracing::debug!(
            session_id = %session_id,
            retrieval_query = %retrieval_query,
            results = ?search_results_summary,
            "RAG: retrieval complete"
        );

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
}

/// Deduplicate chunks from multiple sub-query searches, keeping the highest
/// score for each unique chunk text.
fn dedup_chunks(chunks: Vec<VectorSearchResult>) -> Vec<VectorSearchResult> {
    let mut best: HashMap<String, VectorSearchResult> = HashMap::new();
    for chunk in chunks {
        match best.entry(chunk.chunk_text.clone()) {
            std::collections::hash_map::Entry::Occupied(mut e) => {
                if chunk.score > e.get().score {
                    *e.get_mut() = chunk;
                }
            }
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(chunk);
            }
        }
    }
    let mut result: Vec<VectorSearchResult> = best.into_values().collect();
    result.sort_by(|a, b| b.score.total_cmp(&a.score));
    result
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
                "score={:.4} slug={} title={} chunk={}",
                result.score,
                result.document_slug,
                result.document_title,
                preview_text(&result.chunk_text, 240)
            )
        })
        .collect()
}

fn build_source_references(
    results: &[crate::rag::vectorstore::VectorSearchResult],
) -> Vec<SourceReference> {
    let mut deduped: HashMap<&str, SourceReference> = HashMap::new();

    for result in results {
        let snippet = preview_text(&result.chunk_text, 180);
        let candidate = SourceReference {
            document_slug: result.document_slug.clone(),
            document_title: result.document_title.clone(),
            score: result.score,
            snippet: if snippet.is_empty() {
                None
            } else {
                Some(snippet)
            },
        };

        match deduped.get(result.document_slug.as_str()) {
            Some(existing) if existing.score >= candidate.score => {}
            _ => {
                deduped.insert(result.document_slug.as_str(), candidate);
            }
        }
    }

    let mut sources: Vec<SourceReference> = deduped.into_values().collect();
    sources.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.document_slug.cmp(&b.document_slug))
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
                chunk_text: "First chunk".into(),
                document_slug: "docs/a".into(),
                document_title: "Doc A".into(),
                score: 0.42,
            },
            VectorSearchResult {
                chunk_text: "Better chunk".into(),
                document_slug: "docs/a".into(),
                document_title: "Doc A".into(),
                score: 0.81,
            },
            VectorSearchResult {
                chunk_text: "Other chunk".into(),
                document_slug: "docs/b".into(),
                document_title: "Doc B".into(),
                score: 0.65,
            },
        ]);

        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].document_slug, "docs/a");
        assert_eq!(sources[0].score, 0.81);
        assert_eq!(sources[0].snippet.as_deref(), Some("Better chunk"));
        assert_eq!(sources[1].document_slug, "docs/b");
    }
}
