use crate::db::service_token_repository::ServiceTokenRepository;
use crate::error::AppError;

/// Validate a raw service token against the legacy env-var token and/or the
/// database of scoped service tokens.
///
/// Returns `Ok(())` when the token is valid (either legacy match or active DB
/// token). Returns an appropriate `AppError` otherwise.
pub async fn validate_service_token(
    service_token_repo: &dyn ServiceTokenRepository,
    legacy_token: Option<&str>,
    raw_token: &str,
) -> Result<(), AppError> {
    // 1. Legacy token bypass
    if let Some(legacy) = legacy_token {
        if !legacy.is_empty() && raw_token == legacy {
            return Ok(());
        }
    }

    // 2. Look up scoped token by hash
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

    Ok(())
}
