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
    /// Opt-in to the automated documentation reviewer (the docs-agent). When
    /// `false` (the default), the agent never touches this source. Off by
    /// default so enabling the source registry never implies automated changes.
    #[serde(default)]
    pub review_enabled: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl DocumentSource {
    /// Build a "view source" URL pointing at `source_path` on the source repo's
    /// mainline branch, when the repository host is a recognized provider.
    ///
    /// Returns `None` when the repo URL is absent, the host is not a known
    /// provider (GitHub / GitLab / Bitbucket), or `source_path` is empty. The
    /// branch defaults to `main` when [`DocumentSource::mainline_branch`] is
    /// unset.
    pub fn source_view_url(&self, source_path: &str) -> Option<String> {
        let repo_url = self.repo_url.as_deref()?.trim().trim_end_matches('/');
        if repo_url.is_empty() {
            return None;
        }
        let path = source_path.trim().trim_start_matches('/');
        if path.is_empty() {
            return None;
        }
        let branch = self
            .mainline_branch
            .as_deref()
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .unwrap_or("main");

        // Strip a trailing `.git` so `repo_url` values copied from clone URLs
        // still produce valid web links.
        let repo_url = repo_url.trim_end_matches(".git");

        // Route by host to each provider's file-view path scheme.
        let host = repo_url
            .split("://")
            .nth(1)
            .unwrap_or(repo_url)
            .split('/')
            .next()
            .unwrap_or("");
        let segment = if host.contains("github.com") {
            "blob"
        } else if host.contains("gitlab.com") {
            "-/blob"
        } else if host.contains("bitbucket.org") {
            "src"
        } else {
            return None;
        };

        Some(format!("{repo_url}/{segment}/{branch}/{path}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn source(repo_url: Option<&str>, branch: Option<&str>) -> DocumentSource {
        DocumentSource {
            id: "svc".into(),
            display_name: None,
            repo_url: repo_url.map(str::to_string),
            mainline_branch: branch.map(str::to_string),
            maintainers: vec![],
            description: None,
            review_enabled: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn github_view_url() {
        let s = source(Some("https://github.com/org/repo"), Some("develop"));
        assert_eq!(
            s.source_view_url("docs/guide.md").as_deref(),
            Some("https://github.com/org/repo/blob/develop/docs/guide.md")
        );
    }

    #[test]
    fn gitlab_view_url() {
        let s = source(Some("https://gitlab.com/org/repo"), None);
        assert_eq!(
            s.source_view_url("docs/guide.md").as_deref(),
            Some("https://gitlab.com/org/repo/-/blob/main/docs/guide.md")
        );
    }

    #[test]
    fn bitbucket_view_url() {
        let s = source(Some("https://bitbucket.org/org/repo"), Some("master"));
        assert_eq!(
            s.source_view_url("docs/guide.md").as_deref(),
            Some("https://bitbucket.org/org/repo/src/master/docs/guide.md")
        );
    }

    #[test]
    fn normalizes_trailing_slash_git_suffix_and_leading_path_slash() {
        let s = source(Some("https://github.com/org/repo.git/"), Some("main"));
        assert_eq!(
            s.source_view_url("/docs/guide.md").as_deref(),
            Some("https://github.com/org/repo/blob/main/docs/guide.md")
        );
    }

    #[test]
    fn unknown_provider_or_missing_data_returns_none() {
        assert!(source(Some("https://example.com/org/repo"), None)
            .source_view_url("docs/guide.md")
            .is_none());
        assert!(source(None, None)
            .source_view_url("docs/guide.md")
            .is_none());
        assert!(source(Some("https://github.com/org/repo"), None)
            .source_view_url("  ")
            .is_none());
    }

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
