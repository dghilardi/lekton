//! Repository for Learn mode: paths, lessons, and learning records.

use async_trait::async_trait;

use crate::db::learn_models::{LearningPath, LearningRecord, Lesson};
use crate::error::AppError;

#[cfg(feature = "ssr")]
use crate::db::learn_models::LearnPreference;

#[async_trait]
pub trait LearnRepository: Send + Sync {
    /// Create a new learning path.
    async fn create_path(&self, path: LearningPath) -> Result<(), AppError>;

    /// Find a path by ID.
    async fn get_path(&self, id: &str) -> Result<Option<LearningPath>, AppError>;

    /// List all paths for a user, most recent first.
    async fn list_paths_for_user(&self, user_id: &str) -> Result<Vec<LearningPath>, AppError>;

    /// Replace a path's covered-anchor set and bump `updated_at` to now.
    async fn update_path_progress(
        &self,
        id: &str,
        covered_anchors: &[String],
    ) -> Result<(), AppError>;

    /// Delete a path together with its lessons and records.
    async fn delete_path(&self, id: &str) -> Result<(), AppError>;

    /// Append a lesson to a path.
    async fn add_lesson(&self, lesson: Lesson) -> Result<(), AppError>;

    /// Find a lesson by ID.
    async fn get_lesson(&self, id: &str) -> Result<Option<Lesson>, AppError>;

    /// List a path's lessons ordered by `seq` ascending.
    async fn list_lessons_for_path(&self, path_id: &str) -> Result<Vec<Lesson>, AppError>;

    /// Append a learning record.
    async fn add_record(&self, record: LearningRecord) -> Result<(), AppError>;

    /// List a path's records, most recent first.
    async fn list_records_for_path(&self, path_id: &str) -> Result<Vec<LearningRecord>, AppError>;

    /// Privacy: delete all learning data (paths, lessons, records) for a user.
    async fn delete_all_for_user(&self, user_id: &str) -> Result<(), AppError>;

    /// Whether the user opted into persisting learning data. Defaults to `true`
    /// when no preference has been set.
    async fn get_persist(&self, user_id: &str) -> Result<bool, AppError>;

    /// Set the user's persistence preference.
    async fn set_persist(&self, user_id: &str, persist: bool) -> Result<(), AppError>;
}

// ── MongoDB implementation ───────────────────────────────────────────────────

#[cfg(feature = "ssr")]
pub struct MongoLearnRepository {
    paths: mongodb::Collection<LearningPath>,
    lessons: mongodb::Collection<Lesson>,
    records: mongodb::Collection<LearningRecord>,
    preferences: mongodb::Collection<LearnPreference>,
}

