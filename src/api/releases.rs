//! `POST /api/v1/releases/promote` — move a source's `latest` alias.
//!
//! Called by `lekton-sync --latest` *after* the documents have been uploaded, so
//! the alias only ever moves onto a fully published release. Promoting first
//! would leave readers on a half-uploaded one if the run failed partway.

use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::error::AppError;

/// Request payload for `POST /api/v1/releases/promote`.
#[derive(Debug, Deserialize)]
pub struct PromoteReleaseRequest {
    /// Service authentication token (legacy or scoped).
    pub service_token: String,
    /// The source whose alias should move.
    pub source_id: String,
    /// The release to alias as `latest`. Must already be published.
    pub release: String,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct PromoteReleaseResponse {
    pub source_id: String,
    pub release: String,
    /// Documents whose `is_latest` flag changed, and which are therefore marked
    /// stale for search and RAG.
    pub reindex_pending: usize,
}

/// Request payload for `POST /api/v1/releases/finalize`.
#[derive(Debug, Deserialize)]
pub struct FinalizeReleaseRequest {
    pub service_token: String,
    pub source_id: String,
    pub release: String,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct FinalizeReleaseResponse {
    pub source_id: String,
    pub release: String,
}

#[cfg(feature = "ssr")]
fn incomplete_release_documents(
    expected: &[crate::db::release_repository::ReleaseDocumentExpectation],
    documents: &[crate::db::models::Document],
) -> Vec<String> {
    expected
        .iter()
        .filter(|expectation| {
            !documents.iter().any(|document| {
                document.slug == expectation.slug
                    && document.source_path.as_deref() == Some(expectation.source_path.as_str())
                    && document.content_hash.as_deref() == Some(expectation.content_hash.as_str())
                    && expectation
                        .metadata_hash
                        .as_deref()
                        .is_none_or(|hash| document.metadata_hash.as_deref() == Some(hash))
            })
        })
        .map(|expectation| expectation.slug.clone())
        .collect()
}

#[cfg(feature = "ssr")]
fn ensure_documents_in_scope(
    documents: &[crate::db::models::Document],
    scopes: &[String],
) -> Result<(), AppError> {
    if let Some(document) = documents
        .iter()
        .find(|document| !crate::api::sync::scope_matches_any(&document.slug, scopes))
    {
        return Err(AppError::Forbidden(format!(
            "Token does not have access to slug '{}'",
            document.slug
        )));
    }
    Ok(())
}

/// Verify a staged manifest and publish it to the release catalogue.
#[cfg(feature = "ssr")]
pub async fn process_finalize_release(
    repo: &dyn crate::db::repository::DocumentRepository,
    release_repo: &dyn crate::db::release_repository::ReleaseRepository,
    service_token_repo: &dyn crate::db::service_token_repository::ServiceTokenRepository,
    legacy_token: Option<&str>,
    request: FinalizeReleaseRequest,
) -> Result<FinalizeReleaseResponse, AppError> {
    let scopes = crate::api::sync::validate_sync_token(
        service_token_repo,
        legacy_token,
        &request.service_token,
    )
    .await?;

    let staged = release_repo
        .find(&request.source_id, &request.release)
        .await?
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "release '{}' is not staged for source '{}'",
                request.release, request.source_id
            ))
        })?;

    if let Some(expectation) = staged
        .expected_documents
        .iter()
        .find(|expectation| !crate::api::sync::scope_matches_any(&expectation.slug, &scopes))
    {
        return Err(AppError::Forbidden(format!(
            "Token does not have access to slug '{}'",
            expectation.slug
        )));
    }

    let documents = repo
        .find_all_by_source_id_and_release(&request.source_id, Some(&request.release))
        .await?;
    ensure_documents_in_scope(&documents, &scopes)?;

    let incomplete = incomplete_release_documents(&staged.expected_documents, &documents);
    if !incomplete.is_empty() {
        return Err(AppError::BadRequest(format!(
            "release '{}' is incomplete; missing or stale documents: {}",
            request.release,
            incomplete.join(", ")
        )));
    }

    release_repo
        .finalize(&request.source_id, &request.release)
        .await?;

    Ok(FinalizeReleaseResponse {
        source_id: request.source_id,
        release: request.release,
    })
}

