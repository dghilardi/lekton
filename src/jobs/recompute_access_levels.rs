//! Background job: recompute `effective_access_levels` for all users whose
//! `assigned_access_levels` contain a level whose inheritance chain was
//! modified — the changed level itself or any level inheriting from it.
//!
//! Called (via `tokio::spawn`) whenever an access level's `inherits_from` changes.
//! Errors are logged but not propagated — the caller should not block on this job.

use std::sync::Arc;

use crate::db::access_level_repository::AccessLevelRepository;
use crate::db::user_repository::UserRepository;

/// Spawn a background task that recomputes effective access levels for all users
/// transitively affected by a change to `changed_level_name`.
///
/// The task finds every user assigned `changed_level_name` or a level that
/// inherits from it (transitively), recomputes their effective set through the
/// current DAG, and persists the result.
pub fn spawn_recompute_for_level(
    changed_level_name: String,
    access_level_repo: Arc<dyn AccessLevelRepository>,
    user_repo: Arc<dyn UserRepository>,
) {
    tokio::spawn(async move {
        if let Err(e) =
            recompute_for_level(&changed_level_name, &*access_level_repo, &*user_repo).await
        {
            tracing::error!(
                level = %changed_level_name,
                error = %e,
                "Failed to recompute effective access levels after hierarchy change"
            );
        }
    });
}

async fn recompute_for_level(
    changed_level_name: &str,
    access_level_repo: &dyn AccessLevelRepository,
    user_repo: &dyn UserRepository,
) -> Result<(), crate::error::AppError> {
    // A user's effective set changes when any level they are assigned reaches
    // the changed level through inheritance, not only when they are assigned
    // it directly.
    let levels = access_level_repo.list_all().await?;
    let mut affected_users = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for level in levels_inheriting(changed_level_name, &levels) {
        for user in user_repo.list_users_with_assigned_level(&level).await? {
            if seen.insert(user.id.clone()) {
                affected_users.push(user);
            }
        }
    }

    tracing::info!(
        level = %changed_level_name,
        user_count = affected_users.len(),
        "Recomputing effective access levels"
    );

    for user in affected_users {
        let new_effective = access_level_repo
            .compute_effective_levels(&user.assigned_access_levels)
            .await?;

        user_repo
            .update_user_effective_levels(&user.id, new_effective)
            .await?;
    }

    Ok(())
}

