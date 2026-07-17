use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A person responsible for a documentation source.
///
/// Either an external contact (identified by `name`/`email`) or a Lekton user
/// (linked by the stable internal `lekton_user_id`). The UUID link is preferred
/// over email because a user's provider email can change; `email` is retained
/// for external maintainers who have no Lekton account.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Maintainer {
    /// Free-form display name (for external maintainers, or an override).
    #[serde(default)]
    pub name: Option<String>,
    /// Contact email (for external maintainers; also shown for linked users).
    #[serde(default)]
    pub email: Option<String>,
    /// Optional link to a Lekton user by internal UUID. `None` for externals.
    #[serde(default)]
    pub lekton_user_id: Option<String>,
}

impl Maintainer {
    /// A maintainer is valid when it carries at least a contact email or a
    /// Lekton user link — a bare name is not enough to reach anyone.
    pub fn is_valid(&self) -> bool {
        self.email.as_deref().is_some_and(|e| !e.trim().is_empty())
            || self
                .lekton_user_id
                .as_deref()
                .is_some_and(|id| !id.trim().is_empty())
    }
}

/// Admin-curated metadata attached to a documentation import source.
///
/// The set of sources is discovered from the `source_id` values stamped on
/// documents during sync/ingest (the `id` field of a repo's `.lekton.yml`).
/// This record layers repository provenance and ownership on top of that id,
/// so the portal can render "edit on the source repo" links and point tooling
/// at the right repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSource {
    /// Matches the `source_id` stamped on documents (from `.lekton.yml` `id`).
    pub id: String,
    /// Human-friendly label; falls back to `id` in the UI when absent.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Repository URL, e.g. `https://github.com/org/repo`.
    #[serde(default)]
    pub repo_url: Option<String>,
    /// Mainline branch documents are edited against, e.g. `main`.
    #[serde(default)]
    pub mainline_branch: Option<String>,
    /// People responsible for this source.
    #[serde(default)]
    pub maintainers: Vec<Maintainer>,
    /// Free-form notes about the source.
    #[serde(default)]
    pub description: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maintainer_requires_email_or_user_link() {
        assert!(!Maintainer::default().is_valid());
        assert!(!Maintainer {
            name: Some("Jane".into()),
            ..Default::default()
        }
        .is_valid());
        assert!(Maintainer {
            email: Some("jane@example.com".into()),
            ..Default::default()
        }
        .is_valid());
        assert!(Maintainer {
            lekton_user_id: Some("uuid-1".into()),
            ..Default::default()
        }
        .is_valid());
    }

    #[test]
    fn maintainer_blank_values_are_invalid() {
        assert!(!Maintainer {
            email: Some("  ".into()),
            lekton_user_id: Some("".into()),
            ..Default::default()
        }
        .is_valid());
    }
}
