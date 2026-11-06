use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::app::AppState;
#[cfg(feature = "ssr")]
use crate::server::require_any_user;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

/// A prompt with its text, for reading it in the library.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PromptDetail {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub owner: String,
    pub tags: Vec<String>,
    pub variables: Vec<PromptDetailVariable>,
    /// The prompt text, as served to MCP clients.
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PromptDetailVariable {
    pub name: String,
    pub description: String,
    pub required: bool,
}

/// Library filters. Empty or `None` fields do not filter.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PromptFilter {
    /// Matched (case-insensitively) against name, slug, description, owner and tags.
    pub query: String,
    pub owner: Option<String>,
    pub tag: Option<String>,
    /// `low`, `medium` or `high`.
    pub cost: Option<String>,
    pub favorites_only: bool,
}

impl PromptFilter {
    pub fn matches(&self, item: &PromptLibraryItem) -> bool {
        let query = self.query.trim().to_lowercase();
        let matches_query = query.is_empty()
            || item.name.to_lowercase().contains(&query)
            || item.slug.to_lowercase().contains(&query)
            || item.description.to_lowercase().contains(&query)
            || item.owner.to_lowercase().contains(&query)
            || item
                .tags
                .iter()
                .any(|tag| tag.to_lowercase().contains(&query));
        matches_query
            && self.owner.as_ref().is_none_or(|owner| &item.owner == owner)
            && self.tag.as_ref().is_none_or(|tag| item.tags.contains(tag))
            && self
                .cost
                .as_ref()
                .is_none_or(|cost| &item.context_cost == cost)
            && (!self.favorites_only || item.is_favorite)
    }
}

/// Distinct owners, and tags ordered by how many prompts use them (then by
/// name), for the library filter menus.
pub fn prompt_filter_options(items: &[PromptLibraryItem]) -> (Vec<String>, Vec<(String, usize)>) {
    use std::collections::{BTreeMap, BTreeSet};

    let owners: BTreeSet<String> = items.iter().map(|item| item.owner.clone()).collect();
    let mut tag_counts: BTreeMap<String, usize> = BTreeMap::new();
    for tag in items.iter().flat_map(|item| item.tags.iter()) {
        *tag_counts.entry(tag.clone()).or_default() += 1;
    }
    let mut tags: Vec<(String, usize)> = tag_counts.into_iter().collect();
    tags.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    (owners.into_iter().collect(), tags)
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
            budget_plan: None,
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

/// Load a prompt the user may see, or "Prompt not found" for any prompt
/// outside their access levels (or a draft they cannot read).
#[cfg(feature = "ssr")]
async fn find_visible_prompt(
    state: &AppState,
    user: &crate::auth::models::AuthenticatedUser,
    levels: &Option<Vec<String>>,
    include_draft: bool,
    slug: &str,
) -> Result<crate::db::prompt_models::Prompt, ServerFnError> {
    let prompt = state
        .prompt_repo
        .find_by_slug(slug)
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
    Ok(prompt)
}

#[server(GetPromptDetail, "/api")]
pub async fn get_prompt_detail(slug: String) -> Result<PromptDetail, ServerFnError> {
    /// The part of the stored prompt YAML the library shows.
    #[derive(Deserialize)]
    struct StoredPromptBody {
        prompt_body: String,
    }

    let state = expect_context::<AppState>();
    let user = require_any_user(&state).await?;
    let (levels, include_draft) = prompt_visibility_for_user(&state, &user).await?;
    let prompt = find_visible_prompt(&state, &user, &levels, include_draft, &slug).await?;

    let bytes = state
        .storage_client
        .get_object(&prompt.s3_key)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("Prompt content not found"))?;
    let stored: StoredPromptBody = serde_saphyr::from_slice(&bytes)
        .map_err(|e| ServerFnError::new(format!("Invalid prompt content: {e}")))?;

    Ok(PromptDetail {
        slug: prompt.slug,
        name: prompt.name,
        description: prompt.description,
        owner: prompt.owner,
        tags: prompt.tags,
        variables: prompt
            .variables
            .into_iter()
            .map(|v| PromptDetailVariable {
                name: v.name,
                description: v.description,
                required: v.required,
            })
            .collect(),
        body: stored.prompt_body,
    })
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
    find_visible_prompt(&state, &user, &levels, include_draft, &prompt_slug).await?;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn item(
        slug: &str,
        owner: &str,
        tags: &[&str],
        cost: &str,
        favorite: bool,
    ) -> PromptLibraryItem {
        PromptLibraryItem {
            slug: slug.into(),
            name: format!("Prompt {slug}"),
            description: format!("Describes {slug}"),
            access_level: "internal".into(),
            status: "active".into(),
            owner: owner.into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            publish_to_mcp: true,
            default_primary: false,
            context_cost: cost.into(),
            is_favorite: favorite,
            is_hidden: false,
        }
    }

    fn slugs(items: &[PromptLibraryItem], filter: &PromptFilter) -> Vec<String> {
        items
            .iter()
            .filter(|i| filter.matches(i))
            .map(|i| i.slug.clone())
            .collect()
    }

    fn library() -> Vec<PromptLibraryItem> {
        vec![
            item("review", "platform", &["code", "review"], "low", true),
            item("adr", "architecture", &["docs"], "high", false),
            item("tests", "platform", &["code", "testing"], "medium", false),
        ]
    }

    #[test]
    fn empty_filter_matches_everything() {
        assert_eq!(slugs(&library(), &PromptFilter::default()).len(), 3);
    }

    #[test]
    fn filters_combine() {
        let items = library();
        let by_owner = PromptFilter {
            owner: Some("platform".into()),
            ..Default::default()
        };
        assert_eq!(slugs(&items, &by_owner), vec!["review", "tests"]);

        let owner_and_cost = PromptFilter {
            cost: Some("medium".into()),
            ..by_owner.clone()
        };
        assert_eq!(slugs(&items, &owner_and_cost), vec!["tests"]);

        let by_tag = PromptFilter {
            tag: Some("code".into()),
            favorites_only: true,
            ..Default::default()
        };
        assert_eq!(slugs(&items, &by_tag), vec!["review"]);
    }

    #[test]
    fn query_matches_description_and_tags_case_insensitively() {
        let items = library();
        let filter = |q: &str| PromptFilter {
            query: q.into(),
            ..Default::default()
        };
        assert_eq!(slugs(&items, &filter("DESCRIBES adr")), vec!["adr"]);
        assert_eq!(slugs(&items, &filter("testing")), vec!["tests"]);
        assert!(slugs(&items, &filter("nothing")).is_empty());
    }

    #[test]
    fn tag_options_are_ordered_by_use() {
        let (owners, tags) = prompt_filter_options(&library());
        assert_eq!(owners, vec!["architecture", "platform"]);
        assert_eq!(tags[0], ("code".to_string(), 2));
        assert_eq!(tags.len(), 4);
    }
}
