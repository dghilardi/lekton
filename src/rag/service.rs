use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use crate::config::RagConfig;
use crate::error::AppError;

use super::embedding::{build_embedding_service, EmbeddingService};
use super::splitter::split_document;
use super::vectorstore::{ChunkPayload, QdrantVectorStore, SourceKind, VectorPoint, VectorStore};

// ── Data types ─────────────────────────────────────────────────────────────────

/// Extracted text for one page of an attachment, produced by the extraction
/// layer. Plain-text attachments yield a single page with `page_number = None`.
#[derive(Debug, Clone)]
pub struct AttachmentPage {
    /// 1-based page number, or `None` for non-paginated sources.
    pub page_number: Option<u32>,
    pub text: String,
}

// ── Trait ─────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait RagService: Send + Sync {
    /// Index (or re-index) a document: split, embed, upsert new chunks, then
    /// delete stale ones (upsert-then-delete-stale, so a failed embedding never
    /// leaves the document missing from the store).
    async fn index_document(
        &self,
        slug: &str,
        title: &str,
        content: &str,
        access_level: &str,
        is_draft: bool,
        tags: &[String],
    ) -> Result<(), AppError>;

    /// Remove all chunks for a document.
    async fn delete_document(&self, slug: &str) -> Result<(), AppError>;

    /// Index (or re-index) an attachment from its extracted, per-page text.
    ///
    /// `access_levels` is the set inherited from the attachment's referencing
    /// (published) documents. Chunks are stored with `source_kind = Attachment`
    /// and `is_draft = false`. Returns the number of chunks indexed.
    async fn index_attachment(
        &self,
        attachment_key: &str,
        filename: &str,
        pages: &[AttachmentPage],
        access_levels: &[String],
        tags: &[String],
    ) -> Result<usize, AppError>;

    /// Remove all chunks for an attachment.
    async fn delete_attachment(&self, attachment_key: &str) -> Result<(), AppError>;

    /// Update only the access levels of an attachment's chunks (no re-embedding),
    /// for when the referencing-document graph changes.
    async fn update_attachment_access_levels(
        &self,
        attachment_key: &str,
        access_levels: &[String],
    ) -> Result<(), AppError>;
}

// ── Default implementation ───────────────────────────────────────────────────

pub struct DefaultRagService {
    embedding: Arc<dyn EmbeddingService>,
    vectorstore: Arc<dyn VectorStore>,
    chunk_size_tokens: usize,
    chunk_overlap_tokens: usize,
}

impl DefaultRagService {
    /// Create from pre-built service components with explicit chunk sizing.
    pub fn new(
        embedding: Arc<dyn EmbeddingService>,
        vectorstore: Arc<dyn VectorStore>,
        chunk_size_tokens: usize,
        chunk_overlap_tokens: usize,
    ) -> Self {
        Self {
            embedding,
            vectorstore,
            chunk_size_tokens,
            chunk_overlap_tokens,
        }
    }

    /// Build from application config. Returns `Err` when required config is missing.
    pub async fn from_rag_config(config: &RagConfig) -> Result<Self, AppError> {
        let embedding = build_embedding_service(config).await?;
        let vectorstore = QdrantVectorStore::from_rag_config(config)?;
        Ok(Self {
            embedding,
            vectorstore: Arc::new(vectorstore),
            chunk_size_tokens: config.chunk_size_tokens as usize,
            chunk_overlap_tokens: config.chunk_overlap_tokens as usize,
        })
    }
}

