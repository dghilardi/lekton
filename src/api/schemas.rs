use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::db::access_level_repository::AccessLevelRepository;
#[cfg(feature = "ssr")]
use crate::db::models::{Schema, SchemaVersion};
#[cfg(feature = "ssr")]
use crate::db::schema_repository::SchemaRepository;
#[cfg(feature = "ssr")]
use crate::error::AppError;
#[cfg(feature = "ssr")]
use crate::storage::client::StorageClient;

/// Request payload for ingesting a schema version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestSchemaRequest {
    pub service_token: String,
    pub name: String,
    pub schema_type: String,
    pub version: String,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default = "default_public")]
    pub access_level: String,
    #[serde(default)]
    pub service_owner: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub content: String,
}

fn default_status() -> String {
    "stable".to_string()
}

fn default_public() -> String {
    "public".to_string()
}

/// Response from a successful schema ingestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestSchemaResponse {
    pub message: String,
    pub name: String,
    pub version: String,
    pub s3_key: String,
    #[serde(default = "default_true")]
    pub changed: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct SchemaSyncEntry {
    pub name: String,
    pub version: String,
    pub content_hash: String,
    #[serde(default)]
    pub metadata_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SchemaSyncRequest {
    pub service_token: String,
    pub schemas: Vec<SchemaSyncEntry>,
    #[serde(default)]
    pub archive_missing: bool,
}

#[derive(Debug, Serialize)]
pub struct SchemaSyncResponse {
    pub to_upload: Vec<String>,
    pub to_archive: Vec<String>,
    pub unchanged: Vec<String>,
}

/// Response for listing schemas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaListItem {
    pub name: String,
    pub schema_type: String,
    pub service_owner: String,
    pub latest_version: Option<String>,
    pub version_count: usize,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Response for a single schema with all versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaDetail {
    pub name: String,
    pub schema_type: String,
    pub service_owner: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub versions: Vec<SchemaVersionInfo>,
}

/// Version info returned in API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaVersionInfo {
    pub version: String,
    pub status: String,
    pub access_level: String,
    pub s3_key: String,
}

#[cfg(feature = "ssr")]
const VALID_SCHEMA_TYPES: &[&str] = &["openapi", "asyncapi", "jsonschema"];
#[cfg(feature = "ssr")]
const VALID_STATUSES: &[&str] = &["stable", "beta", "deprecated"];

#[cfg(feature = "ssr")]
pub struct SchemaIngestContext<'a> {
    pub schema_repo: &'a dyn SchemaRepository,
    pub storage: &'a dyn StorageClient,
    pub access_level_repo: &'a dyn AccessLevelRepository,
    pub service_token_repo: &'a dyn crate::db::service_token_repository::ServiceTokenRepository,
    pub legacy_token: Option<&'a str>,
}

#[cfg(feature = "ssr")]
fn schema_version_ref(name: &str, version: &str) -> String {
    format!("{name}@{version}")
}

#[cfg(feature = "ssr")]
fn schema_level_visible(access_level: &str, allowed_levels: Option<&[String]>) -> bool {
    match allowed_levels {
        None => true,
        Some(levels) => levels.iter().any(|level| level == access_level),
    }
}

#[cfg(feature = "ssr")]
fn visible_versions<'a>(
    schema: &'a Schema,
    allowed_levels: Option<&[String]>,
) -> Vec<&'a SchemaVersion> {
    schema
        .versions
        .iter()
        .filter(|version| !version.is_archived)
        .filter(|version| schema_level_visible(&version.access_level, allowed_levels))
        .collect()
}

#[cfg(feature = "ssr")]
fn compute_schema_content_hash(content: &str) -> String {
    format!(
        "sha256:{}",
        crate::auth::token_service::TokenService::hash_token(content)
    )
}

#[cfg(feature = "ssr")]
pub fn compute_schema_metadata_hash(status: &str, access_level: &str) -> String {
    let canonical = format!(
        "status={status}\naccess_level={}",
        access_level.to_lowercase()
    );
    compute_schema_content_hash(&canonical)
}

