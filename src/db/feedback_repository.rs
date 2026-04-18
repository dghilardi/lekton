//! Repository for AI message feedback.

use async_trait::async_trait;

use crate::db::chat_models::MessageFeedback;
use crate::error::AppError;

/// Pagination and filter params for listing feedback.
#[derive(Debug, Clone, Default)]
pub struct FeedbackListParams {
    /// Only return feedback with this rating ("positive" or "negative").
    pub rating: Option<String>,
    /// Only return feedback created at or after this timestamp (RFC 3339).
    pub date_from: Option<chrono::DateTime<chrono::Utc>>,
    /// Only return feedback created before this timestamp (RFC 3339).
    pub date_to: Option<chrono::DateTime<chrono::Utc>>,
    /// Filter by user_id (admin only).
    pub user_id: Option<String>,
    /// Zero-based page index.
    pub page: u64,
    /// Items per page (capped server-side).
    pub per_page: u64,
}

/// A single page of feedback items with total count.
#[derive(Debug, Clone)]
pub struct FeedbackPage {
    pub items: Vec<MessageFeedback>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

#[async_trait]
pub trait FeedbackRepository: Send + Sync {
    /// Insert or replace the feedback a user gave to a specific message.
    /// Keyed on (message_id, user_id) — only one feedback per user per message.
    async fn upsert_feedback(&self, feedback: MessageFeedback) -> Result<(), AppError>;

    /// Retrieve the feedback a user left on a specific message, if any.
    async fn get_feedback(
        &self,
        message_id: &str,
        user_id: &str,
    ) -> Result<Option<MessageFeedback>, AppError>;

    /// Get all feedback left by a user for an entire session (used to hydrate
    /// the chat UI when loading session history).
    async fn get_session_feedback(
        &self,
        session_id: &str,
        user_id: &str,
    ) -> Result<Vec<MessageFeedback>, AppError>;

    /// Delete the feedback a user left on a specific message.
    async fn delete_feedback(&self, message_id: &str, user_id: &str) -> Result<(), AppError>;

    /// Paginated list of feedback for a single user (profile history page).
    async fn list_user_feedback(
        &self,
        user_id: &str,
        params: FeedbackListParams,
    ) -> Result<FeedbackPage, AppError>;

    /// Paginated list of all feedback with optional filters (admin export).
    async fn list_all_feedback(&self, params: FeedbackListParams)
        -> Result<FeedbackPage, AppError>;

    /// Delete all feedback belonging to a session (called when session is deleted).
    async fn delete_session_feedback(&self, session_id: &str) -> Result<(), AppError>;
}

// ── MongoDB implementation ───────────────────────────────────────────────────

#[cfg(feature = "ssr")]
pub struct MongoFeedbackRepository {
    col: mongodb::Collection<MessageFeedback>,
}

#[cfg(feature = "ssr")]
impl MongoFeedbackRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            col: db.collection("message_feedback"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl FeedbackRepository for MongoFeedbackRepository {
    async fn upsert_feedback(&self, feedback: MessageFeedback) -> Result<(), AppError> {
        use mongodb::options::ReplaceOptions;

        let filter = mongodb::bson::doc! {
            "message_id": &feedback.message_id,
            "user_id":    &feedback.user_id,
        };
        let opts = ReplaceOptions::builder().upsert(true).build();
        self.col
            .replace_one(filter, feedback)
            .with_options(opts)
            .await
            .map_err(|e| AppError::Internal(format!("mongo upsert feedback: {e}")))?;
        Ok(())
    }

    async fn get_feedback(
        &self,
        message_id: &str,
        user_id: &str,
    ) -> Result<Option<MessageFeedback>, AppError> {
        self.col
            .find_one(mongodb::bson::doc! {
                "message_id": message_id,
                "user_id":    user_id,
            })
            .await
            .map_err(|e| AppError::Internal(format!("mongo get feedback: {e}")))
    }

    async fn get_session_feedback(
        &self,
        session_id: &str,
        user_id: &str,
    ) -> Result<Vec<MessageFeedback>, AppError> {
        use futures::TryStreamExt;

        let cursor = self
            .col
            .find(mongodb::bson::doc! {
                "session_id": session_id,
                "user_id":    user_id,
            })
            .await
            .map_err(|e| AppError::Internal(format!("mongo find session feedback: {e}")))?;

        cursor
            .try_collect()
            .await
            .map_err(|e| AppError::Internal(format!("mongo collect session feedback: {e}")))
    }

    async fn delete_feedback(&self, message_id: &str, user_id: &str) -> Result<(), AppError> {
        self.col
            .delete_one(mongodb::bson::doc! {
                "message_id": message_id,
                "user_id":    user_id,
            })
            .await
            .map_err(|e| AppError::Internal(format!("mongo delete feedback: {e}")))?;
        Ok(())
    }

    async fn list_user_feedback(
        &self,
        user_id: &str,
        params: FeedbackListParams,
    ) -> Result<FeedbackPage, AppError> {
        let mut filter = mongodb::bson::doc! { "user_id": user_id };
        apply_filters(&mut filter, &params);
        paginate(&self.col, filter, params).await
    }

    async fn list_all_feedback(
        &self,
        params: FeedbackListParams,
    ) -> Result<FeedbackPage, AppError> {
        let mut filter = mongodb::bson::Document::new();
        if let Some(uid) = &params.user_id {
            filter.insert("user_id", uid.clone());
        }
        apply_filters(&mut filter, &params);
        paginate(&self.col, filter, params).await
    }

    async fn delete_session_feedback(&self, session_id: &str) -> Result<(), AppError> {
        self.col
            .delete_many(mongodb::bson::doc! { "session_id": session_id })
            .await
            .map_err(|e| AppError::Internal(format!("mongo delete session feedback: {e}")))?;
        Ok(())
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

#[cfg(feature = "ssr")]
fn apply_filters(filter: &mut mongodb::bson::Document, params: &FeedbackListParams) {
    if let Some(rating) = &params.rating {
        filter.insert("rating", rating.clone());
    }
    let mut date_range = mongodb::bson::Document::new();
    if let Some(from) = params.date_from {
        date_range.insert("$gte", mongodb::bson::DateTime::from_chrono(from));
    }
    if let Some(to) = params.date_to {
        date_range.insert("$lt", mongodb::bson::DateTime::from_chrono(to));
    }
    if !date_range.is_empty() {
        filter.insert("created_at", date_range);
    }
}

#[cfg(feature = "ssr")]
async fn paginate(
    col: &mongodb::Collection<MessageFeedback>,
    filter: mongodb::bson::Document,
    params: FeedbackListParams,
) -> Result<FeedbackPage, AppError> {
    use futures::TryStreamExt;

    const MAX_PER_PAGE: u64 = 200;
    let per_page = params.per_page.min(MAX_PER_PAGE).max(1);
    let skip = params.page * per_page;

    let total = col
        .count_documents(filter.clone())
        .await
        .map_err(|e| AppError::Internal(format!("mongo count feedback: {e}")))?;

    let cursor = col
        .find(filter)
        .sort(mongodb::bson::doc! { "created_at": -1 })
        .skip(skip)
        .limit(per_page as i64)
        .await
        .map_err(|e| AppError::Internal(format!("mongo find feedback: {e}")))?;

    let items: Vec<MessageFeedback> = cursor
        .try_collect()
        .await
        .map_err(|e| AppError::Internal(format!("mongo collect feedback: {e}")))?;

    Ok(FeedbackPage {
        items,
        total,
        page: params.page,
        per_page,
    })
}
