use std::sync::Arc;

use async_openai::config::OpenAIConfig;
use async_openai::types::chat::{
    ChatCompletionRequestAssistantMessage, ChatCompletionRequestAssistantMessageContent,
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
    ChatCompletionRequestSystemMessageContent, ChatCompletionRequestUserMessage,
    ChatCompletionRequestUserMessageContent, CreateChatCompletionRequest,
};
use async_openai::Client;
use chrono::Utc;
use futures::StreamExt;
use uuid::Uuid;

use crate::auth::models::UserContext;
use crate::config::RagConfig;
use crate::db::chat_models::{ChatMessage, ChatSession};
use crate::db::chat_repository::ChatRepository;
use crate::error::AppError;
use crate::rag::embedding::EmbeddingService;
use crate::rag::query_rewriter::QueryRewriter;
use crate::rag::vectorstore::VectorStore;

/// Maximum number of conversation history messages to include in the prompt.
const MAX_HISTORY_MESSAGES: usize = 20;
/// Maximum number of context chunks to retrieve from the vector store.
const MAX_CONTEXT_CHUNKS: usize = 5;

/// Orchestrates RAG chat: retrieval, prompt building, and LLM streaming.
pub struct ChatService {
    embedding: Arc<dyn EmbeddingService>,
    vectorstore: Arc<dyn VectorStore>,
    chat_repo: Arc<dyn ChatRepository>,
    llm_client: Client<OpenAIConfig>,
    chat_model: String,
    tera: tera::Tera,
    system_template_name: String,
    query_rewriter: Option<QueryRewriter>,
}

/// A token event yielded by the streaming chat response.
#[derive(Debug, serde::Serialize)]
#[serde(tag = "type")]
pub enum ChatEvent {
    /// First event — carries the session ID.
    #[serde(rename = "session")]
    Session { session_id: String },
    /// A content delta token.
    #[serde(rename = "delta")]
    Delta { content: String },
    /// Stream finished — carries the saved message ID so the client can
    /// attach feedback to the correct message.
    #[serde(rename = "done")]
    Done { message_id: Option<String> },
    /// An error occurred.
    #[serde(rename = "error")]
    Error { message: String },
}

impl ChatService {
    pub fn from_rag_config(
        config: &RagConfig,
        chat_repo: Arc<dyn ChatRepository>,
        embedding: Arc<dyn EmbeddingService>,
        vectorstore: Arc<dyn VectorStore>,
    ) -> Result<Self, AppError> {
        if config.chat_url.is_empty() {
            return Err(AppError::Internal("chat_url is required for RAG chat".into()));
        }

        let mut oai_config = OpenAIConfig::new().with_api_base(&config.chat_url);
        if !config.chat_api_key.is_empty() {
            oai_config = oai_config.with_api_key(&config.chat_api_key);
        }

        let mut tera = tera::Tera::default();
        let template_name = "system_prompt";
        tera.add_raw_template(template_name, &config.system_prompt_template)
            .map_err(|e| AppError::Internal(format!("invalid system_prompt_template: {e}")))?;

        Ok(Self {
            embedding,
            vectorstore,
            chat_repo,
            llm_client: Client::with_config(oai_config),
            chat_model: config.chat_model.clone(),
            tera,
            system_template_name: template_name.to_string(),
            query_rewriter: QueryRewriter::from_rag_config(config),
        })
    }

