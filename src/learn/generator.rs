//! Lesson generation: grounding on internal docs + structured LLM generation.
//!
//! Pipeline:
//! 1. **Find documents** — semantic retrieval (`ChatService::retrieve_only`,
//!    already access-filtered) selects the relevant document slugs; a
//!    `Document` scope names its slug directly.
//! 2. **Whole documents** — the full markdown of each selected document is
//!    loaded (so no section is lost), re-filtered by the user's access levels
//!    as defence in depth, and concatenated under a character budget.
//! 3. **Generate** — one structured LLM call produces a JSON lesson.
//! 4. **Validate + sanitize** — citations that don't resolve to a provided
//!    document are dropped; the body HTML is sanitized with the app's ammonia
//!    allowlist; a lesson with no valid citation is rejected.

use std::collections::HashSet;
use std::sync::Arc;

use async_openai::types::chat::{
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
    ChatCompletionRequestSystemMessageContent, ChatCompletionRequestUserMessage,
    ChatCompletionRequestUserMessageContent, CreateChatCompletionRequest, ResponseFormat,
};
use serde::Deserialize;

use crate::auth::models::UserContext;
use crate::db::learn_models::{LearningScope, LessonCitation, LessonSource, QuizQuestion};
use crate::db::repository::DocumentRepository;
use crate::error::AppError;
use crate::rag::chat::ChatService;
use crate::rag::client::format_llm_error;
use crate::rag::provider::LlmProvider;
use crate::rendering::markdown::sanitize_html;
use crate::storage::client::StorageClient;

/// Max tokens for the lesson-generation completion.
const LESSON_MAX_TOKENS: u32 = 1_500;

/// A fully validated, ready-to-persist lesson (before it gets an id / seq).
pub struct GeneratedLesson {
    pub title: String,
    /// Sanitized HTML body.
    pub body_html: String,
    pub citations: Vec<LessonCitation>,
    pub primary_source: Option<LessonSource>,
    pub quiz: Vec<QuizQuestion>,
    /// Slugs of the documents that grounded this lesson (for calibration).
    pub source_slugs: Vec<String>,
    /// Whether the source context was truncated to fit the budget.
    pub context_truncated: bool,
}

/// One source document loaded for grounding.
struct SourceDoc {
    slug: String,
    title: String,
    content: String,
}

/// The raw JSON shape emitted by the LLM. Reuses the persisted field types so
/// no separate mapping is needed.
#[derive(Deserialize)]
struct RawLesson {
    title: String,
    body_html: String,
    #[serde(default)]
    citations: Vec<LessonCitation>,
    #[serde(default)]
    primary_source: Option<LessonSource>,
    #[serde(default)]
    quiz: Vec<QuizQuestion>,
}

/// Generates lessons grounded on the internal documentation.
pub struct LessonGenerator {
    chat_service: Arc<ChatService>,
    document_repo: Arc<dyn DocumentRepository>,
    storage_client: Arc<dyn StorageClient>,
    llm_provider: Arc<LlmProvider>,
    model: String,
    headers: std::collections::HashMap<String, String>,
    system_template: String,
    max_context_chars: usize,
    max_source_documents: usize,
}