#[cfg(feature = "ssr")]
impl MongoLearnRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            paths: db.collection("learn_paths"),
            lessons: db.collection("learn_lessons"),
            records: db.collection("learn_records"),
            preferences: db.collection("learn_preferences"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl LearnRepository for MongoLearnRepository {
    async fn create_path(&self, path: LearningPath) -> Result<(), AppError> {
        self.paths
            .insert_one(path)
            .await
            .map_err(|e| AppError::Internal(format!("mongo insert learn_path: {e}")))?;
        Ok(())
    }

    async fn get_path(&self, id: &str) -> Result<Option<LearningPath>, AppError> {
        self.paths
            .find_one(mongodb::bson::doc! { "id": id })
            .await
            .map_err(|e| AppError::Internal(format!("mongo find learn_path: {e}")))
    }

    async fn list_paths_for_user(&self, user_id: &str) -> Result<Vec<LearningPath>, AppError> {
        use futures::TryStreamExt;

        let cursor = self
            .paths
            .find(mongodb::bson::doc! { "user_id": user_id })
            .sort(mongodb::bson::doc! { "updated_at": -1 })
            .await
            .map_err(|e| AppError::Internal(format!("mongo list learn_paths: {e}")))?;

        cursor
            .try_collect()
            .await
            .map_err(|e| AppError::Internal(format!("mongo collect learn_paths: {e}")))
    }

    async fn update_path_progress(
        &self,
        id: &str,
        covered_anchors: &[String],
    ) -> Result<(), AppError> {
        let now = mongodb::bson::DateTime::from_chrono(chrono::Utc::now());
        let anchors = mongodb::bson::to_bson(covered_anchors)
            .map_err(|e| AppError::Internal(format!("bson encode covered_anchors: {e}")))?;
        self.paths
            .update_one(
                mongodb::bson::doc! { "id": id },
                mongodb::bson::doc! { "$set": { "covered_anchors": anchors, "updated_at": now } },
            )
            .await
            .map_err(|e| AppError::Internal(format!("mongo update learn_path progress: {e}")))?;
        Ok(())
    }

    async fn delete_path(&self, id: &str) -> Result<(), AppError> {
        // Delete dependent records and lessons first, then the path itself.
        self.records
            .delete_many(mongodb::bson::doc! { "path_id": id })
            .await
            .map_err(|e| AppError::Internal(format!("mongo delete learn_records: {e}")))?;
        self.lessons
            .delete_many(mongodb::bson::doc! { "path_id": id })
            .await
            .map_err(|e| AppError::Internal(format!("mongo delete learn_lessons: {e}")))?;
        self.paths
            .delete_one(mongodb::bson::doc! { "id": id })
            .await
            .map_err(|e| AppError::Internal(format!("mongo delete learn_path: {e}")))?;
        Ok(())
    }

    async fn add_lesson(&self, lesson: Lesson) -> Result<(), AppError> {
        self.lessons
            .insert_one(lesson)
            .await
            .map_err(|e| AppError::Internal(format!("mongo insert learn_lesson: {e}")))?;
        Ok(())
    }

    async fn get_lesson(&self, id: &str) -> Result<Option<Lesson>, AppError> {
        self.lessons
            .find_one(mongodb::bson::doc! { "id": id })
            .await
            .map_err(|e| AppError::Internal(format!("mongo find learn_lesson: {e}")))
    }

    async fn list_lessons_for_path(&self, path_id: &str) -> Result<Vec<Lesson>, AppError> {
        use futures::TryStreamExt;

        let cursor = self
            .lessons
            .find(mongodb::bson::doc! { "path_id": path_id })
            .sort(mongodb::bson::doc! { "seq": 1 })
            .await
            .map_err(|e| AppError::Internal(format!("mongo list learn_lessons: {e}")))?;

        cursor
            .try_collect()
            .await
            .map_err(|e| AppError::Internal(format!("mongo collect learn_lessons: {e}")))
    }

    async fn add_record(&self, record: LearningRecord) -> Result<(), AppError> {
        self.records
            .insert_one(record)
            .await
            .map_err(|e| AppError::Internal(format!("mongo insert learn_record: {e}")))?;
        Ok(())
    }

    async fn list_records_for_path(&self, path_id: &str) -> Result<Vec<LearningRecord>, AppError> {
        use futures::TryStreamExt;

        let cursor = self
            .records
            .find(mongodb::bson::doc! { "path_id": path_id })
            .sort(mongodb::bson::doc! { "created_at": -1 })
            .await
            .map_err(|e| AppError::Internal(format!("mongo list learn_records: {e}")))?;

        cursor
            .try_collect()
            .await
            .map_err(|e| AppError::Internal(format!("mongo collect learn_records: {e}")))
    }

    async fn delete_all_for_user(&self, user_id: &str) -> Result<(), AppError> {
        let filter = mongodb::bson::doc! { "user_id": user_id };
        self.records
            .delete_many(filter.clone())
            .await
            .map_err(|e| AppError::Internal(format!("mongo delete learn_records for user: {e}")))?;
        self.lessons
            .delete_many(filter.clone())
            .await
            .map_err(|e| AppError::Internal(format!("mongo delete learn_lessons for user: {e}")))?;
        self.paths
            .delete_many(filter)
            .await
            .map_err(|e| AppError::Internal(format!("mongo delete learn_paths for user: {e}")))?;
        Ok(())
    }

    async fn get_persist(&self, user_id: &str) -> Result<bool, AppError> {
        let pref = self
            .preferences
            .find_one(mongodb::bson::doc! { "user_id": user_id })
            .await
            .map_err(|e| AppError::Internal(format!("mongo find learn_preference: {e}")))?;
        Ok(pref.map(|p| p.persist).unwrap_or(true))
    }

    async fn set_persist(&self, user_id: &str, persist: bool) -> Result<(), AppError> {
        self.preferences
            .update_one(
                mongodb::bson::doc! { "user_id": user_id },
                mongodb::bson::doc! { "$set": { "persist": persist } },
            )
            .upsert(true)
            .await
            .map_err(|e| AppError::Internal(format!("mongo upsert learn_preference: {e}")))?;
        Ok(())
    }
}
