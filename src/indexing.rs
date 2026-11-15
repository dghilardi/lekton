//! Bringing one document's search and RAG entries in line with MongoDB.
//!
//! `needs_reindex` is a single flag meaning "some enabled index does not hold
//! this document as MongoDB describes it". It may only be cleared by a pass
//! that has just brought the document into *every* enabled index, so the
//! write paths and both reindexes go through [`index_everywhere`] /
//! [`remove_everywhere`] for flagged documents and clear the flag only when
//! they report no failure.

use std::collections::HashSet;

use crate::db::models::{Asset, Document, DocumentReference};
use crate::db::repository::DocumentRepository;
use crate::error::AppError;
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
    index_everywhere_replacing(targets, doc, content, None).await
}

/// [`index_everywhere`] for a write that replaces a stored version whose
/// access was `previous_access` (access level, draft).
///
/// When RAG indexing fails, the old chunks normally stay (stale but better
/// than a gap). If the access changed, they would keep answering to the old
/// audience — after a narrowing, to readers who lost access — so they are
/// removed instead: fail closed until a retry indexes the document again.
pub async fn index_everywhere_replacing(
    targets: IndexTargets<'_>,
    doc: &Document,
    content: &str,
    previous_access: Option<(&str, bool)>,
) -> Vec<String> {
    let mut failures = Vec::new();

    if let Some(search) = targets.search {
        if let Err(e) = search
            .index_document(&build_search_document(doc, content))
            .await
        {
            failures.push(format!("search indexing failed: {e}"));
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
            let access_changed = previous_access
                .is_some_and(|(level, draft)| level != doc.access_level || draft != doc.is_draft);
            let mut failure = format!("rag indexing failed: {e}");
            if access_changed {
                match rag.delete_document(&doc.slug).await {
                    Ok(()) => failure.push_str("; old chunks removed, the access changed"),
                    Err(e) => failure.push_str(&format!("; removing the old chunks failed: {e}")),
                }
            }
            failures.push(failure);
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
            failures.push(format!("search removal failed: {e}"));
        }
    }
    if let Some(rag) = targets.rag {
        if let Err(e) = rag.delete_document(slug).await {
            failures.push(format!("rag removal failed: {e}"));
        }
    }
    failures
}

/// Which of `candidates` MongoDB knows *now* as live documents: the latest
/// copy of the slug is neither archived nor hidden.
///
/// An orphan sweep compares the index against a snapshot taken when its
/// reindex started, which can be half an hour old by the time it deletes. It
/// re-asks MongoDB about its candidates right before deleting, so an entry
/// ingested during the run is never removed.
pub async fn currently_live_slugs(
    repo: &dyn DocumentRepository,
    candidates: &[String],
) -> Result<HashSet<String>, AppError> {
    if candidates.is_empty() {
        return Ok(HashSet::new());
    }
    let mut by_slug = std::collections::HashMap::<String, Vec<Document>>::new();
    for document in repo.find_by_slugs(candidates).await? {
        by_slug
            .entry(document.slug.clone())
            .or_default()
            .push(document);
    }
    Ok(by_slug
        .into_values()
        .filter_map(|copies| {
            crate::db::repository::resolve_by_release(
                copies,
                &crate::versioning::ReleasePins::default(),
            )
        })
        .filter(|document| !document.is_archived && !document.is_hidden)
        .map(|document| document.slug)
        .collect())
}

/// The references hidden documents answer to: every hidden release copy. A
/// hidden document grants its attachments nothing.
pub fn hidden_references(documents: &[Document]) -> HashSet<DocumentReference> {
    documents
        .iter()
        .filter(|document| document.is_hidden)
        .map(|document| DocumentReference::new(document.slug.clone(), document.release.clone()))
        .collect()
}

/// Whether an attachment's entries belong in the indexes: not every
/// reference is to a hidden document. That includes an asset nothing
/// references yet: an upload is extracted before the document linking it is
/// ingested, and removing its entries in that window would lose them for
/// good (removing an unreferenced asset is the ACL recompute's job, which
/// deletes the record with its entries). Mirrors the `ineligible` rule of the
/// attachment ACL derivation.
pub fn attachment_is_live(asset: &Asset, hidden: &HashSet<DocumentReference>) -> bool {
    asset.referenced_by.is_empty()
        || !asset
            .referenced_by
            .iter()
            .all(|reference| hidden.contains(reference))
}
