//! The tutor prompt for lesson generation.
//!
//! Bundled with the binary for now, but resolved behind [`tutor_system_template`]
//! so it can later be promoted to the prompt library without touching the
//! generator. `[learn].system_prompt_template` overrides it.

/// Tera template for the tutor system prompt. Variables: `{{ target }}` (what to
/// teach) and `{{ covered }}` (already-covered points, may be empty).
pub const TUTOR_SYSTEM_TEMPLATE: &str = r#"You are an expert tutor for an internal developer portal. Teach the learner
exactly ONE tightly-scoped, self-contained concept about: {{ target }}. If the
context covers several, pick the single most useful one and ignore the rest —
never bundle multiple concepts into one lesson.

Ground every statement ONLY in the provided documentation context. Never use
outside knowledge. If the context is insufficient, teach the most useful thing
it does support.
{% if mission %}
The learner's goal, in their own words: "{{ mission }}". Of everything the
context supports, choose the concept that most directly moves them toward this
goal, and frame the lesson and its examples around it. Do not invent material to
fit the goal — if the context does not serve it, teach the most useful thing it
does support and stay grounded.
{% endif %}
{% if directive %}
{{ directive }}
{% endif %}
{% if covered %}
Avoid re-teaching these already-covered points: {{ covered }}.
{% endif %}
{% if glossary %}
These terms have already been defined for this learner — reuse them consistently
with the same meaning, and do NOT redefine them:
{{ glossary }}
{% endif %}
Write a short lesson (a few short paragraphs) that gives the learner one tangible
takeaway. Then write exactly 3 multiple-choice questions that test genuine recall
of that takeaway (not trivial recognition). Make the 4 options indistinguishable
by form: the same length (within a few characters), the same grammatical shape,
and the same capitalization and formatting. Do NOT let the correct option be the
longest or most detailed, do NOT reuse distinctive words from the question stem
in the correct option, and vary which position (correct_index) is correct across
the 3 questions. Formatting must give no clue to the correct answer.

Respond with a SINGLE JSON object and nothing else — no markdown code fences,
no preamble — with exactly this shape:
{
  "title": "the lesson title",
  "body_html": "the lesson body as simple safe HTML using only <p>, <ul>, <ol>, <li>, <strong>, <em>, <code>, <pre>, <h3>, <blockquote>. No <script>, no inline styles, no event handlers.",
  "citations": [{"document_slug": "one of the provided documents", "section_anchor": "anchor string or null", "quote": "a short verbatim quote from the context"}],
  "primary_source": {"document_slug": "one of the provided documents", "section_anchor": "anchor string or null"},
  "quiz": [{"prompt": "the question", "options": ["a", "b", "c", "d"], "correct_index": 0, "explanation": "why the answer is correct"}],
  "glossary": [{"term": "a key term this lesson introduces", "definition": "a one-line, self-contained definition grounded in the context"}]
}

Every citation's document_slug MUST be one of the provided documents. Include at
least one citation. In "glossary", list only NEW key terms this lesson
introduces (omit ones already defined above); keep definitions to one line. Reply
in the same language as the documentation."#;

/// Returns the tutor system-prompt template, honouring a config override.
///
/// Kept as a function so the source of the template (bundled vs prompt library)
/// can change later without touching the generator.
pub fn tutor_system_template(override_template: Option<&str>) -> String {
    match override_template {
        Some(t) if !t.trim().is_empty() => t.to_string(),
        _ => TUTOR_SYSTEM_TEMPLATE.to_string(),
    }
}

use std::sync::Arc;

use async_trait::async_trait;

use crate::db::prompt_repository::PromptRepository;
use crate::error::AppError;
use crate::storage::client::StorageClient;

/// Where the generator gets its tutor system-prompt template. The indirection
/// lets the template come from the bundled default, a config override, or the
/// prompt library, without the generator caring which.
#[async_trait]
pub trait LessonPromptSource: Send + Sync {
    /// Resolve the tutor system-prompt template (Tera). Implementations must
    /// always return a usable template — falling back to the bundled one on any
    /// miss — so lesson generation never fails on a prompt-lookup problem.
    async fn tutor_template(&self) -> String;
}

/// The template shipped with the binary, optionally replaced by
/// `[learn].system_prompt_template`. Resolved once at construction.
pub struct BundledPromptSource {
    template: String,
}

impl BundledPromptSource {
    pub fn new(override_template: Option<&str>) -> Self {
        Self {
            template: tutor_system_template(override_template),
        }
    }
}

#[async_trait]
impl LessonPromptSource for BundledPromptSource {
    async fn tutor_template(&self) -> String {
        self.template.clone()
    }
}

/// Resolves the tutor prompt from the prompt library by slug at generation
/// time, so admins can edit it there without a redeploy. Falls back to the
/// bundled template when the prompt is missing, empty, or unreadable.
pub struct RepositoryPromptSource {
    slug: String,
    prompt_repo: Arc<dyn PromptRepository>,
    storage: Arc<dyn StorageClient>,
    fallback: String,
}

impl RepositoryPromptSource {
    pub fn new(
        slug: String,
        prompt_repo: Arc<dyn PromptRepository>,
        storage: Arc<dyn StorageClient>,
        override_template: Option<&str>,
    ) -> Self {
        Self {
            slug,
            prompt_repo,
            storage,
            fallback: tutor_system_template(override_template),
        }
    }

    async fn load(&self) -> Result<Option<String>, AppError> {
        let Some(prompt) = self.prompt_repo.find_by_slug(&self.slug).await? else {
            return Ok(None);
        };
        let Some(bytes) = self.storage.get_object(&prompt.s3_key).await? else {
            return Ok(None);
        };
        // Prompt bodies are stored as a YAML blob; we only need the body.
        #[derive(serde::Deserialize)]
        struct Blob {
            prompt_body: String,
        }
        let blob: Blob = serde_yaml::from_slice(&bytes)
            .map_err(|e| AppError::Internal(format!("learn: invalid prompt blob: {e}")))?;
        Ok(Some(blob.prompt_body))
    }
}

#[async_trait]
impl LessonPromptSource for RepositoryPromptSource {
    async fn tutor_template(&self) -> String {
        match self.load().await {
            Ok(Some(body)) if !body.trim().is_empty() => body,
            Ok(_) => {
                tracing::warn!(
                    slug = %self.slug,
                    "learn: tutor prompt not found/empty in library — using bundled template"
                );
                self.fallback.clone()
            }
            Err(e) => {
                tracing::warn!(
                    slug = %self.slug,
                    "learn: could not load tutor prompt from library ({e}) — using bundled template"
                );
                self.fallback.clone()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bundled_source_uses_override_then_default() {
        let overridden = BundledPromptSource::new(Some("CUSTOM TUTOR PROMPT"));
        assert_eq!(overridden.tutor_template().await, "CUSTOM TUTOR PROMPT");

        // A blank override falls back to the bundled template.
        let blank = BundledPromptSource::new(Some("   "));
        assert!(blank.tutor_template().await.contains("expert tutor"));

        let default = BundledPromptSource::new(None);
        assert!(default.tutor_template().await.contains("expert tutor"));
    }
}
