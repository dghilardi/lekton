//! Lesson generation: grounding on internal docs + structured LLM generation.
//!
//! Pipeline:
//! 1-2. **Grounding sections** — Tag/Topic scopes ground on the semantically
//!    retrieved sections (`ChatService::retrieve_only`, already access-filtered),
//!    keeping their real section anchors; a `Document` scope grounds on that
//!    document — whole when it fits the context budget, otherwise split into
//!    heading-delimited sections — re-filtered by the user's access levels as
//!    defence in depth, and concatenated under a character budget.
//! 3. **Generate** — one structured LLM call produces a JSON lesson.
//! 4. **Validate + sanitize** — citations that don't resolve to a provided
//!    section are dropped; the body HTML is sanitized with the app's ammonia
//!    allowlist; citations are backfilled from the grounding sections if none
//!    resolve.

use std::collections::HashSet;
use std::sync::Arc;

use async_openai::types::chat::{
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
    ChatCompletionRequestSystemMessageContent, ChatCompletionRequestUserMessage,
    ChatCompletionRequestUserMessageContent, CreateChatCompletionRequest, ResponseFormat,
};
use serde::Deserialize;

use crate::auth::models::UserContext;
use crate::db::learn_models::{
    GlossaryTerm, LearningScope, LessonCitation, LessonSource, QuizQuestion,
};
use crate::db::repository::DocumentRepository;
use crate::error::AppError;
use crate::learn::prompt::LessonPromptSource;
use crate::rag::chat::ChatService;
use crate::rag::client::format_llm_error;
use crate::rag::provider::LlmProvider;
use crate::rag::splitter::split_document_sections;
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
    /// New glossary terms the lesson introduced, to persist and reuse later.
    pub glossary: Vec<GlossaryTerm>,
    /// Whether the source context was truncated to fit the budget.
    pub context_truncated: bool,
}

/// One grounding section fed to the tutor. For retrieval-based scopes (Tag /
/// Topic) a section is a reranked chunk; for a small Document scope it is the
/// whole document; for a large one it is a heading-delimited section. `anchor`
/// is the real section anchor when known, so citations can deep-link.
struct Section {
    slug: String,
    title: String,
    anchor: Option<String>,
    text: String,
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
    #[serde(default)]
    glossary: Vec<GlossaryTerm>,
}