    /// Stream a chat response as a series of [`ChatEvent`]s.
    ///
    /// Returns a stream that the caller can forward as SSE.
    pub async fn stream_response(
        &self,
        user_ctx: &UserContext,
        session_id: Option<String>,
        user_message: String,
    ) -> Result<
        std::pin::Pin<Box<dyn futures::Stream<Item = ChatEvent> + Send>>,
        AppError,
    > {
        // 1. Resolve or create session
        let session_id = match session_id {
            Some(id) => {
                // Verify the session exists and belongs to this user
                let session = self
                    .chat_repo
                    .get_session(&id)
                    .await?
                    .ok_or_else(|| AppError::NotFound("Chat session not found".into()))?;
                if session.user_id != user_ctx.user.user_id {
                    return Err(AppError::NotFound("Chat session not found".into()));
                }
                id
            }
            None => {
                let id = Uuid::new_v4().to_string();
                let title = truncate_title(&user_message);
                let now = Utc::now();
                self.chat_repo
                    .create_session(ChatSession {
                        id: id.clone(),
                        user_id: user_ctx.user.user_id.clone(),
                        title,
                        created_at: now,
                        updated_at: now,
                    })
                    .await?;
                id
            }
        };

        // 2. Fetch conversation history (needed both for query rewriting and prompt building)
        let history = self
            .chat_repo
            .get_messages(&session_id, MAX_HISTORY_MESSAGES)
            .await?;

        // 3. Save user message
        self.chat_repo
            .add_message(ChatMessage {
                id: Uuid::new_v4().to_string(),
                session_id: session_id.clone(),
                role: "user".into(),
                content: user_message.clone(),
                created_at: Utc::now(),
            })
            .await?;

        // 4. Rewrite the query into a standalone question when history is non-empty.
        //    This improves vector-search relevance for follow-up / elliptic questions.
        //    Falls back to the original message when rewriting is disabled or history is empty.
        let retrieval_query = match &self.query_rewriter {
            Some(rewriter) => rewriter.rewrite(&user_message, &history).await?,
            None => user_message.clone(),
        };

        // 5. Embed the (possibly rewritten) retrieval query
        let query_vectors = self.embedding.embed(&[retrieval_query]).await?;
        let query_vector = query_vectors
            .into_iter()
            .next()
            .ok_or_else(|| AppError::Internal("embedding returned no vectors".into()))?;

        // 6. Search vector store with user's access filters
        let (allowed_levels, include_draft) = user_ctx.document_visibility();
        let search_results = self
            .vectorstore
            .search(
                query_vector,
                MAX_CONTEXT_CHUNKS,
                allowed_levels.as_deref(),
                include_draft,
            )
            .await?;

        // 7. Build context string from search results
        let context = search_results
            .iter()
            .map(|r| format!("[{}] ({})\n{}", r.document_title, r.document_slug, r.chunk_text))
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");

        // 8. Render system prompt via Tera
        let mut tera_ctx = tera::Context::new();
        tera_ctx.insert("context", &context);
        tera_ctx.insert("question", &user_message);
        let system_prompt = self
            .tera
            .render(&self.system_template_name, &tera_ctx)
            .map_err(|e| AppError::Internal(format!("tera render failed: {e}")))?;

        // 9. Build message array: system prompt + history + current user message
        let mut messages: Vec<ChatCompletionRequestMessage> = Vec::new();
        messages.push(ChatCompletionRequestMessage::System(
            ChatCompletionRequestSystemMessage {
                content: ChatCompletionRequestSystemMessageContent::Text(system_prompt),
                name: None,
            },
        ));

        // Add history (skip the user message we just saved — it's the last one)
        for msg in &history[..history.len().saturating_sub(1)] {
            match msg.role.as_str() {
                "user" => {
                    messages.push(ChatCompletionRequestMessage::User(
                        ChatCompletionRequestUserMessage {
                            content: ChatCompletionRequestUserMessageContent::Text(
                                msg.content.clone(),
                            ),
                            name: None,
                        },
                    ));
                }
                "assistant" => {
                    messages.push(ChatCompletionRequestMessage::Assistant(
                        ChatCompletionRequestAssistantMessage {
                            content: Some(ChatCompletionRequestAssistantMessageContent::Text(
                                msg.content.clone(),
                            )),
                            name: None,
                            tool_calls: None,
                            refusal: None,
                            audio: None,
                            ..Default::default()
                        },
                    ));
                }
                _ => {}
            }
        }

        // Add current user message
        messages.push(ChatCompletionRequestMessage::User(
            ChatCompletionRequestUserMessage {
                content: ChatCompletionRequestUserMessageContent::Text(user_message),
                name: None,
            },
        ));

        // 10. Create streaming LLM request
        let request = CreateChatCompletionRequest {
            messages,
            model: self.chat_model.clone(),
            stream: Some(true),
            ..Default::default()
        };

        let mut stream = self
            .llm_client
            .chat()
            .create_stream(request)
            .await
            .map_err(|e| AppError::Internal(format!("LLM stream creation failed: {e}")))?;

        // 11. Build SSE event stream
        let chat_repo = self.chat_repo.clone();
        let sid = session_id.clone();

        let event_stream = async_stream::stream! {
            // First event: session ID
            yield ChatEvent::Session { session_id: sid.clone() };

            let mut full_response = String::new();

            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(chunk) => {
                        for choice in &chunk.choices {
                            if let Some(content) = &choice.delta.content {
                                if !content.is_empty() {
                                    full_response.push_str(content);
                                    yield ChatEvent::Delta {
                                        content: content.clone(),
                                    };
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("LLM stream error: {e}");
                        yield ChatEvent::Error {
                            message: format!("LLM error: {e}"),
                        };
                        return;
                    }
                }
            }

            // Save the full assistant response
            let saved_message_id = if !full_response.is_empty() {
                let msg_id = Uuid::new_v4().to_string();
                let msg = ChatMessage {
                    id: msg_id.clone(),
                    session_id: sid.clone(),
                    role: "assistant".into(),
                    content: full_response,
                    created_at: Utc::now(),
                };
                if let Err(e) = chat_repo.add_message(msg).await {
                    tracing::error!("Failed to save assistant message: {e}");
                }
                if let Err(e) = chat_repo.touch_session(&sid).await {
                    tracing::error!("Failed to touch session: {e}");
                }
                Some(msg_id)
            } else {
                None
            };

            yield ChatEvent::Done { message_id: saved_message_id };
        };

        Ok(Box::pin(event_stream))
    }
}

/// Truncate a message to use as a session title.
fn truncate_title(message: &str) -> String {
    let first_line = message.lines().next().unwrap_or(message);
    if first_line.len() > 80 {
        format!("{}…", &first_line[..77])
    } else {
        first_line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_title_short() {
        assert_eq!(truncate_title("Hello world"), "Hello world");
    }

    #[test]
    fn truncate_title_long() {
        let long = "a".repeat(100);
        let title = truncate_title(&long);
        assert!(title.len() <= 81); // 77 + "…" (3 bytes)
        assert!(title.ends_with('…'));
    }

    #[test]
    fn truncate_title_multiline() {
        assert_eq!(truncate_title("First line\nSecond line"), "First line");
    }
}
