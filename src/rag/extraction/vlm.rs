//! Vision-LLM transcription of rendered document pages.

use std::collections::HashMap;

use async_openai::types::chat::{
    ChatCompletionRequestMessage, ChatCompletionRequestMessageContentPartImage,
    ChatCompletionRequestMessageContentPartText, ChatCompletionRequestUserMessage,
    ChatCompletionRequestUserMessageContent, ChatCompletionRequestUserMessageContentPart,
    CreateChatCompletionRequest, ImageUrl,
};
use base64::engine::{general_purpose::STANDARD, Engine as _};

use crate::error::AppError;
use crate::rag::client::format_llm_error;
use crate::rag::provider::LlmProvider;

const DEFAULT_VLM_MAX_TOKENS: u32 = 1024;

const TRANSCRIPTION_PROMPT: &str = "Transcribe this document page into clean Markdown. \
Preserve tables as Markdown tables and describe diagrams, charts, and screenshots concisely in prose. \
Output only the transcription, with no preamble or commentary. Do not invent content that is not \
visible in the image; in particular, never fabricate numbers in tables.";

/// Transcribes rendered page images to text via a vision-capable LLM, using the
/// shared [`LlmProvider`] (OpenAI-compatible or Vertex AI over its OpenAI bridge).
pub struct VlmTranscriber {
    provider: LlmProvider,
    model: String,
    max_tokens: u32,
    headers: HashMap<String, String>,
}

impl VlmTranscriber {
    pub fn new(
        provider: LlmProvider,
        model: String,
        max_tokens: Option<u32>,
        headers: HashMap<String, String>,
    ) -> Self {
        Self {
            provider,
            model,
            max_tokens: max_tokens.unwrap_or(DEFAULT_VLM_MAX_TOKENS),
            headers,
        }
    }

    /// Transcribe a single rendered page (PNG bytes) to text. Returns the empty
    /// string when the model produces no content.
    pub async fn transcribe_page(&self, png: &[u8]) -> Result<String, AppError> {
        let data_url = format!("data:image/png;base64,{}", STANDARD.encode(png));

        let content = ChatCompletionRequestUserMessageContent::Array(vec![
            ChatCompletionRequestUserMessageContentPart::Text(
                ChatCompletionRequestMessageContentPartText {
                    text: TRANSCRIPTION_PROMPT.to_string(),
                },
            ),
            ChatCompletionRequestUserMessageContentPart::ImageUrl(
                ChatCompletionRequestMessageContentPartImage {
                    image_url: ImageUrl {
                        url: data_url,
                        detail: None,
                    },
                },
            ),
        ]);

        let request = CreateChatCompletionRequest {
            messages: vec![ChatCompletionRequestMessage::User(
                ChatCompletionRequestUserMessage {
                    content,
                    name: None,
                },
            )],
            model: self.model.clone(),
            max_completion_tokens: Some(self.max_tokens),
            temperature: Some(0.0),
            stream: Some(false),
            ..Default::default()
        };

        let client = self.provider.get_client_with_headers(&self.headers).await?;
        let response = client.chat().create(request).await.map_err(|e| {
            AppError::Internal(format!(
                "VLM transcription failed: {}",
                format_llm_error(&e)
            ))
        })?;

        let text = response
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        Ok(text)
    }
}
