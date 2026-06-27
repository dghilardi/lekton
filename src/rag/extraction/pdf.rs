//! PDF attachment extractor backed by the native `libpdfium` library.

use async_trait::async_trait;

use crate::error::AppError;
use crate::rag::service::AttachmentPage;

use super::AttachmentExtractor;

/// Extracts the native text layer of a PDF, one [`AttachmentPage`] per page
/// (1-based page numbers; empty pages are dropped).
///
/// pdfium is a synchronous, non-`Send` native library, so all work runs inside
/// `spawn_blocking` with a freshly-bound library instance that never crosses an
/// await point. VLM transcription of image-heavy pages is added in a later
/// commit; for now a page's native text is kept as-is.
pub struct PdfExtractor;

impl PdfExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PdfExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AttachmentExtractor for PdfExtractor {
    fn supports(&self, content_type: &str) -> bool {
        content_type.starts_with("application/pdf")
    }

    async fn extract(
        &self,
        bytes: &[u8],
        _content_type: &str,
    ) -> Result<Vec<AttachmentPage>, AppError> {
        let bytes = bytes.to_vec();
        tokio::task::spawn_blocking(move || extract_pdf_text(&bytes))
            .await
            .map_err(|e| AppError::Internal(format!("pdf extraction task failed: {e}")))?
    }
}

/// Bind libpdfium, load the document, and return per-page text. Blocking.
fn extract_pdf_text(bytes: &[u8]) -> Result<Vec<AttachmentPage>, AppError> {
    use pdfium_render::prelude::*;

    let bindings = Pdfium::bind_to_system_library()
        .map_err(|e| AppError::Internal(format!("failed to bind libpdfium: {e}")))?;
    let pdfium = Pdfium::new(bindings);

    let document = pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .map_err(|e| AppError::Internal(format!("failed to load PDF: {e}")))?;

    let mut pages = Vec::new();
    for (index, page) in document.pages().iter().enumerate() {
        let text = page.text().map(|t| t.all()).unwrap_or_default();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        pages.push(AttachmentPage {
            page_number: Some((index + 1) as u32),
            text: trimmed.to_string(),
        });
    }

    Ok(pages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_only_pdf() {
        let e = PdfExtractor::new();
        assert!(e.supports("application/pdf"));
        assert!(!e.supports("text/plain"));
        assert!(!e.supports("image/png"));
    }
}
