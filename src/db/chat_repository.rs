//! Repository for chat sessions and messages (RAG conversations).

use async_trait::async_trait;

use crate::db::chat_models::{ChatMessage, ChatSession};
use crate::error::AppError;

#[async_trait]
pub trait ChatRepository: Send + Sync {
    /// Create a new chat session.
    async fn create_session(&self, session: ChatSession) -> Result<(), AppError>;

    /// Find a session by ID.
    async fn get_session(&self, id: &str) -> Result<Option<ChatSession>, AppError>;

    /// List all sessions for a user, most recent first.
    async fn list_sessions_for_user(&self, user_id: &str) -> Result<Vec<ChatSession>, AppError>;

    /// Update the session title.
    async fn update_session_title(&self, id: &str, title: &str) -> Result<(), AppError>;

    /// Bump the `updated_at` timestamp to now.
    async fn touch_session(&self, id: &str) -> Result<(), AppError>;

    /// Append a message to a session.
    async fn add_message(&self, msg: ChatMessage) -> Result<(), AppError>;

    /// Get messages for a session, ordered by creation time.
    /// `limit` controls the maximum number of messages returned (most recent).
    async fn get_messages(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<ChatMessage>, AppError>;

    /// Delete a session and all its messages.
    async fn delete_session(&self, id: &str) -> Result<(), AppError>;
}

// ── MongoDB implementation ───────────────────────────────────────────────────

#[cfg(feature = "ssr")]
pub struct MongoChatRepository {
    sessions: mongodb::Collection<ChatSession>,
    messages: mongodb::Collection<ChatMessage>,
}

#[cfg(feature = "ssr")]
impl MongoChatRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            sessions: db.collection("chat_sessions"),
            messages: db.collection("chat_messages"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl ChatRepository for MongoChatRepository {
    async fn create_session(&self, session: ChatSession) -> Result<(), AppError> {
        self.sessions
            .insert_one(session)
            .await
            .map_err(|e| AppError::Internal(format!("mongo insert chat_session: {e}")))?;
        Ok(())
    }

    async fn get_session(&self, id: &str) -> Result<Option<ChatSession>, AppError> {
        self.sessions
            .find_one(mongodb::bson::doc! { "id": id })
            .await
            .map_err(|e| AppError::Internal(format!("mongo find chat_session: {e}")))
    }

    async fn list_sessions_for_user(&self, user_id: &str) -> Result<Vec<ChatSession>, AppError> {
        use futures::TryStreamExt;

        let cursor = self
            .sessions
            .find(mongodb::bson::doc! { "user_id": user_id })
            .sort(mongodb::bson::doc! { "updated_at": -1 })
            .await
            .map_err(|e| AppError::Internal(format!("mongo list chat_sessions: {e}")))?;

        cursor
            .try_collect()
            .await
            .map_err(|e| AppError::Internal(format!("mongo collect chat_sessions: {e}")))
    }

    async fn update_session_title(&self, id: &str, title: &str) -> Result<(), AppError> {
        self.sessions
            .update_one(
                mongodb::bson::doc! { "id": id },
                mongodb::bson::doc! { "$set": { "title": title } },
            )
            .await
            .map_err(|e| AppError::Internal(format!("mongo update chat_session title: {e}")))?;
        Ok(())
    }

    async fn touch_session(&self, id: &str) -> Result<(), AppError> {
        let now = mongodb::bson::DateTime::from_chrono(chrono::Utc::now());
        self.sessions
            .update_one(
                mongodb::bson::doc! { "id": id },
                mongodb::bson::doc! { "$set": { "updated_at": now } },
            )
            .await
            .map_err(|e| AppError::Internal(format!("mongo touch chat_session: {e}")))?;
        Ok(())
    }

    async fn add_message(&self, msg: ChatMessage) -> Result<(), AppError> {
        self.messages
            .insert_one(msg)
            .await
            .map_err(|e| AppError::Internal(format!("mongo insert chat_message: {e}")))?;
        Ok(())
    }

    async fn get_messages(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<ChatMessage>, AppError> {
        use futures::TryStreamExt;

        // Get the most recent `limit` messages, then reverse for chronological order
        let cursor = self
            .messages
            .find(mongodb::bson::doc! { "session_id": session_id })
            .sort(mongodb::bson::doc! { "created_at": -1 })
            .limit(limit as i64)
            .await
            .map_err(|e| AppError::Internal(format!("mongo find chat_messages: {e}")))?;

        let mut messages: Vec<ChatMessage> = cursor
            .try_collect()
            .await
            .map_err(|e| AppError::Internal(format!("mongo collect chat_messages: {e}")))?;

        messages.reverse();
        Ok(messages)
    }

    async fn delete_session(&self, id: &str) -> Result<(), AppError> {
        // Delete all messages first, then the session
        self.messages
            .delete_many(mongodb::bson::doc! { "session_id": id })
            .await
            .map_err(|e| AppError::Internal(format!("mongo delete chat_messages: {e}")))?;

        self.sessions
            .delete_one(mongodb::bson::doc! { "id": id })
            .await
            .map_err(|e| AppError::Internal(format!("mongo delete chat_session: {e}")))?;

        Ok(())
    }
}
