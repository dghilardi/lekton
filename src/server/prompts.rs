use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::app::AppState;
#[cfg(feature = "ssr")]
use crate::server::require_any_user;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptLibraryItem {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub access_level: String,
    pub status: String,
    pub owner: String,
    pub tags: Vec<String>,
    pub publish_to_mcp: bool,
    pub default_primary: bool,
    pub context_cost: String,
    pub is_favorite: bool,
    pub is_hidden: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptLibraryState {
    pub items: Vec<PromptLibraryItem>,
    pub estimated_context_cost: String,
    pub warnings: Vec<String>,
}

#[cfg(feature = "ssr")]
async fn prompt_visibility_for_user(
    state: &AppState,
    user: &crate::auth::models::AuthenticatedUser,
) -> Result<(Option<Vec<String>>, bool), ServerFnError> {
    if user.is_admin {
        return Ok((None, true));
    }

    let user_doc = state
        .user_repo
        .find_user_by_id(&user.user_id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let user_ctx = match user_doc {
        Some(u) => crate::auth::models::UserContext::from_user_doc(user.clone(), &u),
        None => crate::auth::models::UserContext {
            user: user.clone(),
            effective_access_levels: vec![],
            can_write: false,
            can_read_draft: false,
            can_write_draft: false,
        },
    };
    Ok(user_ctx.document_visibility())
}

#[cfg(feature = "ssr")]
fn prompt_context_cost_label(weight: u32) -> String {
    if weight >= 12 {
        "high".to_string()
    } else if weight >= 6 {
        "medium".to_string()
    } else {
        "low".to_string()
    }
}

#[cfg(feature = "ssr")]
pub(crate) fn build_prompt_library_state(
    prompts: Vec<crate::db::prompt_models::Prompt>,
    preferences: Vec<crate::db::user_prompt_preference_repository::UserPromptPreference>,
) -> PromptLibraryState {
    use std::collections::HashMap;

    let pref_by_slug: HashMap<
        String,
        crate::db::user_prompt_preference_repository::UserPromptPreference,
    > = preferences
        .into_iter()
        .map(|pref| (pref.prompt_slug.clone(), pref))
        .collect();

    let mut items = Vec::new();
    let mut total_context_weight = 0u32;

    for prompt in prompts {
        let pref = pref_by_slug.get(&prompt.slug);
        let is_favorite = pref.map(|p| p.is_favorite).unwrap_or(false);
        let is_hidden = pref.map(|p| p.is_hidden).unwrap_or(false);

        if prompt.publish_to_mcp && ((prompt.default_primary && !is_hidden) || is_favorite) {
            total_context_weight += prompt.context_cost.weight() as u32;
        }

        items.push(PromptLibraryItem {
            slug: prompt.slug,
            name: prompt.name,
            description: prompt.description,
            access_level: prompt.access_level,
            status: match prompt.status {
                crate::db::prompt_models::PromptStatus::Draft => "draft".to_string(),
                crate::db::prompt_models::PromptStatus::Active => "active".to_string(),
                crate::db::prompt_models::PromptStatus::Deprecated => "deprecated".to_string(),
            },
            owner: prompt.owner,
            tags: prompt.tags,
            publish_to_mcp: prompt.publish_to_mcp,
            default_primary: prompt.default_primary,
            context_cost: match prompt.context_cost {
                crate::db::prompt_models::ContextCost::Low => "low".to_string(),
                crate::db::prompt_models::ContextCost::Medium => "medium".to_string(),
                crate::db::prompt_models::ContextCost::High => "high".to_string(),
            },
            is_favorite,
            is_hidden,
        });
    }

    items.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.slug.cmp(&b.slug)));

    let mut warnings = Vec::new();
    if total_context_weight >= 12 {
        warnings.push(
            "Selected prompts add heavy context overhead; reduce favorites or hide some primary prompts.".to_string(),
        );
    } else if total_context_weight >= 8 {
        warnings.push("Selected prompts may add significant context overhead.".to_string());
    }

    PromptLibraryState {
        items,
        estimated_context_cost: prompt_context_cost_label(total_context_weight),
        warnings,
    }
}

#[server(GetPromptLibraryState, "/api")]
pub async fn get_prompt_library_state() -> Result<PromptLibraryState, ServerFnError> {
    let state = expect_context::<AppState>();
    let user = require_any_user(&state).await?;
    let (levels, include_draft) = prompt_visibility_for_user(&state, &user).await?;

    let prompts = state
        .prompt_repo
        .list_by_access_levels(levels.as_deref(), include_draft)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let preferences = state
        .user_prompt_preference_repo
        .list_by_user_id(&user.user_id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(build_prompt_library_state(prompts, preferences))
}

#[server(SavePromptPreference, "/api")]
pub async fn save_prompt_preference(
    prompt_slug: String,
    is_favorite: bool,
    is_hidden: bool,
) -> Result<PromptLibraryState, ServerFnError> {
    let state = expect_context::<AppState>();
    let user = require_any_user(&state).await?;
    let (levels, include_draft) = prompt_visibility_for_user(&state, &user).await?;

    let prompt = state
        .prompt_repo
        .find_by_slug(&prompt_slug)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("Prompt not found"))?;

    let allowed = user.is_admin
        || levels
            .as_ref()
            .is_none_or(|ls| ls.contains(&prompt.access_level));
    let can_read_draft = if user.is_admin { true } else { include_draft };
    if !allowed
        || (prompt.status == crate::db::prompt_models::PromptStatus::Draft && !can_read_draft)
    {
        return Err(ServerFnError::new("Prompt not found"));
    }

    let preference = crate::db::user_prompt_preference_repository::UserPromptPreference {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: user.user_id.clone(),
        prompt_slug: prompt_slug.clone(),
        is_favorite,
        is_hidden,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    state
        .user_prompt_preference_repo
        .upsert(preference)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let prompts = state
        .prompt_repo
        .list_by_access_levels(levels.as_deref(), include_draft)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let preferences = state
        .user_prompt_preference_repo
        .list_by_user_id(&user.user_id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(build_prompt_library_state(prompts, preferences))
}