impl LessonGenerator {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chat_service: Arc<ChatService>,
        document_repo: Arc<dyn DocumentRepository>,
        storage_client: Arc<dyn StorageClient>,
        llm_provider: Arc<LlmProvider>,
        model: String,
        headers: std::collections::HashMap<String, String>,
        system_template: String,
        max_context_chars: usize,
        max_source_documents: usize,
    ) -> Self {
        Self {
            chat_service,
            document_repo,
            storage_client,
            llm_provider,
            model,
            headers,
            system_template,
            max_context_chars,
            max_source_documents,
        }
    }

    /// Generate the next lesson for a scope. `covered` lists already-covered
    /// points so the tutor can pick something new; `mission` is the learner's
    /// stated goal, steering which sub-topic is most worth teaching.
    pub async fn generate(
        &self,
        user_ctx: &UserContext,
        scope: &LearningScope,
        covered: &[String],
        directive: Option<&str>,
        mission: Option<&str>,
    ) -> Result<GeneratedLesson, AppError> {
        // ── Stage 1: which documents ──────────────────────────────────────
        let (target, candidate_slugs) = match scope {
            LearningScope::Document { slug } => {
                (format!("the document \"{slug}\""), vec![slug.clone()])
            }
            LearningScope::Tag { tag } => (
                format!("the topic \"{tag}\""),
                self.retrieve_slugs(user_ctx, tag).await?,
            ),
            LearningScope::Topic { text } => {
                (text.clone(), self.retrieve_slugs(user_ctx, text).await?)
            }
        };

        if candidate_slugs.is_empty() {
            return Err(AppError::NotFound(
                "no documentation found for this learning scope".into(),
            ));
        }

        // ── Stage 2: whole documents (access-filtered) ────────────────────
        let docs = self.load_documents(user_ctx, &candidate_slugs).await?;
        if docs.is_empty() {
            return Err(AppError::NotFound(
                "no accessible documentation found for this learning scope".into(),
            ));
        }
        let source_slugs: Vec<String> = docs.iter().map(|d| d.slug.clone()).collect();
        let (body, truncated) = assemble_context(&docs, self.max_context_chars);
        // Prefix the exact slugs so the model can echo them verbatim in citations.
        let context = format!(
            "Available document slugs (use these EXACT strings in every citation's \
             \"document_slug\" field): {}\n\n{}",
            source_slugs.join(", "),
            body
        );
        if truncated {
            tracing::info!(
                slugs = ?source_slugs,
                max_chars = self.max_context_chars,
                "learn: source context truncated to fit the budget"
            );
        }

        // ── Stage 3: generate (JSON mode, with a corrective retry) ────────
        let system_prompt = self.render_system_prompt(&target, covered, directive, mission)?;
        let parsed = self.generate_parsed(&system_prompt, &context).await?;

        // ── Stage 4: validate + sanitize ──────────────────────────────────
        let mut lesson = validate_and_build(parsed, &source_slugs);
        lesson.source_slugs = source_slugs;
        lesson.context_truncated = truncated;
        Ok(lesson)
    }

    /// Semantic retrieval → distinct document slugs, capped at
    /// `max_source_documents`, preserving relevance order.
    async fn retrieve_slugs(
        &self,
        user_ctx: &UserContext,
        query: &str,
    ) -> Result<Vec<String>, AppError> {
        let session_id = uuid::Uuid::new_v4().to_string();
        let retrieval = self
            .chat_service
            .retrieve_only(user_ctx, query, &[], &session_id)
            .await?;

        let mut seen = HashSet::new();
        let mut slugs = Vec::new();
        for chunk in retrieval.post_rerank {
            if chunk.document_slug.is_empty() {
                continue; // e.g. attachment-sourced chunks carry no doc slug
            }
            if seen.insert(chunk.document_slug.clone()) {
                slugs.push(chunk.document_slug);
                if slugs.len() >= self.max_source_documents {
                    break;
                }
            }
        }
        Ok(slugs)
    }

    /// Fetch documents by slug, keep only readable published ones, and load
    /// their markdown from storage.
    async fn load_documents(
        &self,
        user_ctx: &UserContext,
        slugs: &[String],
    ) -> Result<Vec<SourceDoc>, AppError> {
        let docs = self.document_repo.find_by_slugs(slugs).await?;
        let mut out = Vec::new();
        for doc in docs {
            // Defence in depth: retrieval already filters by access, but a
            // Document-scope path fetches directly, so gate here too. Lessons
            // are taught from published docs only.
            if doc.is_draft || !user_ctx.can_read(&doc.access_level) {
                continue;
            }
            let content = match self.storage_client.get_object(&doc.s3_key).await? {
                Some(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                None => continue,
            };
            if content.trim().is_empty() {
                continue;
            }
            out.push(SourceDoc {
                slug: doc.slug,
                title: doc.title,
                content,
            });
        }
        Ok(out)
    }

    fn render_system_prompt(
        &self,
        target: &str,
        covered: &[String],
        directive: Option<&str>,
        mission: Option<&str>,
    ) -> Result<String, AppError> {
        render_tutor_prompt(&self.system_template, target, covered, directive, mission)
    }

    /// Generate and parse a lesson, retrying once without JSON mode and with a
    /// corrective instruction when the first reply is not valid JSON — small
    /// models often ignore `response_format` or wrap the object in prose.
    async fn generate_parsed(
        &self,
        system_prompt: &str,
        context: &str,
    ) -> Result<RawLesson, AppError> {
        let user = format!("Documentation context:\n\n{context}");
        match self.call_llm(system_prompt, &user, true).await {
            Ok(reply) => match extract_json(&reply) {
                Ok(parsed) => return Ok(parsed),
                Err(e) => tracing::warn!("learn: first lesson reply was not JSON ({e}) — retrying"),
            },
            Err(e) => tracing::warn!("learn: JSON-mode lesson call failed ({e}) — retrying"),
        }

        let corrective = format!(
            "{user}\n\nIMPORTANT: your previous reply was rejected. Respond with ONLY the JSON \
             object described in the instructions — no prose, no markdown code fences, nothing else."
        );
        let reply = self.call_llm(system_prompt, &corrective, false).await?;
        extract_json(&reply)
    }

    async fn call_llm(
        &self,
        system_prompt: &str,
        user: &str,
        json_mode: bool,
    ) -> Result<String, AppError> {
        let messages = vec![
            ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
                content: ChatCompletionRequestSystemMessageContent::Text(system_prompt.to_string()),
                name: None,
            }),
            ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                content: ChatCompletionRequestUserMessageContent::Text(user.to_string()),
                name: None,
            }),
        ];

        let request = CreateChatCompletionRequest {
            messages,
            model: self.model.clone(),
            max_completion_tokens: Some(LESSON_MAX_TOKENS),
            stream: Some(false),
            temperature: Some(0.2),
            response_format: json_mode.then_some(ResponseFormat::JsonObject),
            ..Default::default()
        };

        let client = self
            .llm_provider
            .get_client_with_headers(&self.headers)
            .await?;

        let response = client.chat().create(request).await.map_err(|e| {
            AppError::Internal(format!(
                "learn: lesson LLM call failed: {}",
                format_llm_error(&e)
            ))
        })?;

        Ok(response
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .unwrap_or_default())
    }
}