/// `changed` plus every level that inherits from it, directly or through
/// other levels, in the current hierarchy.
fn levels_inheriting(
    changed: &str,
    levels: &[crate::db::auth_models::AccessLevelEntity],
) -> Vec<String> {
    let mut found = vec![changed.to_string()];
    let mut next = 0;
    while next < found.len() {
        let current = found[next].clone();
        next += 1;
        for level in levels {
            if level.inherits_from.contains(&current) && !found.contains(&level.name) {
                found.push(level.name.clone());
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::auth_models::{AccessLevelEntity, User};
    use crate::error::AppError;
    use async_trait::async_trait;
    use chrono::Utc;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct MockAccessLevelRepo {
        // name -> inherits_from
        levels: HashMap<String, Vec<String>>,
    }

    #[async_trait]
    impl AccessLevelRepository for MockAccessLevelRepo {
        async fn create(&self, _: AccessLevelEntity) -> Result<(), AppError> {
            Ok(())
        }
        async fn find_by_name(&self, name: &str) -> Result<Option<AccessLevelEntity>, AppError> {
            Ok(self.levels.get(name).map(|parents| AccessLevelEntity {
                name: name.to_string(),
                label: name.to_string(),
                description: String::new(),
                inherits_from: parents.clone(),
                is_system: false,
                created_at: Utc::now(),
            }))
        }
        async fn list_all(&self) -> Result<Vec<AccessLevelEntity>, AppError> {
            let mut names: Vec<_> = self.levels.keys().cloned().collect();
            names.sort();
            let mut all = Vec::new();
            for name in names {
                all.extend(self.find_by_name(&name).await?);
            }
            Ok(all)
        }
        async fn update(&self, _: AccessLevelEntity) -> Result<(), AppError> {
            Ok(())
        }
        async fn delete(&self, _: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn exists(&self, name: &str) -> Result<bool, AppError> {
            Ok(self.levels.contains_key(name))
        }
        async fn seed_defaults(&self) -> Result<(), AppError> {
            Ok(())
        }
        async fn compute_effective_levels(
            &self,
            roots: &[String],
        ) -> Result<Vec<String>, AppError> {
            use std::collections::{HashSet, VecDeque};

            let mut effective: HashSet<String> = HashSet::new();
            let mut queue: VecDeque<String> = roots.iter().cloned().collect();

            while let Some(current) = queue.pop_front() {
                if effective.insert(current.clone()) {
                    if let Some(parents) = self.levels.get(&current) {
                        for parent in parents {
                            if !effective.contains(parent) {
                                queue.push_back(parent.clone());
                            }
                        }
                    }
                }
            }
            Ok(effective.into_iter().collect())
        }
    }

    struct MockUserRepo {
        users: Mutex<Vec<User>>,
    }

    impl MockUserRepo {
        fn with_user(id: &str, assigned: Vec<String>) -> Self {
            let user = User {
                id: id.to_string(),
                email: format!("{id}@test.com"),
                name: None,
                provider_sub: format!("sub-{id}"),
                provider_type: "oidc".to_string(),
                is_admin: false,
                assigned_access_levels: assigned.clone(),
                effective_access_levels: assigned,
                budget_plan: None,
                can_write: false,
                can_read_draft: false,
                can_write_draft: false,
                created_at: Utc::now(),
                last_login_at: None,
            };
            Self {
                users: Mutex::new(vec![user]),
            }
        }
    }

    #[async_trait]
    impl UserRepository for MockUserRepo {
        async fn set_user_budget_plan(
            &self,
            _user_id: &str,
            _plan: Option<String>,
        ) -> Result<(), AppError> {
            Ok(())
        }

        async fn create_user(&self, user: User) -> Result<(), AppError> {
            self.users.lock().unwrap().push(user);
            Ok(())
        }
        async fn find_user_by_id(&self, id: &str) -> Result<Option<User>, AppError> {
            Ok(self
                .users
                .lock()
                .unwrap()
                .iter()
                .find(|u| u.id == id)
                .cloned())
        }
        async fn find_user_by_email(&self, _: &str) -> Result<Option<User>, AppError> {
            Ok(None)
        }
        async fn find_user_by_provider_sub(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Option<User>, AppError> {
            Ok(None)
        }
        async fn touch_last_login(&self, _: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn list_users(&self) -> Result<Vec<User>, AppError> {
            Ok(self.users.lock().unwrap().clone())
        }
        async fn set_user_access_levels(
            &self,
            user_id: &str,
            assigned: Vec<String>,
            effective: Vec<String>,
            can_write: bool,
            can_read_draft: bool,
            can_write_draft: bool,
        ) -> Result<(), AppError> {
            let mut users = self.users.lock().unwrap();
            if let Some(u) = users.iter_mut().find(|u| u.id == user_id) {
                u.assigned_access_levels = assigned;
                u.effective_access_levels = effective;
                u.can_write = can_write;
                u.can_read_draft = can_read_draft;
                u.can_write_draft = can_write_draft;
            }
            Ok(())
        }
        async fn update_user_effective_levels(
            &self,
            user_id: &str,
            effective: Vec<String>,
        ) -> Result<(), AppError> {
            let mut users = self.users.lock().unwrap();
            if let Some(u) = users.iter_mut().find(|u| u.id == user_id) {
                u.effective_access_levels = effective;
            }
            Ok(())
        }
        async fn list_users_with_assigned_level(
            &self,
            level_name: &str,
        ) -> Result<Vec<User>, AppError> {
            Ok(self
                .users
                .lock()
                .unwrap()
                .iter()
                .filter(|u| u.assigned_access_levels.contains(&level_name.to_string()))
                .cloned()
                .collect())
        }
        async fn create_refresh_token(
            &self,
            _: crate::db::auth_models::RefreshToken,
        ) -> Result<(), AppError> {
            Ok(())
        }
        async fn find_refresh_token_by_hash(
            &self,
            _: &str,
        ) -> Result<Option<crate::db::auth_models::RefreshToken>, AppError> {
            Ok(None)
        }
        async fn revoke_refresh_token(&self, _: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn revoke_all_user_tokens(&self, _: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn revoke_refresh_token_family(&self, _: &str) -> Result<(), AppError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_recompute_expands_inherited_levels() {
        // Graph: cloud-dev -> developer -> internal
        let mut levels = HashMap::new();
        levels.insert("cloud-dev".to_string(), vec!["developer".to_string()]);
        levels.insert("developer".to_string(), vec!["internal".to_string()]);
        levels.insert("internal".to_string(), vec![]);

        let access_repo = MockAccessLevelRepo { levels };
        // User has assigned = ["cloud-dev"], effective starts the same
        let user_repo = MockUserRepo::with_user("u1", vec!["cloud-dev".to_string()]);

        // "developer" inheritance changed — recompute users assigned cloud-dev via developer
        recompute_for_level("cloud-dev", &access_repo, &user_repo)
            .await
            .unwrap();

        let user = user_repo.find_user_by_id("u1").await.unwrap().unwrap();
        let mut effective = user.effective_access_levels.clone();
        effective.sort();
        assert_eq!(effective, vec!["cloud-dev", "developer", "internal"]);
    }

    #[tokio::test]
    async fn test_recompute_reaches_users_assigned_a_level_inheriting_the_changed_one() {
        // Graph: cloud-dev -> developer -> internal; `developer` now also
        // inherits `secrets`.
        let mut levels = HashMap::new();
        levels.insert("cloud-dev".to_string(), vec!["developer".to_string()]);
        levels.insert(
            "developer".to_string(),
            vec!["internal".to_string(), "secrets".to_string()],
        );
        levels.insert("internal".to_string(), vec![]);
        levels.insert("secrets".to_string(), vec![]);
        levels.insert("unrelated".to_string(), vec![]);
        let access_repo = MockAccessLevelRepo { levels };

        // Effective sets computed before the change.
        let user_repo = MockUserRepo::with_user("u1", vec!["cloud-dev".to_string()]);
        user_repo
            .update_user_effective_levels(
                "u1",
                vec!["cloud-dev".into(), "developer".into(), "internal".into()],
            )
            .await
            .unwrap();
        user_repo
            .create_user(User {
                id: "u2".to_string(),
                assigned_access_levels: vec!["unrelated".to_string()],
                effective_access_levels: vec!["stale-but-untouched".to_string()],
                ..user_repo.find_user_by_id("u1").await.unwrap().unwrap()
            })
            .await
            .unwrap();

        recompute_for_level("developer", &access_repo, &user_repo)
            .await
            .unwrap();

        let user = user_repo.find_user_by_id("u1").await.unwrap().unwrap();
        let mut effective = user.effective_access_levels.clone();
        effective.sort();
        assert_eq!(
            effective,
            vec!["cloud-dev", "developer", "internal", "secrets"]
        );
        let other = user_repo.find_user_by_id("u2").await.unwrap().unwrap();
        assert_eq!(other.effective_access_levels, vec!["stale-but-untouched"]);
    }
}
