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
    let mut versions = state
        .document_repo
        .find_all_by_slug(&slug)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    if versions.is_empty() {
        // Older lekton-sync releases stored a prefixed root `index.md` as
        // `prefix/`; serve it at the section URL until it is re-synced.
        versions = state
            .document_repo
            .find_all_by_slug(&format!("{slug}/"))
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?;
    }
    let doc = crate::db::repository::resolve_by_release(versions, &pins);

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
        return Ok(section_page(&slug, &all_docs));
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
            // Scoped to this document's own release: a relative link inside 1.0.0
            // must resolve against 1.0.0's tree, not whichever release happens to
            // share the source path.
            let siblings_docs = state
                .document_repo
                .find_all_by_source_id_and_release(source_id, doc.release.as_deref())
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

    // Release selector data. Only looked up when versioning is on and the
    // document belongs to a source, so the common case costs nothing.
    let (releases, latest_release) = match (state.features.doc_versioning, doc.source_id.as_deref())
    {
        (true, Some(source_id)) => {
            let published = state
                .release_repo
                .list_by_source(source_id)
                .await
                .map_err(|e| ServerFnError::new(e.to_string()))?;
            let latest = state
                .release_repo
                .latest(source_id)
                .await
                .map_err(|e| ServerFnError::new(e.to_string()))?;
            (
                published.into_iter().map(|r| r.release).collect::<Vec<_>>(),
                latest,
            )
        }
        _ => (Vec::new(), None),
    };

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
        source_id: doc.source_id,
        current_release: doc.release,
        releases,
        latest_release,
    }))
}

/// One entry on a section landing page.
#[cfg(feature = "ssr")]
struct SectionEntry {
    slug: String,
    title: String,
    is_section: bool,
}

