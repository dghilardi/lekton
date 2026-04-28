use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::app::AppState;
#[cfg(feature = "ssr")]
use crate::server::require_admin_user;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminUserInfo {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
    pub is_admin: bool,
    pub assigned_access_levels: Vec<String>,
    pub effective_access_levels: Vec<String>,
    pub can_write: bool,
    pub can_read_draft: bool,
    pub can_write_draft: bool,
    pub last_login_at: Option<String>,
}

#[server(ListAdminUsers, "/api")]
pub async fn list_admin_users() -> Result<Vec<AdminUserInfo>, ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    let users = state
        .user_repo
        .list_users()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(users
        .into_iter()
        .map(|u| AdminUserInfo {
            id: u.id,
            email: u.email,
            name: u.name,
            is_admin: u.is_admin,
            assigned_access_levels: u.assigned_access_levels,
            effective_access_levels: u.effective_access_levels,
            can_write: u.can_write,
            can_read_draft: u.can_read_draft,
            can_write_draft: u.can_write_draft,
            last_login_at: u
                .last_login_at
                .map(|d| d.format("%Y-%m-%d %H:%M").to_string()),
        })
        .collect())
}

#[server(SetAdminUserAccessLevels, "/api")]
pub async fn set_admin_user_access_levels(
    user_id: String,
    assigned: Vec<String>,
    can_write: bool,
    can_read_draft: bool,
    can_write_draft: bool,
) -> Result<(), ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    for level in &assigned {
        if !state
            .access_level_repo
            .exists(level)
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?
        {
            return Err(ServerFnError::new(format!(
                "Access level '{level}' does not exist"
            )));
        }
    }

    let effective = state
        .access_level_repo
        .compute_effective_levels(&assigned)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    state
        .user_repo
        .set_user_access_levels(
            &user_id,
            assigned,
            effective,
            can_write,
            can_read_draft,
            can_write_draft,
        )
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}
