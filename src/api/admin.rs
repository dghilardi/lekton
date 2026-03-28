//! Admin API endpoints.
//!
//! All routes require an authenticated admin user (`is_admin = true`).
//!
//! | Method | Path                                        | Description                         |
//! |--------|---------------------------------------------|-------------------------------------|
//! | GET    | `/api/v1/admin/access-levels`               | List all access levels              |
//! | POST   | `/api/v1/admin/access-levels`               | Create a new access level           |
//! | PUT    | `/api/v1/admin/access-levels/{name}`        | Update an access level              |
//! | DELETE | `/api/v1/admin/access-levels/{name}`        | Delete an access level              |
//! | GET    | `/api/v1/admin/users`                       | List all users                      |
//! | GET    | `/api/v1/admin/users/{user_id}/permissions` | Get user's permissions              |
//! | PUT    | `/api/v1/admin/users/{user_id}/permissions` | Replace a user's permission set     |
//! | DELETE | `/api/v1/admin/users/{user_id}/permissions/{level}` | Remove one permission         |

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::app::AppState;
use crate::auth::extractor::RequiredAuthUser;
use crate::db::auth_models::{AccessLevelEntity, User, UserPermission};
use crate::error::AppError;

// ── Guard helper ──────────────────────────────────────────────────────────────

/// Reject the request with 403 Forbidden if the caller is not an admin.
fn require_admin(user: &crate::auth::models::AuthenticatedUser) -> Result<(), AppError> {
    if user.is_admin {
        Ok(())
    } else {
        Err(AppError::Forbidden("Admin privileges required".into()))
    }
}

// ── Access-level management ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateAccessLevelRequest {
    pub name: String,
    pub label: String,
    pub description: String,
    pub sort_order: u32,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAccessLevelRequest {
    pub label: String,
    pub description: String,
    pub sort_order: u32,
}

/// `GET /api/v1/admin/access-levels`
pub async fn list_access_levels_handler(
    State(state): State<AppState>,
    RequiredAuthUser(user): RequiredAuthUser,
) -> Result<Json<Vec<AccessLevelEntity>>, AppError> {
    require_admin(&user)?;
    let levels = state.access_level_repo.list_all().await?;
    Ok(Json(levels))
}

/// `POST /api/v1/admin/access-levels`
pub async fn create_access_level_handler(
    State(state): State<AppState>,
    RequiredAuthUser(user): RequiredAuthUser,
    Json(req): Json<CreateAccessLevelRequest>,
) -> Result<(StatusCode, Json<AccessLevelEntity>), AppError> {
    require_admin(&user)?;

    let name = req.name.trim().to_lowercase();
    if name.is_empty() {
        return Err(AppError::BadRequest("Access level name cannot be empty".into()));
    }

    let level = AccessLevelEntity {
        name: name.clone(),
        label: req.label,
        description: req.description,
        sort_order: req.sort_order,
        is_system: false,
        created_at: Utc::now(),
    };

    state.access_level_repo.create(level.clone()).await?;
    Ok((StatusCode::CREATED, Json(level)))
}

