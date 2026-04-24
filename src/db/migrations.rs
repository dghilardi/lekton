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
}

#[cfg(feature = "ssr")]
pub use inner::build_plan;
