use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::app::AppState;
#[cfg(feature = "ssr")]
use crate::server::require_admin_user;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceTokenInfo {
    pub id: String,
    pub name: String,
    pub allowed_scopes: Vec<String>,
    pub can_write: bool,
    pub is_active: bool,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateTokenResult {
    pub id: String,
    pub name: String,
    pub raw_token: String,
    pub allowed_scopes: Vec<String>,
}

#[server(ListServiceTokens, "/api")]
pub async fn list_service_tokens() -> Result<Vec<ServiceTokenInfo>, ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    let tokens = state
        .service_token_repo
        .list_all()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(tokens
        .into_iter()
        .map(|t| ServiceTokenInfo {
            id: t.id,
            name: t.name,
            allowed_scopes: t.allowed_scopes,
            can_write: t.can_write,
            is_active: t.is_active,
            created_at: t.created_at.format("%Y-%m-%d %H:%M").to_string(),
            last_used_at: t
                .last_used_at
                .map(|d| d.format("%Y-%m-%d %H:%M").to_string()),
        })
        .collect())
}

#[server(CreateServiceToken, "/api")]
pub async fn create_service_token(
    name: String,
    scopes: String,
    can_write: bool,
) -> Result<CreateTokenResult, ServerFnError> {
    use crate::auth::token_service::TokenService;

    let state = expect_context::<AppState>();
    let user = require_admin_user(&state).await?;

    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(ServerFnError::new("Token name cannot be empty"));
    }

    let allowed_scopes: Vec<String> = scopes
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if allowed_scopes.is_empty() {
        return Err(ServerFnError::new("At least one scope is required"));
    }

    let has_overlap = state
        .service_token_repo
        .check_scope_overlap(&allowed_scopes, None)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    if has_overlap {
        return Err(ServerFnError::new(
            "Scopes overlap with an existing service token",
        ));
    }

    let raw_token = TokenService::generate_opaque_token();
    let token_hash = TokenService::hash_token(&raw_token);

    let token = crate::db::service_token_models::ServiceToken {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.clone(),
        token_hash,
        allowed_scopes: allowed_scopes.clone(),
        token_type: "service".to_string(),
        user_id: None,
        can_write,
        created_by: user.user_id,
        created_at: chrono::Utc::now(),
        last_used_at: None,
        is_active: true,
    };

    state
        .service_token_repo
        .create(token)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(CreateTokenResult {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        raw_token,
        allowed_scopes,
    })
}

#[server(DeactivateServiceToken, "/api")]
pub async fn deactivate_service_token(id: String) -> Result<(), ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    state
        .service_token_repo
        .deactivate(&id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(())
}
