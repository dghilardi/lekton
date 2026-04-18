//! Personal Access Token (PAT) API endpoints.
//!
//! | Method | Path                          | Auth     | Description                        |
//! |--------|-------------------------------|----------|------------------------------------|
//! | GET    | `/api/v1/user/pats`           | User     | List caller's PATs                 |
//! | POST   | `/api/v1/user/pats`           | User     | Create a new PAT (token shown once)|
//! | PATCH  | `/api/v1/user/pats/{id}`      | User     | Toggle active/inactive (own token) |
//! | DELETE | `/api/v1/user/pats/{id}`      | User     | Permanently delete (own token)     |
//! | GET    | `/api/v1/admin/pats`          | Admin    | List all PATs, paginated           |
//! | PATCH  | `/api/v1/admin/pats/{id}`     | Admin    | Toggle any PAT active/inactive     |

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::app::AppState;
use crate::auth::extractor::RequiredAuthUser;
use crate::auth::token_service::TokenService;
use crate::db::service_token_models::ServiceToken;
use crate::error::AppError;

// ── Request / Response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreatePatRequest {
    /// Human-readable name for the token. Must be unique.
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct CreatePatResponse {
    pub id: String,
    pub name: String,
    /// Raw token value — only returned once, never stored.
    pub raw_token: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct PatSummary {
    pub id: String,
    pub name: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct AdminPatSummary {
    pub id: String,
    pub name: String,
    pub is_active: bool,
    /// `None` for admin-PATs without a linked user.
    pub user_id: Option<String>,
    /// Resolved from `user_id`; `None` for admin-PATs.
    pub user_email: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct PaginatedPatResponse {
    pub items: Vec<AdminPatSummary>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    #[serde(default = "default_page")]
    pub page: u64,
    #[serde(default = "default_per_page")]
    pub per_page: u64,
}

fn default_page() -> u64 {
    1
}
fn default_per_page() -> u64 {
    20
}

#[derive(Debug, Deserialize)]
pub struct TogglePatRequest {
    pub active: bool,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn to_pat_summary(t: &ServiceToken) -> PatSummary {
    PatSummary {
        id: t.id.clone(),
        name: t.name.clone(),
        is_active: t.is_active,
        created_at: t.created_at,
        last_used_at: t.last_used_at,
    }
}

// ── User endpoints ────────────────────────────────────────────────────────────

/// `GET /api/v1/user/pats` — list the caller's PATs.
pub async fn list_user_pats_handler(
    State(state): State<AppState>,
    RequiredAuthUser(user): RequiredAuthUser,
) -> Result<Json<Vec<PatSummary>>, AppError> {
    let tokens = state
        .service_token_repo
        .list_by_user_id(&user.user_id)
        .await?;
    Ok(Json(tokens.iter().map(to_pat_summary).collect()))
}

/// `POST /api/v1/user/pats` — create a new PAT for the caller.
pub async fn create_user_pat_handler(
    State(state): State<AppState>,
    RequiredAuthUser(user): RequiredAuthUser,
    Json(req): Json<CreatePatRequest>,
) -> Result<(StatusCode, Json<CreatePatResponse>), AppError> {
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("PAT name cannot be empty".into()));
    }

    let raw_token = TokenService::generate_opaque_token();
    let token_hash = TokenService::hash_token(&raw_token);
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();

    let token = ServiceToken {
        id: id.clone(),
        name: name.clone(),
        token_hash,
        allowed_scopes: vec![],
        token_type: "pat".to_string(),
        user_id: Some(user.user_id.clone()),
        can_write: false,
        created_by: user.user_id,
        created_at: now,
        last_used_at: None,
        is_active: true,
    };

    state.service_token_repo.create(token).await?;

    Ok((
        StatusCode::CREATED,
        Json(CreatePatResponse {
            id,
            name,
            raw_token,
            created_at: now,
        }),
    ))
}

/// `PATCH /api/v1/user/pats/{id}` — toggle a PAT active/inactive (own tokens only).
pub async fn toggle_user_pat_handler(
    State(state): State<AppState>,
    RequiredAuthUser(user): RequiredAuthUser,
    Path(id): Path<String>,
    Json(req): Json<TogglePatRequest>,
) -> Result<StatusCode, AppError> {
    // Verify ownership before toggling
    let token = state
        .service_token_repo
        .find_by_id(&id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("PAT '{id}' not found")))?;

    if token.user_id.as_deref() != Some(&user.user_id) {
        return Err(AppError::Forbidden("You do not own this token".into()));
    }

    state.service_token_repo.set_active(&id, req.active).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/v1/user/pats/{id}` — permanently delete a PAT (own tokens only).
pub async fn delete_user_pat_handler(
    State(state): State<AppState>,
    RequiredAuthUser(user): RequiredAuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    state
        .service_token_repo
        .delete_pat(&id, &user.user_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Admin endpoints ───────────────────────────────────────────────────────────

/// `GET /api/v1/admin/pats` — paginated list of all PATs with resolved user emails.
pub async fn admin_list_pats_handler(
    State(state): State<AppState>,
    RequiredAuthUser(user): RequiredAuthUser,
    Query(params): Query<PaginationParams>,
) -> Result<Json<PaginatedPatResponse>, AppError> {
    if !user.is_admin {
        return Err(AppError::Forbidden("Admin privileges required".into()));
    }

    let per_page = params.per_page.clamp(1, 100);
    let (tokens, total) = state
        .service_token_repo
        .list_pats_paginated(params.page, per_page)
        .await?;

    // Resolve user emails: collect unique user_ids, batch-fetch users
    let user_ids: Vec<&str> = tokens
        .iter()
        .filter_map(|t| t.user_id.as_deref())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let mut email_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for uid in user_ids {
        if let Ok(Some(u)) = state.user_repo.find_user_by_id(uid).await {
            email_map.insert(u.id, u.email);
        }
    }

    let items = tokens
        .iter()
        .map(|t| AdminPatSummary {
            id: t.id.clone(),
            name: t.name.clone(),
            is_active: t.is_active,
            user_id: t.user_id.clone(),
            user_email: t
                .user_id
                .as_ref()
                .and_then(|uid| email_map.get(uid).cloned()),
            created_at: t.created_at,
            last_used_at: t.last_used_at,
        })
        .collect();

    Ok(Json(PaginatedPatResponse {
        items,
        total,
        page: params.page,
        per_page,
    }))
}

/// `PATCH /api/v1/admin/pats/{id}` — toggle any PAT active/inactive.
pub async fn admin_toggle_pat_handler(
    State(state): State<AppState>,
    RequiredAuthUser(user): RequiredAuthUser,
    Path(id): Path<String>,
    Json(req): Json<TogglePatRequest>,
) -> Result<StatusCode, AppError> {
    if !user.is_admin {
        return Err(AppError::Forbidden("Admin privileges required".into()));
    }

    // Verify the token is a PAT (not a service token)
    let token = state
        .service_token_repo
        .find_by_id(&id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("PAT '{id}' not found")))?;

    if !token.is_pat() {
        return Err(AppError::BadRequest(
            "This endpoint only manages PATs".into(),
        ));
    }

    state.service_token_repo.set_active(&id, req.active).await?;
    Ok(StatusCode::NO_CONTENT)
}
