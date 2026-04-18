//! Prompt Library domain models.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Lifecycle state of a prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptStatus {
    Draft,
    Active,
    Deprecated,
}

/// Estimated context overhead for published prompts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContextCost {
    Low,
    Medium,
    High,
}

impl Default for ContextCost {
    fn default() -> Self {
        Self::Medium
    }
}

impl ContextCost {
    pub fn weight(&self) -> u8 {
        match self {
            Self::Low => 1,
            Self::Medium => 2,
            Self::High => 4,
        }
    }
}

/// Declared input variable for a prompt template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptVariable {
    pub name: String,
    pub description: String,
    #[serde(default = "default_true")]
    pub required: bool,
}

/// Metadata for a prompt stored in MongoDB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prompt {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub s3_key: String,
    pub access_level: String,
    pub status: PromptStatus,
    pub owner: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub last_updated: DateTime<Utc>,
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
    #[serde(default)]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub metadata_hash: Option<String>,
    #[serde(default)]
    pub is_archived: bool,
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_status_serializes_lowercase() {
        let json = serde_json::to_string(&PromptStatus::Deprecated).unwrap();
        assert_eq!(json, "\"deprecated\"");
    }

    #[test]
    fn context_cost_defaults_to_medium() {
        #[derive(Deserialize)]
        struct Wrapper {
            #[serde(default)]
            cost: ContextCost,
        }

        let value: Wrapper = serde_json::from_str("{}").unwrap();
        assert_eq!(value.cost, ContextCost::Medium);
        assert_eq!(value.cost.weight(), 2);
    }

    #[test]
    fn prompt_variable_required_defaults_true() {
        let var: PromptVariable =
            serde_json::from_str(r#"{"name":"input","description":"Input text"}"#).unwrap();
        assert!(var.required);
    }

    #[test]
    fn prompt_defaults_support_older_records() {
        let json = r###"{
            "slug": "engineering/review-rfc",
            "name": "Review RFC",
            "description": "Checks an RFC",
            "s3_key": "prompts/engineering_review-rfc.yaml",
            "access_level": "architect",
            "status": "active",
            "owner": "platform-team",
            "last_updated": { "$date": { "$numberLong": "1704067200000" } }
        }"###;

        let prompt: Prompt = serde_json::from_str(json).unwrap();
        assert!(prompt.tags.is_empty());
        assert!(prompt.variables.is_empty());
        assert!(!prompt.publish_to_mcp);
        assert!(!prompt.default_primary);
        assert_eq!(prompt.context_cost, ContextCost::Medium);
        assert_eq!(prompt.content_hash, None);
        assert!(!prompt.is_archived);
    }
}
