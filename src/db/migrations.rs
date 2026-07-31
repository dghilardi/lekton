//! Registered database migrations for Lekton.
//!
//! Add new migrations at the end. Never remove or reorder existing entries.

#[cfg(feature = "ssr")]
mod inner {
    use crate::db::migration::MigrationPlan;
    use futures::TryStreamExt;
    use mongodb::Database;

    pub fn build_plan() -> MigrationPlan {
        MigrationPlan::new()
            .register(
                "001_add_created_at_to_access_levels",
                "ghilardi.davide@gmail.com",
                add_created_at_to_access_levels,
            )
            .register(
                "002_add_created_at_to_users",
                "ghilardi.davide@gmail.com",
                add_created_at_to_users,
            )
            .register(
                "003_convert_string_dates_access_levels",
                "ghilardi.davide@gmail.com",
                convert_string_dates_access_levels,
            )
            .register(
                "004_convert_string_dates_assets",
                "ghilardi.davide@gmail.com",
                convert_string_dates_assets,
            )
            .register(
                "005_add_schemas_name_index",
                "davide.ghilardi@comelit.it",
                add_schemas_name_index,
            )
            .register(
                "006_add_users_indexes",
                "davide.ghilardi@comelit.it",
                add_users_indexes,
            )
            .register(
                "007_add_refresh_tokens_hash_index",
                "davide.ghilardi@comelit.it",
                add_refresh_tokens_hash_index,
            )
            .register(
                "008_add_documents_indexes",
                "davide.ghilardi@comelit.it",
                add_documents_indexes,
            )
            .register(
                "009_add_documentation_feedback_indexes",
                "davide.ghilardi@comelit.it",
                add_documentation_feedback_indexes,
            )
            .register(
                "010_add_embedding_cache_index",
                "davide.ghilardi@comelit.it",
                add_embedding_cache_index,
            )
            .register(
                "011_add_remaining_collection_indexes",
                "davide.ghilardi@comelit.it",
                add_remaining_collection_indexes,
            )
            .register(
                "012_add_refresh_tokens_ttl_and_family",
                "davide.ghilardi@comelit.it",
                add_refresh_tokens_ttl_and_family,
            )
            .register(
                "013_add_document_sources_index",
                "davide.ghilardi@comelit.it",
                add_document_sources_index,
            )
            .register(
                "014_add_document_release_fields",
                "davide.ghilardi@comelit.it",
                add_document_release_fields,
            )
            .register(
                "015_rename_document_versions_to_revisions",
                "davide.ghilardi@comelit.it",
                rename_document_versions_to_revisions,
            )
    }

    fn format_duplicate_group_id(id: &bson::Bson) -> String {
        match id {
            bson::Bson::Document(doc) => serde_json::to_string(doc)
                .unwrap_or_else(|_| format!("{doc:?}"))
                .replace("\\\"", "\""),
            other => other.to_string(),
        }
    }

    async fn fail_on_duplicate_keys(
        collection: &mongodb::Collection<bson::Document>,
        change_id: &str,
        label: &str,
        group_id: bson::Bson,
    ) -> Result<(), mongodb::error::Error> {
        let pipeline = vec![
            bson::doc! { "$group": { "_id": group_id, "count": { "$sum": 1 } } },
            bson::doc! { "$match": { "count": { "$gt": 1 } } },
            bson::doc! { "$sort": { "_id": 1 } },
        ];

        let mut cursor = collection.aggregate(pipeline).await?;
        let mut duplicates = Vec::new();
        while let Some(doc) = cursor.try_next().await? {
            duplicates.push(format_duplicate_group_id(
                doc.get("_id").unwrap_or(&bson::Bson::Null),
            ));
        }

        if duplicates.is_empty() {
            return Ok(());
        }

        Err(mongodb::error::Error::custom(format!(
            "Migration {change_id} pre-flight failed: {} duplicate key(s) found in the \
             '{label}' collection. Resolve them before restarting, then retry.\n\
             Duplicate keys: {}",
            duplicates.len(),
            duplicates.join(", ")
        )))
    }