#[async_trait]
impl RagService for DefaultRagService {
    #[tracing::instrument(skip(self, content, tags), fields(slug = %slug, access_level = %access_level))]
    async fn index_document(
        &self,
        slug: &str,
        title: &str,
        content: &str,
        access_level: &str,
        is_draft: bool,
        tags: &[String],
    ) -> Result<(), AppError> {
        // 1. Split content into token-aware chunks
        let chunks = split_document(content, self.chunk_size_tokens, self.chunk_overlap_tokens);
        if chunks.is_empty() {
            // Document has no indexable content: remove any previously indexed chunks.
            self.vectorstore.delete_by_slug(slug).await?;
            return Ok(());
        }

        // 2. Build enriched embedding texts: "Title > Section\n\nChunk text"
        // The embedding vector is computed on the enriched text for better recall of
        // context-ambiguous chunks. The display text (chunk.text) stays clean for prompt
        // injection and UI rendering; only embedding_text is sent to the embedder.
        let embedding_texts: Vec<String> = chunks
            .iter()
            .map(|c| {
                let mut prefix = title.to_string();
                if !c.section_path.is_empty() {
                    prefix.push_str(" > ");
                    prefix.push_str(&c.section_path.join(" > "));
                }
                format!("{}\n\n{}", prefix, c.text)
            })
            .collect();
        let vectors = self.embedding.embed(&embedding_texts).await?;

        // 3. Build Qdrant points, skipping any chunk whose embedding is empty.
        // Some embedding backends (e.g. Ollama) return [] for whitespace-only
        // or otherwise problematic inputs; sending a zero-dim vector to Qdrant
        // causes a hard error ("expected dim: 768, got 0").
        let points: Vec<VectorPoint> = chunks
            .into_iter()
            .zip(vectors)
            .filter_map(|(chunk, vector)| {
                if vector.is_empty() {
                    tracing::warn!(
                        slug,
                        idx = chunk.chunk_index,
                        "RAG: embedding returned empty vector for chunk, skipping"
                    );
                    return None;
                }
                Some(VectorPoint {
                    id: Uuid::new_v4().to_string(),
                    vector,
                    payload: ChunkPayload {
                        chunk_text: chunk.text,
                        section_path: chunk.section_path,
                        section_anchor: chunk.section_anchor,
                        document_slug: slug.to_string(),
                        document_title: title.to_string(),
                        source_kind: super::vectorstore::SourceKind::Document,
                        attachment_key: None,
                        source_page: None,
                        access_levels: vec![access_level.to_string()],
                        is_draft,
                        tags: tags.to_vec(),
                        chunk_index: chunk.chunk_index,
                    },
                })
            })
            .collect();

        // All chunks failed to embed: leave the existing index untouched rather
        // than wiping it on a degenerate embedding run (avoids silent data loss).
        if points.is_empty() {
            tracing::warn!(
                slug,
                "RAG: no embeddable chunks produced; existing index left intact"
            );
            return Ok(());
        }

        // 4. Upsert the new chunks, then 5. delete the stale ones (everything for
        // this slug except the ids just written). Upsert-then-delete means the
        // document is never absent from the store: embedding is computed before
        // any destructive write, and a failure after upsert leaves duplicates
        // (corrected on the next reindex) instead of a gap.
        let num_chunks = points.len();
        let new_ids: Vec<String> = points.iter().map(|p| p.id.clone()).collect();
        self.vectorstore.upsert_chunks(points).await?;
        self.vectorstore.delete_stale_chunks(slug, &new_ids).await?;

        tracing::debug!(slug, chunks = num_chunks, "RAG: indexed document");
        Ok(())
    }

    async fn delete_document(&self, slug: &str) -> Result<(), AppError> {
        self.vectorstore.delete_by_slug(slug).await?;
        tracing::debug!(slug, "RAG: deleted document chunks");
        Ok(())
    }