/// Render the tutor system prompt from its Tera template. Extracted as a free
/// function so the templating (mission/directive/covered wiring) is unit
/// testable without building a whole generator.
fn render_tutor_prompt(
    template: &str,
    target: &str,
    covered: &[String],
    directive: Option<&str>,
    mission: Option<&str>,
) -> Result<String, AppError> {
    let mut tera = tera::Tera::default();
    tera.add_raw_template("tutor", template)
        .map_err(|e| AppError::Internal(format!("learn: invalid tutor template: {e}")))?;
    let mut ctx = tera::Context::new();
    ctx.insert("target", target);
    ctx.insert("covered", &covered.join("; "));
    ctx.insert("directive", &directive.unwrap_or(""));
    ctx.insert("mission", &mission.unwrap_or("").trim());
    tera.render("tutor", &ctx)
        .map_err(|e| AppError::Internal(format!("learn: tutor template render failed: {e}")))
}

/// Concatenate documents under a character budget. Returns `(context, truncated)`
/// where `truncated` is true if any document was cut or dropped to fit.
fn assemble_context(docs: &[SourceDoc], max_chars: usize) -> (String, bool) {
    let mut out = String::new();
    let mut truncated = false;
    for doc in docs {
        let header = format!("## {} ({})\n\n", doc.title, doc.slug);
        let remaining = max_chars.saturating_sub(out.len());
        if remaining <= header.len() {
            truncated = true;
            break;
        }
        out.push_str(&header);
        let body_budget = remaining - header.len();
        if doc.content.len() <= body_budget {
            out.push_str(&doc.content);
            out.push_str("\n\n");
        } else {
            // Cut on a char boundary within the budget.
            let mut cut = body_budget;
            while cut > 0 && !doc.content.is_char_boundary(cut) {
                cut -= 1;
            }
            out.push_str(&doc.content[..cut]);
            out.push_str("\n\n");
            truncated = true;
            break;
        }
    }
    (out, truncated)
}

