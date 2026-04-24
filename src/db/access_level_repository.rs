//! Repository for `AccessLevelEntity` — the configurable content categories.

use async_trait::async_trait;
#[cfg(feature = "ssr")]
use chrono::Utc;

use crate::db::auth_models::AccessLevelEntity;
use crate::error::AppError;

/// Default access levels seeded on first startup.
///
/// Tuple: (name, label, description, inherits_from, is_system).
/// `"public"` and `"loggeduser"` are system levels injected implicitly at query time
/// and cannot be deleted. The others are pre-populated for convenience.
pub const DEFAULT_ACCESS_LEVELS: &[(&str, &str, &str, &[&str], bool)] = &[
    ("public", "Public", "Publicly accessible content", &[], true),
    (
        "loggeduser",
        "Logged User",
        "Content for authenticated users",
        &[],
        true,
    ),
    (
        "internal",
        "Internal",
        "Internal company documentation",
        &[],
        false,
    ),
    (
        "developer",
        "Developer",
        "Developer-focused documentation",
        &["internal"],
        false,
    ),
    (
        "architect",
        "Architect",
        "Architecture-level documentation",
        &["developer"],
        false,
    ),
];

/// CRUD operations for `AccessLevelEntity`.
#[async_trait]
pub trait AccessLevelRepository: Send + Sync {
    /// Insert a new access level. Fails if `name` already exists.
    async fn create(&self, level: AccessLevelEntity) -> Result<(), AppError>;

    /// Find a level by its slug name.
    async fn find_by_name(&self, name: &str) -> Result<Option<AccessLevelEntity>, AppError>;

    /// List all levels ordered by system-first then alphabetically.
    async fn list_all(&self) -> Result<Vec<AccessLevelEntity>, AppError>;

    /// Replace an existing level (matched by `name`). Validates that the new
    /// `inherits_from` list does not introduce a cycle in the DAG.
    async fn update(&self, level: AccessLevelEntity) -> Result<(), AppError>;

    /// Delete a level by name. Returns `Forbidden` if `is_system = true`.
    async fn delete(&self, name: &str) -> Result<(), AppError>;

    /// Return `true` if a level with the given name exists.
    async fn exists(&self, name: &str) -> Result<bool, AppError>;

    /// Seed the default access levels if the collection is empty.
    async fn seed_defaults(&self) -> Result<(), AppError>;

    /// Compute the transitive closure of `roots` through the inheritance DAG.
    ///
    /// Returns every level reachable from any root (including the roots themselves).
    /// The result is deduplicated but not ordered.
    async fn compute_effective_levels(&self, roots: &[String]) -> Result<Vec<String>, AppError>;
}

// ── Cycle detection ───────────────────────────────────────────────────────────

/// Returns `true` if assigning `new_parents` to `updated_name` would create a
/// cycle in the DAG represented by `all_levels` (map of name -> inherits_from).
///
/// Algorithm: DFS from each node in `new_parents`, following the existing
/// inheritance links. If we reach `updated_name`, a cycle would be introduced.
#[cfg(feature = "ssr")]
fn would_introduce_cycle(
    updated_name: &str,
    new_parents: &[String],
    all_levels: &std::collections::HashMap<String, Vec<String>>,
) -> bool {
    use std::collections::HashSet;

    let mut visited: HashSet<&str> = HashSet::new();
    let mut stack: Vec<&str> = new_parents.iter().map(|s| s.as_str()).collect();

    while let Some(current) = stack.pop() {
        if current == updated_name {
            return true;
        }
        if visited.insert(current) {
            if let Some(parents) = all_levels.get(current) {
                for parent in parents {
                    stack.push(parent.as_str());
                }
            }
        }
    }
    false
}

// ── MongoDB implementation ────────────────────────────────────────────────────

/// MongoDB implementation of `AccessLevelRepository`.
#[cfg(feature = "ssr")]
pub struct MongoAccessLevelRepository {
    collection: mongodb::Collection<AccessLevelEntity>,
}

