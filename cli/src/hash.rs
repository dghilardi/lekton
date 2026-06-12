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
/// Uses a BTreeMap serialised to JSON so keys are always alphabetically sorted,
/// producing a deterministic canonical form that the server replicates exactly.
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
    use std::collections::BTreeMap;
    let mut sorted_tags: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
    sorted_tags.sort_unstable();
    let tags_str = sorted_tags.join(",");
    let order_str = order.to_string();
    let is_hidden_str = is_hidden.to_string();
    let access_level_lower = access_level.to_lowercase();
    let mut map = BTreeMap::new();
    map.insert("access_level", access_level_lower.as_str());
    map.insert("is_hidden", is_hidden_str.as_str());
    map.insert("order", order_str.as_str());
    map.insert("parent_slug", parent_slug.unwrap_or(""));
    map.insert("service_owner", service_owner);
    map.insert("summary", summary.unwrap_or(""));
    map.insert("tags", tags_str.as_str());
    map.insert("title", title);
    let canonical =
        serde_json::to_string(&map).expect("BTreeMap<&str,&str> serialization is infallible");
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

    /// Regression test for CLI-BUG-1: verifies that the canonical string for prompt hashing
    /// uses lowercase status and context_cost, matching the server's Display-based wire format.
    #[test]
    fn prompt_metadata_hash_canonical_format() {
        let canonical = "name=Code Review\ndescription=Review a diff\naccess_level=internal\nstatus=active\nowner=platform\ntags=review\nvariables=diff:Patch diff:true\npublish_to_mcp=true\ndefault_primary=true\ncontext_cost=medium";
        let expected = compute_hash(canonical);
        let got = compute_prompt_metadata_hash(
            "Code Review",
            "Review a diff",
            "internal",
            "active",
            "platform",
            &["review".to_string()],
            &[PromptVariable {
                name: "diff".to_string(),
                description: "Patch diff".to_string(),
                required: true,
            }],
            true,
            true,
            "medium",
        );
        assert_eq!(
            got, expected,
            "CLI canonical format must match server wire format"
        );
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