/// Core promotion logic — separated from the HTTP layer for testability.
///
/// Returns the response plus the slugs whose `latest` membership changed, which
/// the caller feeds to [`reindex_promoted`].
#[cfg(feature = "ssr")]
pub async fn process_promote_release(
    repo: &dyn crate::db::repository::DocumentRepository,
    release_repo: &dyn crate::db::release_repository::ReleaseRepository,
    service_token_repo: &dyn crate::db::service_token_repository::ServiceTokenRepository,
    legacy_token: Option<&str>,
    request: PromoteReleaseRequest,
) -> Result<(PromoteReleaseResponse, Vec<String>), AppError> {
    // Promotion is a write, so reuse the sync token rules (must have can_write).
    let scopes = crate::api::sync::validate_sync_token(
        service_token_repo,
        legacy_token,
        &request.service_token,
    )
    .await?;

    // Refuse to alias a release nobody published: otherwise a typo would point
    // `latest` at nothing and hide the whole source.
    let published = release_repo.list_by_source(&request.source_id).await?;
    if !published.iter().any(|r| r.release == request.release) {
        return Err(AppError::BadRequest(format!(
            "release '{}' is not published for source '{}'",
            request.release, request.source_id
        )));
    }

    let target_documents = repo
        .find_all_by_source_id_and_release(&request.source_id, Some(&request.release))
        .await?;
    ensure_documents_in_scope(&target_documents, &scopes)?;

    // Alias first (a single atomic write), then bring the denormalized
    // `is_latest` flags in line. If the second step failed, the flags would be
    // repaired by re-running the promotion — whereas flags without an alias
    // would leave the two disagreeing with nothing to reconcile them.
    release_repo
        .set_latest(&request.source_id, &request.release)
        .await?;
    let affected = repo
        .promote_release(&request.source_id, &request.release)
        .await?;

    Ok((
        PromoteReleaseResponse {
            source_id: request.source_id,
            release: request.release,
            reindex_pending: affected.len(),
        },
        affected,
    ))
}

/// Axum handler for `POST /api/v1/releases/finalize`.
#[cfg(feature = "ssr")]
pub async fn finalize_release_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::Json(request): axum::Json<FinalizeReleaseRequest>,
) -> Result<axum::Json<FinalizeReleaseResponse>, AppError> {
    if !state.features.doc_versioning {
        return Err(AppError::BadRequest(
            "documentation versioning is disabled on this instance".into(),
        ));
    }

    let response = process_finalize_release(
        state.document_repo.as_ref(),
        state.release_repo.as_ref(),
        state.service_token_repo.as_ref(),
        Some(&state.service_token),
        request,
    )
    .await?;

    Ok(axum::Json(response))
}

