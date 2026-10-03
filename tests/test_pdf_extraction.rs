/// Integration tests for PDF extraction against a real libpdfium.
///
/// libpdfium is a system library, so these tests skip when it cannot be
/// loaded. CI installs the version pinned in the Dockerfile and sets
/// `LEKTON_REQUIRE_PDFIUM=1`, which turns a missing library into a failure.
#[cfg(feature = "ssr")]
mod pdf_extraction {
    use lekton::rag::extraction::{extract_preview, AttachmentExtractor, PdfExtractor};

    const TWO_PAGES: &[u8] = include_bytes!("fixtures/two-pages.pdf");

    /// Returns false (and the caller skips) when libpdfium is not installed.
    async fn pdfium_available() -> bool {
        match extract_preview(TWO_PAGES, 1).await {
            Ok(_) => true,
            Err(e) if std::env::var("LEKTON_REQUIRE_PDFIUM").is_ok() => {
                panic!("libpdfium is required but unavailable: {e}")
            }
            Err(e) => {
                eprintln!("skipping: libpdfium unavailable: {e}");
                false
            }
        }
    }

    // pdfium-render binds libpdfium once per process; every extraction after
    // the first must reuse that binding instead of failing.
    #[tokio::test]
    async fn extracts_repeatedly_in_one_process() {
        if !pdfium_available().await {
            return;
        }
        let extractor = PdfExtractor::new(10, None);

        for _ in 0..3 {
            let preview = extract_preview(TWO_PAGES, 1).await.expect("preview");
            assert_eq!(preview, "Lekton first page");

            let pages = extractor
                .extract(TWO_PAGES, "application/pdf")
                .await
                .expect("extract");
            let texts: Vec<_> = pages
                .iter()
                .map(|p| (p.page_number, p.text.as_str()))
                .collect();
            assert_eq!(
                texts,
                vec![
                    (Some(1), "Lekton first page"),
                    (Some(2), "Lekton second page")
                ]
            );
        }
    }
}
