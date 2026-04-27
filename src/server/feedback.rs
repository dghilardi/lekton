use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::app::AppState;
#[cfg(feature = "ssr")]
use crate::server::{require_admin_user, require_any_user};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeedbackInfo {
    pub message_id: String,
    pub session_id: String,
    pub rating: String,
    pub comment: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeedbackListResult {
    pub items: Vec<FeedbackInfo>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DocumentationFeedbackAdminItem {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub title: String,
    pub summary: String,
    pub related_resources: Vec<String>,
    pub search_queries: Vec<String>,
    pub created_by: String,
    pub created_at: String,
    pub duplicate_of: Option<String>,
    pub resolution_note: Option<String>,
    pub related_feedback_ids: Vec<String>,
    pub user_goal: Option<String>,
    pub missing_information: Option<String>,
    pub impact: Option<String>,
    pub suggested_target_resource: Option<String>,
    pub target_resource_uri: Option<String>,
    pub problem_summary: Option<String>,
    pub proposal: Option<String>,
    pub supporting_resources: Vec<String>,
    pub expected_benefit: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DocumentationFeedbackAdminListResult {
    pub items: Vec<DocumentationFeedbackAdminItem>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

#[cfg(feature = "ssr")]
fn map_documentation_feedback_item(
    item: crate::db::documentation_feedback_models::DocumentationFeedback,
) -> DocumentationFeedbackAdminItem {
    DocumentationFeedbackAdminItem {
        id: item.id,
        kind: item.kind.as_str().to_string(),
        status: item.status.as_str().to_string(),
        title: item.title,
        summary: item.summary,
        related_resources: item.related_resources,
        search_queries: item.search_queries,
        created_by: item.created_by,
        created_at: item.created_at.format("%Y-%m-%d %H:%M").to_string(),
        duplicate_of: item.duplicate_of,
        resolution_note: item.resolution_note,
        related_feedback_ids: item.related_feedback_ids,
        user_goal: item.user_goal,
        missing_information: item.missing_information,
        impact: item.impact,
        suggested_target_resource: item.suggested_target_resource,
        target_resource_uri: item.target_resource_uri,
        problem_summary: item.problem_summary,
        proposal: item.proposal,
        supporting_resources: item.supporting_resources,
        expected_benefit: item.expected_benefit,
    }
}

#[server(ListUserFeedback, "/api")]
pub async fn list_user_feedback(
    page: u64,
    per_page: u64,
) -> Result<FeedbackListResult, ServerFnError> {
    use crate::db::chat_models::FeedbackRating;
    use crate::db::feedback_repository::FeedbackListParams;

    let state = expect_context::<AppState>();
    let user = require_any_user(&state).await?;

    let fb_repo = state
        .feedback_repo
        .as_ref()
        .ok_or_else(|| ServerFnError::new("Feedback not available"))?;

    let per_page = per_page.clamp(1, 50);
    let params = FeedbackListParams {
        page,
        per_page,
        ..Default::default()
    };

    let result = fb_repo
        .list_user_feedback(&user.user_id, params)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let items = result
        .items
        .into_iter()
        .map(|fb| {
            let rating = match fb.rating {
                FeedbackRating::Positive => "positive".to_string(),
                FeedbackRating::Negative => "negative".to_string(),
            };
            FeedbackInfo {
                message_id: fb.message_id,
                session_id: fb.session_id,
                rating,
                comment: fb.comment,
                created_at: fb.created_at.format("%Y-%m-%d %H:%M").to_string(),
            }
        })
        .collect();

    Ok(FeedbackListResult {
        items,
        total: result.total,
        page: result.page,
        per_page: result.per_page,
    })
}

#[server(DeleteUserFeedback, "/api")]
pub async fn delete_user_feedback(message_id: String) -> Result<(), ServerFnError> {
    let state = expect_context::<AppState>();
    let user = require_any_user(&state).await?;

    let fb_repo = state
        .feedback_repo
        .as_ref()
        .ok_or_else(|| ServerFnError::new("Feedback not available"))?;

    fb_repo
        .delete_feedback(&message_id, &user.user_id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(())
}

#[server(ListDocumentationFeedback, "/api")]
pub async fn list_documentation_feedback(
    page: u64,
    per_page: u64,
    query: Option<String>,
    kind: Option<String>,
    status: Option<String>,
) -> Result<DocumentationFeedbackAdminListResult, ServerFnError> {
    use crate::db::documentation_feedback_models::{
        DocumentationFeedbackKind, DocumentationFeedbackStatus,
    };
    use crate::db::documentation_feedback_repository::DocumentationFeedbackListParams;

    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    let kind = kind
        .map(|value| value.parse::<DocumentationFeedbackKind>())
        .transpose()
        .map_err(ServerFnError::new)?;
    let status = status
        .map(|value| value.parse::<DocumentationFeedbackStatus>())
        .transpose()
        .map_err(ServerFnError::new)?;

    let result = state
        .documentation_feedback_repo
        .list(DocumentationFeedbackListParams {
            query,
            kind,
            status,
            page,
            per_page: per_page.clamp(1, 50),
        })
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(DocumentationFeedbackAdminListResult {
        items: result
            .items
            .into_iter()
            .map(map_documentation_feedback_item)
            .collect(),
        total: result.total,
        page: result.page,
        per_page: result.per_page,
    })
}

#[server(ResolveDocumentationFeedback, "/api")]
pub async fn resolve_documentation_feedback(
    id: String,
    resolution_note: Option<String>,
) -> Result<(), ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    state
        .documentation_feedback_repo
        .resolve(
            &id,
            resolution_note.filter(|value| !value.trim().is_empty()),
        )
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

#[server(MarkDocumentationFeedbackDuplicate, "/api")]
pub async fn mark_documentation_feedback_duplicate(
    id: String,
    duplicate_of: String,
    resolution_note: Option<String>,
) -> Result<(), ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    let duplicate_of = duplicate_of.trim().to_string();
    if duplicate_of.is_empty() {
        return Err(ServerFnError::new("Duplicate target id is required"));
    }
    if duplicate_of == id {
        return Err(ServerFnError::new(
            "A feedback item cannot duplicate itself",
        ));
    }

    state
        .documentation_feedback_repo
        .find_by_id(&duplicate_of)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("Duplicate target not found"))?;

    state
        .documentation_feedback_repo
        .mark_duplicate(
            &id,
            &duplicate_of,
            resolution_note.filter(|value| !value.trim().is_empty()),
        )
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}