    /// Backfills `created_at` on AccessLevelEntity documents created before the
    /// field was introduced. Uses `$$NOW` so all backfilled entries share a
    /// consistent timestamp (the migration run time).
    async fn add_created_at_to_access_levels(db: Database) -> Result<(), mongodb::error::Error> {
        db.collection::<bson::Document>("access_levels")
            .update_many(
                bson::doc! { "created_at": { "$exists": false } },
                vec![bson::doc! { "$set": { "created_at": "$$NOW" } }],
            )
            .await?;
        Ok(())
    }

    /// Backfills `created_at` on User documents created before the field was
    /// introduced.
    async fn add_created_at_to_users(db: Database) -> Result<(), mongodb::error::Error> {
        db.collection::<bson::Document>("users")
            .update_many(
                bson::doc! { "created_at": { "$exists": false } },
                vec![bson::doc! { "$set": { "created_at": "$$NOW" } }],
            )
            .await?;
        Ok(())
    }

    /// Converts `created_at` from ISO 8601 string to BSON Date in access_levels.
    /// Old documents were written with the default chrono serializer (string);
    /// the model now uses `chrono_datetime_as_bson_datetime` which expects a Date type.
    async fn convert_string_dates_access_levels(db: Database) -> Result<(), mongodb::error::Error> {
        db.collection::<bson::Document>("access_levels")
            .update_many(
                bson::doc! { "created_at": { "$type": "string" } },
                vec![bson::doc! { "$set": { "created_at": { "$toDate": "$created_at" } } }],
            )
            .await?;
        Ok(())
    }

    /// Converts `uploaded_at` from ISO 8601 string to BSON Date in assets.
    async fn convert_string_dates_assets(db: Database) -> Result<(), mongodb::error::Error> {
        db.collection::<bson::Document>("assets")
            .update_many(
                bson::doc! { "uploaded_at": { "$type": "string" } },
                vec![bson::doc! { "$set": { "uploaded_at": { "$toDate": "$uploaded_at" } } }],
            )
            .await?;
        Ok(())
    }

    /// Creates a unique index on `schemas.name` to speed up lookups by schema name.
    async fn add_schemas_name_index(db: Database) -> Result<(), mongodb::error::Error> {
        use mongodb::options::IndexOptions;
        use mongodb::IndexModel;

        db.collection::<bson::Document>("schemas")
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "name": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;
        Ok(())
    }

