//! PDF attachment extractor backed by the native `libpdfium` library.

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::AppError;
use crate::rag::service::AttachmentPage;

use super::vlm::VlmTranscriber;
use super::AttachmentExtractor;

/// Width (px) rendered pages are scaled to before VLM transcription.
const RENDER_TARGET_WIDTH: i32 = 1024;
/// Cap on rendered page height (px) to bound memory for tall pages.
const RENDER_MAX_HEIGHT: i32 = 1536;

/// Extracts text from PDFs via libpdfium.
///
/// Each page yields one [`AttachmentPage`] (1-based page numbers; empty pages
/// dropped). A page whose native text layer is shorter than
/// `page_text_threshold` is considered image-heavy and, when a [`VlmTranscriber`]
/// is configured, is rendered to an image and transcribed instead.
///
/// pdfium is synchronous and non-`Send`, so loading and rendering run inside
/// `spawn_blocking`; only the (async) VLM calls happen on the async runtime.
pub struct PdfExtractor {
    page_text_threshold: usize,
    vlm: Option<Arc<VlmTranscriber>>,
}

impl PdfExtractor {
    pub fn new(page_text_threshold: usize, vlm: Option<Arc<VlmTranscriber>>) -> Self {
        Self {
            page_text_threshold,
            vlm,
        }
    }
}

/// A page's native text plus, for image-heavy pages, its rendered PNG.
struct RawPage {
    page_number: u32,
    native_text: String,
    image: Option<Vec<u8>>,
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
        let threshold = self.page_text_threshold;
        let render_images = self.vlm.is_some();

        let raw = tokio::task::spawn_blocking(move || load_pdf(&bytes, threshold, render_images))
            .await
            .map_err(|e| AppError::Internal(format!("pdf extraction task failed: {e}")))??;

        let mut pages = Vec::new();
        for page in raw {
            // Prefer VLM transcription for image-heavy pages, falling back to the
            // (sparse) native text if transcription fails or returns nothing.
            let text = match (&page.image, &self.vlm) {
                (Some(png), Some(vlm)) => match vlm.transcribe_page(png).await {
                    Ok(t) if !t.trim().is_empty() => t,
                    Ok(_) => page.native_text.clone(),
                    Err(e) => {
                        tracing::warn!(
                            page = page.page_number,
                            "VLM transcription failed, using native text: {e}"
                        );
                        page.native_text.clone()
                    }
                },
                _ => page.native_text.clone(),
            };

            let trimmed = text.trim();
            if !trimmed.is_empty() {
                pages.push(AttachmentPage {
                    page_number: Some(page.page_number),
                    text: trimmed.to_string(),
                });
            }
        }

        Ok(pages)
    }
}

/// Bind libpdfium, load the document, extract per-page native text, and render
/// image-heavy pages to PNG when `render_images` is set. Blocking.
fn load_pdf(
    bytes: &[u8],
    page_text_threshold: usize,
    render_images: bool,
) -> Result<Vec<RawPage>, AppError> {
    use pdfium_render::prelude::*;

    let bindings = Pdfium::bind_to_system_library()
        .map_err(|e| AppError::Internal(format!("failed to bind libpdfium: {e}")))?;
    let pdfium = Pdfium::new(bindings);

    let document = pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .map_err(|e| AppError::Internal(format!("failed to load PDF: {e}")))?;

    let mut pages = Vec::new();
    for (index, page) in document.pages().iter().enumerate() {
        let native_text = page.text().map(|t| t.all()).unwrap_or_default();
        let page_number = (index + 1) as u32;

        // Render image-heavy pages (sparse native text) for VLM transcription.
        let image = if render_images && native_text.trim().chars().count() < page_text_threshold {
            render_page_png(&page)
        } else {
            None
        };

        // Skip pages with neither usable text nor a rendered image.
        if native_text.trim().is_empty() && image.is_none() {
            continue;
        }

        pages.push(RawPage {
            page_number,
            native_text,
            image,
        });
    }

    Ok(pages)
}

/// Render a single page to PNG bytes; `None` on any rendering/encoding failure
/// (the caller falls back to native text).
fn render_page_png(page: &pdfium_render::prelude::PdfPage) -> Option<Vec<u8>> {
    use pdfium_render::prelude::PdfRenderConfig;

    let config = PdfRenderConfig::new()
        .set_target_width(RENDER_TARGET_WIDTH)
        .set_maximum_height(RENDER_MAX_HEIGHT);

    let bitmap = page.render_with_config(&config).ok()?;
    let width = bitmap.width() as u32;
    let height = bitmap.height() as u32;
    let rgba = bitmap.as_rgba_bytes();

    let img = image::RgbaImage::from_raw(width, height, rgba)?;
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut png, image::ImageFormat::Png)
        .ok()?;
    Some(png.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_only_pdf() {
        let e = PdfExtractor::new(100, None);
        assert!(e.supports("application/pdf"));
        assert!(!e.supports("text/plain"));
        assert!(!e.supports("image/png"));
    }
}