    #[tracing::instrument(skip(self, pages, tags), fields(attachment_key))]
    async fn index_attachment(
        &self,
        attachment_key: &str,
        filename: &str,
        pages: &[AttachmentPage],
        access_levels: &[String],
        tags: &[String],
    ) -> Result<usize, AppError> {
        // 1. Split each page into chunks and build enriched embedding texts:
        // "filename > p.N\n\nChunk text". A running index keeps chunk_index unique
        // across pages.
        let mut embedding_texts: Vec<String> = Vec::new();
        // (display text, source_page, chunk_index)
        let mut metas: Vec<(String, Option<u32>, u32)> = Vec::new();
        let mut running_index: u32 = 0;
        for page in pages {
            let chunks = split_document(
                &page.text,
                self.chunk_size_tokens,
                self.chunk_overlap_tokens,
            );
            for chunk in chunks {
                let mut prefix = filename.to_string();
                if let Some(p) = page.page_number {
                    prefix.push_str(&format!(" > p.{p}"));
                }
                embedding_texts.push(format!("{}\n\n{}", prefix, chunk.text));
                metas.push((chunk.text, page.page_number, running_index));
                running_index += 1;
            }
        }

        if embedding_texts.is_empty() {
            // Nothing indexable: remove any previously indexed chunks.
            self.vectorstore
                .delete_by_attachment_key(attachment_key)
                .await?;
            return Ok(0);
        }

        // 2. Embed (cache-backed), skipping any chunk whose embedding is empty.
        let vectors = self.embedding.embed(&embedding_texts).await?;
        let points: Vec<VectorPoint> = metas
            .into_iter()
            .zip(vectors)
            .filter_map(|((text, page, idx), vector)| {
                if vector.is_empty() {
                    tracing::warn!(
                        attachment_key,
                        idx,
                        "RAG: embedding returned empty vector for attachment chunk, skipping"
                    );
                    return None;
                }
                Some(VectorPoint {
                    id: Uuid::new_v4().to_string(),
                    vector,
                    payload: ChunkPayload {
                        chunk_text: text,
                        document_slug: String::new(),
                        document_title: filename.to_string(),
                        source_kind: SourceKind::Attachment,
                        attachment_key: Some(attachment_key.to_string()),
                        source_page: page,
                        access_levels: access_levels.to_vec(),
                        is_draft: false,
                        tags: tags.to_vec(),
                        chunk_index: idx,
                        section_path: Vec::new(),
                        section_anchor: String::new(),
                    },
                })
            })
            .collect();

        if points.is_empty() {
            // We had text to embed but every vector came back empty (degenerate
            // embedding run). Leave the existing index intact and report an error
            // so the caller marks the attachment failed and retries later, rather
            // than recording success and pinning stale chunks.
            return Err(AppError::Internal(format!(
                "all {} attachment chunks produced empty embeddings",
                embedding_texts.len()
            )));
        }

        // 3. Upsert then delete stale (mirrors index_document).
        let num_chunks = points.len();
        let new_ids: Vec<String> = points.iter().map(|p| p.id.clone()).collect();
        self.vectorstore.upsert_chunks(points).await?;
        // The new chunks are already indexed at this point, so a failure here is
        // non-fatal: log and continue rather than marking the whole attachment
        // Failed (which would force a wasteful full re-embed on retry). Leftover
        // stale chunks self-heal on the next successful run, since that run's
        // delete_stale call supersedes everything not in its own new_ids set —
        // including chunks left behind by this failure.
        if let Err(e) = self
            .vectorstore
            .delete_stale_attachment_chunks(attachment_key, &new_ids)
            .await
        {
            tracing::warn!(
                attachment_key,
                "RAG: failed to delete stale attachment chunks after a successful upsert, \
                 leaving duplicate/stale chunks until the next successful index: {e}"
            );
        }

        tracing::debug!(
            attachment_key,
            chunks = num_chunks,
            "RAG: indexed attachment"
        );
        Ok(num_chunks)
    }

    async fn delete_attachment(&self, attachment_key: &str) -> Result<(), AppError> {
        self.vectorstore
            .delete_by_attachment_key(attachment_key)
            .await?;
        tracing::debug!(attachment_key, "RAG: deleted attachment chunks");
        Ok(())
    }

    async fn update_attachment_access_levels(
        &self,
        attachment_key: &str,
        access_levels: &[String],
    ) -> Result<(), AppError> {
        self.vectorstore
            .set_attachment_access_levels(attachment_key, access_levels)
            .await?;
        tracing::debug!(attachment_key, "RAG: updated attachment access levels");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChatStepConfig, LlmConfig};
    use crate::rag::vectorstore::VectorSearchResult;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    struct FakeEmbedding;

    #[async_trait]
    impl EmbeddingService for FakeEmbedding {
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, AppError> {
            Ok(texts.iter().map(|_| vec![0.1, 0.2, 0.3]).collect())
        }
    }

