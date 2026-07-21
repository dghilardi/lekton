use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentationFeedbackKind {
    MissingInfo,
    Improvement,
}

impl DocumentationFeedbackKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MissingInfo => "missing_info",
            Self::Improvement => "improvement",
        }
    }
}

impl std::str::FromStr for DocumentationFeedbackKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "missing_info" => Ok(Self::MissingInfo),
            "improvement" => Ok(Self::Improvement),
            other => Err(format!(
                "Unsupported feedback kind '{other}'. Expected 'missing_info' or 'improvement'"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentationFeedbackStatus {
    Open,
    /// Taken in charge (typically by the documentation agent): a fix is being
    /// delivered via the recorded `delivery_ref`. Excluded from the open queue
    /// so it is not re-worked.
    InProgress,
    Resolved,
}

impl DocumentationFeedbackStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::InProgress => "in_progress",
            Self::Resolved => "resolved",
        }
    }
}

impl std::str::FromStr for DocumentationFeedbackStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "open" => Ok(Self::Open),
            "in_progress" => Ok(Self::InProgress),
            "resolved" => Ok(Self::Resolved),
            other => Err(format!(
                "Unsupported feedback status '{other}'. Expected 'open', 'in_progress', or 'resolved'"
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentationFeedback {
    pub id: String,
    pub kind: DocumentationFeedbackKind,
    pub status: DocumentationFeedbackStatus,
    pub title: String,
    pub summary: String,
    #[serde(default)]
    pub related_resources: Vec<String>,
    #[serde(default)]
    pub search_queries: Vec<String>,
    pub created_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub duplicate_of: Option<String>,
    #[serde(default)]
    pub resolution_note: Option<String>,
    #[serde(default)]
    pub related_feedback_ids: Vec<String>,
    #[serde(default)]
    pub user_goal: Option<String>,
    #[serde(default)]
    pub missing_information: Option<String>,
    #[serde(default)]
    pub impact: Option<String>,
    #[serde(default)]
    pub suggested_target_resource: Option<String>,
    #[serde(default)]
    pub target_resource_uri: Option<String>,
    #[serde(default)]
    pub problem_summary: Option<String>,
    #[serde(default)]
    pub proposal: Option<String>,
    #[serde(default)]
    pub supporting_resources: Vec<String>,
    #[serde(default)]
    pub expected_benefit: Option<String>,
    // ── Claim / delivery metadata (set when status is `in_progress`) ──────────
    /// The `source_id` where the fix is being delivered (may differ from where
    /// the feedback was originally filed). Used to authorize resolution.
    #[serde(default)]
    pub delivery_source_id: Option<String>,
    /// Human/audit reference to the delivery (e.g. the pull request URL).
    #[serde(default)]
    pub delivery_ref: Option<String>,
    /// Per-claim nonce echoed in the resolving commit trailer, so a stale
    /// marker from a previous claim cycle cannot auto-resolve a re-opened item.
    #[serde(default)]
    pub claim_nonce: Option<String>,
    /// When the item was taken in charge.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::db::auth_models::option_bson_datetime"
    )]
    pub claimed_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_from_str_accepts_supported_values() {
        assert_eq!(
            "missing_info".parse::<DocumentationFeedbackKind>().unwrap(),
            DocumentationFeedbackKind::MissingInfo
        );
        assert_eq!(
            "improvement".parse::<DocumentationFeedbackKind>().unwrap(),
            DocumentationFeedbackKind::Improvement
        );
    }

    #[test]
    fn status_from_str_accepts_supported_values() {
        assert_eq!(
            "open".parse::<DocumentationFeedbackStatus>().unwrap(),
            DocumentationFeedbackStatus::Open
        );
        assert_eq!(
            "in_progress"
                .parse::<DocumentationFeedbackStatus>()
                .unwrap(),
            DocumentationFeedbackStatus::InProgress
        );
        assert_eq!(
            "resolved".parse::<DocumentationFeedbackStatus>().unwrap(),
            DocumentationFeedbackStatus::Resolved
        );
        assert_eq!(
            DocumentationFeedbackStatus::InProgress.as_str(),
            "in_progress"
        );
    }
}
