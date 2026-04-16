//! Shared helper for building `async-openai` clients with custom headers.

use std::collections::HashMap;

use async_openai::{config::OpenAIConfig, error::OpenAIError, Client};
use reqwest::header::HeaderName;
use serde_json::Value;

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

/// Format provider errors into a stable user-facing string.
///
/// Some OpenAI-compatible endpoints, notably Vertex AI's OpenAPI bridge,
/// return non-OpenAI error payloads. `async-openai` reports those as a JSON
/// deserialization failure and includes the raw payload in the error. This
/// helper extracts the nested provider message when possible so the caller sees
/// the real cause instead of the deserializer detail.
pub fn format_llm_error(error: &OpenAIError) -> String {
    match error {
        OpenAIError::JSONDeserialize(_, content) => {
            extract_error_message(content).unwrap_or_else(|| error.to_string())
        }
        _ => error.to_string(),
    }
}

fn extract_error_message(content: &str) -> Option<String> {
    let value: Value = serde_json::from_str(content).ok()?;
    find_error_object(&value)
        .and_then(format_error_object)
        .or_else(|| find_message_field(&value).map(ToOwned::to_owned))
}

fn find_error_object(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    match value {
        Value::Object(map) => {
            if let Some(Value::Object(error)) = map.get("error") {
                return Some(error);
            }

            map.values().find_map(find_error_object)
        }
        Value::Array(items) => items.iter().find_map(find_error_object),
        _ => None,
    }
}

fn format_error_object(error: &serde_json::Map<String, Value>) -> Option<String> {
    let message = error.get("message").and_then(Value::as_str)?.trim();
    if message.is_empty() {
        return None;
    }

    let status = error.get("status").and_then(Value::as_str);
    let code = error
        .get("code")
        .and_then(|value| match value {
            Value::String(value) => Some(value.clone()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        });

    let mut parts = Vec::new();

    if let Some(status) = status.filter(|status| !status.is_empty()) {
        parts.push(status.to_string());
    }

    parts.push(message.to_string());

    if let Some(code) = code.filter(|code| !code.is_empty()) {
        parts.push(format!("(code: {code})"));
    }

    Some(parts.join(": ").replace(": (code:", " (code:"))
}

fn find_message_field(value: &Value) -> Option<&str> {
    match value {
        Value::Object(map) => map
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| map.values().find_map(find_message_field)),
        Value::Array(items) => items.iter().find_map(find_message_field),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use async_openai::error::OpenAIError;

    use super::{build_oai_client, format_llm_error};

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

    #[test]
    fn formats_vertex_json_deserialize_error() {
        let raw = r#"[{
          "error": {
            "code": 403,
            "message": "Permission 'aiplatform.endpoints.predict' denied.",
            "status": "PERMISSION_DENIED"
          }
        }]"#;
        let source = serde_json::from_str::<serde_json::Value>("not-json").unwrap_err();
        let error = OpenAIError::JSONDeserialize(source, raw.to_string());

        assert_eq!(
            format_llm_error(&error),
            "PERMISSION_DENIED: Permission 'aiplatform.endpoints.predict' denied. (code: 403)"
        );
    }

    #[test]
    fn preserves_non_json_deserialize_errors() {
        let error = OpenAIError::InvalidArgument("bad request".to_string());

        assert_eq!(format_llm_error(&error), "invalid args: bad request");
    }
}