    /// A vector store whose `delete_stale_attachment_chunks` always fails, to
    /// exercise the partial-failure path in `index_attachment` (upsert
    /// succeeds, stale cleanup fails).
    #[derive(Default)]
    struct FailingDeleteStaleStore {
        upserted: Mutex<Vec<VectorPoint>>,
        delete_stale_called: AtomicBool,
    }

    #[async_trait]
    impl VectorStore for FailingDeleteStaleStore {
        async fn ensure_collection(&self, _: u32) -> Result<(), AppError> {
            Ok(())
        }
        async fn upsert_chunks(&self, points: Vec<VectorPoint>) -> Result<(), AppError> {
            self.upserted.lock().unwrap().extend(points);
            Ok(())
        }
        async fn delete_by_slug(&self, _: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn delete_stale_chunks(&self, _: &str, _: &[String]) -> Result<(), AppError> {
            Ok(())
        }
        async fn delete_by_attachment_key(&self, _: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn delete_stale_attachment_chunks(
            &self,
            _: &str,
            _: &[String],
        ) -> Result<(), AppError> {
            self.delete_stale_called.store(true, Ordering::SeqCst);
            Err(AppError::Internal("qdrant unavailable".to_string()))
        }
        async fn set_attachment_access_levels(
            &self,
            _: &str,
            _: &[String],
        ) -> Result<(), AppError> {
            Ok(())
        }
        async fn search(
            &self,
            _: Vec<f32>,
            _: usize,
            _: Option<&[String]>,
            _: bool,
        ) -> Result<Vec<VectorSearchResult>, AppError> {
            Ok(vec![])
        }
        async fn get_section_chunks(
            &self,
            _: &str,
            _: &str,
            _: Option<&[String]>,
            _: bool,
        ) -> Result<Vec<VectorSearchResult>, AppError> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn index_attachment_succeeds_despite_stale_cleanup_failure() {
        let vectorstore = Arc::new(FailingDeleteStaleStore::default());
        let service = DefaultRagService::new(Arc::new(FakeEmbedding), vectorstore.clone(), 256, 32);

        let pages = vec![AttachmentPage {
            page_number: Some(1),
            text: "some extracted PDF text long enough to form a chunk".to_string(),
        }];

        let result = service
            .index_attachment("pdfs/a.pdf", "a.pdf", &pages, &["public".to_string()], &[])
            .await;

        assert!(
            result.is_ok(),
            "a stale-cleanup failure after a successful upsert must not fail the whole index: {result:?}"
        );
        assert!(
            !vectorstore.upserted.lock().unwrap().is_empty(),
            "new chunks should still have been upserted"
        );
        assert!(
            vectorstore.delete_stale_called.load(Ordering::SeqCst),
            "stale cleanup should still have been attempted"
        );
    }

    #[tokio::test]
    async fn from_rag_config_fails_when_not_configured() {
        let config = RagConfig {
            qdrant_url: String::new(),
            qdrant_collection: "test".into(),
            embedding_url: String::new(),
            embedding_model: "nomic-embed-text".into(),
            embedding_dimensions: 768,
            embedding_api_key: String::new(),
            embedding_headers: std::collections::HashMap::new(),
            embedding_vertex_project_id: String::new(),
            embedding_vertex_location: String::new(),
            embedding_cache_store_text: false,
            embedding_cache_query: false,
            chunk_size_tokens: 256,
            chunk_overlap_tokens: 64,
            expand_to_parent: false,
            hybrid_search_enabled: false,
            reranker_url: String::new(),
            reranker_model: String::new(),
            reranker_api_key: String::new(),
            reranker_headers: std::collections::HashMap::new(),
            llm: LlmConfig {
                url: String::new(),
                api_key: String::new(),
                model: String::new(),
                headers: std::collections::HashMap::new(),
                vertex_project_id: String::new(),
                vertex_location: String::new(),
            },
            chat: ChatStepConfig {
                model: None,
                url: None,
                api_key: None,
                headers: None,
                vertex_project_id: None,
                vertex_location: None,
                system_prompt_template: String::new(),
            },
            analyzer: None,
            hyde: None,
            rewriter: None,
            vlm: None,
            attachment_page_text_threshold: 100,
        };
        assert!(DefaultRagService::from_rag_config(&config).await.is_err());
    }
}
