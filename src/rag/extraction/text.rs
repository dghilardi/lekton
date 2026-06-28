//! Plain-text attachment extractor.

use async_trait::async_trait;

use crate::error::AppError;
use crate::rag::service::AttachmentPage;

use super::AttachmentExtractor;

/// Extracts text from `text/*` attachments (e.g. `text/plain` for `.txt`/`.md`).
///
/// The whole file is returned as a single, non-paginated page. Lossy UTF-8
/// decoding keeps malformed bytes from failing the extraction.
pub struct TextExtractor;

#[async_trait]
impl AttachmentExtractor for TextExtractor {
    fn supports(&self, content_type: &str) -> bool {
        content_type.starts_with("text/")
    }

    async fn extract(
        &self,
        bytes: &[u8],
        _content_type: &str,
    ) -> Result<Vec<AttachmentPage>, AppError> {
        let text = String::from_utf8_lossy(bytes).into_owned();
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        Ok(vec![AttachmentPage {
            page_number: None,
            text,
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_text_types_only() {
        let e = TextExtractor;
        assert!(e.supports("text/plain; charset=utf-8"));
        assert!(e.supports("text/markdown"));
        assert!(!e.supports("application/pdf"));
        assert!(!e.supports("image/png"));
        assert!(!e.supports("application/octet-stream"));
    }

    #[tokio::test]
    async fn extracts_single_page() {
        let pages = TextExtractor
            .extract(b"hello world", "text/plain")
            .await
            .unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].page_number, None);
        assert_eq!(pages[0].text, "hello world");
    }

    #[tokio::test]
    async fn empty_input_yields_no_pages() {
        let pages = TextExtractor
            .extract(b"   \n\t", "text/plain")
            .await
            .unwrap();
        assert!(pages.is_empty());
    }

    #[tokio::test]
    async fn lossy_decoding_does_not_fail() {
        let pages = TextExtractor
            .extract(&[0xff, 0xfe, b'h', b'i'], "text/plain")
            .await
            .unwrap();
        assert_eq!(pages.len(), 1);
        assert!(pages[0].text.contains("hi"));
    }
}