/// Bring search and RAG in line with a promotion, for the documents whose
/// `latest` membership changed.
///
/// Only `latest` is indexed, so moving the alias makes the index stale in two
/// ways at once, and both have to be handled per slug:
/// - a slug the new release still ships must be re-indexed from the new
///   release's body (the entry currently holds the old release's content);
/// - a slug the new release dropped is no longer latest anywhere, so its entry
///   has to go.
///
/// Failures are logged and leave `needs_reindex` set, so an operator can see
/// what is stale and retry, rather than the drift being silent.
#[cfg(feature = "ssr")]
pub async fn reindex_promoted(
    repo: &dyn crate::db::repository::DocumentRepository,
    storage: &dyn crate::storage::client::StorageClient,
    search: Option<&dyn crate::search::client::SearchService>,
    rag: Option<&dyn crate::rag::service::RagService>,
    slugs: &[String],
) {
    for slug in slugs {
        let candidates = match repo.find_all_by_slug(slug).await {
            Ok(candidates) => candidates,
            Err(e) => {
                tracing::warn!(slug = %slug, "promotion reindex: cannot load document: {e}");
                continue;
            }
        };

        let latest = crate::db::repository::resolve_by_release(
            candidates,
            &crate::versioning::ReleasePins::default(),
        )
        .filter(|d| !d.is_archived);

        let Some(doc) = latest else {
            // Dropped by the promoted release: nothing is latest under this slug.
            if let Some(search) = search {
                if let Err(e) = search.delete_document(slug).await {
                    tracing::warn!(slug = %slug, "promotion reindex: search delete failed: {e}");
                }
            }
            if let Some(rag) = rag {
                if let Err(e) = rag.delete_document(slug).await {
                    tracing::warn!(slug = %slug, "promotion reindex: RAG delete failed: {e}");
                }
            }
            continue;
        };

        let content = match storage.get_object(&doc.s3_key).await {
            Ok(Some(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
            Ok(None) => {
                tracing::warn!(slug = %slug, "promotion reindex: body missing from storage");
                continue;
            }
            Err(e) => {
                tracing::warn!(slug = %slug, "promotion reindex: cannot read body: {e}");
                continue;
            }
        };

        let mut ok = true;

        if let Some(search) = search {
            let search_doc = crate::search::client::build_search_document(&doc, &content);
            if let Err(e) = search.index_document(&search_doc).await {
                tracing::warn!(slug = %slug, "promotion reindex: search index failed: {e}");
                ok = false;
            }
        }

        if let Some(rag) = rag {
            let result = if doc.skip_rag {
                rag.delete_document(slug).await
            } else {
                rag.index_document(
                    &doc.slug,
                    &doc.title,
                    &content,
                    &doc.access_level,
                    doc.is_draft,
                    &doc.tags,
                    doc.source_id.as_deref(),
                    doc.release.as_deref(),
                )
                .await
            };
            if let Err(e) = result {
                tracing::warn!(slug = %slug, "promotion reindex: RAG index failed: {e}");
                ok = false;
            }
        }

        // Field-scoped, never a full write: indexing above may have taken long
        // enough for another promotion to land, and replacing the document from
        // this stale snapshot would undo it.
        if ok && doc.needs_reindex {
            if let Err(e) = repo
                .clear_needs_reindex(&doc.slug, doc.release.as_deref())
                .await
            {
                tracing::warn!(slug = %slug, "promotion reindex: cannot clear needs_reindex: {e}");
            }
        }
    }
}

/// Axum handler for `POST /api/v1/releases/promote`.
#[cfg(feature = "ssr")]
pub async fn promote_release_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::Json(request): axum::Json<PromoteReleaseRequest>,
) -> Result<axum::Json<PromoteReleaseResponse>, AppError> {
    if !state.features.doc_versioning {
        return Err(AppError::BadRequest(
            "documentation versioning is disabled on this instance".into(),
        ));
    }

    let (response, affected) = process_promote_release(
        state.document_repo.as_ref(),
        state.release_repo.as_ref(),
        state.service_token_repo.as_ref(),
        Some(&state.service_token),
        request,
    )
    .await?;

    // Re-indexing runs detached: it re-embeds documents, which can take far
    // longer than the CLI is willing to wait on a tag operation. `needs_reindex`
    // stays set until it succeeds, so nothing is lost if this task dies.
    if !affected.is_empty() {
        let repo = state.document_repo.clone();
        let storage = state.storage_client.clone();
        let search = state.search_service.clone();
        let rag = state.rag_service.clone();
        tokio::spawn(async move {
            reindex_promoted(
                repo.as_ref(),
                storage.as_ref(),
                search.as_deref(),
                rag.as_deref(),
                &affected,
            )
            .await;
        });
    }

    Ok(axum::Json(response))
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::db::models::Document;
    use crate::db::release_repository::ReleaseDocumentExpectation;

    fn document(slug: &str, release: &str, content_hash: &str) -> Document {
        Document {
            slug: slug.to_string(),
            title: slug.to_string(),
            summary: None,
            s3_key: format!("docs/{release}/{slug}.md"),
            access_level: "public".to_string(),
            is_draft: false,
            service_owner: "platform".to_string(),
            last_updated: Utc::now(),
            tags: vec![],
            links_out: vec![],
            backlinks: vec![],
            parent_slug: None,
            order: 0,
            is_hidden: false,
            content_hash: Some(content_hash.to_string()),
            metadata_hash: Some("metadata".to_string()),
            is_archived: false,
            source_path: Some(format!("{slug}.md")),
            source_id: Some("source".to_string()),
            release: Some(release.to_string()),
            is_latest: false,
            needs_reindex: false,
            skip_rag: false,
        }
    }

    fn expectation(slug: &str, content_hash: &str) -> ReleaseDocumentExpectation {
        ReleaseDocumentExpectation {
            slug: slug.to_string(),
            source_path: format!("{slug}.md"),
            content_hash: content_hash.to_string(),
            metadata_hash: Some("metadata".to_string()),
        }
    }

    #[test]
    fn finalization_detects_missing_or_stale_documents() {
        let expected = vec![expectation("guide", "new"), expectation("api", "current")];
        let documents = vec![document("guide", "2.0.0", "old")];

        assert_eq!(
            incomplete_release_documents(&expected, &documents),
            vec!["guide".to_string(), "api".to_string()]
        );
    }

    #[test]
    fn finalization_accepts_the_exact_staged_snapshot() {
        let expected = vec![expectation("guide", "current")];
        let documents = vec![document("guide", "2.0.0", "current")];

        assert!(incomplete_release_documents(&expected, &documents).is_empty());
    }

    #[test]
    fn promotion_scope_check_covers_every_document_in_the_release() {
        let documents = vec![
            document("team/guide", "2.0.0", "a"),
            document("other/private", "2.0.0", "b"),
        ];

        assert!(matches!(
            ensure_documents_in_scope(&documents, &["team/*".to_string()]),
            Err(AppError::Forbidden(message)) if message.contains("other/private")
        ));
        assert!(ensure_documents_in_scope(&documents, &["*".to_string()]).is_ok());
    }
}