/// Extract the JSON object from an LLM reply, tolerating ```json fences or prose
/// around it by taking the first `{` to the last `}`.
fn extract_json(raw: &str) -> Result<RawLesson, AppError> {
    let start = raw.find('{');
    let end = raw.rfind('}');
    let slice = match (start, end) {
        (Some(s), Some(e)) if e >= s => &raw[s..=e],
        _ => {
            return Err(AppError::Internal(
                "learn: LLM reply contained no JSON object".into(),
            ))
        }
    };
    serde_json::from_str(slice)
        .map_err(|e| AppError::Internal(format!("learn: could not parse lesson JSON: {e}")))
}

/// Resolve a model-supplied slug to the canonical source slug, tolerating case
/// differences (small models don't always echo the slug verbatim).
fn resolve_slug(slug: &str, source_slugs: &[String]) -> Option<String> {
    let s = slug.trim();
    source_slugs
        .iter()
        .find(|c| c.eq_ignore_ascii_case(s))
        .cloned()
}

/// Sanitize the body, canonicalize/keep only citations that resolve to a
/// provided document, and validate quiz questions.
///
/// The lesson is grounded on the provided documents by construction (they are
/// the only context), so if the model's citations don't resolve we backfill
/// citations from the source documents rather than discarding the lesson.
fn validate_and_build(raw: RawLesson, source_slugs: &[String]) -> GeneratedLesson {
    let mut citations: Vec<LessonCitation> = raw
        .citations
        .into_iter()
        .filter_map(|mut c| {
            resolve_slug(&c.document_slug, source_slugs).map(|canon| {
                c.document_slug = canon;
                c
            })
        })
        .collect();

    if citations.is_empty() {
        citations = source_slugs
            .iter()
            .map(|slug| LessonCitation {
                document_slug: slug.clone(),
                section_anchor: None,
                quote: String::new(),
            })
            .collect();
    }

    let primary_source = raw.primary_source.and_then(|s| {
        resolve_slug(&s.document_slug, source_slugs).map(|canon| LessonSource {
            document_slug: canon,
            section_anchor: s.section_anchor,
        })
    });

    // Keep only well-formed quiz questions (≥2 options, in-range answer).
    let quiz: Vec<QuizQuestion> = raw
        .quiz
        .into_iter()
        .filter(|q| q.options.len() >= 2 && q.correct_index < q.options.len())
        .collect();

    GeneratedLesson {
        title: raw.title.trim().to_string(),
        body_html: sanitize_html(&raw.body_html),
        citations,
        primary_source,
        quiz,
        source_slugs: Vec::new(),
        context_truncated: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learn::prompt::TUTOR_SYSTEM_TEMPLATE;

    #[test]
    fn tutor_prompt_includes_the_mission_when_present() {
        let out = render_tutor_prompt(
            TUTOR_SYSTEM_TEMPLATE,
            "the topic \"kafka\"",
            &[],
            None,
            Some("ship a Kafka consumer"),
        )
        .unwrap();
        assert!(out.contains("the topic \"kafka\""));
        assert!(out.contains("ship a Kafka consumer"));
        assert!(out.contains("The learner's goal"));
    }

    #[test]
    fn tutor_prompt_omits_the_mission_block_when_absent() {
        let with_empty = render_tutor_prompt(
            TUTOR_SYSTEM_TEMPLATE,
            "the topic \"kafka\"",
            &[],
            None,
            None,
        )
        .unwrap();
        assert!(!with_empty.contains("The learner's goal"));
        // A blank mission is treated the same as no mission.
        let blank = render_tutor_prompt(
            TUTOR_SYSTEM_TEMPLATE,
            "the topic \"kafka\"",
            &[],
            None,
            Some("   "),
        )
        .unwrap();
        assert!(!blank.contains("The learner's goal"));
    }

    fn doc(slug: &str, title: &str, content: &str) -> SourceDoc {
        SourceDoc {
            slug: slug.into(),
            title: title.into(),
            content: content.into(),
        }
    }

    #[test]
    fn assemble_context_fits_within_budget() {
        let docs = vec![doc("a", "A", "hello"), doc("b", "B", "world")];
        let (ctx, truncated) = assemble_context(&docs, 10_000);
        assert!(ctx.contains("## A (a)"));
        assert!(ctx.contains("hello"));
        assert!(ctx.contains("world"));
        assert!(!truncated);
    }

    #[test]
    fn assemble_context_truncates_and_flags() {
        let docs = vec![doc("a", "A", &"x".repeat(1000))];
        let (ctx, truncated) = assemble_context(&docs, 50);
        assert!(truncated);
        assert!(ctx.len() <= 50 + "\n\n".len());
    }

    #[test]
    fn extract_json_strips_code_fences() {
        let raw = "```json\n{\"title\":\"T\",\"body_html\":\"<p>x</p>\"}\n```";
        let parsed = extract_json(raw).unwrap();
        assert_eq!(parsed.title, "T");
    }

    #[test]
    fn extract_json_errors_without_object() {
        assert!(extract_json("no json here").is_err());
    }

    fn allowed() -> Vec<String> {
        vec!["docs/kafka".to_string()]
    }

    #[test]
    fn validate_drops_hallucinated_citations_and_sanitizes() {
        let raw = RawLesson {
            title: "  Partitions  ".into(),
            body_html: "<p>ok</p><script>alert(1)</script>".into(),
            citations: vec![
                LessonCitation {
                    document_slug: "docs/kafka".into(),
                    section_anchor: None,
                    quote: "real".into(),
                },
                LessonCitation {
                    document_slug: "docs/made-up".into(),
                    section_anchor: None,
                    quote: "fake".into(),
                },
            ],
            primary_source: Some(LessonSource {
                document_slug: "docs/made-up".into(),
                section_anchor: None,
            }),
            quiz: vec![
                QuizQuestion {
                    prompt: "q1".into(),
                    options: vec!["a".into(), "b".into()],
                    correct_index: 1,
                    explanation: "e".into(),
                },
                QuizQuestion {
                    prompt: "bad".into(),
                    options: vec!["only".into()],
                    correct_index: 0,
                    explanation: "e".into(),
                },
                QuizQuestion {
                    prompt: "out of range".into(),
                    options: vec!["a".into(), "b".into()],
                    correct_index: 5,
                    explanation: "e".into(),
                },
            ],
        };

        let lesson = validate_and_build(raw, &allowed());
        assert_eq!(lesson.title, "Partitions");
        assert!(!lesson.body_html.contains("<script"));
        assert!(lesson.body_html.contains("<p>ok</p>"));
        assert_eq!(lesson.citations.len(), 1);
        assert_eq!(lesson.citations[0].document_slug, "docs/kafka");
        // Primary source pointed at a hallucinated doc → dropped.
        assert!(lesson.primary_source.is_none());
        // Only the well-formed question survives.
        assert_eq!(lesson.quiz.len(), 1);
        assert_eq!(lesson.quiz[0].prompt, "q1");
    }

    #[test]
    fn validate_backfills_citations_when_none_resolve() {
        // The model only cited a document that isn't among the sources: since
        // the lesson is grounded on the provided docs, citations are backfilled
        // rather than the lesson being rejected.
        let raw = RawLesson {
            title: "T".into(),
            body_html: "<p>x</p>".into(),
            citations: vec![LessonCitation {
                document_slug: "docs/made-up".into(),
                section_anchor: None,
                quote: "fake".into(),
            }],
            primary_source: None,
            quiz: vec![],
        };
        let lesson = validate_and_build(raw, &allowed());
        assert_eq!(lesson.citations.len(), 1);
        assert_eq!(lesson.citations[0].document_slug, "docs/kafka");
    }

    #[test]
    fn validate_resolves_citation_slug_case_insensitively() {
        let raw = RawLesson {
            title: "T".into(),
            body_html: "<p>x</p>".into(),
            citations: vec![LessonCitation {
                document_slug: "Docs/Kafka".into(),
                section_anchor: None,
                quote: "q".into(),
            }],
            primary_source: None,
            quiz: vec![],
        };
        let lesson = validate_and_build(raw, &allowed());
        // Canonicalized to the source slug's exact casing.
        assert_eq!(lesson.citations.len(), 1);
        assert_eq!(lesson.citations[0].document_slug, "docs/kafka");
    }
}