#[cfg(feature = "ssr")]
impl MongoAccessLevelRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            collection: db.collection("access_levels"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl AccessLevelRepository for MongoAccessLevelRepository {
    async fn create(&self, level: AccessLevelEntity) -> Result<(), AppError> {
        if self.exists(&level.name).await? {
            return Err(AppError::BadRequest(format!(
                "Access level '{}' already exists",
                level.name
            )));
        }

        // Validate that all inherits_from entries exist
        for parent in &level.inherits_from {
            if !self.exists(parent).await? {
                return Err(AppError::BadRequest(format!(
                    "Inherited access level '{parent}' does not exist"
                )));
            }
        }

        self.collection.insert_one(&level).await?;
        Ok(())
    }

    async fn find_by_name(&self, name: &str) -> Result<Option<AccessLevelEntity>, AppError> {
        use mongodb::bson::doc;
        Ok(self.collection.find_one(doc! { "name": name }).await?)
    }

    async fn list_all(&self) -> Result<Vec<AccessLevelEntity>, AppError> {
        use futures::TryStreamExt;
        use mongodb::options::FindOptions;

        // System levels first, then alphabetically by name
        let options = FindOptions::builder()
            .sort(mongodb::bson::doc! { "is_system": -1, "name": 1 })
            .build();

        let mut cursor = self
            .collection
            .find(mongodb::bson::doc! {})
            .with_options(options)
            .await?;

        let mut levels = Vec::new();
        while let Some(level) = cursor.try_next().await? {
            levels.push(level);
        }
        Ok(levels)
    }

    async fn update(&self, level: AccessLevelEntity) -> Result<(), AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;
        use mongodb::options::ReplaceOptions;
        use std::collections::HashMap;

        // Build the current DAG to perform cycle detection
        let mut cursor = self.collection.find(doc! {}).await?;
        let mut graph: HashMap<String, Vec<String>> = HashMap::new();
        while let Some(existing) = cursor.try_next().await? {
            graph.insert(existing.name.clone(), existing.inherits_from.clone());
        }

        if would_introduce_cycle(&level.name, &level.inherits_from, &graph) {
            return Err(AppError::BadRequest(format!(
                "Updating '{}' would introduce a cycle in the access-level inheritance graph",
                level.name
            )));
        }

        // Validate that all inherits_from entries exist
        for parent in &level.inherits_from {
            if !graph.contains_key(parent.as_str()) {
                return Err(AppError::BadRequest(format!(
                    "Inherited access level '{parent}' does not exist"
                )));
            }
        }

        let filter = doc! { "name": &level.name };
        let options = ReplaceOptions::builder().upsert(false).build();
        let result = self
            .collection
            .replace_one(filter, &level)
            .with_options(options)
            .await?;

        if result.matched_count == 0 {
            return Err(AppError::NotFound(format!(
                "Access level '{}' not found",
                level.name
            )));
        }
        Ok(())
    }

    async fn delete(&self, name: &str) -> Result<(), AppError> {
        use mongodb::bson::doc;

        let level = self
            .find_by_name(name)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Access level '{name}' not found")))?;

        if level.is_system {
            return Err(AppError::Forbidden(format!(
                "Cannot delete system access level '{name}'"
            )));
        }

        self.collection.delete_one(doc! { "name": name }).await?;
        Ok(())
    }

    async fn exists(&self, name: &str) -> Result<bool, AppError> {
        use mongodb::bson::doc;
        let count = self
            .collection
            .count_documents(doc! { "name": name })
            .await?;
        Ok(count > 0)
    }

    async fn seed_defaults(&self) -> Result<(), AppError> {
        use mongodb::bson::doc;

        let count = self.collection.count_documents(doc! {}).await?;
        if count > 0 {
            return Ok(());
        }

        for (name, label, description, inherits_from, is_system) in DEFAULT_ACCESS_LEVELS {
            let level = AccessLevelEntity {
                name: name.to_string(),
                label: label.to_string(),
                description: description.to_string(),
                inherits_from: inherits_from.iter().map(|s| s.to_string()).collect(),
                is_system: *is_system,
                created_at: Utc::now(),
            };
            self.collection.insert_one(&level).await?;
        }
        Ok(())
    }

    async fn compute_effective_levels(&self, roots: &[String]) -> Result<Vec<String>, AppError> {
        use futures::TryStreamExt;
        use std::collections::{HashMap, HashSet, VecDeque};

        // Load full graph
        let mut cursor = self.collection.find(mongodb::bson::doc! {}).await?;
        let mut graph: HashMap<String, Vec<String>> = HashMap::new();
        while let Some(level) = cursor.try_next().await? {
            graph.insert(level.name, level.inherits_from);
        }

        // BFS from roots
        let mut effective: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<String> = roots.iter().cloned().collect();

        while let Some(current) = queue.pop_front() {
            if effective.insert(current.clone()) {
                if let Some(parents) = graph.get(&current) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_levels_have_public_system() {
        let public = DEFAULT_ACCESS_LEVELS
            .iter()
            .find(|(name, ..)| *name == "public");
        assert!(public.is_some(), "public level must be in defaults");
        let (.., is_system) = public.unwrap();
        assert!(*is_system, "public must be a system level");
    }

    #[test]
    fn test_default_levels_have_loggeduser_system() {
        let loggeduser = DEFAULT_ACCESS_LEVELS
            .iter()
            .find(|(name, ..)| *name == "loggeduser");
        assert!(loggeduser.is_some(), "loggeduser level must be in defaults");
        let (.., is_system) = loggeduser.unwrap();
        assert!(*is_system, "loggeduser must be a system level");
    }

    #[test]
    fn test_default_levels_no_unknown_parents() {
        let names: std::collections::HashSet<&str> =
            DEFAULT_ACCESS_LEVELS.iter().map(|(n, ..)| *n).collect();
        for (name, _, _, parents, _) in DEFAULT_ACCESS_LEVELS {
            for parent in *parents {
                assert!(
                    names.contains(parent),
                    "Default level '{name}' references unknown parent '{parent}'"
                );
            }
        }
    }

    #[test]
    fn test_cycle_detection_direct() {
        use std::collections::HashMap;

        let mut graph: HashMap<String, Vec<String>> = HashMap::new();
        graph.insert("a".into(), vec!["b".into()]);
        graph.insert("b".into(), vec![]);

        // Updating "b" to inherit from "a" would create a -> b -> a cycle
        assert!(would_introduce_cycle("b", &["a".to_string()], &graph));

        // Updating "a" to inherit from "b" is fine (already exists, but no cycle in new direction)
        // Actually "a" already has b as child, so having a inherit b means a -> b -> a? No.
        // graph has "a" -> ["b"] meaning a inherits b. If we update "b" to inherit "a":
        // b.inherits_from = ["a"], and a.inherits_from = ["b"]. That's a cycle.
        // If we update "a" to inherit from "c" (new unknown): not a cycle but parent doesn't exist.
        assert!(!would_introduce_cycle("a", &["b".to_string()], &graph));
    }

    #[test]
    fn test_cycle_detection_indirect() {
        use std::collections::HashMap;

        // a -> b -> c; updating c to inherit a would create a cycle
        let mut graph: HashMap<String, Vec<String>> = HashMap::new();
        graph.insert("a".into(), vec!["b".into()]);
        graph.insert("b".into(), vec!["c".into()]);
        graph.insert("c".into(), vec![]);

        assert!(would_introduce_cycle("c", &["a".to_string()], &graph));
        assert!(!would_introduce_cycle("c", &["d".to_string()], &graph));
    }
}
