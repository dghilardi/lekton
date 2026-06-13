use crate::db::service_token_repository::ServiceTokenRepository;
use crate::error::AppError;

/// The effective grants of a service token after legacy-bypass or DB lookup.
///
/// This is the single source of truth for service-token resolution shared by
/// the ingest, sync, prompt, schema, and asset handlers — each then applies the
/// policy it needs (scope match, `can_write`, …).
pub struct ResolvedServiceToken {
    /// Scopes this token may act on. The legacy env token resolves to `["*"]`.
    pub scopes: Vec<String>,
    /// Whether the token may create/update content. Read access is implicit.
    pub can_write: bool,
    /// Human-readable token name, for logging. `"legacy"` for the env token.
    pub name: String,
}

/// Resolve a raw service token to its effective grants.
///
/// Accepts the legacy env-var token (full access) or an active scoped token
/// from the database, updating `last_used_at` fire-and-forget. Returns an
/// `AppError` for unknown or deactivated tokens. Callers apply their own scope
/// and `can_write` policy on top of the returned value.
pub async fn resolve_service_token(
    service_token_repo: &dyn ServiceTokenRepository,
    legacy_token: Option<&str>,
    raw_token: &str,
) -> Result<ResolvedServiceToken, AppError> {
    // 1. Legacy token bypass — full access.
    if let Some(legacy) = legacy_token {
        if !legacy.is_empty() && raw_token == legacy {
            return Ok(ResolvedServiceToken {
                scopes: vec!["*".to_string()],
                can_write: true,
                name: "legacy".to_string(),
            });
        }
    }

    // 2. Look up scoped token by hash.
    let token_hash = crate::auth::token_service::TokenService::hash_token(raw_token);
    let token = service_token_repo
        .find_by_hash(&token_hash)
        .await?
        .ok_or_else(|| AppError::Auth("Invalid service token".into()))?;

    if !token.is_active {
        return Err(AppError::Auth("Service token is deactivated".into()));
    }

    // Fire-and-forget last_used update
    if let Err(e) = service_token_repo.touch_last_used(&token.id).await {
        tracing::warn!("Failed to update last_used_at for token {}: {e}", token.id);
    }

    Ok(ResolvedServiceToken {
        scopes: token.allowed_scopes,
        can_write: token.can_write,
        name: token.name,
    })
}

/// Validate a raw service token (legacy or active DB token) without any scope
/// or `can_write` policy. Used where read-level validity is sufficient.
pub async fn validate_service_token(
    service_token_repo: &dyn ServiceTokenRepository,
    legacy_token: Option<&str>,
    raw_token: &str,
) -> Result<(), AppError> {
    resolve_service_token(service_token_repo, legacy_token, raw_token)
        .await
        .map(|_| ())
}
