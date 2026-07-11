//! Startup migration framework for MongoDB schema evolution.
//!
//! Migrations are idempotent: each runs at most once and is tracked in the
//! `__migrations` collection. A failed migration blocks startup until the
//! issue is resolved manually.

#[cfg(feature = "ssr")]
mod inner {
    use chrono::{DateTime, Utc};
    use futures::StreamExt as _;
    use mongodb::Database;
    use mongodb::{options::IndexOptions, IndexModel};
    use serde::{Deserialize, Serialize};
    use std::collections::HashSet;
    use std::future::Future;
    use std::pin::Pin;
    use std::time::Instant;

    const CHANGELOG: &str = "__migrations";

    type BoxError = Box<dyn std::error::Error + Send + Sync>;
    type MigrationFn = Box<
        dyn FnOnce(Database) -> Pin<Box<dyn Future<Output = Result<(), BoxError>> + Send>> + Send,
    >;

    #[derive(Debug, Serialize, Deserialize)]
    struct MigrationEntry {
        change_id: String,
        author: String,
        #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
        timestamp: DateTime<Utc>,
        state: MigrationState,
        execution_millis: u64,
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(tag = "state")]
    enum MigrationState {
        #[serde(rename = "STARTED")]
        Started,
        #[serde(rename = "EXECUTED")]
        Executed,
        #[serde(rename = "FAILED")]
        Failed { message: String },
    }

    #[derive(Debug)]
    pub enum MigrationError {
        Db(mongodb::error::Error),
        ChangelogHasFailure(String),
        ChangelogHasInProgress(String),
        ConcurrentMigration(String),
    }

    impl std::fmt::Display for MigrationError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Db(e) => write!(f, "Database error: {e}"),
                Self::ChangelogHasFailure(id) => {
                    write!(
                        f,
                        "Migration '{id}' previously failed — resolve it and restart"
                    )
                }
                Self::ChangelogHasInProgress(id) => {
                    write!(
                        f,
                        "Migration '{id}' is marked STARTED — another instance may be running or a previous run was interrupted"
                    )
                }
                Self::ConcurrentMigration(id) => {
                    write!(
                        f,
                        "Migration '{id}' is already being started by another instance"
                    )
                }
            }
        }
    }

    impl std::error::Error for MigrationError {}

    struct MigrationDef {
        change_id: &'static str,
        author: &'static str,
        run: MigrationFn,
    }

    /// Sequential plan of database migrations executed at startup.
    pub struct MigrationPlan {
        migrations: Vec<MigrationDef>,
    }

    impl Default for MigrationPlan {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MigrationPlan {
        pub fn new() -> Self {
            Self {
                migrations: Vec::new(),
            }
        }

        /// Register a migration. Migrations are executed in registration order.
        pub fn register<F, Fut, E>(
            mut self,
            change_id: &'static str,
            author: &'static str,
            run: F,
        ) -> Self
        where
            F: FnOnce(Database) -> Fut + Send + 'static,
            Fut: Future<Output = Result<(), E>> + Send + 'static,
            E: std::error::Error + Send + Sync + 'static,
        {
            self.migrations.push(MigrationDef {
                change_id,
                author,
                run: Box::new(move |db| {
                    Box::pin(async move { run(db).await.map_err(|e| Box::new(e) as BoxError) })
                }),
            });
            self
        }

        /// Execute all pending migrations against the given database.
        pub async fn run(self, db: Database) -> Result<(), MigrationError> {
            ensure_changelog_indexes(&db).await?;

            let entries: Vec<MigrationEntry> = db
                .collection::<MigrationEntry>(CHANGELOG)
                .find(bson::doc! {})
                .await
                .map_err(MigrationError::Db)?
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .map_err(MigrationError::Db)?;

            if let Some(failed) = entries
                .iter()
                .find(|e| matches!(e.state, MigrationState::Failed { .. }))
            {
                return Err(MigrationError::ChangelogHasFailure(
                    failed.change_id.clone(),
                ));
            }

            if let Some(started) = entries
                .iter()
                .find(|e| matches!(e.state, MigrationState::Started))
            {
                return Err(MigrationError::ChangelogHasInProgress(
                    started.change_id.clone(),
                ));
            }

            let performed: HashSet<String> = entries
                .into_iter()
                .filter(|e| matches!(e.state, MigrationState::Executed))
                .map(|e| e.change_id)
                .collect();

            for def in self.migrations {
                if performed.contains(def.change_id) {
                    tracing::debug!("Skipping already-applied migration '{}'", def.change_id);
                    continue;
                }
                run_migration(&db, def).await?;
            }

            Ok(())
        }
    }

    async fn run_migration(db: &Database, def: MigrationDef) -> Result<(), MigrationError> {
        let coll = db.collection::<MigrationEntry>(CHANGELOG);
        let start = Instant::now();

        let mut entry = MigrationEntry {
            change_id: def.change_id.to_string(),
            author: def.author.to_string(),
            timestamp: Utc::now(),
            state: MigrationState::Started,
            execution_millis: 0,
        };
        if let Err(err) = coll.insert_one(&entry).await {
            if is_duplicate_key(&err) {
                return Err(MigrationError::ConcurrentMigration(entry.change_id.clone()));
            }
            return Err(MigrationError::Db(err));
        }

        let result = (def.run)(db.clone()).await;

        entry.state = match result {
            Ok(_) => MigrationState::Executed,
            Err(e) => MigrationState::Failed {
                message: e.to_string(),
            },
        };
        entry.execution_millis = start.elapsed().as_millis() as u64;

        coll.replace_one(bson::doc! { "change_id": &entry.change_id }, &entry)
            .await
            .map_err(MigrationError::Db)?;

        match &entry.state {
            MigrationState::Failed { message } => {
                tracing::error!("Migration '{}' failed: {}", entry.change_id, message);
                Err(MigrationError::ChangelogHasFailure(
                    entry.change_id.to_string(),
                ))
            }
            _ => {
                tracing::info!(
                    "Migration '{}' applied in {}ms",
                    entry.change_id,
                    entry.execution_millis
                );
                Ok(())
            }
        }
    }

    async fn ensure_changelog_indexes(db: &Database) -> Result<(), MigrationError> {
        db.collection::<MigrationEntry>(CHANGELOG)
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "change_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await
            .map_err(MigrationError::Db)?;
        Ok(())
    }

    fn is_duplicate_key(err: &mongodb::error::Error) -> bool {
        is_duplicate_key_message(&err.to_string())
    }

    fn is_duplicate_key_message(message: &str) -> bool {
        message.contains("E11000")
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn duplicate_key_detection_matches_mongo_server_messages() {
            assert!(is_duplicate_key_message(
                "E11000 duplicate key error collection: lekton.__migrations index: change_id_1 dup key"
            ));
        }

        #[test]
        fn duplicate_key_detection_rejects_other_database_errors() {
            assert!(!is_duplicate_key_message("connection reset by peer"));
        }
    }
}

#[cfg(feature = "ssr")]
pub use inner::{MigrationError, MigrationPlan};
