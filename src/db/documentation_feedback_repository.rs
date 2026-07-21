use async_trait::async_trait;

use crate::db::documentation_feedback_models::{
    DocumentationFeedback, DocumentationFeedbackKind, DocumentationFeedbackStatus,
};
use crate::error::AppError;

#[derive(Debug, Clone, Default)]
pub struct DocumentationFeedbackListParams {
    pub query: Option<String>,
    pub kind: Option<DocumentationFeedbackKind>,
    pub status: Option<DocumentationFeedbackStatus>,
    /// Filter to items whose claim delivers to this source id.
    pub delivery_source_id: Option<String>,
    pub page: u64,
    pub per_page: u64,
}

#[derive(Debug, Clone)]
pub struct DocumentationFeedbackPage {
    pub items: Vec<DocumentationFeedback>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

#[async_trait]
pub trait DocumentationFeedbackRepository: Send + Sync {
    async fn create(&self, feedback: DocumentationFeedback) -> Result<(), AppError>;
    async fn find_by_id(&self, id: &str) -> Result<Option<DocumentationFeedback>, AppError>;
    async fn search(
        &self,
        query: &str,
        kind: Option<DocumentationFeedbackKind>,
        status: Option<DocumentationFeedbackStatus>,
        limit: usize,
        created_by: Option<&str>,
    ) -> Result<Vec<DocumentationFeedback>, AppError>;
    async fn list(
        &self,
        params: DocumentationFeedbackListParams,
    ) -> Result<DocumentationFeedbackPage, AppError>;
    async fn resolve(&self, id: &str, resolution_note: Option<String>) -> Result<(), AppError>;
    async fn mark_duplicate(
        &self,
        id: &str,
        duplicate_of: &str,
        resolution_note: Option<String>,
    ) -> Result<(), AppError>;

    /// Take an item in charge: set it `in_progress` and record the delivery
    /// (where the fix will land) plus a per-claim `nonce`. Allowed from `open`
    /// or `in_progress` (re-claim updates the delivery); rejected for a
    /// `resolved` item (returns `NotFound`).
    async fn claim(
        &self,
        id: &str,
        delivery_source_id: &str,
        delivery_ref: Option<&str>,
        nonce: &str,
    ) -> Result<(), AppError>;

    /// Resolve an item only if it is `in_progress` and claimed for
    /// `delivery_source_id`. Returns `true` when it matched (and was resolved),
    /// `false` otherwise — so a caller can authorize resolution to the source
    /// that actually holds the claim. Idempotent-safe: a non-match is not an
    /// error.
    async fn resolve_claimed(&self, id: &str, delivery_source_id: &str) -> Result<bool, AppError>;