/// Generates lessons grounded on the internal documentation.
pub struct LessonGenerator {
    chat_service: Arc<ChatService>,
    document_repo: Arc<dyn DocumentRepository>,
    storage_client: Arc<dyn StorageClient>,
    llm_provider: Arc<LlmProvider>,
    model: String,
    headers: std::collections::HashMap<String, String>,
    prompt_source: Arc<dyn LessonPromptSource>,
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
        prompt_source: Arc<dyn LessonPromptSource>,
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
            prompt_source,
            max_context_chars,
            max_source_documents,
        }
    }

    /// Generate the next lesson for a scope. `covered` lists already-covered
    /// points so the tutor can pick something new; `mission` is the learner's
    /// stated goal, steering which sub-topic is most worth teaching.
    #[allow(clippy::too_many_arguments)]
    pub async fn generate(
        &self,
        user_ctx: &UserContext,
        scope: &LearningScope,
        covered: &[String],
        directive: Option<&str>,
        mission: Option<&str>,
        known_terms: &[GlossaryTerm],
    ) -> Result<GeneratedLesson, AppError> {
        // ── Stages 1-2: assemble the grounding sections ───────────────────
        // Tag/Topic ground on the semantically retrieved sections (relevant
        // slices, with real anchors); a Document scope grounds on that document
        // — whole when it fits the budget, split into sections when it doesn't.
        let (target, sections) = match scope {
            LearningScope::Document { slug } => (
                format!("the document \"{slug}\""),
                self.document_sections(user_ctx, slug).await?,
            ),
            LearningScope::Tag { tag } => (
                format!("the topic \"{tag}\""),
                self.retrieved_sections(user_ctx, tag).await?,
            ),
            LearningScope::Topic { text } => {
                (text.clone(), self.retrieved_sections(user_ctx, text).await?)
            }
        };

        if sections.is_empty() {
            return Err(AppError::NotFound(
                "no accessible documentation found for this learning scope".into(),
            ));
        }

        // Distinct source slugs (relevance order), for calibration/coverage.
        let mut source_slugs: Vec<String> = Vec::new();
        for s in &sections {
            if !source_slugs.contains(&s.slug) {
                source_slugs.push(s.slug.clone());
            }
        }

        let (body, truncated) = assemble_context(&sections, self.max_context_chars);
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
        let template = self.prompt_source.tutor_template().await;
        let system_prompt =
            render_tutor_prompt(&template, &target, covered, directive, mission, known_terms)?;
        let parsed = self.generate_parsed(&system_prompt, &context).await?;

        // ── Stage 4: validate + sanitize ──────────────────────────────────
        let mut lesson = validate_and_build(parsed, &sections);
        lesson.source_slugs = source_slugs;
        lesson.context_truncated = truncated;
        Ok(lesson)
    }

    /// Semantic retrieval → grounding sections (the reranked chunks), deduped
    /// by `(slug, anchor)` in relevance order, limited to `max_source_documents`
    /// distinct documents. Access is already filtered by `retrieve_only`.
    async fn retrieved_sections(
        &self,
        user_ctx: &UserContext,
        query: &str,
    ) -> Result<Vec<Section>, AppError> {
        let session_id = uuid::Uuid::new_v4().to_string();
        let retrieval = self
            .chat_service
            .retrieve_only(user_ctx, query, &[], &session_id)
            .await?;

        let mut slugs_seen: HashSet<String> = HashSet::new();
        let mut section_keys: HashSet<(String, String)> = HashSet::new();
        let mut sections = Vec::new();
        for chunk in retrieval.post_rerank {
            if chunk.document_slug.is_empty() {
                continue; // e.g. attachment-sourced chunks carry no doc slug
            }
            if chunk.chunk_text.trim().is_empty() {
                continue;
            }
            // Cap the number of distinct documents, but keep several sections
            // from each so a lesson can span a document's structure.
            if !slugs_seen.contains(&chunk.document_slug)
                && slugs_seen.len() >= self.max_source_documents
            {
                continue;
            }
            let anchor = (!chunk.section_anchor.is_empty()).then(|| chunk.section_anchor.clone());
            let key = (
                chunk.document_slug.clone(),
                anchor.clone().unwrap_or_default(),
            );
            if !section_keys.insert(key) {
                continue; // already have this section
            }
            slugs_seen.insert(chunk.document_slug.clone());
            sections.push(Section {
                slug: chunk.document_slug,
                title: chunk.document_title,
                anchor,
                text: chunk.chunk_text,
            });
        }
        Ok(sections)
    }

    /// Ground on a single document by slug: whole document when it fits the
    /// context budget, otherwise split into heading-delimited sections. Keeps
    /// only a readable, published document (defence in depth — a Document scope
    /// fetches directly, bypassing retrieval's access filter).
    async fn document_sections(
        &self,
        user_ctx: &UserContext,
        slug: &str,
    ) -> Result<Vec<Section>, AppError> {
        let docs = self
            .document_repo
            .find_by_slugs(&[slug.to_string()])
            .await?;
        let Some(doc) = docs.into_iter().next() else {
            return Ok(Vec::new());
        };
        if doc.is_draft || !user_ctx.can_read(&doc.access_level) {
            return Ok(Vec::new());
        }
        let content = match self.storage_client.get_object(&doc.s3_key).await? {
            Some(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            None => return Ok(Vec::new()),
        };
        if content.trim().is_empty() {
            return Ok(Vec::new());
        }

        // Small enough to teach whole; keep it as one section (no anchor).
        if content.len() <= self.max_context_chars {
            return Ok(vec![Section {
                slug: doc.slug,
                title: doc.title,
                anchor: None,
                text: content,
            }]);
        }

        // Too large: split into sections so selection/citations are per-section.
        let sections = split_document_sections(&content)
            .into_iter()
            .filter(|(_, text)| !text.trim().is_empty())
            .map(|(anchor, text)| Section {
                slug: doc.slug.clone(),
                title: doc.title.clone(),
                anchor: (!anchor.is_empty()).then_some(anchor),
                text,
            })
            .collect();
        Ok(sections)
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
    known_terms: &[GlossaryTerm],
) -> Result<String, AppError> {
    let mut tera = tera::Tera::default();
    tera.add_raw_template("tutor", template)
        .map_err(|e| AppError::Internal(format!("learn: invalid tutor template: {e}")))?;
    let glossary = known_terms
        .iter()
        .map(|t| format!("{} — {}", t.term, t.definition))
        .collect::<Vec<_>>()
        .join("\n");
    let mut ctx = tera::Context::new();
    ctx.insert("target", target);
    ctx.insert("covered", &covered.join("; "));
    ctx.insert("directive", &directive.unwrap_or(""));
    ctx.insert("mission", &mission.unwrap_or("").trim());
    ctx.insert("glossary", &glossary);
    tera.render("tutor", &ctx)
        .map_err(|e| AppError::Internal(format!("learn: tutor template render failed: {e}")))
}

/// Concatenate sections under a character budget. Returns `(context, truncated)`
/// where `truncated` is true if any section was cut or dropped to fit. Each
/// section is headed with its document title, slug, and anchor so the model can
/// cite it precisely.
fn assemble_context(sections: &[Section], max_chars: usize) -> (String, bool) {
    let mut out = String::new();
    let mut truncated = false;
    for section in sections {
        let header = match &section.anchor {
            Some(a) => format!("## {} ({}#{})\n\n", section.title, section.slug, a),
            None => format!("## {} ({})\n\n", section.title, section.slug),
        };
        let remaining = max_chars.saturating_sub(out.len());
        if remaining <= header.len() {
            truncated = true;
            break;
        }
        out.push_str(&header);
        let body_budget = remaining - header.len();
        if section.text.len() <= body_budget {
            out.push_str(&section.text);
            out.push_str("\n\n");
        } else {
            // Cut on a char boundary within the budget.
            let mut cut = body_budget;
            while cut > 0 && !section.text.is_char_boundary(cut) {
                cut -= 1;
            }
            out.push_str(&section.text[..cut]);
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
/// The lesson is grounded on the provided sections by construction (they are
/// the only context), so if the model's citations don't resolve we backfill
/// citations from the source sections — with their real anchors — rather than
/// discarding the lesson.
fn validate_and_build(raw: RawLesson, sections: &[Section]) -> GeneratedLesson {
    // Distinct source slugs, in order, for slug resolution.
    let mut source_slugs: Vec<String> = Vec::new();
    for s in sections {
        if !source_slugs.contains(&s.slug) {
            source_slugs.push(s.slug.clone());
        }
    }

    let mut citations: Vec<LessonCitation> = raw
        .citations
        .into_iter()
        .filter_map(|mut c| {
            resolve_slug(&c.document_slug, &source_slugs).map(|canon| {
                c.document_slug = canon;
                c
            })
        })
        .collect();

    if citations.is_empty() {
        // Backfill one anchored citation per distinct source section.
        let mut seen: HashSet<(String, Option<String>)> = HashSet::new();
        citations = sections
            .iter()
            .filter(|s| seen.insert((s.slug.clone(), s.anchor.clone())))
            .map(|s| LessonCitation {
                document_slug: s.slug.clone(),
                section_anchor: s.anchor.clone(),
                quote: String::new(),
            })
            .collect();
    }

    let primary_source = raw.primary_source.and_then(|s| {
        resolve_slug(&s.document_slug, &source_slugs).map(|canon| LessonSource {
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

    // Keep only well-formed glossary terms (both fields non-empty), trimmed.
    let glossary: Vec<GlossaryTerm> = raw
        .glossary
        .into_iter()
        .map(|t| GlossaryTerm {
            term: t.term.trim().to_string(),
            definition: t.definition.trim().to_string(),
        })
        .filter(|t| !t.term.is_empty() && !t.definition.is_empty())
        .collect();

    GeneratedLesson {
        title: raw.title.trim().to_string(),
        body_html: sanitize_html(&raw.body_html),
        citations,
        primary_source,
        quiz,
        source_slugs: Vec::new(),
        glossary,
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
            &[],
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
            &[],
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
            &[],
        )
        .unwrap();
        assert!(!blank.contains("The learner's goal"));
    }

    #[test]
    fn tutor_prompt_injects_known_glossary_terms() {
        let terms = vec![GlossaryTerm {
            term: "partition".into(),
            definition: "an ordered, append-only log".into(),
        }];
        let out = render_tutor_prompt(
            TUTOR_SYSTEM_TEMPLATE,
            "the topic \"kafka\"",
            &[],
            None,
            None,
            &terms,
        )
        .unwrap();
        assert!(out.contains("partition — an ordered, append-only log"));
    }

    fn section(slug: &str, title: &str, anchor: Option<&str>, text: &str) -> Section {
        Section {
            slug: slug.into(),
            title: title.into(),
            anchor: anchor.map(Into::into),
            text: text.into(),
        }
    }

    #[test]
    fn assemble_context_fits_within_budget() {
        let secs = vec![
            section("a", "A", None, "hello"),
            section("b", "B", None, "world"),
        ];
        let (ctx, truncated) = assemble_context(&secs, 10_000);
        assert!(ctx.contains("## A (a)"));
        assert!(ctx.contains("hello"));
        assert!(ctx.contains("world"));
        assert!(!truncated);
    }

    #[test]
    fn assemble_context_includes_the_section_anchor_in_the_header() {
        let secs = vec![section("docs/kafka", "Kafka", Some("partitions"), "text")];
        let (ctx, _) = assemble_context(&secs, 10_000);
        assert!(ctx.contains("## Kafka (docs/kafka#partitions)"));
    }

    #[test]
    fn assemble_context_truncates_and_flags() {
        let secs = vec![section("a", "A", None, &"x".repeat(1000))];
        let (ctx, truncated) = assemble_context(&secs, 50);
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

    fn allowed() -> Vec<Section> {
        vec![section("docs/kafka", "Kafka", Some("partitions"), "body")]
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
            glossary: vec![],
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
            glossary: vec![],
        };
        let lesson = validate_and_build(raw, &allowed());
        assert_eq!(lesson.citations.len(), 1);
        assert_eq!(lesson.citations[0].document_slug, "docs/kafka");
        // Backfilled citation carries the section's real anchor.
        assert_eq!(
            lesson.citations[0].section_anchor.as_deref(),
            Some("partitions")
        );
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
            glossary: vec![],
        };
        let lesson = validate_and_build(raw, &allowed());
        // Canonicalized to the source slug's exact casing.
        assert_eq!(lesson.citations.len(), 1);
        assert_eq!(lesson.citations[0].document_slug, "docs/kafka");
    }
}