    /// Creates indexes on the `users` collection to speed up auth and access-level lookups.
    async fn add_users_indexes(db: Database) -> Result<(), mongodb::error::Error> {
        use mongodb::options::IndexOptions;
        use mongodb::IndexModel;

        let col = db.collection::<bson::Document>("users");

        col.create_index(
            IndexModel::builder()
                .keys(bson::doc! { "id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;

        col.create_index(
            IndexModel::builder()
                .keys(bson::doc! { "email": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;

        col.create_index(
            IndexModel::builder()
                .keys(bson::doc! { "provider_sub": 1, "provider_type": 1 })
                .build(),
        )
        .await?;

        Ok(())
    }

    /// Creates an index on `refresh_tokens.token_hash` used on every authenticated request.
    async fn add_refresh_tokens_hash_index(db: Database) -> Result<(), mongodb::error::Error> {
        use mongodb::options::IndexOptions;
        use mongodb::IndexModel;

        db.collection::<bson::Document>("refresh_tokens")
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "token_hash": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;
        Ok(())
    }

    /// Creates indexes on the `documents` collection:
    /// - unique on `slug` (primary lookup key; prevents concurrent-ingest duplicates)
    /// - non-unique on `source_path` and `source_id` (sync rename detection)
    ///
    /// Runs a pre-flight check before creating the unique `slug` index: if any
    /// duplicate slugs exist (possible due to the concurrent-ingest race described in
    /// BUG-3), the migration fails with a clear message listing the offending slugs
    /// instead of an opaque MongoDB E11000 error.
    async fn add_documents_indexes(db: Database) -> Result<(), mongodb::error::Error> {
        use mongodb::options::IndexOptions;
        use mongodb::IndexModel;

        let col = db.collection::<bson::Document>("documents");

        // Pre-flight: detect duplicate slugs before attempting to build the unique index.
        let pipeline = vec![
            bson::doc! { "$group": { "_id": "$slug", "count": { "$sum": 1 } } },
            bson::doc! { "$match": { "count": { "$gt": 1 } } },
            bson::doc! { "$sort": { "_id": 1 } },
        ];
        let mut cursor = col.aggregate(pipeline).await?;
        let mut duplicates: Vec<String> = Vec::new();
        while let Some(doc) = cursor.try_next().await? {
            if let Ok(slug) = doc.get_str("_id") {
                duplicates.push(slug.to_string());
            }
        }
        if !duplicates.is_empty() {
            return Err(mongodb::error::Error::custom(format!(
                "Migration 008 pre-flight failed: {} duplicate slug(s) found in the \
                 'documents' collection. Resolve them before restarting, then retry.\n\
                 Duplicate slugs: {}",
                duplicates.len(),
                duplicates.join(", ")
            )));
        }

        col.create_index(
            IndexModel::builder()
                .keys(bson::doc! { "slug": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;

        col.create_index(
            IndexModel::builder()
                .keys(bson::doc! { "source_path": 1 })
                .build(),
        )
        .await?;

        col.create_index(
            IndexModel::builder()
                .keys(bson::doc! { "source_id": 1 })
                .build(),
        )
        .await?;

        Ok(())
    }

    /// Creates indexes on `documentation_feedback` (previously in `ensure_indexes()`).
    async fn add_documentation_feedback_indexes(db: Database) -> Result<(), mongodb::error::Error> {
        use mongodb::options::IndexOptions;
        use mongodb::IndexModel;

        let col = db.collection::<bson::Document>("documentation_feedback");

        col.create_index(
            IndexModel::builder()
                .keys(bson::doc! { "id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;

        col.create_index(
            IndexModel::builder()
                .keys(bson::doc! { "status": 1, "kind": 1, "created_at": -1 })
                .build(),
        )
        .await?;

        Ok(())
    }

    /// Creates the unique compound index on `(hash, model)` for `embedding_cache`
    /// (previously in `ensure_index()`).
    async fn add_embedding_cache_index(db: Database) -> Result<(), mongodb::error::Error> {
        use mongodb::options::IndexOptions;
        use mongodb::IndexModel;

        db.collection::<bson::Document>("embedding_cache")
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "hash": 1, "model": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;
        Ok(())
    }

    /// Creates missing indexes for repositories that still rely on collection scans
    /// or race-prone logical keys.
    async fn add_remaining_collection_indexes(db: Database) -> Result<(), mongodb::error::Error> {
        use mongodb::options::IndexOptions;
        use mongodb::IndexModel;

        let service_tokens = db.collection::<bson::Document>("service_tokens");
        fail_on_duplicate_keys(
            &service_tokens,
            "011",
            "service_tokens",
            bson::Bson::String("$id".to_string()),
        )
        .await?;
        fail_on_duplicate_keys(
            &service_tokens,
            "011",
            "service_tokens",
            bson::Bson::String("$name".to_string()),
        )
        .await?;
        fail_on_duplicate_keys(
            &service_tokens,
            "011",
            "service_tokens",
            bson::Bson::String("$token_hash".to_string()),
        )
        .await?;
        service_tokens
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;
        service_tokens
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "name": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;
        service_tokens
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "token_hash": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;
        service_tokens
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "user_id": 1, "token_type": 1, "created_at": -1 })
                    .build(),
            )
            .await?;
        service_tokens
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "token_type": 1, "created_at": -1 })
                    .build(),
            )
            .await?;

        let prompts = db.collection::<bson::Document>("prompts");
        fail_on_duplicate_keys(
            &prompts,
            "011",
            "prompts",
            bson::Bson::String("$slug".to_string()),
        )
        .await?;
        prompts
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "slug": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;
        prompts
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! {
                        "access_level": 1,
                        "status": 1,
                        "is_archived": 1,
                        "name": 1,
                        "slug": 1
                    })
                    .build(),
            )
            .await?;

        let document_versions = db.collection::<bson::Document>("document_versions");
        fail_on_duplicate_keys(
            &document_versions,
            "011",
            "document_versions",
            bson::Bson::Document(bson::doc! { "slug": "$slug", "version": "$version" }),
        )
        .await?;
        document_versions
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "slug": 1, "version": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;
        document_versions
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "slug": 1, "version": -1 })
                    .build(),
            )
            .await?;

        let prompt_versions = db.collection::<bson::Document>("prompt_versions");
        fail_on_duplicate_keys(
            &prompt_versions,
            "011",
            "prompt_versions",
            bson::Bson::Document(bson::doc! { "slug": "$slug", "version": "$version" }),
        )
        .await?;
        prompt_versions
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "slug": 1, "version": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;
        prompt_versions
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "slug": 1, "version": -1 })
                    .build(),
            )
            .await?;

        let message_feedback = db.collection::<bson::Document>("message_feedback");
        fail_on_duplicate_keys(
            &message_feedback,
            "011",
            "message_feedback",
            bson::Bson::Document(bson::doc! { "message_id": "$message_id", "user_id": "$user_id" }),
        )
        .await?;
        message_feedback
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "message_id": 1, "user_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;
        message_feedback
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "session_id": 1, "user_id": 1 })
                    .build(),
            )
            .await?;
        message_feedback
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "user_id": 1, "created_at": -1 })
                    .build(),
            )
            .await?;
        message_feedback
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "rating": 1, "created_at": -1 })
                    .build(),
            )
            .await?;

        let access_levels = db.collection::<bson::Document>("access_levels");
        fail_on_duplicate_keys(
            &access_levels,
            "011",
            "access_levels",
            bson::Bson::String("$name".to_string()),
        )
        .await?;
        access_levels
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "name": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;
        access_levels
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "is_system": -1, "name": 1 })
                    .build(),
            )
            .await?;

        let chat_sessions = db.collection::<bson::Document>("chat_sessions");
        fail_on_duplicate_keys(
            &chat_sessions,
            "011",
            "chat_sessions",
            bson::Bson::String("$id".to_string()),
        )
        .await?;
        chat_sessions
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;
        chat_sessions
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "user_id": 1, "updated_at": -1 })
                    .build(),
            )
            .await?;

        let chat_messages = db.collection::<bson::Document>("chat_messages");
        fail_on_duplicate_keys(
            &chat_messages,
            "011",
            "chat_messages",
            bson::Bson::String("$id".to_string()),
        )
        .await?;
        chat_messages
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;
        chat_messages
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "session_id": 1, "created_at": -1 })
                    .build(),
            )
            .await?;

        let user_prompt_preferences = db.collection::<bson::Document>("user_prompt_preferences");
        fail_on_duplicate_keys(
            &user_prompt_preferences,
            "011",
            "user_prompt_preferences",
            bson::Bson::Document(
                bson::doc! { "user_id": "$user_id", "prompt_slug": "$prompt_slug" },
            ),
        )
        .await?;
        user_prompt_preferences
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "user_id": 1, "prompt_slug": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;

        let settings = db.collection::<bson::Document>("settings");
        fail_on_duplicate_keys(
            &settings,
            "011",
            "settings",
            bson::Bson::String("$key".to_string()),
        )
        .await?;
        settings
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "key": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;

        let navigation_order = db.collection::<bson::Document>("navigation_order");
        fail_on_duplicate_keys(
            &navigation_order,
            "011",
            "navigation_order",
            bson::Bson::String("$slug".to_string()),
        )
        .await?;
        navigation_order
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "slug": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;
        navigation_order
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "weight": 1 })
                    .build(),
            )
            .await?;

        Ok(())
    }

    /// Refresh-token lifecycle hardening:
    /// - a TTL index on `expires_at` so expired/revoked tokens are pruned by
    ///   MongoDB instead of accumulating forever;
    /// - backfills `family_id` (added for rotation reuse-detection) to each
    ///   legacy token's own `id`, so every pre-existing token is its own family.
    async fn add_refresh_tokens_ttl_and_family(db: Database) -> Result<(), mongodb::error::Error> {
        use mongodb::options::IndexOptions;
        use mongodb::IndexModel;
        use std::time::Duration;

        let refresh_tokens = db.collection::<bson::Document>("refresh_tokens");

        // expireAfterSeconds: 0 → delete each document once `expires_at` passes.
        refresh_tokens
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "expires_at": 1 })
                    .options(
                        IndexOptions::builder()
                            .expire_after(Duration::from_secs(0))
                            .build(),
                    )
                    .build(),
            )
            .await?;

        // Backfill family_id = id for records missing it or with an empty value.
        refresh_tokens
            .update_many(
                bson::doc! {
                    "$or": [
                        { "family_id": { "$exists": false } },
                        { "family_id": "" },
                    ]
                },
                vec![bson::doc! { "$set": { "family_id": "$id" } }],
            )
            .await?;

        Ok(())
    }

    /// Creates the unique index on `document_sources.id`, the source-registry
    /// metadata keyed by a document's `source_id`.
    async fn add_document_sources_index(db: Database) -> Result<(), mongodb::error::Error> {
        use mongodb::options::IndexOptions;
        use mongodb::IndexModel;

        db.collection::<bson::Document>("document_sources")
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;
        Ok(())
    }

    /// Introduces per-source release versioning on `documents`.
    ///
    /// Backfills `is_latest: true` on every existing document (they keep
    /// `release` absent, i.e. "not release-managed"), then replaces the unique
    /// `slug` index created by migration 008 with a unique `(slug, release)`
    /// index — `slug` alone can no longer be unique once one slug exists in
    /// several releases.
    ///
    /// The invariant that a slug is owned by exactly one source, previously
    /// implied by the unique `slug` index, is enforced from here on by a
    /// pre-upload check in the sync API.
    ///
    /// Runs the same duplicate pre-flight as 008 so a pre-existing duplicate
    /// surfaces as an actionable message rather than an opaque E11000 from the
    /// index build.
    async fn add_document_release_fields(db: Database) -> Result<(), mongodb::error::Error> {
        use mongodb::options::IndexOptions;
        use mongodb::IndexModel;

        let col = db.collection::<bson::Document>("documents");

        // 1. Backfill: every existing document belongs to its source's `latest`.
        col.update_many(
            bson::doc! { "is_latest": { "$exists": false } },
            bson::doc! { "$set": { "is_latest": true } },
        )
        .await?;

        // 2. Pre-flight on the compound key before building the unique index.
        let pipeline = vec![
            bson::doc! { "$group": {
                "_id": { "slug": "$slug", "release": "$release" },
                "count": { "$sum": 1 },
            }},
            bson::doc! { "$match": { "count": { "$gt": 1 } } },
            bson::doc! { "$sort": { "_id": 1 } },
        ];
        let mut cursor = col.aggregate(pipeline).await?;
        let mut duplicates: Vec<String> = Vec::new();
        while let Some(doc) = cursor.try_next().await? {
            if let Some(id) = doc.get("_id") {
                duplicates.push(format_duplicate_group_id(id));
            }
        }
        if !duplicates.is_empty() {
            return Err(mongodb::error::Error::custom(format!(
                "Migration 014 pre-flight failed: {} duplicate (slug, release) pair(s) found \
                 in the 'documents' collection. Resolve them before restarting, then retry.\n\
                 Duplicates: {}",
                duplicates.len(),
                duplicates.join(", ")
            )));
        }

        // 3. Drop the now-invalid unique index on `slug` alone. Checked rather
        //    than assumed so a retry after a partial run is a no-op instead of
        //    an IndexNotFound failure.
        let existing = col.list_index_names().await?;
        if existing.iter().any(|name| name == "slug_1") {
            col.drop_index("slug_1").await?;
        }

        // 4. Unique per release: one document per slug within a given release.
        col.create_index(
            IndexModel::builder()
                .keys(bson::doc! { "slug": 1, "release": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;

        // 5. Sync path: scope a source's documents to one release.
        col.create_index(
            IndexModel::builder()
                .keys(bson::doc! { "source_id": 1, "release": 1 })
                .build(),
        )
        .await?;

        // 6. Default resolution path (everything at `latest`).
        col.create_index(
            IndexModel::builder()
                .keys(bson::doc! { "is_latest": 1, "source_id": 1 })
                .build(),
        )
        .await?;

        // 7. Release catalogue and the movable `latest` alias.
        db.collection::<bson::Document>("source_releases")
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "source_id": 1, "release": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;

        db.collection::<bson::Document>("source_release_aliases")
            .create_index(
                IndexModel::builder()
                    .keys(bson::doc! { "source_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;

        Ok(())
    }

    /// Renames the editorial history to `document_revisions`, with `version`
    /// becoming `revision`.
    ///
    /// The two concepts were both called "version": the per-slug counter bumped
    /// on every content change, and the product release a document belongs to.
    /// Leaving the collection named `document_versions` next to a `release` field
    /// keeps that ambiguity alive for anyone reading the database directly.
    ///
    /// Idempotent and safe on a database that never wrote history: the rename is
    /// skipped when the old collection is absent, and the field rename touches
    /// only documents that still carry `version`.
    async fn rename_document_versions_to_revisions(
        db: Database,
    ) -> Result<(), mongodb::error::Error> {
        use mongodb::options::IndexOptions;
        use mongodb::IndexModel;

        let names = db.list_collection_names().await?;
        let old_exists = names.iter().any(|n| n == "document_versions");
        let new_exists = names.iter().any(|n| n == "document_revisions");

        if old_exists && !new_exists {
            // `renameCollection` runs against the admin database.
            let admin = db.client().database("admin");
            let db_name = db.name();
            admin
                .run_command(bson::doc! {
                    "renameCollection": format!("{db_name}.document_versions"),
                    "to": format!("{db_name}.document_revisions"),
                })
                .await?;
        }

        let col = db.collection::<bson::Document>("document_revisions");

        // Drop the indexes migration 011 built on `version` *before* renaming the
        // field. They travelled with the collection rename, and `$rename` is
        // applied document by document: the moment two revisions of one slug have
        // lost `version`, both index as `{slug, version: null}` and the unique
        // index rejects the write, aborting the rename half-way.
        for stale in ["slug_1_version_1", "slug_1_version_-1"] {
            if col.list_index_names().await?.iter().any(|n| n == stale) {
                col.drop_index(stale).await?;
            }
        }

        // Only rows that still carry the old field, so a retry after a partial
        // run finishes the job instead of failing on the ones already converted.
        col.update_many(
            bson::doc! { "version": { "$exists": true } },
            bson::doc! { "$rename": { "version": "revision" } },
        )
        .await?;

        col.create_index(
            IndexModel::builder()
                .keys(bson::doc! { "slug": 1, "revision": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
        col.create_index(
            IndexModel::builder()
                .keys(bson::doc! { "slug": 1, "revision": -1 })
                .build(),
        )
        .await?;

        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn format_duplicate_group_id_renders_compound_keys_readably() {
            let rendered = format_duplicate_group_id(&bson::Bson::Document(
                bson::doc! { "slug": "docs/a", "version": 3 },
            ));

            assert!(rendered.contains("\"slug\":\"docs/a\""));
            assert!(rendered.contains("\"version\":3"));
        }
    }
}

#[cfg(feature = "ssr")]
pub use inner::build_plan;