    /// Reopen `in_progress` items claimed before `cutoff` (abandoned deliveries):
    /// set them back to `open` and clear the claim metadata so a fresh claim
    /// starts clean. Returns how many were reopened.
    async fn reopen_stale_claims(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Result<u64, AppError>;
}

#[cfg(feature = "ssr")]
pub struct MongoDocumentationFeedbackRepository {
    collection: mongodb::Collection<DocumentationFeedback>,
}

#[cfg(feature = "ssr")]
impl MongoDocumentationFeedbackRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            collection: db.collection("documentation_feedback"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl DocumentationFeedbackRepository for MongoDocumentationFeedbackRepository {
    async fn create(&self, feedback: DocumentationFeedback) -> Result<(), AppError> {
        self.collection
            .insert_one(feedback)
            .await
            .map_err(|e| AppError::Database(format!("insert documentation_feedback: {e}")))?;
        Ok(())
    }

    async fn find_by_id(&self, id: &str) -> Result<Option<DocumentationFeedback>, AppError> {
        self.collection
            .find_one(mongodb::bson::doc! { "id": id })
            .await
            .map_err(|e| AppError::Database(format!("find documentation_feedback by id: {e}")))
    }

    async fn search(
        &self,
        query: &str,
        kind: Option<DocumentationFeedbackKind>,
        status: Option<DocumentationFeedbackStatus>,
        limit: usize,
        created_by: Option<&str>,
    ) -> Result<Vec<DocumentationFeedback>, AppError> {
        use futures::TryStreamExt;

        let filter = build_filter(Some(query), kind, status, created_by, None);
        let cursor = self
            .collection
            .find(filter)
            .sort(mongodb::bson::doc! { "created_at": -1 })
            .limit(limit.clamp(1, 50) as i64)
            .await
            .map_err(|e| AppError::Database(format!("search documentation_feedback: {e}")))?;

        cursor
            .try_collect()
            .await
            .map_err(|e| AppError::Database(format!("collect documentation_feedback search: {e}")))
    }

    async fn list(
        &self,
        params: DocumentationFeedbackListParams,
    ) -> Result<DocumentationFeedbackPage, AppError> {
        use futures::TryStreamExt;

        const MAX_PER_PAGE: u64 = 100;
        let per_page = params.per_page.clamp(1, MAX_PER_PAGE);
        let skip = params.page * per_page;
        let filter = build_filter(
            params.query.as_deref(),
            params.kind,
            params.status,
            None,
            params.delivery_source_id.as_deref(),
        );

        let total = self
            .collection
            .count_documents(filter.clone())
            .await
            .map_err(|e| AppError::Database(format!("count documentation_feedback: {e}")))?;

        let cursor = self
            .collection
            .find(filter)
            .sort(mongodb::bson::doc! { "created_at": -1 })
            .skip(skip)
            .limit(per_page as i64)
            .await
            .map_err(|e| AppError::Database(format!("list documentation_feedback: {e}")))?;

        let items = cursor
            .try_collect()
            .await
            .map_err(|e| AppError::Database(format!("collect documentation_feedback list: {e}")))?;

        Ok(DocumentationFeedbackPage {
            items,
            total,
            page: params.page,
            per_page,
        })
    }

    async fn resolve(&self, id: &str, resolution_note: Option<String>) -> Result<(), AppError> {
        let result = self
            .collection
            .update_one(
                mongodb::bson::doc! { "id": id },
                mongodb::bson::doc! {
                    "$set": {
                        "status": "resolved",
                        "resolution_note": resolution_note,
                    }
                },
            )
            .await
            .map_err(|e| AppError::Database(format!("resolve documentation_feedback: {e}")))?;

        if result.matched_count == 0 {
            return Err(AppError::NotFound(format!(
                "Documentation feedback '{id}' not found"
            )));
        }

        Ok(())
    }

    async fn mark_duplicate(
        &self,
        id: &str,
        duplicate_of: &str,
        resolution_note: Option<String>,
    ) -> Result<(), AppError> {
        let result = self
            .collection
            .update_one(
                mongodb::bson::doc! { "id": id },
                mongodb::bson::doc! {
                    "$set": {
                        "status": "resolved",
                        "duplicate_of": duplicate_of,
                        "resolution_note": resolution_note,
                    }
                },
            )
            .await
            .map_err(|e| {
                AppError::Database(format!("mark documentation_feedback duplicate: {e}"))
            })?;

        if result.matched_count == 0 {
            return Err(AppError::NotFound(format!(
                "Documentation feedback '{id}' not found"
            )));
        }

        Ok(())
    }

    async fn claim(
        &self,
        id: &str,
        delivery_source_id: &str,
        delivery_ref: Option<&str>,
        nonce: &str,
    ) -> Result<(), AppError> {
        use mongodb::bson::{doc, DateTime};

        let now = DateTime::from_millis(chrono::Utc::now().timestamp_millis());
        let result = self
            .collection
            .update_one(
                // Only claim an open or already-in-progress item; a resolved
                // item cannot be re-claimed.
                doc! { "id": id, "status": { "$in": ["open", "in_progress"] } },
                doc! {
                    "$set": {
                        "status": "in_progress",
                        "delivery_source_id": delivery_source_id,
                        "delivery_ref": delivery_ref,
                        "claim_nonce": nonce,
                        "claimed_at": now,
                    }
                },
            )
            .await
            .map_err(|e| AppError::Database(format!("claim documentation_feedback: {e}")))?;

        if result.matched_count == 0 {
            return Err(AppError::NotFound(format!(
                "Open or in-progress documentation feedback '{id}' not found"
            )));
        }

        Ok(())
    }

    async fn resolve_claimed(&self, id: &str, delivery_source_id: &str) -> Result<bool, AppError> {
        use mongodb::bson::doc;

        let result = self
            .collection
            .update_one(
                doc! {
                    "id": id,
                    "status": "in_progress",
                    "delivery_source_id": delivery_source_id,
                },
                doc! { "$set": { "status": "resolved" } },
            )
            .await
            .map_err(|e| {
                AppError::Database(format!("resolve_claimed documentation_feedback: {e}"))
            })?;

        Ok(result.matched_count == 1)
    }

    async fn reopen_stale_claims(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Result<u64, AppError> {
        use mongodb::bson::{doc, DateTime};

        let cutoff = DateTime::from_millis(cutoff.timestamp_millis());
        let result = self
            .collection
            .update_many(
                doc! { "status": "in_progress", "claimed_at": { "$lt": cutoff } },
                doc! {
                    "$set": { "status": "open" },
                    "$unset": {
                        "delivery_source_id": "",
                        "delivery_ref": "",
                        "claim_nonce": "",
                        "claimed_at": "",
                    },
                },
            )
            .await
            .map_err(|e| {
                AppError::Database(format!("reopen_stale_claims documentation_feedback: {e}"))
            })?;

        Ok(result.modified_count)
    }
}

#[cfg(feature = "ssr")]
fn build_filter(
    query: Option<&str>,
    kind: Option<DocumentationFeedbackKind>,
    status: Option<DocumentationFeedbackStatus>,
    created_by: Option<&str>,
    delivery_source_id: Option<&str>,
) -> mongodb::bson::Document {
    use mongodb::bson::{doc, Bson};

    let mut filter_parts = Vec::new();

    if let Some(owner) = created_by {
        filter_parts.push(doc! { "created_by": owner });
    }

    if let Some(source_id) = delivery_source_id {
        filter_parts.push(doc! { "delivery_source_id": source_id });
    }

    if let Some(kind) = kind {
        filter_parts.push(doc! { "kind": kind.as_str() });
    }

    if let Some(status) = status {
        filter_parts.push(doc! { "status": status.as_str() });
    }

    if let Some(query) = query {
        let trimmed = query.trim();
        if !trimmed.is_empty() {
            let escaped = regex_escape(trimmed);
            filter_parts.push(doc! {
                "$or": [
                    { "id": { "$regex": &escaped, "$options": "i" } },
                    { "title": { "$regex": &escaped, "$options": "i" } },
                    { "summary": { "$regex": &escaped, "$options": "i" } },
                    { "related_resources": { "$elemMatch": { "$regex": &escaped, "$options": "i" } } },
                    { "search_queries": { "$elemMatch": { "$regex": &escaped, "$options": "i" } } },
                    { "user_goal": { "$regex": &escaped, "$options": "i" } },
                    { "missing_information": { "$regex": &escaped, "$options": "i" } },
                    { "impact": { "$regex": &escaped, "$options": "i" } },
                    { "suggested_target_resource": { "$regex": &escaped, "$options": "i" } },
                    { "target_resource_uri": { "$regex": &escaped, "$options": "i" } },
                    { "problem_summary": { "$regex": &escaped, "$options": "i" } },
                    { "proposal": { "$regex": &escaped, "$options": "i" } },
                    { "supporting_resources": { "$elemMatch": { "$regex": &escaped, "$options": "i" } } },
                    { "expected_benefit": { "$regex": &escaped, "$options": "i" } },
                    { "created_by": { "$regex": &escaped, "$options": "i" } },
                    { "related_feedback_ids": { "$elemMatch": { "$regex": &escaped, "$options": "i" } } },
                    { "duplicate_of": { "$regex": &escaped, "$options": "i" } }
                ]
            });
        }
    }

    match filter_parts.len() {
        0 => mongodb::bson::Document::new(),
        1 => filter_parts.into_iter().next().unwrap_or_default(),
        _ => doc! { "$and": Bson::Array(filter_parts.into_iter().map(Bson::Document).collect()) },
    }
}

#[cfg(feature = "ssr")]
fn regex_escape(s: &str) -> String {
    let special = [
        '.', '*', '+', '?', '(', ')', '[', ']', '{', '}', '\\', '^', '$', '|',
    ];
    let mut escaped = String::with_capacity(s.len());
    for c in s.chars() {
        if special.contains(&c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "ssr")]
    use super::{build_filter, regex_escape};

    #[cfg(feature = "ssr")]
    #[test]
    fn regex_escape_escapes_special_chars() {
        assert_eq!(regex_escape("docs+(api)"), "docs\\+\\(api\\)");
    }

    /// Regression test for MCP-BUG-1: when created_by is Some, the filter must include
    /// a created_by clause so non-admins only see their own feedback.
    #[cfg(feature = "ssr")]
    #[test]
    fn build_filter_scopes_to_created_by_when_provided() {
        let filter = build_filter(None, None, None, Some("alice@example.com"), None);
        let bson_str = filter.to_string();
        assert!(
            bson_str.contains("created_by") && bson_str.contains("alice@example.com"),
            "filter must contain a created_by constraint: {bson_str}"
        );
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn build_filter_no_created_by_for_admins() {
        let filter = build_filter(None, None, None, None, None);
        let bson_str = filter.to_string();
        assert!(
            !bson_str.contains("created_by"),
            "admin filter must not restrict by created_by: {bson_str}"
        );
    }
}