#[cfg(feature = "ssr")]
async fn validate_schema_token(
    ctx: &SchemaIngestContext<'_>,
    raw_token: &str,
    schema_name: &str,
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

    if !token.matches_slug(schema_name) {
        return Err(AppError::Forbidden(
            "Token does not have access to this schema scope".into(),
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

/// Core schema ingestion logic.
#[cfg(feature = "ssr")]
pub async fn process_schema_ingest(
    ctx: &SchemaIngestContext<'_>,
    request: IngestSchemaRequest,
) -> Result<IngestSchemaResponse, AppError> {
    validate_schema_token(ctx, &request.service_token, &request.name).await?;

    if request.name.trim().is_empty() {
        return Err(AppError::BadRequest("Schema name cannot be empty".into()));
    }
    if request.name.contains("..") {
        return Err(AppError::BadRequest(
            "Schema name must not contain '..'".into(),
        ));
    }
    if request.name.starts_with('/') {
        return Err(AppError::BadRequest(
            "Schema name must not start with '/'".into(),
        ));
    }
    if request.version.trim().is_empty() {
        return Err(AppError::BadRequest("Version cannot be empty".into()));
    }
    if !VALID_SCHEMA_TYPES.contains(&request.schema_type.as_str()) {
        return Err(AppError::BadRequest(format!(
            "Invalid schema type '{}'. Expected: {}",
            request.schema_type,
            VALID_SCHEMA_TYPES.join(", ")
        )));
    }
    if !VALID_STATUSES.contains(&request.status.as_str()) {
        return Err(AppError::BadRequest(format!(
            "Invalid status '{}'. Expected: {}",
            request.status,
            VALID_STATUSES.join(", ")
        )));
    }
    if request.access_level.trim().is_empty() {
        return Err(AppError::BadRequest("Access level cannot be empty".into()));
    }

    let access_level = request.access_level.to_lowercase();
    if !ctx.access_level_repo.exists(&access_level).await? {
        return Err(AppError::BadRequest(format!(
            "Unknown access level: '{access_level}'"
        )));
    }

    let new_content_hash = compute_schema_content_hash(&request.content);
    let new_metadata_hash = compute_schema_metadata_hash(&request.status, &access_level);
    let extension = if request.content.trim_start().starts_with('{') {
        "json"
    } else {
        "yaml"
    };
    let s3_key = format!("schemas/{}/{}.{}", request.name, request.version, extension);

    let existing = ctx.schema_repo.find_by_name(&request.name).await?;
    let existing_version = existing.as_ref().and_then(|schema| {
        schema
            .versions
            .iter()
            .find(|v| v.version == request.version)
    });

    if let Some(existing_schema_type) = existing
        .as_ref()
        .filter(|schema| schema.schema_type != request.schema_type)
        .map(|schema| schema.schema_type.clone())
    {
        return Err(AppError::BadRequest(format!(
            "Schema '{}' already exists with type '{}'",
            request.name, existing_schema_type
        )));
    }

    let content_changed = existing_version.and_then(|version| version.content_hash.as_deref())
        != Some(new_content_hash.as_str());
    let version_metadata_changed = existing_version.is_none_or(|version| {
        version.status != request.status
            || version.access_level != access_level
            || version.is_archived
            || version.metadata_hash.as_deref() != Some(new_metadata_hash.as_str())
    });
    let schema_metadata_changed = existing.as_ref().is_none_or(|schema| {
        schema.service_owner != request.service_owner || schema.tags != request.tags
    });

    if !content_changed && !version_metadata_changed && !schema_metadata_changed {
        return Ok(IngestSchemaResponse {
            message: "Schema unchanged".to_string(),
            name: request.name,
            version: request.version,
            s3_key,
            changed: false,
        });
    }

    if content_changed {
        ctx.storage
            .put_object(&s3_key, request.content.into_bytes())
            .await?;
    }

    let mut versions = existing
        .as_ref()
        .map(|schema| schema.versions.clone())
        .unwrap_or_default();

    let updated_version = SchemaVersion {
        version: request.version.clone(),
        s3_key: s3_key.clone(),
        status: request.status,
        access_level,
        content_hash: Some(new_content_hash),
        metadata_hash: Some(new_metadata_hash),
        is_archived: false,
    };

    if let Some(version) = versions.iter_mut().find(|v| v.version == request.version) {
        *version = updated_version;
    } else {
        versions.push(updated_version);
    }

    let schema = Schema {
        name: request.name.clone(),
        schema_type: request.schema_type,
        service_owner: request.service_owner,
        tags: request.tags,
        versions,
    };
    ctx.schema_repo.create_or_update(schema).await?;

    Ok(IngestSchemaResponse {
        message: "Schema version ingested successfully".to_string(),
        name: request.name,
        version: request.version,
        s3_key,
        changed: true,
    })
}

/// Core sync logic for schemas.
#[cfg(feature = "ssr")]
pub async fn process_schema_sync(
    schema_repo: &dyn SchemaRepository,
    service_token_repo: &dyn crate::db::service_token_repository::ServiceTokenRepository,
    legacy_token: Option<&str>,
    request: SchemaSyncRequest,
) -> Result<SchemaSyncResponse, AppError> {
    use std::collections::HashMap;

    let scopes = crate::api::sync::validate_sync_token(
        service_token_repo,
        legacy_token,
        &request.service_token,
    )
    .await?;

    for entry in &request.schemas {
        if !crate::api::sync::scope_matches_any(&entry.name, &scopes) {
            return Err(AppError::Forbidden(format!(
                "Token does not have access to schema '{}'",
                entry.name
            )));
        }
    }

    let mut server_versions: HashMap<String, (String, String, Option<String>, Option<String>)> =
        HashMap::new();
    for scope in &scopes {
        if scope == "*" {
            for schema in schema_repo.find_by_name_prefix("").await? {
                for version in schema.versions.into_iter().filter(|v| !v.is_archived) {
                    server_versions.insert(
                        schema_version_ref(&schema.name, &version.version),
                        (
                            schema.name.clone(),
                            version.version.clone(),
                            version.content_hash,
                            version.metadata_hash,
                        ),
                    );
                }
            }
        } else if let Some(prefix) = scope.strip_suffix("/*") {
            for schema in schema_repo.find_by_name_prefix(prefix).await? {
                for version in schema.versions.into_iter().filter(|v| !v.is_archived) {
                    server_versions.insert(
                        schema_version_ref(&schema.name, &version.version),
                        (
                            schema.name.clone(),
                            version.version.clone(),
                            version.content_hash,
                            version.metadata_hash,
                        ),
                    );
                }
            }
        } else if let Some(schema) = schema_repo.find_by_name(scope).await? {
            for version in schema.versions.into_iter().filter(|v| !v.is_archived) {
                server_versions.insert(
                    schema_version_ref(&schema.name, &version.version),
                    (
                        schema.name.clone(),
                        version.version.clone(),
                        version.content_hash,
                        version.metadata_hash,
                    ),
                );
            }
        }
    }

    let client_versions: HashMap<String, &SchemaSyncEntry> = request
        .schemas
        .iter()
        .map(|entry| (schema_version_ref(&entry.name, &entry.version), entry))
        .collect();

    let mut to_upload = Vec::new();
    let mut unchanged = Vec::new();
    let mut to_archive = Vec::new();

    for entry in &request.schemas {
        let version_ref = schema_version_ref(&entry.name, &entry.version);
        match server_versions.get(&version_ref) {
            Some((_, _, server_content_hash, server_metadata_hash)) => {
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
                    unchanged.push(version_ref);
                } else {
                    to_upload.push(version_ref);
                }
            }
            None => to_upload.push(version_ref),
        }
    }

    for (version_ref, (name, version, _, _)) in &server_versions {
        if !client_versions.contains_key(version_ref) {
            to_archive.push(version_ref.clone());
            if request.archive_missing {
                schema_repo
                    .set_version_archived(name, version, true)
                    .await?;
            }
        }
    }

    to_upload.sort();
    unchanged.sort();
    to_archive.sort();

    Ok(SchemaSyncResponse {
        to_upload,
        to_archive,
        unchanged,
    })
}

/// Core logic to list all schemas visible to the caller.
#[cfg(feature = "ssr")]
pub async fn process_list_schemas(
    schema_repo: &dyn SchemaRepository,
    allowed_levels: Option<&[String]>,
) -> Result<Vec<SchemaListItem>, AppError> {
    let schemas = schema_repo.list_all().await?;

    Ok(schemas
        .into_iter()
        .filter_map(|schema| {
            let visible = visible_versions(&schema, allowed_levels);
            if visible.is_empty() {
                return None;
            }

            let latest = visible
                .iter()
                .rfind(|v| v.status != "deprecated")
                .or_else(|| visible.last())
                .map(|v| v.version.clone());

            Some(SchemaListItem {
                name: schema.name.clone(),
                schema_type: schema.schema_type.clone(),
                service_owner: schema.service_owner.clone(),
                latest_version: latest,
                version_count: visible.len(),
                tags: schema.tags.clone(),
            })
        })
        .collect())
}

/// Core logic to get a schema's details.
#[cfg(feature = "ssr")]
pub async fn process_get_schema(
    schema_repo: &dyn SchemaRepository,
    name: &str,
    allowed_levels: Option<&[String]>,
) -> Result<SchemaDetail, AppError> {
    let schema = schema_repo.find_by_name(name).await?;
    let schema =
        schema.ok_or_else(|| AppError::NotFound(format!("Schema '{}' not found", name)))?;

    let versions = visible_versions(&schema, allowed_levels);
    if versions.is_empty() {
        return Err(AppError::NotFound(format!("Schema '{}' not found", name)));
    }

    Ok(SchemaDetail {
        name: schema.name.clone(),
        schema_type: schema.schema_type.clone(),
        service_owner: schema.service_owner.clone(),
        tags: schema.tags.clone(),
        versions: versions
            .into_iter()
            .map(|v| SchemaVersionInfo {
                version: v.version.clone(),
                status: v.status.clone(),
                access_level: v.access_level.clone(),
                s3_key: v.s3_key.clone(),
            })
            .collect(),
    })
}

/// Core logic to get a specific schema version's content from S3.
#[cfg(feature = "ssr")]
pub async fn process_get_schema_content(
    schema_repo: &dyn SchemaRepository,
    storage: &dyn StorageClient,
    name: &str,
    version: &str,
    allowed_levels: Option<&[String]>,
) -> Result<String, AppError> {
    let schema = schema_repo.find_by_name(name).await?;
    let schema =
        schema.ok_or_else(|| AppError::NotFound(format!("Schema '{}' not found", name)))?;

    let ver = schema
        .versions
        .iter()
        .find(|v| v.version == version && !v.is_archived)
        .filter(|v| schema_level_visible(&v.access_level, allowed_levels))
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "Version '{}' not found for schema '{}'",
                version, name
            ))
        })?;

    let content_bytes = storage.get_object(&ver.s3_key).await?;
    let content_bytes = content_bytes.ok_or_else(|| {
        AppError::NotFound(format!(
            "Schema content not found in storage for '{}/{}'",
            name, version
        ))
    })?;

    String::from_utf8(content_bytes)
        .map_err(|e| AppError::Internal(format!("Invalid UTF-8 in schema content: {e}")))
}

