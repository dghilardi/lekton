use serde::{Deserialize, Serialize};

use crate::db::prompt_models::{ContextCost, Prompt, PromptStatus, PromptVariable};
use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptIngestRequest {
    pub service_token: String,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub prompt_body: String,
    pub access_level: String,
    pub status: PromptStatus,
    pub owner: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub variables: Vec<PromptVariable>,
    #[serde(default)]
    pub publish_to_mcp: bool,
    #[serde(default)]
    pub default_primary: bool,
    #[serde(default)]
    pub context_cost: ContextCost,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptIngestResponse {
    pub message: String,
    pub slug: String,
    pub s3_key: String,
    #[serde(default = "default_true")]
    pub changed: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PromptSyncEntry {
    pub slug: String,
    pub content_hash: String,
    #[serde(default)]
    pub metadata_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PromptSyncRequest {
    pub service_token: String,
    pub prompts: Vec<PromptSyncEntry>,
    #[serde(default)]
    pub archive_missing: bool,
}

#[derive(Debug, Serialize)]
pub struct PromptSyncResponse {
    pub to_upload: Vec<String>,
    pub to_archive: Vec<String>,
    pub unchanged: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PromptBlob {
    slug: String,
    name: String,
    description: String,
    access_level: String,
    status: PromptStatus,
    owner: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    variables: Vec<PromptVariable>,
    #[serde(default)]
    publish_to_mcp: bool,
    #[serde(default)]
    default_primary: bool,
    #[serde(default)]
    context_cost: ContextCost,
    prompt_body: String,
}

fn default_true() -> bool {
    true
}

#[cfg(feature = "ssr")]
pub struct PromptIngestContext<'a> {
    pub repo: &'a dyn crate::db::prompt_repository::PromptRepository,
    pub storage: &'a dyn crate::storage::client::StorageClient,
    pub access_level_repo: &'a dyn crate::db::access_level_repository::AccessLevelRepository,
    pub service_token_repo: &'a dyn crate::db::service_token_repository::ServiceTokenRepository,
    pub version_repo: &'a dyn crate::db::prompt_version_repository::PromptVersionRepository,
    pub legacy_token: Option<&'a str>,
}

#[cfg(feature = "ssr")]
pub async fn process_prompt_ingest(
    ctx: &PromptIngestContext<'_>,
    request: PromptIngestRequest,
) -> Result<PromptIngestResponse, AppError> {
    validate_prompt_token(ctx, &request.service_token, &request.slug).await?;

    validate_prompt_request(ctx, &request).await?;

    let normalized_access_level = request.access_level.to_lowercase();
    let new_content_hash = format!(
        "sha256:{}",
        crate::auth::token_service::TokenService::hash_token(&request.prompt_body)
    );
    let new_metadata_hash = compute_prompt_metadata_hash(&request);
    let s3_key = format!("prompts/{}.yaml", request.slug.replace('/', "_"));

    let existing = ctx.repo.find_by_slug(&request.slug).await?;
    let old_content_hash = existing
        .as_ref()
        .and_then(|prompt| prompt.content_hash.clone());

    let content_changed = old_content_hash.as_deref() != Some(new_content_hash.as_str());
    let metadata_changed = existing.as_ref().map_or(true, |prompt| {
        prompt.name != request.name
            || prompt.description != request.description
            || prompt.access_level != normalized_access_level
            || prompt.status != request.status
            || prompt.owner != request.owner
            || prompt.tags != request.tags
            || prompt.variables != request.variables
            || prompt.publish_to_mcp != request.publish_to_mcp
            || prompt.default_primary != request.default_primary
            || prompt.context_cost != request.context_cost
    });

    if !content_changed && !metadata_changed {
        return Ok(PromptIngestResponse {
            message: "Prompt unchanged".to_string(),
            slug: request.slug,
            s3_key,
            changed: false,
        });
    }

    if content_changed {
        if let Some(old_prompt) = existing.as_ref() {
            if let Some(old_hash) = old_prompt.content_hash.as_ref() {
                let version_num = ctx.version_repo.next_version_number(&request.slug).await?;
                let history_key = format!(
                    "prompts/history/{}/{}.yaml",
                    request.slug.replace('/', "_"),
                    version_num
                );

                if let Ok(Some(old_content)) = ctx.storage.get_object(&old_prompt.s3_key).await {
                    if let Err(err) = ctx.storage.put_object(&history_key, old_content).await {
                        tracing::warn!("Failed to archive old prompt version to S3: {err}");
                    }
                }

                let updated_by = resolve_prompt_token_name(ctx, &request.service_token).await;
                let version = crate::db::prompt_version_repository::PromptVersion {
                    id: uuid::Uuid::new_v4().to_string(),
                    slug: request.slug.clone(),
                    version: version_num,
                    content_hash: old_hash.clone(),
                    s3_key: history_key,
                    updated_by,
                    created_at: chrono::Utc::now(),
                };

                if let Err(err) = ctx.version_repo.create(version).await {
                    tracing::warn!("Failed to create prompt version record: {err}");
                }
            }
        }

        let blob = PromptBlob {
            slug: request.slug.clone(),
            name: request.name.clone(),
            description: request.description.clone(),
            access_level: normalized_access_level.clone(),
            status: request.status.clone(),
            owner: request.owner.clone(),
            tags: request.tags.clone(),
            variables: request.variables.clone(),
            publish_to_mcp: request.publish_to_mcp,
            default_primary: request.default_primary,
            context_cost: request.context_cost.clone(),
            prompt_body: request.prompt_body.clone(),
        };
        let serialized = serde_yaml::to_string(&blob)
            .map_err(|err| AppError::Internal(format!("Failed to serialize prompt blob: {err}")))?;
        ctx.storage
            .put_object(&s3_key, serialized.into_bytes())
            .await?;
    }

    let prompt = Prompt {
        slug: request.slug.clone(),
        name: request.name,
        description: request.description,
        s3_key: s3_key.clone(),
        access_level: normalized_access_level,
        status: request.status,
        owner: request.owner,
        last_updated: chrono::Utc::now(),
        tags: request.tags,
        variables: request.variables,
        publish_to_mcp: request.publish_to_mcp,
        default_primary: request.default_primary,
        context_cost: request.context_cost,
        content_hash: Some(new_content_hash),
        metadata_hash: Some(new_metadata_hash),
        is_archived: false,
    };

    ctx.repo.create_or_update(prompt).await?;

    Ok(PromptIngestResponse {
        message: "Prompt ingested successfully".to_string(),
        slug: request.slug,
        s3_key,
        changed: true,
    })
}

#[cfg(feature = "ssr")]
pub async fn process_prompt_sync(
    repo: &dyn crate::db::prompt_repository::PromptRepository,
    service_token_repo: &dyn crate::db::service_token_repository::ServiceTokenRepository,
    legacy_token: Option<&str>,
    request: PromptSyncRequest,
) -> Result<PromptSyncResponse, AppError> {
    use std::collections::HashMap;

    let scopes =
        validate_sync_token(service_token_repo, legacy_token, &request.service_token).await?;

    for entry in &request.prompts {
        if !slug_matches_scopes(&entry.slug, &scopes) {
            return Err(AppError::Forbidden(format!(
                "Token does not have access to slug '{}'",
                entry.slug
            )));
        }
    }

    let mut server_prompts: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
    for scope in &scopes {
        if scope == "*" {
            for prompt in repo.find_by_slug_prefix("").await? {
                server_prompts.insert(prompt.slug, (prompt.content_hash, prompt.metadata_hash));
            }
        } else if let Some(prefix) = scope.strip_suffix("/*") {
            for prompt in repo.find_by_slug_prefix(prefix).await? {
                server_prompts.insert(prompt.slug, (prompt.content_hash, prompt.metadata_hash));
            }
        } else if let Some(prompt) = repo.find_by_slug(scope).await? {
            if !prompt.is_archived {
                server_prompts.insert(prompt.slug, (prompt.content_hash, prompt.metadata_hash));
            }
        }
    }

    let client_prompts: HashMap<&str, &str> = request
        .prompts
        .iter()
        .map(|entry| (entry.slug.as_str(), entry.content_hash.as_str()))
        .collect();

    let mut to_upload = Vec::new();
    let mut unchanged = Vec::new();
    let mut to_archive = Vec::new();

    for entry in &request.prompts {
        match server_prompts.get(&entry.slug) {
            Some((server_content_hash, server_metadata_hash)) => {
                let content_ok =
                    server_content_hash.as_deref() == Some(entry.content_hash.as_str());
                let metadata_ok = match (
                    entry.metadata_hash.as_deref(),
                    server_metadata_hash.as_deref(),
                ) {
                    (Some(client), Some(server)) => client == server,
                    (Some(_), None) => false,
                    (None, _) => true,
                };

                if content_ok && metadata_ok {
                    unchanged.push(entry.slug.clone());
                } else {
                    to_upload.push(entry.slug.clone());
                }
            }
            None => to_upload.push(entry.slug.clone()),
        }
    }

    for slug in server_prompts.keys() {
        if !client_prompts.contains_key(slug.as_str()) {
            to_archive.push(slug.clone());
        }
    }

    if request.archive_missing {
        for slug in &to_archive {
            repo.set_archived(slug, true).await?;
        }
    }

    to_upload.sort();
    unchanged.sort();
    to_archive.sort();

    Ok(PromptSyncResponse {
        to_upload,
        to_archive,
        unchanged,
    })
}

#[cfg(feature = "ssr")]
pub fn compute_prompt_metadata_hash(request: &PromptIngestRequest) -> String {
    let mut sorted_tags: Vec<&str> = request.tags.iter().map(|tag| tag.as_str()).collect();
    sorted_tags.sort_unstable();

    let mut vars: Vec<String> = request
        .variables
        .iter()
        .map(|var| format!("{}:{}:{}", var.name, var.description, var.required))
        .collect();
    vars.sort_unstable();

    let canonical = format!(
        "name={}\ndescription={}\naccess_level={}\nstatus={:?}\nowner={}\ntags={}\nvariables={}\npublish_to_mcp={}\ndefault_primary={}\ncontext_cost={:?}",
        request.name,
        request.description,
        request.access_level.to_lowercase(),
        request.status,
        request.owner,
        sorted_tags.join(","),
        vars.join("|"),
        request.publish_to_mcp,
        request.default_primary,
        request.context_cost,
    );

    format!(
        "sha256:{}",
        crate::auth::token_service::TokenService::hash_token(&canonical)
    )
}

#[cfg(feature = "ssr")]
async fn validate_prompt_request(
    ctx: &PromptIngestContext<'_>,
    request: &PromptIngestRequest,
) -> Result<(), AppError> {
    if request.slug.is_empty() {
        return Err(AppError::BadRequest("Slug cannot be empty".into()));
    }
    if request.slug.contains("..") {
        return Err(AppError::BadRequest("Slug must not contain '..'".into()));
    }
    if request.slug.starts_with('/') {
        return Err(AppError::BadRequest("Slug must not start with '/'".into()));
    }
    if request.name.trim().is_empty() {
        return Err(AppError::BadRequest("Prompt name cannot be empty".into()));
    }
    if request.description.trim().is_empty() {
        return Err(AppError::BadRequest(
            "Prompt description cannot be empty".into(),
        ));
    }
    if request.prompt_body.trim().is_empty() {
        return Err(AppError::BadRequest("Prompt body cannot be empty".into()));
    }
    if request.owner.trim().is_empty() {
        return Err(AppError::BadRequest("Prompt owner cannot be empty".into()));
    }
    if request.default_primary && !request.publish_to_mcp {
        return Err(AppError::BadRequest(
            "default_primary requires publish_to_mcp = true".into(),
        ));
    }

    let access_level = request.access_level.trim().to_lowercase();
    if access_level.is_empty() {
        return Err(AppError::BadRequest("Access level cannot be empty".into()));
    }
    if !ctx.access_level_repo.exists(&access_level).await? {
        return Err(AppError::BadRequest(format!(
            "Unknown access level: '{access_level}'"
        )));
    }

    let mut variable_names = std::collections::HashSet::new();
    for var in &request.variables {
        if var.name.trim().is_empty() {
            return Err(AppError::BadRequest(
                "Prompt variable name cannot be empty".into(),
            ));
        }
        if !variable_names.insert(var.name.to_lowercase()) {
            return Err(AppError::BadRequest(format!(
                "Duplicate prompt variable '{}'",
                var.name
            )));
        }
    }

    Ok(())
}

#[cfg(feature = "ssr")]
async fn validate_prompt_token(
    ctx: &PromptIngestContext<'_>,
    raw_token: &str,
    slug: &str,
) -> Result<(), AppError> {
    if let Some(legacy) = ctx.legacy_token {
        if !legacy.is_empty() && raw_token == legacy {
            return Ok(());
        }
    }

    let token_hash = crate::auth::token_service::TokenService::hash_token(raw_token);
    let token = ctx
        .service_token_repo
        .find_by_hash(&token_hash)
        .await?
        .ok_or_else(|| AppError::Auth("Invalid service token".into()))?;

    if !token.is_active {
        return Err(AppError::Auth("Service token is deactivated".into()));
    }
    if !token.can_write {
        return Err(AppError::Forbidden(
            "Token does not have write permission".into(),
        ));
    }
    if !token.matches_slug(slug) {
        return Err(AppError::Forbidden(
            "Token does not have access to this prompt scope".into(),
        ));
    }

    if let Err(err) = ctx.service_token_repo.touch_last_used(&token.id).await {
        tracing::warn!(
            "Failed to update last_used_at for token {}: {err}",
            token.id
        );
    }

    Ok(())
}

#[cfg(feature = "ssr")]
async fn resolve_prompt_token_name(ctx: &PromptIngestContext<'_>, raw_token: &str) -> String {
    if let Some(legacy) = ctx.legacy_token {
        if !legacy.is_empty() && raw_token == legacy {
            return "legacy".to_string();
        }
    }

    let token_hash = crate::auth::token_service::TokenService::hash_token(raw_token);
    match ctx.service_token_repo.find_by_hash(&token_hash).await {
        Ok(Some(token)) => token.name,
        _ => "unknown".to_string(),
    }
}

#[cfg(feature = "ssr")]
async fn validate_sync_token(
    service_token_repo: &dyn crate::db::service_token_repository::ServiceTokenRepository,
    legacy_token: Option<&str>,
    raw_token: &str,
) -> Result<Vec<String>, AppError> {
    if let Some(legacy) = legacy_token {
        if !legacy.is_empty() && raw_token == legacy {
            return Ok(vec!["*".to_string()]);
        }
    }

    let token_hash = crate::auth::token_service::TokenService::hash_token(raw_token);
    let token = service_token_repo
        .find_by_hash(&token_hash)
        .await?
        .ok_or_else(|| AppError::Auth("Invalid service token".into()))?;

    if !token.is_active {
        return Err(AppError::Auth("Service token is deactivated".into()));
    }

    if let Err(err) = service_token_repo.touch_last_used(&token.id).await {
        tracing::warn!(
            "Failed to update last_used_at for token {}: {err}",
            token.id
        );
    }

    Ok(token.allowed_scopes)
}

#[cfg(feature = "ssr")]
fn slug_matches_scopes(slug: &str, scopes: &[String]) -> bool {
    scopes.iter().any(|scope| {
        if scope == "*" {
            return true;
        }
        if let Some(prefix) = scope.strip_suffix("/*") {
            slug == prefix || slug.starts_with(&format!("{prefix}/"))
        } else {
            scope == slug
        }
    })
}

#[cfg(feature = "ssr")]
pub async fn prompt_ingest_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::Json(request): axum::Json<PromptIngestRequest>,
) -> Result<axum::Json<PromptIngestResponse>, AppError> {
    let ctx = PromptIngestContext {
        repo: state.prompt_repo.as_ref(),
        storage: state.storage_client.as_ref(),
        access_level_repo: state.access_level_repo.as_ref(),
        service_token_repo: state.service_token_repo.as_ref(),
        version_repo: state.prompt_version_repo.as_ref(),
        legacy_token: Some(&state.service_token),
    };

    let response = process_prompt_ingest(&ctx, request).await?;
    Ok(axum::Json(response))
}

#[cfg(feature = "ssr")]
pub async fn prompt_sync_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::Json(request): axum::Json<PromptSyncRequest>,
) -> Result<axum::Json<PromptSyncResponse>, AppError> {
    let response = process_prompt_sync(
        state.prompt_repo.as_ref(),
        state.service_token_repo.as_ref(),
        Some(&state.service_token),
        request,
    )
    .await?;
    Ok(axum::Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::Utc;
    use std::sync::Mutex;

    use crate::db::access_level_repository::AccessLevelRepository;
    use crate::db::auth_models::AccessLevelEntity;
    use crate::db::prompt_repository::PromptRepository;
    use crate::db::prompt_version_repository::{PromptVersion, PromptVersionRepository};
    use crate::db::service_token_models::ServiceToken;
    use crate::db::service_token_repository::ServiceTokenRepository;
    use crate::storage::client::StorageClient;
    use crate::test_utils::MockStorage;

    struct MockAccessLevelRepo;

    #[async_trait]
    impl AccessLevelRepository for MockAccessLevelRepo {
        async fn create(&self, _level: AccessLevelEntity) -> Result<(), AppError> {
            Ok(())
        }
        async fn find_by_name(&self, _name: &str) -> Result<Option<AccessLevelEntity>, AppError> {
            Ok(None)
        }
        async fn list_all(&self) -> Result<Vec<AccessLevelEntity>, AppError> {
            Ok(vec![])
        }
        async fn update(&self, _level: AccessLevelEntity) -> Result<(), AppError> {
            Ok(())
        }
        async fn delete(&self, _name: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn exists(&self, name: &str) -> Result<bool, AppError> {
            Ok(!name.trim().is_empty())
        }
        async fn seed_defaults(&self) -> Result<(), AppError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct MockPromptRepo {
        prompts: Mutex<Vec<Prompt>>,
    }

    #[async_trait]
    impl PromptRepository for MockPromptRepo {
        async fn create_or_update(&self, prompt: Prompt) -> Result<(), AppError> {
            let mut prompts = self.prompts.lock().unwrap();
            prompts.retain(|existing| existing.slug != prompt.slug);
            prompts.push(prompt);
            Ok(())
        }

        async fn find_by_slug(&self, slug: &str) -> Result<Option<Prompt>, AppError> {
            Ok(self
                .prompts
                .lock()
                .unwrap()
                .iter()
                .find(|p| p.slug == slug)
                .cloned())
        }

        async fn list_by_access_levels(
            &self,
            _allowed_levels: Option<&[String]>,
            _include_draft: bool,
        ) -> Result<Vec<Prompt>, AppError> {
            Ok(self.prompts.lock().unwrap().clone())
        }

        async fn find_by_slug_prefix(&self, prefix: &str) -> Result<Vec<Prompt>, AppError> {
            Ok(self
                .prompts
                .lock()
                .unwrap()
                .iter()
                .filter(|prompt| {
                    !prompt.is_archived
                        && (prefix.is_empty()
                            || prompt.slug == prefix
                            || prompt.slug.starts_with(&format!("{prefix}/")))
                })
                .cloned()
                .collect())
        }

        async fn set_archived(&self, slug: &str, archived: bool) -> Result<(), AppError> {
            if let Some(prompt) = self
                .prompts
                .lock()
                .unwrap()
                .iter_mut()
                .find(|p| p.slug == slug)
            {
                prompt.is_archived = archived;
            }
            Ok(())
        }

        async fn search_metadata(
            &self,
            _query: &str,
            _allowed_levels: Option<&[String]>,
            _include_draft: bool,
            _limit: usize,
        ) -> Result<Vec<Prompt>, AppError> {
            Ok(vec![])
        }
    }

    #[derive(Default)]
    struct MockPromptVersionRepo {
        versions: Mutex<Vec<PromptVersion>>,
    }

    #[async_trait]
    impl PromptVersionRepository for MockPromptVersionRepo {
        async fn create(&self, version: PromptVersion) -> Result<(), AppError> {
            self.versions.lock().unwrap().push(version);
            Ok(())
        }

        async fn find_latest(&self, slug: &str) -> Result<Option<PromptVersion>, AppError> {
            Ok(self
                .versions
                .lock()
                .unwrap()
                .iter()
                .filter(|version| version.slug == slug)
                .max_by_key(|version| version.version)
                .cloned())
        }

        async fn list_by_slug(&self, slug: &str) -> Result<Vec<PromptVersion>, AppError> {
            let mut versions: Vec<_> = self
                .versions
                .lock()
                .unwrap()
                .iter()
                .filter(|version| version.slug == slug)
                .cloned()
                .collect();
            versions.sort_by_key(|version| std::cmp::Reverse(version.version));
            Ok(versions)
        }

        async fn next_version_number(&self, slug: &str) -> Result<u64, AppError> {
            Ok(self
                .find_latest(slug)
                .await?
                .map_or(1, |version| version.version + 1))
        }
    }

    struct MockServiceTokenRepo {
        token: ServiceToken,
        touched: Mutex<Vec<String>>,
    }

    impl MockServiceTokenRepo {
        fn new() -> Self {
            Self {
                token: ServiceToken {
                    id: "token-1".into(),
                    name: "prompts-ci".into(),
                    token_hash: crate::auth::token_service::TokenService::hash_token("test-token"),
                    allowed_scopes: vec!["prompts/*".into()],
                    token_type: "service".into(),
                    user_id: None,
                    can_write: true,
                    created_by: "admin".into(),
                    created_at: Utc::now(),
                    last_used_at: None,
                    is_active: true,
                },
                touched: Mutex::new(vec![]),
            }
        }
    }

    #[async_trait]
    impl ServiceTokenRepository for MockServiceTokenRepo {
        async fn create(&self, _token: ServiceToken) -> Result<(), AppError> {
            Ok(())
        }
        async fn find_by_id(&self, _id: &str) -> Result<Option<ServiceToken>, AppError> {
            Ok(None)
        }
        async fn find_by_name(&self, _name: &str) -> Result<Option<ServiceToken>, AppError> {
            Ok(None)
        }
        async fn find_by_hash(&self, hash: &str) -> Result<Option<ServiceToken>, AppError> {
            Ok((self.token.token_hash == hash).then(|| self.token.clone()))
        }
        async fn list_all(&self) -> Result<Vec<ServiceToken>, AppError> {
            Ok(vec![self.token.clone()])
        }
        async fn deactivate(&self, _id: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn touch_last_used(&self, id: &str) -> Result<(), AppError> {
            self.touched.lock().unwrap().push(id.to_string());
            Ok(())
        }
        async fn check_scope_overlap(
            &self,
            _scopes: &[String],
            _exclude_id: Option<&str>,
        ) -> Result<bool, AppError> {
            Ok(false)
        }
        async fn set_active(&self, _id: &str, _active: bool) -> Result<(), AppError> {
            Ok(())
        }
        async fn list_by_user_id(&self, _user_id: &str) -> Result<Vec<ServiceToken>, AppError> {
            Ok(vec![])
        }
        async fn list_pats_paginated(
            &self,
            _page: u64,
            _per_page: u64,
        ) -> Result<(Vec<ServiceToken>, u64), AppError> {
            Ok((vec![], 0))
        }
        async fn delete_pat(&self, _id: &str, _user_id: &str) -> Result<(), AppError> {
            Ok(())
        }
    }

    fn make_request() -> PromptIngestRequest {
        PromptIngestRequest {
            service_token: "test-token".into(),
            slug: "prompts/code-review".into(),
            name: "Code Review".into(),
            description: "Review a diff".into(),
            prompt_body: "Review this patch".into(),
            access_level: "internal".into(),
            status: PromptStatus::Active,
            owner: "platform".into(),
            tags: vec!["review".into()],
            variables: vec![PromptVariable {
                name: "diff".into(),
                description: "Patch diff".into(),
                required: true,
            }],
            publish_to_mcp: true,
            default_primary: true,
            context_cost: ContextCost::Medium,
        }
    }

    fn make_context<'a>(
        repo: &'a MockPromptRepo,
        storage: &'a MockStorage,
        access_levels: &'a MockAccessLevelRepo,
        tokens: &'a MockServiceTokenRepo,
        versions: &'a MockPromptVersionRepo,
    ) -> PromptIngestContext<'a> {
        PromptIngestContext {
            repo,
            storage,
            access_level_repo: access_levels,
            service_token_repo: tokens,
            version_repo: versions,
            legacy_token: None,
        }
    }

    #[tokio::test]
    async fn prompt_ingest_stores_hashes_and_blob() {
        let repo = MockPromptRepo::default();
        let storage = MockStorage::new();
        let access_levels = MockAccessLevelRepo;
        let tokens = MockServiceTokenRepo::new();
        let versions = MockPromptVersionRepo::default();
        let ctx = make_context(&repo, &storage, &access_levels, &tokens, &versions);

        let response = process_prompt_ingest(&ctx, make_request()).await.unwrap();

        assert!(response.changed);
        let prompt = repo
            .find_by_slug("prompts/code-review")
            .await
            .unwrap()
            .unwrap();
        assert!(prompt.content_hash.unwrap().starts_with("sha256:"));
        assert!(prompt.metadata_hash.unwrap().starts_with("sha256:"));
        let stored = storage.get_object(&response.s3_key).await.unwrap().unwrap();
        let yaml = String::from_utf8(stored).unwrap();
        assert!(yaml.contains("prompt_body: Review this patch"));
    }

    #[tokio::test]
    async fn prompt_ingest_unchanged_returns_changed_false() {
        let repo = MockPromptRepo::default();
        let storage = MockStorage::new();
        let access_levels = MockAccessLevelRepo;
        let tokens = MockServiceTokenRepo::new();
        let versions = MockPromptVersionRepo::default();
        let ctx = make_context(&repo, &storage, &access_levels, &tokens, &versions);

        process_prompt_ingest(&ctx, make_request()).await.unwrap();
        let response = process_prompt_ingest(&ctx, make_request()).await.unwrap();

        assert!(!response.changed);
        assert_eq!(
            versions
                .list_by_slug("prompts/code-review")
                .await
                .unwrap()
                .len(),
            0
        );
    }

    #[tokio::test]
    async fn prompt_ingest_body_change_creates_version() {
        let repo = MockPromptRepo::default();
        let storage = MockStorage::new();
        let access_levels = MockAccessLevelRepo;
        let tokens = MockServiceTokenRepo::new();
        let versions = MockPromptVersionRepo::default();
        let ctx = make_context(&repo, &storage, &access_levels, &tokens, &versions);

        process_prompt_ingest(&ctx, make_request()).await.unwrap();

        let mut updated = make_request();
        updated.prompt_body = "Review this patch carefully".into();
        process_prompt_ingest(&ctx, updated).await.unwrap();

        let history = versions.list_by_slug("prompts/code-review").await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].version, 1);
        let archived = storage
            .get_object(&history[0].s3_key)
            .await
            .unwrap()
            .unwrap();
        let archived_yaml = String::from_utf8(archived).unwrap();
        assert!(archived_yaml.contains("prompt_body: Review this patch"));
    }

    #[tokio::test]
    async fn prompt_sync_detects_upload_archive_and_unchanged() {
        let repo = MockPromptRepo::default();
        let prompt = Prompt {
            slug: "prompts/code-review".into(),
            name: "Code Review".into(),
            description: "Review a diff".into(),
            s3_key: "prompts/prompts_code-review.yaml".into(),
            access_level: "internal".into(),
            status: PromptStatus::Active,
            owner: "platform".into(),
            last_updated: Utc::now(),
            tags: vec![],
            variables: vec![],
            publish_to_mcp: true,
            default_primary: true,
            context_cost: ContextCost::Medium,
            content_hash: Some("sha256:a".into()),
            metadata_hash: Some("sha256:m1".into()),
            is_archived: false,
        };
        let archived_candidate = Prompt {
            slug: "prompts/old".into(),
            name: "Old".into(),
            description: "Old prompt".into(),
            s3_key: "prompts/prompts_old.yaml".into(),
            access_level: "internal".into(),
            status: PromptStatus::Active,
            owner: "platform".into(),
            last_updated: Utc::now(),
            tags: vec![],
            variables: vec![],
            publish_to_mcp: false,
            default_primary: false,
            context_cost: ContextCost::Low,
            content_hash: Some("sha256:old".into()),
            metadata_hash: Some("sha256:oldmeta".into()),
            is_archived: false,
        };
        repo.create_or_update(prompt).await.unwrap();
        repo.create_or_update(archived_candidate).await.unwrap();

        let tokens = MockServiceTokenRepo::new();
        let response = process_prompt_sync(
            &repo,
            &tokens,
            None,
            PromptSyncRequest {
                service_token: "test-token".into(),
                prompts: vec![
                    PromptSyncEntry {
                        slug: "prompts/code-review".into(),
                        content_hash: "sha256:a".into(),
                        metadata_hash: Some("sha256:m1".into()),
                    },
                    PromptSyncEntry {
                        slug: "prompts/new".into(),
                        content_hash: "sha256:new".into(),
                        metadata_hash: Some("sha256:newmeta".into()),
                    },
                ],
                archive_missing: true,
            },
        )
        .await
        .unwrap();

        assert_eq!(response.unchanged, vec!["prompts/code-review"]);
        assert_eq!(response.to_upload, vec!["prompts/new"]);
        assert_eq!(response.to_archive, vec!["prompts/old"]);
        assert!(
            repo.find_by_slug("prompts/old")
                .await
                .unwrap()
                .unwrap()
                .is_archived
        );
    }

    #[tokio::test]
    async fn prompt_ingest_rejects_duplicate_variables() {
        let repo = MockPromptRepo::default();
        let storage = MockStorage::new();
        let access_levels = MockAccessLevelRepo;
        let tokens = MockServiceTokenRepo::new();
        let versions = MockPromptVersionRepo::default();
        let ctx = make_context(&repo, &storage, &access_levels, &tokens, &versions);

        let mut request = make_request();
        request.variables.push(PromptVariable {
            name: "diff".into(),
            description: "Duplicate".into(),
            required: true,
        });

        let err = process_prompt_ingest(&ctx, request).await.unwrap_err();
        assert!(
            matches!(err, AppError::BadRequest(msg) if msg.contains("Duplicate prompt variable"))
        );
    }
}
