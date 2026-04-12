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
    ) -> Result<Vec<DocumentationFeedback>, AppError>;
    async fn list(
        &self,
        params: DocumentationFeedbackListParams,
    ) -> Result<DocumentationFeedbackPage, AppError>;
    async fn resolve(
        &self,
        id: &str,
        resolution_note: Option<String>,
    ) -> Result<(), AppError>;
    async fn mark_duplicate(
        &self,
        id: &str,
        duplicate_of: &str,
        resolution_note: Option<String>,
    ) -> Result<(), AppError>;
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

    pub async fn ensure_indexes(&self) -> Result<(), AppError> {
        use mongodb::IndexModel;
        use mongodb::options::IndexOptions;

        self.collection
            .create_index(
                IndexModel::builder()
                    .keys(mongodb::bson::doc! { "id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await
            .map_err(|e| AppError::Database(format!("create documentation_feedback id index: {e}")))?;

        self.collection
            .create_index(
                IndexModel::builder()
                    .keys(mongodb::bson::doc! { "status": 1, "kind": 1, "created_at": -1 })
                    .build(),
            )
            .await
            .map_err(|e| AppError::Database(format!("create documentation_feedback status index: {e}")))?;

        Ok(())
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
    ) -> Result<Vec<DocumentationFeedback>, AppError> {
        use futures::TryStreamExt;

        let filter = build_filter(Some(query), kind, status);
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
        let filter = build_filter(params.query.as_deref(), params.kind, params.status);

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

    async fn resolve(
        &self,
        id: &str,
        resolution_note: Option<String>,
    ) -> Result<(), AppError> {
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
            .map_err(|e| AppError::Database(format!("mark documentation_feedback duplicate: {e}")))?;

        if result.matched_count == 0 {
            return Err(AppError::NotFound(format!(
                "Documentation feedback '{id}' not found"
            )));
        }

        Ok(())
    }
}

#[cfg(feature = "ssr")]
fn build_filter(
    query: Option<&str>,
    kind: Option<DocumentationFeedbackKind>,
    status: Option<DocumentationFeedbackStatus>,
) -> mongodb::bson::Document {
    use mongodb::bson::{doc, Bson};

    let mut filter_parts = Vec::new();

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
    let special = ['.', '*', '+', '?', '(', ')', '[', ']', '{', '}', '\\', '^', '$', '|'];
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
    use super::regex_escape;

    #[cfg(feature = "ssr")]
    #[test]
    fn regex_escape_escapes_special_chars() {
        assert_eq!(regex_escape("docs+(api)"), "docs\\+\\(api\\)");
    }
}