#[cfg(feature = "ssr")]
async fn schema_visibility_from_request(
    state: &crate::app::AppState,
    user: Option<&crate::auth::models::AuthenticatedUser>,
) -> Result<Option<Vec<String>>, AppError> {
    match user {
        Some(user) if user.is_admin => Ok(None),
        Some(user) if state.demo_mode && user.user_id.starts_with("demo-") => {
            Ok(Some(vec!["public".to_string()]))
        }
        Some(user) => {
            let user_doc = state.user_repo.find_user_by_id(&user.user_id).await?;
            let effective = user_doc
                .map(|u| {
                    let mut levels = u.effective_access_levels;
                    if !levels.contains(&"loggeduser".to_string()) {
                        levels.push("loggeduser".to_string());
                    }
                    levels
                })
                .unwrap_or_else(|| vec!["loggeduser".to_string()]);
            Ok(Some(effective))
        }
        None => Ok(Some(vec!["public".to_string()])),
    }
}

/// Axum handler for `POST /api/v1/schemas`.
#[cfg(feature = "ssr")]
pub async fn ingest_schema_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::Json(request): axum::Json<IngestSchemaRequest>,
) -> Result<axum::Json<IngestSchemaResponse>, AppError> {
    let response = process_schema_ingest(
        &SchemaIngestContext {
            schema_repo: state.schema_repo.as_ref(),
            storage: state.storage_client.as_ref(),
            access_level_repo: state.access_level_repo.as_ref(),
            service_token_repo: state.service_token_repo.as_ref(),
            legacy_token: Some(&state.service_token),
        },
        request,
    )
    .await?;

    Ok(axum::Json(response))
}

