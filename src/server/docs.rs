use leptos::prelude::*;

#[cfg(feature = "ssr")]
use crate::app::{doc_is_accessible, AppState};
#[cfg(feature = "ssr")]
use crate::server::request_document_visibility;

#[server(GetDocHtml, "/api")]
pub async fn get_doc_html(
    slug: String,
    pins: Option<Vec<String>>,
) -> Result<Option<crate::pages::DocPageData>, ServerFnError> {
    use crate::rendering::markdown::{extract_headings, render_markdown};

    let state = expect_context::<AppState>();

    // Resolves to the pinned release when one is active, otherwise `latest`.
    let pins = crate::server::resolve_release_pins(&state, &pins.unwrap_or_default()).await?;
    let doc = crate::db::repository::resolve_by_release(
        state
            .document_repo
            .find_all_by_slug(&slug)
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?,
        &pins,
    );

    let Some(doc) = doc else {
        let (allowed_levels, include_draft) = request_document_visibility(&state).await?;
        let all_docs = state
            .document_repo
            .list_by_access_levels(
                allowed_levels.as_deref(),
                include_draft,
                &crate::versioning::ReleasePins::default(),
            )
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?;

        let mut children: Vec<_> = all_docs
            .iter()
            .filter(|d| d.parent_slug.as_deref() == Some(slug.as_str()))
            .cloned()
            .collect();

        if children.is_empty() {
            let prefix = format!("{}/", slug);
            let mut seen = std::collections::HashSet::new();
            let mut virtual_children: Vec<(String, String)> = Vec::new();
            for doc in &all_docs {
                if let Some(relative) = doc.slug.strip_prefix(&prefix) {
                    let first_segment = relative.split('/').next().unwrap_or_default();
                    if first_segment.is_empty() {
                        continue;
                    }
                    let child_slug = format!("{}/{}", slug, first_segment);
                    if seen.insert(child_slug.clone()) {
                        let title = all_docs
                            .iter()
                            .find(|d| d.slug == child_slug)
                            .map(|d| d.title.clone())
                            .unwrap_or_else(|| {
                                first_segment
                                    .split('-')
                                    .map(|word| {
                                        let mut c = word.chars();
                                        match c.next() {
                                            None => String::new(),
                                            Some(f) => {
                                                f.to_uppercase().collect::<String>() + c.as_str()
                                            }
                                        }
                                    })
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            });
                        virtual_children.push((child_slug, title));
                    }
                }
            }

            if virtual_children.is_empty() {
                return Ok(None);
            }

            virtual_children.sort_by(|a, b| a.0.cmp(&b.0));

            let title_part = slug.split('/').next_back().unwrap_or("Section");
            let title = title_part
                .split('-')
                .map(|word| {
                    let mut c = word.chars();
                    match c.next() {
                        None => String::new(),
                        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");

            let mut html = String::from(
                "<div class=\"not-prose space-y-6\">\
                    <div class=\"flex items-start justify-between gap-4 border-b border-base-200 pb-4\">\
                        <p class=\"max-w-2xl text-sm leading-6 text-base-content/70\">Select a document from this section to read.</p>\
                    </div>\
                    <div class=\"grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3\">",
            );
            for (child_slug, child_title) in virtual_children {
                html.push_str(&format!(
                    "<a href=\"/docs/{child_slug}\" class=\"group card bg-base-100 shadow-sm border border-base-200 no-underline hover:-translate-y-0.5 hover:shadow-lg hover:border-primary/30\">\
                        <div class=\"card-body gap-3 p-5\">\
                            <div class=\"flex items-start gap-3\">\
                                <div class=\"mt-0.5 rounded-lg bg-primary/10 p-2 text-primary\">\
                                    <svg class=\"h-5 w-5\" fill=\"none\" stroke=\"currentColor\" viewBox=\"0 0 24 24\"><path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"M3 7a2 2 0 012-2h4l2 2h6a2 2 0 012 2v7a2 2 0 01-2 2H5a2 2 0 01-2-2V7z\"></path></svg>\
                                </div>\
                                <div class=\"min-w-0\">\
                                    <p class=\"text-[0.7rem] font-semibold uppercase tracking-[0.14em] text-base-content/45\">Section</p>\
                                    <h2 class=\"mt-1 text-lg font-semibold leading-snug text-base-content transition-colors group-hover:text-primary\">{child_title}</h2>\
                                </div>\
                            </div>\
                        </div>\
                    </a>",
                ));
            }
            html.push_str("</div></div>");

            return Ok(Some(crate::pages::DocPageData {
                title,
                html,
                headings: vec![],
                last_updated: chrono::Utc::now().format("%B %d, %Y").to_string(),
                tags: vec![],
                is_upload_doc: false,
                is_sync_doc: false,
                pdf_asset_key: None,
                source_view_url: None,
            }));
        }

        children.sort_by_key(|d| d.order);

        let title_part = slug.split('/').next_back().unwrap_or("Section");
        let title = title_part
            .split('-')
            .map(|word| {
                let mut c = word.chars();
                match c.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ");

        let mut html = String::from(
            "<div class=\"not-prose space-y-6\">\
                <div class=\"flex items-start justify-between gap-4 border-b border-base-200 pb-4\">\
                    <p class=\"max-w-2xl text-sm leading-6 text-base-content/70\">Select a document from this section to read.</p>\
                </div>\
                <div class=\"grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3\">",
        );
        for child in children {
            html.push_str(&format!(
                "<a href=\"/docs/{}\" class=\"group card bg-base-100 shadow-sm border border-base-200 no-underline hover:-translate-y-0.5 hover:shadow-lg hover:border-primary/30\">\
                    <div class=\"card-body gap-3 p-5\">\
                        <div class=\"flex items-start gap-3\">\
                            <div class=\"mt-0.5 rounded-lg bg-primary/10 p-2 text-primary\">\
                                <svg class=\"h-5 w-5\" fill=\"none\" stroke=\"currentColor\" viewBox=\"0 0 24 24\"><path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z\"></path></svg>\
                            </div>\
                            <div class=\"min-w-0\">\
                                <p class=\"text-[0.7rem] font-semibold uppercase tracking-[0.14em] text-base-content/45\">Document</p>\
                                <h2 class=\"mt-1 text-lg font-semibold leading-snug text-base-content transition-colors group-hover:text-primary\">{}</h2>\
                            </div>\
                        </div>\
                    </div>\
                </a>",
                child.slug, child.title
            ));
        }
        html.push_str("</div></div>");

        return Ok(Some(crate::pages::DocPageData {
            title,
            html,
            headings: vec![],
            last_updated: chrono::Utc::now().format("%B %d, %Y").to_string(),
            tags: vec![],
            is_upload_doc: false,
            is_sync_doc: false,
            pdf_asset_key: None,
            source_view_url: None,
        }));
    };

    let (allowed_levels, include_draft) = request_document_visibility(&state).await?;
    if doc.is_archived {
        return Ok(None);
    }
    if !doc_is_accessible(
        &doc.access_level,
        doc.is_draft,
        allowed_levels.as_deref(),
        include_draft,
    ) {
        return Ok(None);
    }

    let content_bytes = state
        .storage_client
        .get_object(&doc.s3_key)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let Some(content_bytes) = content_bytes else {
        return Ok(None);
    };

    let raw = String::from_utf8(content_bytes).map_err(|e| ServerFnError::new(e.to_string()))?;

    let last_updated = doc.last_updated.format("%B %d, %Y").to_string();
    let is_upload_doc = doc.service_owner == "document-upload";
    // Externally managed (ingest API / lekton-sync) when it carries a source id
    // and isn't an upload-form document.
    let is_sync_doc = !is_upload_doc && doc.source_id.as_deref().is_some_and(|s| !s.is_empty());

    // For sync documents, offer a "view source" link when the source is
    // registered with a recognized provider repo URL. Absent registration or an
    // unknown host leaves this `None` and the page stays read-only.
    let source_view_url = if is_sync_doc {
        match (doc.source_id.as_deref(), doc.source_path.as_deref()) {
            (Some(source_id), Some(source_path)) => state
                .document_source_repo
                .find_by_id(source_id)
                .await
                .map_err(|e| ServerFnError::new(e.to_string()))?
                .and_then(|source| source.source_view_url(source_path)),
            _ => None,
        }
    } else {
        None
    };

    // Body rendering diverges by provenance:
    // - Upload documents get a specialized PDF layout, so `html` holds only the
    //   rendered summary (the PDF link is shown as a native download card) and
    //   there is no table of contents. The asset key drives that card.
    // - Everything else renders the full markdown body, with sync link-rewriting.
    let (html, headings, pdf_asset_key) = if is_upload_doc {
        let summary_html = doc
            .summary
            .as_deref()
            .map(render_markdown)
            .unwrap_or_default();
        let pdf_asset_key = crate::rendering::links::extract_asset_keys(&raw)
            .into_iter()
            .next();
        (summary_html, Vec::new(), pdf_asset_key)
    } else {
        let base_html = render_markdown(&raw);
        let html = if let Some(ref source_id) = doc.source_id {
            use crate::rendering::link_transform::{
                build_siblings_map, rewrite_links_in_html, LinkContext, TransformTarget,
            };
            let siblings_docs = state
                .document_repo
                .find_all_by_source_id(source_id)
                .await
                .map_err(|e| ServerFnError::new(e.to_string()))?;
            let siblings = build_siblings_map(&siblings_docs);
            let ctx = LinkContext {
                source_path: doc.source_path.as_deref(),
                siblings: &siblings,
            };
            rewrite_links_in_html(&base_html, &ctx, TransformTarget::Web)
        } else {
            base_html
        };
        (html, extract_headings(&raw), None)
    };

    let kind = if is_upload_doc {
        "upload"
    } else if is_sync_doc {
        "sync"
    } else {
        "markdown"
    };
    metrics::counter!("lekton_document_views_total", "kind" => kind).increment(1);

    Ok(Some(crate::pages::DocPageData {
        title: doc.title,
        html,
        headings,
        last_updated,
        tags: doc.tags,
        is_upload_doc,
        is_sync_doc,
        pdf_asset_key,
        source_view_url,
    }))
}
