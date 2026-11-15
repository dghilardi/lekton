//! Bringing one document's search and RAG entries in line with MongoDB.
//!
//! `needs_reindex` is a single flag meaning "some enabled index does not hold
//! this document as MongoDB describes it". It may only be cleared by a pass
//! that has just brought the document into *every* enabled index, so the
//! write paths and both reindexes go through [`index_everywhere`] /
//! [`remove_everywhere`] for flagged documents and clear the flag only when
//! they report no failure.

use crate::db::models::Document;
use crate::rag::service::RagService;
use crate::search::client::{build_search_document, SearchService};

/// The indexes enabled on this instance. `None` means that index is disabled
/// and has nothing to hold.
#[derive(Clone, Copy, Default)]
pub struct IndexTargets<'a> {
    pub search: Option<&'a dyn SearchService>,
    pub rag: Option<&'a dyn RagService>,
}

/// Index `doc`, whose body is `content`, into every enabled index. A
/// `skip_rag` document is removed from RAG instead. Returns the failures, one
/// message per index; empty means every enabled index now holds the document.
pub async fn index_everywhere(
    targets: IndexTargets<'_>,
    doc: &Document,
    content: &str,
) -> Vec<String> {
    let mut failures = Vec::new();

    if let Some(search) = targets.search {
        if let Err(e) = search
            .index_document(&build_search_document(doc, content))
            .await
        {
            failures.push(format!("search: {e}"));
        }
    }

    if let Some(rag) = targets.rag {
        let result = if doc.skip_rag {
            rag.delete_document(&doc.slug).await
        } else {
            rag.index_document(
                &doc.slug,
                &doc.title,
                content,
                &doc.access_level,
                doc.is_draft,
                &doc.tags,
                doc.source_id.as_deref(),
                doc.release.as_deref(),
            )
            .await
        };
        if let Err(e) = result {
            failures.push(format!("rag: {e}"));
        }
    }

    failures
}

/// Remove `slug` from every enabled index. Returns the failures, one message
/// per index; empty means no enabled index holds the slug any more.
pub async fn remove_everywhere(targets: IndexTargets<'_>, slug: &str) -> Vec<String> {
    let mut failures = Vec::new();
    if let Some(search) = targets.search {
        if let Err(e) = search.delete_document(slug).await {
            failures.push(format!("search: {e}"));
        }
    }
    if let Some(rag) = targets.rag {
        if let Err(e) = rag.delete_document(slug).await {
            failures.push(format!("rag: {e}"));
        }
    }
    failures
}