/// `PUT /api/v1/admin/access-levels/{name}`
pub async fn update_access_level_handler(
    State(state): State<AppState>,
    RequiredAuthUser(user): RequiredAuthUser,
    Path(name): Path<String>,
    Json(req): Json<UpdateAccessLevelRequest>,
) -> Result<Json<AccessLevelEntity>, AppError> {
    require_admin(&user)?;

    let existing = state
        .access_level_repo
        .find_by_name(&name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Access level '{name}' not found")))?;

    let updated = AccessLevelEntity {
        name: existing.name,
        label: req.label,
        description: req.description,
        sort_order: req.sort_order,
        is_system: existing.is_system,
        created_at: existing.created_at,
    };

    state.access_level_repo.update(updated.clone()).await?;
    Ok(Json(updated))
}

/// `DELETE /api/v1/admin/access-levels/{name}`
pub async fn delete_access_level_handler(
    State(state): State<AppState>,
    RequiredAuthUser(user): RequiredAuthUser,
    Path(name): Path<String>,
) -> Result<StatusCode, AppError> {
    require_admin(&user)?;
    state.access_level_repo.delete(&name).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── User management ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct UserWithPermissions {
    #[serde(flatten)]
    pub user: User,
    pub permissions: Vec<UserPermission>,
}

/// `GET /api/v1/admin/users`
pub async fn list_users_handler(
    State(state): State<AppState>,
    RequiredAuthUser(user): RequiredAuthUser,
) -> Result<Json<Vec<User>>, AppError> {
    require_admin(&user)?;
    let users = state.user_repo.list_users().await?;
    Ok(Json(users))
}

/// `GET /api/v1/admin/users/{user_id}/permissions`
pub async fn get_user_permissions_handler(
    State(state): State<AppState>,
    RequiredAuthUser(caller): RequiredAuthUser,
    Path(user_id): Path<String>,
) -> Result<Json<Vec<UserPermission>>, AppError> {
    require_admin(&caller)?;
    let perms = state.user_repo.get_permissions(&user_id).await?;
    Ok(Json(perms))
}

/// Request body for replacing a user's full permission set.
#[derive(Debug, Deserialize)]
pub struct SetPermissionsRequest {
    pub permissions: Vec<PermissionEntry>,
}

#[derive(Debug, Deserialize)]
pub struct PermissionEntry {
    pub access_level_name: String,
    pub can_read: bool,
    pub can_write: bool,
    pub can_read_draft: bool,
    pub can_write_draft: bool,
}

/// `PUT /api/v1/admin/users/{user_id}/permissions`
///
/// Replaces the full permission set for a user.  Each entry is upserted
/// individually (insert-or-replace by `(user_id, access_level_name)`).
pub async fn set_user_permissions_handler(
    State(state): State<AppState>,
    RequiredAuthUser(caller): RequiredAuthUser,
    Path(user_id): Path<String>,
    Json(req): Json<SetPermissionsRequest>,
) -> Result<Json<Vec<UserPermission>>, AppError> {
    require_admin(&caller)?;

    // Verify the target user exists
    state
        .user_repo
        .find_user_by_id(&user_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("User '{user_id}' not found")))?;

    let mut upserted = Vec::new();
    for entry in req.permissions {
        let perm = UserPermission {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: user_id.clone(),
            access_level_name: entry.access_level_name,
            can_read: entry.can_read,
            can_write: entry.can_write,
            can_read_draft: entry.can_read_draft,
            can_write_draft: entry.can_write_draft,
        };
        state.user_repo.upsert_permission(perm.clone()).await?;
        upserted.push(perm);
    }

    Ok(Json(upserted))
}

/// `DELETE /api/v1/admin/users/{user_id}/permissions/{level}`
pub async fn delete_user_permission_handler(
    State(state): State<AppState>,
    RequiredAuthUser(caller): RequiredAuthUser,
    Path((user_id, level)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    require_admin(&caller)?;
    state.user_repo.delete_permission(&user_id, &level).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Service token management ─────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateServiceTokenRequest {
    pub name: String,
    pub allowed_scopes: Vec<String>,
    #[serde(default)]
    pub can_write: bool,
}

#[derive(Debug, Serialize)]
pub struct CreateServiceTokenResponse {
    pub id: String,
    pub name: String,
    /// The raw token value — returned only once, never stored or retrievable again.
    pub raw_token: String,
    pub allowed_scopes: Vec<String>,
    pub can_write: bool,
}

#[derive(Debug, Serialize)]
pub struct ServiceTokenSummary {
    pub id: String,
    pub name: String,
    pub allowed_scopes: Vec<String>,
    pub can_write: bool,
    pub created_by: String,
    pub created_at: chrono::DateTime<Utc>,
    pub last_used_at: Option<chrono::DateTime<Utc>>,
    pub is_active: bool,
}

/// `POST /api/v1/admin/service-tokens`
///
/// Creates a new scoped service token. Returns the raw token value once.
pub async fn create_service_token_handler(
    State(state): State<AppState>,
    RequiredAuthUser(user): RequiredAuthUser,
    Json(req): Json<CreateServiceTokenRequest>,
) -> Result<(StatusCode, Json<CreateServiceTokenResponse>), AppError> {
    require_admin(&user)?;

    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("Token name cannot be empty".into()));
    }
    if req.allowed_scopes.is_empty() {
        return Err(AppError::BadRequest("At least one scope is required".into()));
    }

    // Check for scope overlap with existing tokens
    let has_overlap = state
        .service_token_repo
        .check_scope_overlap(&req.allowed_scopes, None)
        .await?;
    if has_overlap {
        return Err(AppError::BadRequest(
            "Scopes overlap with an existing service token".into(),
        ));
    }

    // Generate raw token and hash it
    let raw_token = uuid::Uuid::new_v4().to_string();
    let token_hash = crate::auth::token_service::TokenService::hash_token(&raw_token);
    let id = uuid::Uuid::new_v4().to_string();

    let token = crate::db::service_token_models::ServiceToken {
        id: id.clone(),
        name: name.clone(),
        token_hash,
        allowed_scopes: req.allowed_scopes.clone(),
        can_write: req.can_write,
        created_by: user.user_id,
        created_at: Utc::now(),
        last_used_at: None,
        is_active: true,
    };

    state.service_token_repo.create(token).await?;

    Ok((
        StatusCode::CREATED,
        Json(CreateServiceTokenResponse {
            id,
            name,
            raw_token,
            allowed_scopes: req.allowed_scopes,
            can_write: req.can_write,
        }),
    ))
}

/// `GET /api/v1/admin/service-tokens`
pub async fn list_service_tokens_handler(
    State(state): State<AppState>,
    RequiredAuthUser(user): RequiredAuthUser,
) -> Result<Json<Vec<ServiceTokenSummary>>, AppError> {
    require_admin(&user)?;

    let tokens = state.service_token_repo.list_all().await?;
    let summaries = tokens
        .into_iter()
        .map(|t| ServiceTokenSummary {
            id: t.id,
            name: t.name,
            allowed_scopes: t.allowed_scopes,
            can_write: t.can_write,
            created_by: t.created_by,
            created_at: t.created_at,
            last_used_at: t.last_used_at,
            is_active: t.is_active,
        })
        .collect();

    Ok(Json(summaries))
}

/// `DELETE /api/v1/admin/service-tokens/{id}`
pub async fn deactivate_service_token_handler(
    State(state): State<AppState>,
    RequiredAuthUser(user): RequiredAuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    require_admin(&user)?;
    state.service_token_repo.deactivate(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::models::AuthenticatedUser;

    fn admin_user() -> AuthenticatedUser {
        AuthenticatedUser {
            user_id: "admin-1".to_string(),
            email: "admin@test.com".to_string(),
            name: Some("Admin".to_string()),
            is_admin: true,
        }
    }

    fn regular_user() -> AuthenticatedUser {
        AuthenticatedUser {
            user_id: "user-1".to_string(),
            email: "user@test.com".to_string(),
            name: None,
            is_admin: false,
        }
    }

    #[test]
    fn test_require_admin_allows_admin() {
        assert!(require_admin(&admin_user()).is_ok());
    }

    #[test]
    fn test_require_admin_rejects_regular_user() {
        let result = require_admin(&regular_user());
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Forbidden(msg) => assert!(msg.contains("Admin")),
            other => panic!("expected Forbidden, got {other:?}"),
        }
    }
}
