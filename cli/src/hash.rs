use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use sha2::{Digest, Sha256};

use crate::config::PromptVariable;

/// Compute the same `sha256:<base64url>` hash format the server uses.
pub fn compute_hash(content: &str) -> String {
    let hash = Sha256::digest(content.as_bytes());
    format!("sha256:{}", URL_SAFE_NO_PAD.encode(hash))
}

/// Compute a metadata hash that matches `ingest::compute_metadata_hash` on the server.
///
/// Including front-matter fields in the hash lets the sync protocol detect
/// metadata-only changes (e.g. `access_level`, `title`, `summary`) without relying on
/// content byte differences.
pub fn compute_metadata_hash(
    title: &str,
    summary: Option<&str>,
    access_level: &str,
    service_owner: &str,
    tags: &[String],
    parent_slug: Option<&str>,
    order: i32,
    is_hidden: bool,
) -> String {
    let mut sorted_tags: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
    sorted_tags.sort_unstable();
    let canonical = format!(
        "title={title}\nsummary={}\naccess_level={}\nservice_owner={service_owner}\ntags={}\nparent_slug={}\norder={order}\nis_hidden={is_hidden}",
        summary.unwrap_or(""),
        access_level.to_lowercase(),
        sorted_tags.join(","),
        parent_slug.unwrap_or(""),
    );
    compute_hash(&canonical)
}

pub fn compute_prompt_metadata_hash(
    name: &str,
    description: &str,
    access_level: &str,
    status: &str,
    owner: &str,
    tags: &[String],
    variables: &[PromptVariable],
    publish_to_mcp: bool,
    default_primary: bool,
    context_cost: &str,
) -> String {
    let mut sorted_tags: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
    sorted_tags.sort_unstable();

    let mut sorted_vars: Vec<String> = variables
        .iter()
        .map(|v| format!("{}:{}:{}", v.name, v.description, v.required))
        .collect();
    sorted_vars.sort_unstable();

    let canonical = format!(
        "name={name}\ndescription={description}\naccess_level={}\nstatus={status}\nowner={owner}\ntags={}\nvariables={}\npublish_to_mcp={publish_to_mcp}\ndefault_primary={default_primary}\ncontext_cost={context_cost}",
        access_level.to_lowercase(),
        sorted_tags.join(","),
        sorted_vars.join("|"),
    );
    compute_hash(&canonical)
}

pub fn compute_schema_metadata_hash(status: &str, access_level: &str) -> String {
    let canonical = format!(
        "status={status}\naccess_level={}",
        access_level.to_lowercase()
    );
    compute_hash(&canonical)
}

/// Compute SHA-256 content hash for a file's bytes.
pub fn compute_file_hash(data: &[u8]) -> String {
    let hash = Sha256::digest(data);
    format!("sha256:{}", URL_SAFE_NO_PAD.encode(hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_format() {
        let h = compute_hash("# Hello");
        assert!(h.starts_with("sha256:"), "hash should start with sha256:");
        assert_eq!(h.len(), "sha256:".len() + 43);
    }

    #[test]
    fn compute_metadata_hash_deterministic() {
        let h1 = compute_metadata_hash(
            "Title",
            Some("Summary"),
            "public",
            "owner",
            &[],
            None,
            0,
            false,
        );
        let h2 = compute_metadata_hash(
            "Title",
            Some("Summary"),
            "public",
            "owner",
            &[],
            None,
            0,
            false,
        );
        assert_eq!(h1, h2);
        assert!(h1.starts_with("sha256:"));
    }

    #[test]
    fn compute_metadata_hash_case_insensitive_access_level() {
        let h1 = compute_metadata_hash("T", None, "Public", "o", &[], None, 0, false);
        let h2 = compute_metadata_hash("T", None, "public", "o", &[], None, 0, false);
        assert_eq!(h1, h2, "access_level comparison should be case-insensitive");
    }

    #[test]
    fn compute_metadata_hash_tags_order_independent() {
        let h1 = compute_metadata_hash(
            "T",
            None,
            "public",
            "o",
            &["b".to_string(), "a".to_string()],
            None,
            0,
            false,
        );
        let h2 = compute_metadata_hash(
            "T",
            None,
            "public",
            "o",
            &["a".to_string(), "b".to_string()],
            None,
            0,
            false,
        );
        assert_eq!(h1, h2, "tag order should not affect hash");
    }
}