/// Axum handler for `POST /api/v1/schemas/sync`.
#[cfg(feature = "ssr")]
pub async fn schema_sync_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::Json(request): axum::Json<SchemaSyncRequest>,
) -> Result<axum::Json<SchemaSyncResponse>, AppError> {
    let response = process_schema_sync(
        state.schema_repo.as_ref(),
        state.service_token_repo.as_ref(),
        Some(&state.service_token),
        request,
    )
    .await?;

    Ok(axum::Json(response))
}

/// Axum handler for `GET /api/v1/schemas`.
#[cfg(feature = "ssr")]
pub async fn list_schemas_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    crate::auth::extractor::OptionalAuthUser(user): crate::auth::extractor::OptionalAuthUser,
) -> Result<axum::Json<Vec<SchemaListItem>>, AppError> {
    let allowed_levels = schema_visibility_from_request(&state, user.as_ref()).await?;
    let result =
        process_list_schemas(state.schema_repo.as_ref(), allowed_levels.as_deref()).await?;
    Ok(axum::Json(result))
}

/// Axum handler for `GET /api/v1/schemas/:name`.
#[cfg(feature = "ssr")]
pub async fn get_schema_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    crate::auth::extractor::OptionalAuthUser(user): crate::auth::extractor::OptionalAuthUser,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<axum::Json<SchemaDetail>, AppError> {
    let allowed_levels = schema_visibility_from_request(&state, user.as_ref()).await?;
    let result =
        process_get_schema(state.schema_repo.as_ref(), &name, allowed_levels.as_deref()).await?;
    Ok(axum::Json(result))
}

