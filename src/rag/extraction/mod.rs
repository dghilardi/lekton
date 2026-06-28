//! Attachment text extraction for RAG indexing.
//!
//! Each [`AttachmentExtractor`] turns an attachment's bytes into per-page text
//! ([`AttachmentPage`]) that the [`RagService`](crate::rag::service::RagService)
//! can embed. [`extract_attachment`] dispatches to the right extractor by MIME
//! content type.
//!
//! Currently only plain-text sources are handled; PDF extraction (native text
//! plus VLM transcription of image-heavy pages) lands in a later commit once the
//! native `pdfium` library is wired in.

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::AppError;
use crate::rag::service::AttachmentPage;

mod pdf;
mod text;
mod vlm;

pub use pdf::{extract_preview, PdfExtractor};
pub use text::TextExtractor;
pub use vlm::VlmTranscriber;

/// Extracts indexable, per-page text from an attachment's raw bytes.
#[async_trait]
pub trait AttachmentExtractor: Send + Sync {
    /// Whether this extractor handles the given MIME content type.
    fn supports(&self, content_type: &str) -> bool;

    /// Extract per-page text. Non-paginated sources (e.g. plain text) return a
    /// single page with `page_number = None`. An empty result means the file
    /// held no indexable text.
    async fn extract(
        &self,
        bytes: &[u8],
        content_type: &str,
    ) -> Result<Vec<AttachmentPage>, AppError>;
}

/// Result of attempting to extract an attachment.
#[derive(Debug)]
pub enum ExtractionOutcome {
    /// Extraction ran; the pages (possibly empty when the file had no text).
    Extracted(Vec<AttachmentPage>),
    /// No extractor supports this content type; the attachment is not indexable.
    Unsupported,
}

/// The set of attachment extractors, dispatched by MIME content type.
///
/// Holds any per-extractor configuration (e.g. PDF routing). Construct once and
/// share via `Arc`.
pub struct AttachmentExtractors {
    text: TextExtractor,
    pdf: PdfExtractor,
}

impl AttachmentExtractors {
    /// Build the extractor set. `page_text_threshold` and `vlm` configure PDF
    /// image-heavy-page routing; pass `vlm = None` to disable VLM transcription
    /// (native text only).
    pub fn new(page_text_threshold: usize, vlm: Option<Arc<VlmTranscriber>>) -> Self {
        Self {
            text: TextExtractor,
            pdf: PdfExtractor::new(page_text_threshold, vlm),
        }
    }

    /// Extract using the first extractor that supports `content_type`. Returns
    /// [`ExtractionOutcome::Unsupported`] when none matches, so the caller can
    /// mark the asset `Skipped` rather than `Failed`.
    pub async fn extract(
        &self,
        bytes: &[u8],
        content_type: &str,
    ) -> Result<ExtractionOutcome, AppError> {
        if self.text.supports(content_type) {
            return Ok(ExtractionOutcome::Extracted(
                self.text.extract(bytes, content_type).await?,
            ));
        }
        if self.pdf.supports(content_type) {
            return Ok(ExtractionOutcome::Extracted(
                self.pdf.extract(bytes, content_type).await?,
            ));
        }
        Ok(ExtractionOutcome::Unsupported)
    }
}
