use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::app::AppState;
#[cfg(feature = "ssr")]
use crate::server::require_admin_user;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessLevelInfo {
    pub name: String,
    pub label: String,
    pub description: String,
    pub inherits_from: Vec<String>,
    pub is_system: bool,
}

#[server(ListAdminAccessLevels, "/api")]
pub async fn list_admin_access_levels() -> Result<Vec<AccessLevelInfo>, ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    let levels = state
        .access_level_repo
        .list_all()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(levels
        .into_iter()
        .map(|l| AccessLevelInfo {
            name: l.name,
            label: l.label,
            description: l.description,
            inherits_from: l.inherits_from,
            is_system: l.is_system,
        })
        .collect())
}

#[server(CreateAdminAccessLevel, "/api")]
pub async fn create_admin_access_level(
    name: String,
    label: String,
    description: String,
    inherits_from: Vec<String>,
) -> Result<(), ServerFnError> {
    use crate::db::auth_models::AccessLevelEntity;

    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    let name = name.trim().to_lowercase();
    if name.is_empty() {
        return Err(ServerFnError::new("Name cannot be empty"));
    }

    let level = AccessLevelEntity {
        name,
        label,
        description,
        inherits_from,
        is_system: false,
        created_at: chrono::Utc::now(),
    };
    state
        .access_level_repo
        .create(level)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

#[server(UpdateAdminAccessLevel, "/api")]
pub async fn update_admin_access_level(
    name: String,
    label: String,
    description: String,
    inherits_from: Vec<String>,
) -> Result<(), ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    let existing = state
        .access_level_repo
        .find_by_name(&name)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new(format!("Access level '{name}' not found")))?;

    let inheritance_changed = existing.inherits_from != inherits_from;

    let updated = crate::db::auth_models::AccessLevelEntity {
        name: existing.name.clone(),
        label,
        description,
        inherits_from,
        is_system: existing.is_system,
        created_at: existing.created_at,
    };

    state
        .access_level_repo
        .update(updated)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if inheritance_changed {
        crate::jobs::recompute_access_levels::spawn_recompute_for_level(
            existing.name,
            state.access_level_repo.clone(),
            state.user_repo.clone(),
        );
    }

    Ok(())
}

#[server(DeleteAdminAccessLevel, "/api")]
pub async fn delete_admin_access_level(name: String) -> Result<(), ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    state
        .access_level_repo
        .delete(&name)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}