/// Axum handler for `GET /api/v1/schemas/:name/:version`.
#[cfg(feature = "ssr")]
pub async fn get_schema_version_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    crate::auth::extractor::OptionalAuthUser(user): crate::auth::extractor::OptionalAuthUser,
    axum::extract::Path((name, version)): axum::extract::Path<(String, String)>,
) -> Result<String, AppError> {
    let allowed_levels = schema_visibility_from_request(&state, user.as_ref()).await?;
    process_get_schema_content(
        state.schema_repo.as_ref(),
        state.storage_client.as_ref(),
        &name,
        &version,
        allowed_levels.as_deref(),
    )
    .await
}

/// Catch-all raw schema handler supporting schema names that contain `/`.
///
/// Resolution order:
/// 1. Treat the full path as the schema name and return schema detail.
/// 2. If not found, split on the last `/` and treat the suffix as `version`.
#[cfg(feature = "ssr")]
pub async fn get_schema_route_handler(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    crate::auth::extractor::OptionalAuthUser(user): crate::auth::extractor::OptionalAuthUser,
    axum::extract::Path(rest): axum::extract::Path<String>,
) -> Result<axum::response::Response, AppError> {
    use axum::response::IntoResponse;

    let allowed_levels = schema_visibility_from_request(&state, user.as_ref()).await?;

    match process_get_schema(state.schema_repo.as_ref(), &rest, allowed_levels.as_deref()).await {
        Ok(detail) => return Ok(axum::Json(detail).into_response()),
        Err(AppError::NotFound(_)) => {}
        Err(err) => return Err(err),
    }

    let Some((name, version)) = rest.rsplit_once('/') else {
        return Err(AppError::NotFound(format!("Schema '{}' not found", rest)));
    };

    let content = process_get_schema_content(
        state.schema_repo.as_ref(),
        state.storage_client.as_ref(),
        name,
        version,
        allowed_levels.as_deref(),
    )
    .await?;

    Ok(content.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    use crate::db::auth_models::AccessLevelEntity;
    use crate::db::service_token_models::ServiceToken;
    use crate::db::service_token_repository::ServiceTokenRepository;
    use crate::test_utils::MockStorage;
    use chrono::Utc;

    struct MockAccessLevelRepo;

    #[async_trait]
    impl AccessLevelRepository for MockAccessLevelRepo {
        async fn create(&self, _: AccessLevelEntity) -> Result<(), AppError> {
            Ok(())
        }
        async fn find_by_name(&self, name: &str) -> Result<Option<AccessLevelEntity>, AppError> {
            Ok(if name.is_empty() {
                None
            } else {
                Some(AccessLevelEntity {
                    name: name.to_string(),
                    label: name.to_string(),
                    description: String::new(),
                    inherits_from: vec![],
                    is_system: false,
                    created_at: Utc::now(),
                })
            })
        }
        async fn list_all(&self) -> Result<Vec<AccessLevelEntity>, AppError> {
            Ok(vec![])
        }
        async fn update(&self, _: AccessLevelEntity) -> Result<(), AppError> {
            Ok(())
        }
        async fn delete(&self, _: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn exists(&self, name: &str) -> Result<bool, AppError> {
            Ok(!name.trim().is_empty() && name != "unknown")
        }
        async fn seed_defaults(&self) -> Result<(), AppError> {
            Ok(())
        }
        async fn compute_effective_levels(
            &self,
            roots: &[String],
        ) -> Result<Vec<String>, AppError> {
            Ok(roots.to_vec())
        }
    }

    struct MockServiceTokenRepo;

    #[async_trait]
    impl ServiceTokenRepository for MockServiceTokenRepo {
        async fn create(&self, _: ServiceToken) -> Result<(), AppError> {
            unimplemented!()
        }
        async fn find_by_hash(&self, _: &str) -> Result<Option<ServiceToken>, AppError> {
            Ok(None)
        }
        async fn find_by_name(&self, _: &str) -> Result<Option<ServiceToken>, AppError> {
            Ok(None)
        }
        async fn find_by_id(&self, _: &str) -> Result<Option<ServiceToken>, AppError> {
            Ok(None)
        }
        async fn list_all(&self) -> Result<Vec<ServiceToken>, AppError> {
            Ok(vec![])
        }
        async fn deactivate(&self, _: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn touch_last_used(&self, _: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn check_scope_overlap(
            &self,
            _: &[String],
            _: Option<&str>,
        ) -> Result<bool, AppError> {
            Ok(false)
        }
        async fn set_active(&self, _: &str, _: bool) -> Result<(), AppError> {
            Ok(())
        }
        async fn list_by_user_id(&self, _: &str) -> Result<Vec<ServiceToken>, AppError> {
            Ok(vec![])
        }
        async fn list_pats_paginated(
            &self,
            _: u64,
            _: u64,
        ) -> Result<(Vec<ServiceToken>, u64), AppError> {
            Ok((vec![], 0))
        }
        async fn delete_pat(&self, _: &str, _: &str) -> Result<(), AppError> {
            Ok(())
        }
    }

    struct MockSchemaRepo {
        schemas: Mutex<Vec<Schema>>,
    }

    impl MockSchemaRepo {
        fn new() -> Self {
            Self {
                schemas: Mutex::new(vec![]),
            }
        }
    }

    #[async_trait]
    impl SchemaRepository for MockSchemaRepo {
        async fn create_or_update(&self, schema: Schema) -> Result<(), AppError> {
            let mut schemas = self.schemas.lock().unwrap();
            schemas.retain(|s| s.name != schema.name);
            schemas.push(schema);
            Ok(())
        }

        async fn find_by_name(&self, name: &str) -> Result<Option<Schema>, AppError> {
            Ok(self
                .schemas
                .lock()
                .unwrap()
                .iter()
                .find(|s| s.name == name)
                .cloned())
        }

        async fn list_all(&self) -> Result<Vec<Schema>, AppError> {
            Ok(self.schemas.lock().unwrap().clone())
        }

        async fn find_by_name_prefix(&self, prefix: &str) -> Result<Vec<Schema>, AppError> {
            let schemas = self.schemas.lock().unwrap();
            Ok(schemas
                .iter()
                .filter(|schema| {
                    prefix.is_empty()
                        || schema.name == prefix
                        || schema.name.starts_with(&format!("{prefix}/"))
                })
                .cloned()
                .collect())
        }

        async fn add_version(
            &self,
            schema_name: &str,
            version: SchemaVersion,
        ) -> Result<(), AppError> {
            let mut schemas = self.schemas.lock().unwrap();
            let schema = schemas
                .iter_mut()
                .find(|s| s.name == schema_name)
                .ok_or_else(|| AppError::NotFound(format!("Schema '{}' not found", schema_name)))?;

            if schema.versions.iter().any(|v| v.version == version.version) {
                return Err(AppError::BadRequest(format!(
                    "Version '{}' already exists",
                    version.version
                )));
            }

            schema.versions.push(version);
            Ok(())
        }

        async fn set_version_archived(
            &self,
            schema_name: &str,
            version: &str,
            archived: bool,
        ) -> Result<(), AppError> {
            let mut schemas = self.schemas.lock().unwrap();
            let schema = schemas
                .iter_mut()
                .find(|s| s.name == schema_name)
                .ok_or_else(|| AppError::NotFound(format!("Schema '{}' not found", schema_name)))?;
            let version = schema
                .versions
                .iter_mut()
                .find(|v| v.version == version)
                .ok_or_else(|| AppError::NotFound("Version not found".into()))?;
            version.is_archived = archived;
            Ok(())
        }

        async fn delete(&self, name: &str) -> Result<(), AppError> {
            let mut schemas = self.schemas.lock().unwrap();
            let len_before = schemas.len();
            schemas.retain(|s| s.name != name);
            if schemas.len() == len_before {
                return Err(AppError::NotFound(format!("Schema '{}' not found", name)));
            }
            Ok(())
        }
    }

    fn make_schema_request(token: &str, name: &str, version: &str) -> IngestSchemaRequest {
        IngestSchemaRequest {
            service_token: token.to_string(),
            name: name.to_string(),
            schema_type: "openapi".to_string(),
            version: version.to_string(),
            status: "stable".to_string(),
            access_level: "public".to_string(),
            service_owner: "payments".to_string(),
            tags: vec!["payments".to_string()],
            content: r#"{"openapi": "3.0.0", "info": {"title": "Test", "version": "1.0.0"}}"#
                .to_string(),
        }
    }

    fn ingest_context<'a>(
        repo: &'a MockSchemaRepo,
        storage: &'a MockStorage,
    ) -> SchemaIngestContext<'a> {
        SchemaIngestContext {
            schema_repo: repo,
            storage,
            access_level_repo: &MockAccessLevelRepo,
            service_token_repo: &MockServiceTokenRepo,
            legacy_token: Some("valid-token"),
        }
    }

    #[tokio::test]
    async fn test_ingest_schema_success() {
        let repo = MockSchemaRepo::new();
        let storage = MockStorage::new();
        let request = make_schema_request("valid-token", "test-api", "1.0.0");

        let result = process_schema_ingest(&ingest_context(&repo, &storage), request).await;

        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.name, "test-api");
        assert_eq!(response.version, "1.0.0");
        assert!(response.s3_key.contains("schemas/test-api/1.0.0"));

        let schema = repo.find_by_name("test-api").await.unwrap().unwrap();
        assert_eq!(schema.schema_type, "openapi");
        assert_eq!(schema.service_owner, "payments");
        assert_eq!(schema.tags, vec!["payments".to_string()]);
        assert_eq!(schema.versions.len(), 1);
        assert_eq!(schema.versions[0].access_level, "public");
        assert!(!schema.versions[0].is_archived);
    }

    #[tokio::test]
    async fn test_ingest_schema_invalid_token() {
        let repo = MockSchemaRepo::new();
        let storage = MockStorage::new();
        let request = make_schema_request("wrong-token", "test-api", "1.0.0");

        let result = process_schema_ingest(&ingest_context(&repo, &storage), request).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Auth(msg) => assert!(msg.contains("Invalid service token")),
            other => panic!("Expected Auth error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ingest_schema_invalid_type() {
        let repo = MockSchemaRepo::new();
        let storage = MockStorage::new();
        let mut request = make_schema_request("valid-token", "test-api", "1.0.0");
        request.schema_type = "graphql".to_string();

        let result = process_schema_ingest(&ingest_context(&repo, &storage), request).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => assert!(msg.contains("Invalid schema type")),
            other => panic!("Expected BadRequest error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ingest_schema_invalid_access_level() {
        let repo = MockSchemaRepo::new();
        let storage = MockStorage::new();
        let mut request = make_schema_request("valid-token", "test-api", "1.0.0");
        request.access_level = "unknown".to_string();

        let result = process_schema_ingest(&ingest_context(&repo, &storage), request).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => assert!(msg.contains("Unknown access level")),
            other => panic!("Expected BadRequest error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ingest_schema_unchanged_short_circuits() {
        let repo = MockSchemaRepo::new();
        let storage = MockStorage::new();
        let request = make_schema_request("valid-token", "test-api", "1.0.0");

        process_schema_ingest(&ingest_context(&repo, &storage), request.clone())
            .await
            .unwrap();
        let result = process_schema_ingest(&ingest_context(&repo, &storage), request)
            .await
            .unwrap();

        assert!(!result.changed);
    }

    #[tokio::test]
    async fn test_schema_sync_archives_missing_versions() {
        let repo = MockSchemaRepo::new();
        let storage = MockStorage::new();
        process_schema_ingest(
            &ingest_context(&repo, &storage),
            make_schema_request("valid-token", "payments/api", "1.0.0"),
        )
        .await
        .unwrap();
        process_schema_ingest(
            &ingest_context(&repo, &storage),
            make_schema_request("valid-token", "payments/api", "2.0.0"),
        )
        .await
        .unwrap();

        let result = process_schema_sync(
            &repo,
            &MockServiceTokenRepo,
            Some("valid-token"),
            SchemaSyncRequest {
                service_token: "valid-token".to_string(),
                schemas: vec![SchemaSyncEntry {
                    name: "payments/api".to_string(),
                    version: "2.0.0".to_string(),
                    content_hash: compute_schema_content_hash(
                        r#"{"openapi": "3.0.0", "info": {"title": "Test", "version": "1.0.0"}}"#,
                    ),
                    metadata_hash: Some(compute_schema_metadata_hash("stable", "public")),
                }],
                archive_missing: true,
            },
        )
        .await
        .unwrap();

        assert_eq!(result.to_archive, vec!["payments/api@1.0.0".to_string()]);
        let schema = repo.find_by_name("payments/api").await.unwrap().unwrap();
        assert!(
            schema
                .versions
                .iter()
                .find(|v| v.version == "1.0.0")
                .unwrap()
                .is_archived
        );
    }

    #[tokio::test]
    async fn test_list_schemas_filters_by_access_level() {
        let repo = MockSchemaRepo::new();
        let storage = MockStorage::new();
        process_schema_ingest(
            &ingest_context(&repo, &storage),
            make_schema_request("valid-token", "public-api", "1.0.0"),
        )
        .await
        .unwrap();
        let mut internal = make_schema_request("valid-token", "internal-api", "1.0.0");
        internal.access_level = "internal".to_string();
        process_schema_ingest(&ingest_context(&repo, &storage), internal)
            .await
            .unwrap();

        let public_only = vec!["public".to_string()];
        let list = process_list_schemas(&repo, Some(&public_only))
            .await
            .unwrap();

        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "public-api");
    }

    #[tokio::test]
    async fn test_get_schema_content_respects_access_level() {
        let repo = MockSchemaRepo::new();
        let storage = MockStorage::new();
        let mut request = make_schema_request("valid-token", "internal-api", "1.0.0");
        request.access_level = "internal".to_string();
        process_schema_ingest(&ingest_context(&repo, &storage), request)
            .await
            .unwrap();

        let public_only = vec!["public".to_string()];
        let result = process_get_schema_content(
            &repo,
            &storage,
            "internal-api",
            "1.0.0",
            Some(&public_only),
        )
        .await;

        assert!(matches!(result, Err(AppError::NotFound(_))));
    }
}
