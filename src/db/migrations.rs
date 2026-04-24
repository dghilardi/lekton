//! Registered database migrations for Lekton.
//!
//! Add new migrations at the end. Never remove or reorder existing entries.

#[cfg(feature = "ssr")]
mod inner {
    use crate::db::migration::MigrationPlan;
    use mongodb::Database;

    pub fn build_plan() -> MigrationPlan {
        MigrationPlan::new().register(
            "001_add_created_at_to_access_levels",
            "ghilardi.davide@gmail.com",
            add_created_at_to_access_levels,
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
}

#[cfg(feature = "ssr")]
pub use inner::build_plan;
