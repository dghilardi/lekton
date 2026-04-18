use async_trait::async_trait;

use crate::db::prompt_models::Prompt;
use crate::error::AppError;

/// Repository trait for prompt metadata.
#[async_trait]
pub trait PromptRepository: Send + Sync {
    /// Create a new prompt or replace an existing one (matched by slug).
    async fn create_or_update(&self, prompt: Prompt) -> Result<(), AppError>;

    /// Find a prompt by slug.
    async fn find_by_slug(&self, slug: &str) -> Result<Option<Prompt>, AppError>;

    /// List prompts the caller is allowed to see.
    async fn list_by_access_levels(
        &self,
        allowed_levels: Option<&[String]>,
        include_draft: bool,
    ) -> Result<Vec<Prompt>, AppError>;

    /// List non-archived prompts whose slug matches an exact prefix scope.
    async fn find_by_slug_prefix(&self, prefix: &str) -> Result<Vec<Prompt>, AppError>;

    /// Set the archived flag on a prompt.
    async fn set_archived(&self, slug: &str, archived: bool) -> Result<(), AppError>;

    /// Search prompt metadata and optionally the body excerpt index in future.
    async fn search_metadata(
        &self,
        query: &str,
        allowed_levels: Option<&[String]>,
        include_draft: bool,
        limit: usize,
    ) -> Result<Vec<Prompt>, AppError>;
}

#[cfg(feature = "ssr")]
pub struct MongoPromptRepository {
    collection: mongodb::Collection<Prompt>,
}

#[cfg(feature = "ssr")]
impl MongoPromptRepository {
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            collection: db.collection("prompts"),
        }
    }
}

#[cfg(feature = "ssr")]
#[async_trait]
impl PromptRepository for MongoPromptRepository {
    async fn create_or_update(&self, prompt: Prompt) -> Result<(), AppError> {
        use mongodb::bson::doc;
        use mongodb::options::ReplaceOptions;

        let filter = doc! { "slug": &prompt.slug };
        let options = ReplaceOptions::builder().upsert(true).build();

        self.collection
            .replace_one(filter, &prompt)
            .with_options(options)
            .await?;

        Ok(())
    }

    async fn find_by_slug(&self, slug: &str) -> Result<Option<Prompt>, AppError> {
        use mongodb::bson::doc;

        Ok(self.collection.find_one(doc! { "slug": slug }).await?)
    }

    async fn list_by_access_levels(
        &self,
        allowed_levels: Option<&[String]>,
        include_draft: bool,
    ) -> Result<Vec<Prompt>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::{doc, Bson};
        use mongodb::options::FindOptions;

        let mut filter_parts: Vec<mongodb::bson::Document> = vec![doc! {
            "$or": [
                { "is_archived": { "$exists": false } },
                { "is_archived": false }
            ]
        }];

        if let Some(levels) = allowed_levels {
            let bson_levels: Vec<Bson> = levels
                .iter()
                .map(|level| Bson::String(level.clone()))
                .collect();
            filter_parts.push(doc! { "access_level": { "$in": bson_levels } });
        }

        if !include_draft {
            filter_parts.push(doc! { "status": { "$ne": "draft" } });
        }

        let filter = doc! { "$and": filter_parts };
        let options = FindOptions::builder()
            .sort(doc! { "name": 1, "slug": 1 })
            .build();

        let mut cursor = self.collection.find(filter).with_options(options).await?;

        let mut prompts = Vec::new();
        while let Some(prompt) = cursor.try_next().await? {
            prompts.push(prompt);
        }
        Ok(prompts)
    }

    async fn find_by_slug_prefix(&self, prefix: &str) -> Result<Vec<Prompt>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::doc;

        let filter = if prefix.is_empty() {
            doc! {
                "$or": [
                    { "is_archived": { "$exists": false } },
                    { "is_archived": false }
                ]
            }
        } else {
            doc! {
                "$and": [
                    {
                        "$or": [
                            { "slug": prefix },
                            { "slug": { "$regex": format!("^{}/", regex_escape(prefix)) } }
                        ]
                    },
                    {
                        "$or": [
                            { "is_archived": { "$exists": false } },
                            { "is_archived": false }
                        ]
                    }
                ]
            }
        };

        let mut cursor = self.collection.find(filter).await?;
        let mut prompts = Vec::new();
        while let Some(prompt) = cursor.try_next().await? {
            prompts.push(prompt);
        }
        Ok(prompts)
    }

    async fn set_archived(&self, slug: &str, archived: bool) -> Result<(), AppError> {
        use mongodb::bson::doc;

        self.collection
            .update_one(
                doc! { "slug": slug },
                doc! { "$set": { "is_archived": archived } },
            )
            .await?;
        Ok(())
    }

    async fn search_metadata(
        &self,
        query: &str,
        allowed_levels: Option<&[String]>,
        include_draft: bool,
        limit: usize,
    ) -> Result<Vec<Prompt>, AppError> {
        use futures::TryStreamExt;
        use mongodb::bson::{doc, Bson};
        use mongodb::options::FindOptions;

        let escaped = regex_escape(query);
        let mut filter_parts: Vec<mongodb::bson::Document> = vec![
            doc! {
                "$or": [
                    { "is_archived": { "$exists": false } },
                    { "is_archived": false }
                ]
            },
            doc! {
                "$or": [
                    { "slug": { "$regex": &escaped, "$options": "i" } },
                    { "name": { "$regex": &escaped, "$options": "i" } },
                    { "description": { "$regex": &escaped, "$options": "i" } },
                    { "owner": { "$regex": &escaped, "$options": "i" } },
                    { "tags": { "$elemMatch": { "$regex": &escaped, "$options": "i" } } }
                ]
            },
        ];

        if let Some(levels) = allowed_levels {
            let bson_levels: Vec<Bson> = levels
                .iter()
                .map(|level| Bson::String(level.clone()))
                .collect();
            filter_parts.push(doc! { "access_level": { "$in": bson_levels } });
        }

        if !include_draft {
            filter_parts.push(doc! { "status": { "$ne": "draft" } });
        }

        let options = FindOptions::builder()
            .sort(doc! { "name": 1, "slug": 1 })
            .limit(limit.max(1) as i64)
            .build();

        let mut cursor = self
            .collection
            .find(doc! { "$and": filter_parts })
            .with_options(options)
            .await?;

        let mut prompts = Vec::new();
        while let Some(prompt) = cursor.try_next().await? {
            prompts.push(prompt);
        }
        Ok(prompts)
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
    use super::regex_escape;

    #[cfg(feature = "ssr")]
    #[test]
    fn regex_escape_escapes_special_chars() {
        assert_eq!(regex_escape("a+b(c)"), "a\\+b\\(c\\)");
    }
}
