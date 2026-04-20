//! Headless wiring of the RAG retrieval stack for offline evaluation.
//!
//! Builds a fully-functional [`ChatService`] from an [`AppConfig`] without
//! touching MongoDB chat persistence, S3, Meilisearch index configuration, or
//! the Leptos/Axum surface. Suitable for CLI tooling that needs to call
//! [`ChatService::retrieve_only`] against a real, already-indexed Qdrant
//! collection.

use std::sync::Arc;

use async_trait::async_trait;

use crate::config::AppConfig;
use crate::db::chat_models::{ChatMessage, ChatSession};
use crate::db::chat_repository::ChatRepository;
use crate::error::AppError;
use crate::rag::chat::ChatService;
use crate::rag::embedding::{EmbeddingService, OpenAICompatibleEmbedding};
use crate::rag::provider::LlmProvider;
use crate::rag::reranker::{CrossEncoderReranker, Reranker};
use crate::rag::vectorstore::{QdrantVectorStore, VectorStore};
use crate::search::client::{MeilisearchService, SearchService};

/// A wired-up retrieval stack ready to answer [`ChatService::retrieve_only`]
/// calls without any chat persistence.
pub struct RagEvalContext {
    pub chat_service: Arc<ChatService>,
    pub embedding: Arc<dyn EmbeddingService>,
    pub vectorstore: Arc<dyn VectorStore>,
}

impl RagEvalContext {
    /// Build the retrieval stack from an application config.
    ///
    /// Fails when RAG is not configured (no `qdrant_url`, no `embedding_url`,
    /// no `chat_model`, etc.) — eval requires a real backend to be useful.
    pub async fn from_config(config: &AppConfig) -> Result<Self, AppError> {
        if !config.rag.is_enabled() {
            return Err(AppError::Internal(
                "RAG is not enabled in the loaded configuration — \
                 set rag.qdrant_url, rag.embedding_url and rag.chat_model"
                    .into(),
            ));
        }

        let llm_provider =
            Arc::new(LlmProvider::initialize(&config.rag).await.map_err(|e| {
                AppError::Internal(format!("LLM provider initialization failed: {e}"))
            })?);

        let embedding: Arc<dyn EmbeddingService> =
            Arc::new(OpenAICompatibleEmbedding::from_rag_config(&config.rag)?);

        let vectorstore: Arc<dyn VectorStore> =
            Arc::new(QdrantVectorStore::from_rag_config(&config.rag)?);

        // Skip ensure_collection: in eval the collection is expected to already
        // be indexed. Failing fast when the collection is missing is more
        // informative than silently creating an empty one.

        let search_service: Option<Arc<dyn SearchService>> =
            match MeilisearchService::from_app_config(&config.search) {
                Ok(svc) => Some(Arc::new(svc)),
                Err(e) => {
                    tracing::warn!(
                        "Meilisearch not available for eval: {e} — hybrid RRF will be skipped"
                    );
                    None
                }
            };

        let reranker: Option<Arc<dyn Reranker>> =
            CrossEncoderReranker::from_rag_config(&config.rag)
                .map(|r| Arc::new(r) as Arc<dyn Reranker>);

        let chat_repo: Arc<dyn ChatRepository> = Arc::new(NoopChatRepository);

        let chat_service = ChatService::from_rag_config(
            &config.rag,
            llm_provider,
            chat_repo,
            embedding.clone(),
            vectorstore.clone(),
            search_service,
            reranker,
        )?;

        Ok(Self {
            chat_service: Arc::new(chat_service),
            embedding,
            vectorstore,
        })
    }
}

/// A no-op [`ChatRepository`] used by [`RagEvalContext`].
///
/// `ChatService::retrieve_only` never touches chat persistence, so the eval
/// harness can supply this stub and avoid a MongoDB dependency. Any other
/// `ChatService` method called against this repo will return errors.
struct NoopChatRepository;

#[async_trait]
impl ChatRepository for NoopChatRepository {
    async fn create_session(&self, _session: ChatSession) -> Result<(), AppError> {
        Err(AppError::Internal(
            "NoopChatRepository: create_session called from a context that should be retrieve-only"
                .into(),
        ))
    }

    async fn get_session(&self, _id: &str) -> Result<Option<ChatSession>, AppError> {
        Ok(None)
    }

    async fn list_sessions_for_user(&self, _user_id: &str) -> Result<Vec<ChatSession>, AppError> {
        Ok(Vec::new())
    }

    async fn update_session_title(&self, _id: &str, _title: &str) -> Result<(), AppError> {
        Ok(())
    }

    async fn touch_session(&self, _id: &str) -> Result<(), AppError> {
        Ok(())
    }

    async fn add_message(&self, _msg: ChatMessage) -> Result<(), AppError> {
        Ok(())
    }

    async fn get_messages(
        &self,
        _session_id: &str,
        _limit: usize,
    ) -> Result<Vec<ChatMessage>, AppError> {
        Ok(Vec::new())
    }

    async fn delete_session(&self, _id: &str) -> Result<(), AppError> {
        Ok(())
    }

    async fn get_message_by_id(&self, _id: &str) -> Result<Option<ChatMessage>, AppError> {
        Ok(None)
    }
}
