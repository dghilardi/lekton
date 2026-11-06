use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::app::AppState;
#[cfg(feature = "ssr")]
use crate::server::require_admin_user;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdminUserInfo {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
    pub is_admin: bool,
    pub assigned_access_levels: Vec<String>,
    pub effective_access_levels: Vec<String>,
    /// AI spending plan, independent of the access levels above.
    pub budget_plan: Option<String>,
    pub can_write: bool,
    pub can_read_draft: bool,
    pub can_write_draft: bool,
    pub last_login_at: Option<String>,
}

/// Sorted, de-duplicated access level names. Stored lists keep insertion
/// order (effective levels come out of a graph walk) and may repeat a level.
pub fn normalize_levels(mut levels: Vec<String>) -> Vec<String> {
    levels.sort();
    levels.dedup();
    levels
}

/// Admin user list filters. Empty fields do not filter.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AdminUserFilter {
    /// Matched (case-insensitively) against email and name.
    pub query: String,
    /// Users whose effective levels include this one.
    pub level: Option<String>,
    pub admins_only: bool,
}

impl AdminUserFilter {
    pub fn matches(&self, user: &AdminUserInfo) -> bool {
        let query = self.query.trim().to_lowercase();
        let matches_query = query.is_empty()
            || user.email.to_lowercase().contains(&query)
            || user
                .name
                .as_ref()
                .is_some_and(|name| name.to_lowercase().contains(&query));
        matches_query
            && self
                .level
                .as_ref()
                .is_none_or(|level| user.effective_access_levels.contains(level))
            && (!self.admins_only || user.is_admin)
    }
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

    let mut users: Vec<AdminUserInfo> = users
        .into_iter()
        .map(|u| AdminUserInfo {
            id: u.id,
            email: u.email,
            name: u.name,
            is_admin: u.is_admin,
            assigned_access_levels: normalize_levels(u.assigned_access_levels),
            effective_access_levels: normalize_levels(u.effective_access_levels),
            budget_plan: u.budget_plan,
            can_write: u.can_write,
            can_read_draft: u.can_read_draft,
            can_write_draft: u.can_write_draft,
            last_login_at: u
                .last_login_at
                .map(|d| d.format("%Y-%m-%d %H:%M").to_string()),
        })
        .collect();
    users.sort_by_key(|u| u.email.to_lowercase());
    Ok(users)
}

/// Assign or clear a user's AI spending plan.
///
/// Deliberately a separate call from access-level assignment: the two are
/// administered independently, because what a user may read says nothing about
/// how much they need to spend.
#[server(SetAdminUserBudgetPlan, "/api")]
pub async fn set_admin_user_budget_plan(
    user_id: String,
    plan: Option<String>,
) -> Result<(), ServerFnError> {
    let state = expect_context::<AppState>();
    require_admin_user(&state).await?;

    // An empty string from a form select means "no plan".
    let plan = plan.filter(|name| !name.trim().is_empty());

    state
        .user_repo
        .set_user_budget_plan(&user_id, plan)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn user(email: &str, name: Option<&str>, admin: bool, effective: &[&str]) -> AdminUserInfo {
        AdminUserInfo {
            id: email.into(),
            email: email.into(),
            name: name.map(Into::into),
            is_admin: admin,
            assigned_access_levels: vec![],
            effective_access_levels: effective.iter().map(|l| l.to_string()).collect(),
            budget_plan: None,
            can_write: false,
            can_read_draft: false,
            can_write_draft: false,
            last_login_at: None,
        }
    }

    #[test]
    fn normalize_levels_sorts_and_dedups() {
        let levels = vec!["public", "internal", "cloud", "internal", "architect"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(
            normalize_levels(levels),
            vec!["architect", "cloud", "internal", "public"]
        );
    }

    #[test]
    fn filter_matches_email_name_level_and_role() {
        let alex = user(
            "alex@example.test",
            Some("Alex Rossi"),
            true,
            &["internal", "public"],
        );
        let sam = user("sam@example.test", None, false, &["public"]);

        let by_name = AdminUserFilter {
            query: "rossi".into(),
            ..Default::default()
        };
        assert!(by_name.matches(&alex));
        assert!(!by_name.matches(&sam));

        let by_level = AdminUserFilter {
            level: Some("internal".into()),
            ..Default::default()
        };
        assert!(by_level.matches(&alex) && !by_level.matches(&sam));

        let admins = AdminUserFilter {
            admins_only: true,
            query: "SAM".into(),
            ..Default::default()
        };
        assert!(!admins.matches(&sam));
        assert!(AdminUserFilter::default().matches(&sam));
    }
}
