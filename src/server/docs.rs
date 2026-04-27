use leptos::prelude::*;

#[cfg(feature = "ssr")]
use crate::app::{doc_is_accessible, AppState};
#[cfg(feature = "ssr")]
use crate::server::request_document_visibility;

#[server(GetDocHtml, "/api")]
pub async fn get_doc_html(
    slug: String,
) -> Result<Option<crate::pages::DocPageData>, ServerFnError> {
    use crate::rendering::markdown::{extract_headings, render_markdown};

    let state = expect_context::<AppState>();

    let doc = state
        .document_repo
        .find_by_slug(&slug)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let Some(doc) = doc else {
        let (allowed_levels, include_draft) = request_document_visibility(&state).await?;
        let all_docs = state
            .document_repo
            .list_by_access_levels(allowed_levels.as_deref(), include_draft)
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

            let mut html = String::from("<p class=\"text-base-content/70 pb-4 border-b border-base-200\">Select a document from this section to read.</p><div class=\"grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4 mt-6\">");
            for (child_slug, child_title) in virtual_children {
                html.push_str(&format!(
                    "<a href=\"/docs/{child_slug}\" class=\"card bg-base-100 shadow-sm border border-base-200 hover:shadow-md transition-shadow hover:border-primary/30\"><div class=\"card-body p-5\"><h2 class=\"card-title text-lg flex items-center gap-2\"><svg class=\"w-5 h-5 text-primary opacity-80\" fill=\"none\" stroke=\"currentColor\" viewBox=\"0 0 24 24\"><path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"M3 7a2 2 0 012-2h4l2 2h6a2 2 0 012 2v7a2 2 0 01-2 2H5a2 2 0 01-2-2V7z\"></path></svg>{child_title}</h2></div></a>",
                ));
            }
            html.push_str("</div>");

            return Ok(Some(crate::pages::DocPageData {
                title,
                html,
                headings: vec![],
                last_updated: chrono::Utc::now().format("%B %d, %Y").to_string(),
                tags: vec![],
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

        let mut html = String::from("<p class=\"text-base-content/70 pb-4 border-b border-base-200\">Select a document from this section to read.</p><div class=\"grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4 mt-6\">");
        for child in children {
            html.push_str(&format!(
                "<a href=\"/docs/{}\" class=\"card bg-base-100 shadow-sm border border-base-200 hover:shadow-md transition-shadow hover:border-primary/30\"><div class=\"card-body p-5\"><h2 class=\"card-title text-lg flex items-center gap-2\"><svg class=\"w-5 h-5 text-primary opacity-80\" fill=\"none\" stroke=\"currentColor\" viewBox=\"0 0 24 24\"><path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z\"></path></svg>{}</h2></div></a>",
                child.slug, child.title
            ));
        }
        html.push_str("</div>");

        return Ok(Some(crate::pages::DocPageData {
            title,
            html,
            headings: vec![],
            last_updated: chrono::Utc::now().format("%B %d, %Y").to_string(),
            tags: vec![],
        }));
    };

    let (allowed_levels, include_draft) = request_document_visibility(&state).await?;
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

    let html = render_markdown(&raw);
    let headings = extract_headings(&raw);
    let last_updated = doc.last_updated.format("%B %d, %Y").to_string();

    Ok(Some(crate::pages::DocPageData {
        title: doc.title,
        html,
        headings,
        last_updated,
        tags: doc.tags,
    }))
}
