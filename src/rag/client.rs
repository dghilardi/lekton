//! Shared helper for building `async-openai` clients with custom headers.

use std::collections::HashMap;

use async_openai::{config::OpenAIConfig, Client};
use reqwest::header::HeaderName;

use crate::error::AppError;

/// Build an [`async_openai::Client`] from a URL, an optional API key, and an
/// optional map of extra HTTP headers.
///
/// # Header key normalisation
///
/// Underscores (`_`) in header keys are replaced with hyphens (`-`) before the
/// header is added to the request.  This is necessary because environment
/// variables cannot contain hyphens, so a header like `x-producer` must be
/// supplied as `LKN__RAG__CHAT_HEADERS__X_PRODUCER`.  The normalisation makes
/// the two configuration paths (env vars and TOML file) equivalent for the
/// common case where all header names use hyphens.
///
/// If you need a header whose name genuinely contains an underscore, set it via
/// the TOML file using a quoted key: `"x_literal_underscore" = "value"` —
/// which is then **not** normalised by this function (it still replaces `_`
/// with `-`).  Headers with underscores in their names are non-standard and
/// rejected by many HTTP proxies, so this limitation is intentional.
///
/// # Errors
///
/// Returns [`AppError::Internal`] when a header name or value is rejected by
/// the underlying HTTP layer (e.g. contains invalid bytes).
pub fn build_oai_client(
    url: &str,
    api_key: &str,
    extra_headers: &HashMap<String, String>,
) -> Result<Client<OpenAIConfig>, AppError> {
    let mut oai_config = OpenAIConfig::new().with_api_base(url);

    if !api_key.is_empty() {
        oai_config = oai_config.with_api_key(api_key);
    }

    for (raw_key, value) in extra_headers {
        let normalised = raw_key.replace('_', "-");
        // IntoHeaderName is only implemented for `&'static str` and owned `HeaderName`,
        // so we must parse the dynamic string into an owned `HeaderName`.
        let header_name = HeaderName::from_bytes(normalised.as_bytes()).map_err(|e| {
            AppError::Internal(format!(
                "invalid RAG header name '{}' (normalised from '{}'): {}",
                normalised, raw_key, e
            ))
        })?;
        oai_config = oai_config
            .with_header(header_name, value.as_str())
            .map_err(|e| {
                AppError::Internal(format!(
                    "invalid RAG header value for '{}': {}",
                    normalised, e
                ))
            })?;
    }

    Ok(Client::with_config(oai_config))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::build_oai_client;

    #[test]
    fn builds_client_without_headers() {
        let result = build_oai_client("http://localhost:11434/v1", "", &HashMap::new());
        assert!(result.is_ok());
    }

    #[test]
    fn builds_client_with_api_key() {
        let result = build_oai_client(
            "http://localhost:11434/v1",
            "sk-test",
            &HashMap::new(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn normalises_underscores_to_hyphens() {
        // x_producer → x-producer: this is what allows env-var configuration
        // (LKN__RAG__CHAT_HEADERS__X_PRODUCER) to produce a hyphenated header name.
        let mut headers = HashMap::new();
        headers.insert("x_producer".to_string(), "LEKTON".to_string());
        let result = build_oai_client("http://localhost:11434/v1", "", &headers);
        assert!(result.is_ok(), "header with underscore should be accepted after normalisation");
    }

    #[test]
    fn normalises_multiple_underscores() {
        let mut headers = HashMap::new();
        headers.insert("x_custom_header_name".to_string(), "value".to_string());
        let result = build_oai_client("http://localhost:11434/v1", "", &headers);
        assert!(result.is_ok());
    }

    #[test]
    fn accepts_already_hyphenated_key() {
        // Keys set via TOML with quoted names (e.g. "x-producer" = "v") arrive
        // with hyphens already present; normalisation is idempotent.
        let mut headers = HashMap::new();
        headers.insert("x-producer".to_string(), "LEKTON".to_string());
        let result = build_oai_client("http://localhost:11434/v1", "", &headers);
        assert!(result.is_ok());
    }

    #[test]
    fn accepts_multiple_headers() {
        let mut headers = HashMap::new();
        headers.insert("x_producer".to_string(), "LEKTON".to_string());
        headers.insert("x_request_id".to_string(), "abc123".to_string());
        let result = build_oai_client("http://localhost:11434/v1", "", &headers);
        assert!(result.is_ok());
    }
}
