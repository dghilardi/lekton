//! Registered database migrations for Lekton.
//!
//! Add new migrations at the end. Never remove or reorder existing entries.

#[cfg(feature = "ssr")]
mod inner {
    use crate::db::migration::MigrationPlan;
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
}

#[cfg(feature = "ssr")]
pub use inner::build_plan;
