use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::app::AppState;
#[cfg(feature = "ssr")]
use crate::server::require_admin_user;

/// A maintainer as exchanged with the admin UI.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaintainerDto {
    pub name: Option<String>,
    pub email: Option<String>,
    /// Internal UUID of a linked Lekton user, if any.
    pub lekton_user_id: Option<String>,
}

/// A documentation source with its discovered document count and curated
/// metadata (empty when no metadata has been saved yet).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceInfo {
    pub id: String,
    pub document_count: u64,
    /// `true` once metadata has been saved for this source.
    pub has_metadata: bool,
    pub display_name: Option<String>,
    pub repo_url: Option<String>,
    pub mainline_branch: Option<String>,
    pub description: Option<String>,
    pub maintainers: Vec<MaintainerDto>,
    /// Whether this source is opted into the automated documentation reviewer.
    pub review_enabled: bool,
}

/// List every documentation source: the union of source ids discovered on
/// documents and any saved metadata records (metadata may outlive its
/// documents). Ordered by id.
#[server(ListSources, "/api")]
pub async fn list_sources() -> Result<Vec<SourceInfo>, ServerFnError> {
    use std::collections::BTreeMap;

    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    let discovered = state
        .document_repo
        .list_source_ids()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let stored = state
        .document_source_repo
        .list()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    // Merge counts and metadata by id (BTreeMap keeps the output id-sorted).
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for entry in discovered {
        counts.insert(entry.source_id, entry.document_count);
    }

    let mut meta: BTreeMap<String, crate::db::source_models::DocumentSource> = BTreeMap::new();
    for source in stored {
        meta.insert(source.id.clone(), source);
    }

    let ids: std::collections::BTreeSet<String> =
        counts.keys().chain(meta.keys()).cloned().collect();

    let sources = ids
        .into_iter()
        .map(|id| {
            let document_count = counts.get(&id).copied().unwrap_or(0);
            match meta.remove(&id) {
                Some(m) => SourceInfo {
                    id,
                    document_count,
                    has_metadata: true,
                    display_name: m.display_name,
                    repo_url: m.repo_url,
                    mainline_branch: m.mainline_branch,
                    description: m.description,
                    maintainers: m
                        .maintainers
                        .into_iter()
                        .map(|mt| MaintainerDto {
                            name: mt.name,
                            email: mt.email,
                            lekton_user_id: mt.lekton_user_id,
                        })
                        .collect(),
                    review_enabled: m.review_enabled,
                },
                None => SourceInfo {
                    id,
                    document_count,
                    has_metadata: false,
                    display_name: None,
                    repo_url: None,
                    mainline_branch: None,
                    description: None,
                    maintainers: vec![],
                    review_enabled: false,
                },
            }
        })
        .collect();

    Ok(sources)
}

/// Create or update the metadata for a source id.
#[server(SaveSource, "/api")]
pub async fn save_source(
    id: String,
    display_name: Option<String>,
    repo_url: Option<String>,
    mainline_branch: Option<String>,
    description: Option<String>,
    maintainers: Vec<MaintainerDto>,
    review_enabled: bool,
) -> Result<(), ServerFnError> {
    use crate::db::source_models::{DocumentSource, Maintainer};

    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    let id = id.trim().to_string();
    if id.is_empty() {
        return Err(ServerFnError::new("source id cannot be empty"));
    }

    // Normalise blank strings to None, then validate/keep only real maintainers.
    let mut cleaned = Vec::new();
    for m in maintainers {
        let maintainer = Maintainer {
            name: blank_to_none(m.name),
            email: blank_to_none(m.email),
            lekton_user_id: blank_to_none(m.lekton_user_id),
        };
        // Skip fully-empty rows the UI may have left behind.
        if maintainer.name.is_none()
            && maintainer.email.is_none()
            && maintainer.lekton_user_id.is_none()
        {
            continue;
        }
        if !maintainer.is_valid() {
            return Err(ServerFnError::new(
                "each maintainer needs an email or a linked Lekton user",
            ));
        }
        cleaned.push(maintainer);
    }

    let now = chrono::Utc::now();
    let source = DocumentSource {
        id,
        display_name: blank_to_none(display_name),
        repo_url: blank_to_none(repo_url),
        mainline_branch: blank_to_none(mainline_branch),
        maintainers: cleaned,
        description: blank_to_none(description),
        review_enabled,
        // created_at is only applied on insert (via $setOnInsert); updated_at wins.
        created_at: now,
        updated_at: now,
    };

    state
        .document_source_repo
        .upsert(source)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Delete the metadata for a source id. Documents are left untouched.
#[server(DeleteSource, "/api")]
pub async fn delete_source(id: String) -> Result<(), ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    state
        .document_source_repo
        .delete(id.trim())
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Trim a string and collapse the empty result to `None`.
#[cfg(feature = "ssr")]
fn blank_to_none(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}
