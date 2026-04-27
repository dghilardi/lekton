use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::app::AppState;
#[cfg(feature = "ssr")]
use crate::server::{require_admin_user, require_any_user};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PatInfo {
    pub id: String,
    pub name: String,
    pub is_active: bool,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreatePatResult {
    pub id: String,
    pub name: String,
    pub raw_token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AdminPatInfo {
    pub id: String,
    pub name: String,
    pub is_active: bool,
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

#[server(ListUserPats, "/api")]
pub async fn list_user_pats() -> Result<Vec<PatInfo>, ServerFnError> {
    let state = expect_context::<AppState>();
    let user = require_any_user(&state).await?;

    let tokens = state
        .service_token_repo
        .list_by_user_id(&user.user_id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(tokens
        .into_iter()
        .map(|t| PatInfo {
            id: t.id,
            name: t.name,
            is_active: t.is_active,
            created_at: t.created_at.format("%Y-%m-%d %H:%M").to_string(),
            last_used_at: t
                .last_used_at
                .map(|d| d.format("%Y-%m-%d %H:%M").to_string()),
        })
        .collect())
}

#[server(CreateUserPat, "/api")]
pub async fn create_user_pat(name: String) -> Result<CreatePatResult, ServerFnError> {
    use crate::auth::token_service::TokenService;

    let state = expect_context::<AppState>();
    let user = require_any_user(&state).await?;

    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(ServerFnError::new("PAT name cannot be empty"));
    }

    let raw_token = TokenService::generate_opaque_token();
    let token_hash = TokenService::hash_token(&raw_token);
    let id = uuid::Uuid::new_v4().to_string();

    let token = crate::db::service_token_models::ServiceToken {
        id: id.clone(),
        name: name.clone(),
        token_hash,
        allowed_scopes: vec![],
        token_type: "pat".to_string(),
        user_id: Some(user.user_id.clone()),
        can_write: false,
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

    Ok(CreatePatResult {
        id,
        name,
        raw_token,
    })
}

#[server(ToggleUserPat, "/api")]
pub async fn toggle_user_pat(id: String, active: bool) -> Result<(), ServerFnError> {
    let state = expect_context::<AppState>();
    let user = require_any_user(&state).await?;

    let token = state
        .service_token_repo
        .find_by_id(&id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("PAT not found"))?;

    if token.user_id.as_deref() != Some(&user.user_id) {
        return Err(ServerFnError::new("You do not own this token"));
    }

    state
        .service_token_repo
        .set_active(&id, active)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(())
}

#[server(DeleteUserPat, "/api")]
pub async fn delete_user_pat(id: String) -> Result<(), ServerFnError> {
    let state = expect_context::<AppState>();
    let user = require_any_user(&state).await?;

    state
        .service_token_repo
        .delete_pat(&id, &user.user_id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(())
}

#[server(AdminListPats, "/api")]
pub async fn admin_list_pats(
    page: u64,
    per_page: u64,
) -> Result<(Vec<AdminPatInfo>, u64), ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    let per_page = per_page.clamp(1, 100);
    let (tokens, total) = state
        .service_token_repo
        .list_pats_paginated(page, per_page)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let mut email_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for token in &tokens {
        if let Some(uid) = &token.user_id {
            if !email_map.contains_key(uid) {
                if let Ok(Some(u)) = state.user_repo.find_user_by_id(uid).await {
                    email_map.insert(uid.clone(), u.email);
                }
            }
        }
    }

    let items = tokens
        .into_iter()
        .map(|t| {
            let email = t
                .user_id
                .as_ref()
                .and_then(|uid| email_map.get(uid).cloned());
            AdminPatInfo {
                id: t.id,
                name: t.name,
                is_active: t.is_active,
                user_id: t.user_id,
                user_email: email,
                created_at: t.created_at.format("%Y-%m-%d %H:%M").to_string(),
                last_used_at: t
                    .last_used_at
                    .map(|d| d.format("%Y-%m-%d %H:%M").to_string()),
            }
        })
        .collect();

    Ok((items, total))
}

#[server(AdminTogglePat, "/api")]
pub async fn admin_toggle_pat(id: String, active: bool) -> Result<(), ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    let token = state
        .service_token_repo
        .find_by_id(&id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("PAT not found"))?;

    if !token.is_pat() {
        return Err(ServerFnError::new("This endpoint only manages PATs"));
    }

    state
        .service_token_repo
        .set_active(&id, active)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(())
}