/// Turns a slug segment such as `getting-started` into `Getting Started`.
#[cfg(feature = "ssr")]
fn title_from_segment(segment: &str) -> String {
    segment
        .split('-')
        .map(|word| {
            let mut c = word.chars();
            match c.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The landing page for a slug that has no document of its own but has pages
/// beneath it: its direct children by `parent_slug`, or else the next path
/// segment of every page under `slug/`. `None` when nothing lives there.
#[cfg(feature = "ssr")]
fn section_page(
    slug: &str,
    visible_docs: &[crate::db::models::Document],
) -> Option<crate::pages::DocPageData> {
    let mut children: Vec<_> = visible_docs
        .iter()
        .filter(|d| d.parent_slug.as_deref() == Some(slug))
        .collect();
    let entries: Vec<SectionEntry> = if !children.is_empty() {
        children.sort_by_key(|d| d.order);
        children
            .into_iter()
            .map(|d| SectionEntry {
                slug: d.slug.clone(),
                title: d.title.clone(),
                is_section: false,
            })
            .collect()
    } else {
        let prefix = format!("{slug}/");
        let mut seen = std::collections::HashSet::new();
        let mut entries = Vec::new();
        for doc in visible_docs {
            let Some(relative) = doc.slug.strip_prefix(&prefix) else {
                continue;
            };
            let first_segment = relative.split('/').next().unwrap_or_default();
            if first_segment.is_empty() {
                continue;
            }
            let child_slug = format!("{prefix}{first_segment}");
            if seen.insert(child_slug.clone()) {
                let title = visible_docs
                    .iter()
                    .find(|d| d.slug == child_slug)
                    .map(|d| d.title.clone())
                    .unwrap_or_else(|| title_from_segment(first_segment));
                entries.push(SectionEntry {
                    slug: child_slug,
                    title,
                    is_section: true,
                });
            }
        }
        entries.sort_by(|a, b| a.slug.cmp(&b.slug));
        entries
    };
    if entries.is_empty() {
        return None;
    }

    use crate::rendering::markdown::escape_html;
    let title = title_from_segment(slug.rsplit('/').next().unwrap_or("Section"));
    let count = entries.len();
    let mut html = format!(
        "<div class=\"not-prose space-y-6\">\
            <header class=\"border-b border-base-200 pb-4\">\
                <h1 class=\"text-3xl font-bold mb-2 break-words\">{}</h1>\
                <p class=\"max-w-2xl text-sm leading-6 text-base-content/70\">{count} {} in this section.</p>\
            </header>\
            <ul class=\"grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3\" role=\"list\">",
        escape_html(&title),
        if count == 1 { "page" } else { "pages" },
    );
    for entry in entries {
        let (kind, icon) = if entry.is_section {
            (
                "Section",
                "M3 7a2 2 0 012-2h4l2 2h6a2 2 0 012 2v7a2 2 0 01-2 2H5a2 2 0 01-2-2V7z",
            )
        } else {
            ("Document", "M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z")
        };
        html.push_str(&format!(
            "<li><a href=\"/docs/{}\" class=\"group card h-full bg-base-100 shadow-sm border border-base-200 no-underline hover:-translate-y-0.5 hover:shadow-lg hover:border-primary/30\">\
                <div class=\"card-body gap-3 p-5\">\
                    <div class=\"flex items-start gap-3\">\
                        <div class=\"mt-0.5 rounded-lg bg-primary/10 p-2 text-primary\" aria-hidden=\"true\">\
                            <svg class=\"h-5 w-5\" fill=\"none\" stroke=\"currentColor\" viewBox=\"0 0 24 24\"><path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"{icon}\"></path></svg>\
                        </div>\
                        <div class=\"min-w-0\">\
                            <p class=\"text-[0.7rem] font-semibold uppercase tracking-[0.14em] text-base-content/70\">{kind}</p>\
                            <h2 class=\"mt-1 text-lg font-semibold leading-snug text-base-content transition-colors group-hover:text-primary\">{}</h2>\
                        </div>\
                    </div>\
                </div>\
            </a></li>",
            escape_html(&entry.slug),
            escape_html(&entry.title),
        ));
    }
    html.push_str("</ul></div>");

    Some(crate::pages::DocPageData {
        title,
        html,
        headings: vec![],
        // A generated listing has no edit date of its own.
        last_updated: String::new(),
        tags: vec![],
        is_upload_doc: false,
        is_sync_doc: false,
        pdf_asset_key: None,
        source_view_url: None,
        source_id: None,
        current_release: None,
        releases: vec![],
        latest_release: None,
    })
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::*;
    use crate::db::models::Document;

    fn doc(slug: &str, title: &str) -> Document {
        Document {
            slug: slug.to_string(),
            title: title.to_string(),
            summary: None,
            s3_key: format!("docs/{slug}.md"),
            access_level: "public".to_string(),
            is_draft: false,
            service_owner: "test".to_string(),
            last_updated: chrono::Utc::now(),
            tags: vec![],
            links_out: vec![],
            backlinks: vec![],
            parent_slug: None,
            order: 0,
            is_hidden: false,
            content_hash: None,
            metadata_hash: None,
            is_archived: false,
            source_path: None,
            source_id: None,
            release: None,
            is_latest: true,
            needs_reindex: false,
            skip_rag: false,
        }
    }

    #[test]
    fn a_section_landing_has_a_heading_and_no_edit_date() {
        let docs = vec![
            doc("guides/deploy/kubernetes", "Kubernetes"),
            doc("guides/<script>", "A <b>bold</b> title"),
        ];
        let page = section_page("guides", &docs).unwrap();
        assert_eq!(page.title, "Guides");
        assert!(page
            .html
            .contains(r#"<h1 class="text-3xl font-bold mb-2 break-words">Guides</h1>"#));
        assert!(page.html.contains("2 pages in this section."));
        assert!(page.html.contains("Deploy"));
        assert!(page.html.contains("A &lt;b&gt;bold&lt;/b&gt; title"));
        assert!(!page.html.contains("<script>"));
        assert!(page.last_updated.is_empty());
    }

    #[test]
    fn a_section_landing_lists_direct_children_in_order() {
        let mut second = doc("team/b", "Second");
        second.parent_slug = Some("team".into());
        second.order = 2;
        let mut first = doc("team/a", "First");
        first.parent_slug = Some("team".into());
        first.order = 1;
        let page = section_page("team", &[second, first]).unwrap();
        let first_at = page.html.find("First").unwrap();
        assert!(first_at < page.html.find("Second").unwrap());
        assert!(page.html.contains("2 pages in this section."));
    }

    #[test]
    fn an_empty_slug_has_no_section_landing() {
        assert!(section_page("missing", &[doc("other/page", "Other")]).is_none());
    }
}
