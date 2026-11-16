//! Read-only comparison of MongoDB, the source of truth, with S3, Meilisearch
//! and Qdrant.
//!
//! It only lists and counts: it never deletes or modifies anything. Its
//! membership rules are the ones the reindexes apply (the latest copy of each
//! slug, not archived, not hidden, and for RAG not opted out), so a finding is
//! something a reindex or an operator would act on.
//!
//! Each store is listed once, MongoDB first. Writes made while the report runs
//! can therefore show up as transient findings (an import writes S3 before
//! MongoDB, for one); running it again tells them from real ones.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::db::asset_repository::AssetRepository;
use crate::db::repository::DocumentRepository;
use crate::db::storage_reference_repository::StorageReferenceRepository;
use crate::error::AppError;
use crate::indexing::{attachment_is_live, hidden_references};
use crate::rag::service::{is_attachment_point, orphan_points, RagService};
use crate::search::attachment_search::AttachmentSearchService;
use crate::search::client::SearchService;
use crate::server::maintenance::{ConsistencyFinding, ConsistencyReport, CONSISTENCY_SAMPLE_LIMIT};
use crate::storage::client::StorageClient;

/// Inline images uploaded from the editor. Page bodies link them; no MongoDB
/// row owns them, so they are reported apart from orphans.
pub const INLINE_IMAGE_PREFIX: &str = "images/";

/// The stores to compare. A disabled search or RAG feature is `None`.
pub struct Sources<'a> {
    pub references: &'a dyn StorageReferenceRepository,
    pub documents: &'a dyn DocumentRepository,
    pub assets: &'a dyn AssetRepository,
    pub storage: &'a dyn StorageClient,
    pub search: Option<&'a dyn SearchService>,
    pub attachment_search: Option<&'a dyn AttachmentSearchService>,
    pub rag: Option<&'a dyn RagService>,
}

/// A count and its first few examples.
#[derive(Default)]
struct Tally {
    count: u64,
    samples: Vec<String>,
}

impl Tally {
    fn add(&mut self, sample: impl FnOnce() -> String) {
        self.count += 1;
        if self.samples.len() < CONSISTENCY_SAMPLE_LIMIT {
            self.samples.push(sample());
        }
    }

    fn of<I: IntoIterator<Item = String>>(items: I) -> Self {
        let mut tally = Self::default();
        for item in items {
            tally.add(|| item);
        }
        tally
    }

    fn finding(self, id: &str, title: &str, description: &str) -> ConsistencyFinding {
        ConsistencyFinding {
            id: id.into(),
            title: title.into(),
            description: description.into(),
            count: self.count,
            samples: self.samples,
            informational: false,
        }
    }
}

/// What MongoDB says should exist.
struct Expected {
    /// Latest copies that belong in search: not archived, not hidden.
    search_slugs: BTreeSet<String>,
    /// Of those, the ones that belong in RAG (not `skip_rag`).
    rag_slugs: HashSet<String>,
    /// Assets whose entries belong in the indexes.
    attachment_keys: HashSet<String>,
}

pub async fn build_report(sources: Sources<'_>) -> Result<ConsistencyReport, AppError> {
    // MongoDB first: everything is compared against it, so without it there
    // is no report.
    let documents = sources.documents.list_all().await?;
    let references = sources.references.list_storage_references().await?;
    let assets = sources.assets.list_all().await?;

    let mut report = ConsistencyReport::default();

    let flagged = Tally::of(documents.iter().filter(|d| d.needs_reindex).map(
        |d| match &d.release {
            Some(release) => format!("{}@{release}", d.slug),
            None => d.slug.clone(),
        },
    ));

    let hidden = hidden_references(&documents);
    let attachment_keys: HashSet<String> = assets
        .iter()
        .filter(|asset| attachment_is_live(asset, &hidden))
        .map(|asset| asset.key.clone())
        .collect();
    let active: Vec<_> = crate::rag::reindex::latest_copies(documents)
        .into_iter()
        .filter(|d| !d.is_archived)
        .collect();
    let duplicates = duplicate_copies(&active);
    let live: Vec<_> = active.into_iter().filter(|d| !d.is_hidden).collect();
    let expected = Expected {
        search_slugs: live.iter().map(|d| d.slug.clone()).collect(),
        rag_slugs: live
            .iter()
            .filter(|d| !d.skip_rag)
            .map(|d| d.slug.clone())
            .collect(),
        attachment_keys,
    };
    report.totals.push((
        "Live documents (latest copy, not archived or hidden)".into(),
        expected.search_slugs.len() as u64,
    ));

    compare_storage(&sources, &references, &mut report).await;
    compare_search(&sources, &expected, &mut report).await;
    compare_attachment_search(&sources, &expected, &mut report).await;
    compare_rag(&sources, &expected, &mut report).await;

    let mut duplicates = duplicates.finding(
        "duplicate_documents",
        "Same source file and content under several slugs",
        "These active documents share their source path and content hash but have different \
         slugs, so search and chat show the same text twice. They are typically left over \
         from an import made before source ids, later replaced under new slugs. Archive the \
         stale copy once confirmed; nothing is changed automatically.",
    );
    duplicates.informational = true;
    report.findings.push(duplicates);
    report.findings.push(flagged.finding(
        "needs_reindex",
        "Documents flagged for reindex",
        "An index write failed for these documents (needs_reindex). The next reindex, or \
         saving the document again, retries them.",
    ));
    Ok(report)
}

/// Groups of active documents with the same `source_path` and `content_hash`
/// under different slugs, one sample per group: `path: slug-a, slug-b`.
fn duplicate_copies(active: &[crate::db::models::Document]) -> Tally {
    let mut groups = BTreeMap::<(&str, &str), BTreeSet<&str>>::new();
    for document in active {
        if let (Some(path), Some(hash)) = (&document.source_path, &document.content_hash) {
            groups
                .entry((path.as_str(), hash.as_str()))
                .or_default()
                .insert(document.slug.as_str());
        }
    }
    Tally::of(
        groups
            .into_iter()
            .filter(|(_, slugs)| slugs.len() > 1)
            .map(|((path, _), slugs)| {
                format!(
                    "{path}: {}",
                    slugs.into_iter().collect::<Vec<_>>().join(", ")
                )
            }),
    )
}

async fn compare_storage(
    sources: &Sources<'_>,
    references: &[crate::db::storage_reference_repository::StorageReference],
    report: &mut ConsistencyReport,
) {
    let owners: HashMap<&str, &str> = references
        .iter()
        .map(|r| (r.key.as_str(), r.owner.as_str()))
        .collect();

    let mut listed = HashSet::new();
    let mut orphans = Tally::default();
    let mut images = Tally::default();
    let mut continuation: Option<String> = None;
    loop {
        let page = match sources.storage.list("", continuation.as_deref()).await {
            Ok(page) => page,
            Err(e) => {
                report.skipped.push(format!(
                    "S3 could not be listed, storage was not compared: {e}"
                ));
                return;
            }
        };
        for key in page.keys {
            if !owners.contains_key(key.as_str()) {
                if key.starts_with(INLINE_IMAGE_PREFIX) {
                    images.add(|| key.clone());
                } else {
                    orphans.add(|| key.clone());
                }
            }
            listed.insert(key);
        }
        match page.next {
            Some(next) => continuation = Some(next),
            None => break,
        }
    }

    let missing = Tally::of(
        references
            .iter()
            .filter(|r| !listed.contains(&r.key))
            .map(|r| format!("{} ({})", r.key, r.owner))
            .collect::<BTreeSet<_>>(),
    );

    report
        .totals
        .push(("S3 objects".into(), listed.len() as u64));
    report
        .totals
        .push(("S3 keys referenced by MongoDB".into(), owners.len() as u64));
    report.findings.push(orphans.finding(
        "s3_orphans",
        "S3 objects without a MongoDB owner",
        "No document, revision, asset, schema version, prompt, prompt version or active import \
         references these objects. They are not served and can be deleted once confirmed.",
    ));
    report.findings.push(missing.finding(
        "s3_missing",
        "MongoDB rows whose S3 object is missing",
        "These rows point at an object S3 does not have: reading them fails. Re-import the \
         document, asset or schema to restore it.",
    ));
    let mut images = images.finding(
        "s3_inline_images",
        "Inline editor images",
        "Images uploaded from the editor are linked from page bodies only and have no MongoDB \
         owner by design, so they are not counted as orphans.",
    );
    images.informational = true;
    report.findings.push(images);
}

async fn compare_search(
    sources: &Sources<'_>,
    expected: &Expected,
    report: &mut ConsistencyReport,
) {
    let Some(search) = sources.search else {
        report
            .skipped
            .push("Full-text search is disabled: Meilisearch documents were not compared.".into());
        return;
    };
    let indexed: BTreeSet<String> = match search.list_indexed_slugs().await {
        Ok(slugs) => slugs.into_iter().collect(),
        Err(e) => {
            report.skipped.push(format!(
                "Meilisearch documents could not be listed and were not compared: {e}"
            ));
            return;
        }
    };
    report
        .totals
        .push(("Meilisearch documents".into(), indexed.len() as u64));
    report.findings.push(
        Tally::of(indexed.difference(&expected.search_slugs).cloned()).finding(
            "search_orphans",
            "Search entries without a live document",
            "Meilisearch holds these slugs, but no live document has them (deleted, archived, \
             hidden or renamed). A full search reindex removes them.",
        ),
    );
    report.findings.push(
        Tally::of(expected.search_slugs.difference(&indexed).cloned()).finding(
            "search_missing",
            "Live documents missing from search",
            "These documents cannot be found by search. A full search reindex adds them.",
        ),
    );
}

async fn compare_attachment_search(
    sources: &Sources<'_>,
    expected: &Expected,
    report: &mut ConsistencyReport,
) {
    let Some(attachment_search) = sources.attachment_search else {
        report.skipped.push(
            "Attachment search is disabled: Meilisearch attachment pages were not compared.".into(),
        );
        return;
    };
    let indexed: BTreeSet<String> = match attachment_search.list_indexed_attachment_keys().await {
        Ok(keys) => keys.into_iter().collect(),
        Err(e) => {
            report.skipped.push(format!(
                "Meilisearch attachment pages could not be listed and were not compared: {e}"
            ));
            return;
        }
    };
    report
        .totals
        .push(("Attachments with search pages".into(), indexed.len() as u64));
    report.findings.push(
        Tally::of(
            indexed
                .into_iter()
                .filter(|key| !expected.attachment_keys.contains(key)),
        )
        .finding(
            "attachment_search_orphans",
            "Attachment search pages without a live attachment",
            "Meilisearch holds pages of these attachments, but the asset is gone or linked only \
             by hidden documents. A full search reindex removes them. Attachments are only \
             checked for stale pages: not every asset can be indexed.",
        ),
    );
}

async fn compare_rag(sources: &Sources<'_>, expected: &Expected, report: &mut ConsistencyReport) {
    let Some(rag) = sources.rag else {
        report
            .skipped
            .push("RAG is disabled: Qdrant chunks were not compared.".into());
        return;
    };
    let points = match rag.list_point_sources().await {
        Ok(points) => points,
        Err(e) => {
            report.skipped.push(format!(
                "Qdrant chunks could not be listed and were not compared: {e}"
            ));
            return;
        }
    };
    report
        .totals
        .push(("Qdrant chunks".into(), points.len() as u64));

    let indexed_slugs: HashSet<String> = points
        .iter()
        .filter(|p| !is_attachment_point(p))
        .filter_map(|p| p.document_slug.clone())
        .collect();

    // Grouped by source, so a sample reads "slug (12 chunks)".
    let mut document_orphans = BTreeMap::<String, u64>::new();
    let mut attachment_orphans = BTreeMap::<String, u64>::new();
    for point in orphan_points(points, &expected.rag_slugs, &expected.attachment_keys) {
        let (group, source) = if is_attachment_point(&point) {
            (&mut attachment_orphans, point.attachment_key)
        } else {
            (&mut document_orphans, point.document_slug)
        };
        *group
            .entry(source.unwrap_or_else(|| "(no source in payload)".into()))
            .or_default() += 1;
    }
    let chunks = |orphans: BTreeMap<String, u64>| {
        let mut tally = Tally::default();
        for (source, count) in orphans {
            tally.count += count;
            if tally.samples.len() < CONSISTENCY_SAMPLE_LIMIT {
                tally.samples.push(format!("{source} ({count} chunks)"));
            }
        }
        tally
    };

    report.findings.push(chunks(document_orphans).finding(
        "rag_document_orphans",
        "RAG chunks without a live document",
        "Qdrant holds chunks of these slugs, but no live document that is not opted out of RAG \
         has them. A full RAG reindex removes them.",
    ));
    report.findings.push(chunks(attachment_orphans).finding(
        "rag_attachment_orphans",
        "RAG chunks without a live attachment",
        "Qdrant holds chunks of these attachments, but the asset is gone or linked only by \
         hidden documents. A full RAG reindex removes them.",
    ));
    let mut missing: Vec<String> = expected
        .rag_slugs
        .iter()
        .filter(|slug| !indexed_slugs.contains(*slug))
        .cloned()
        .collect();
    missing.sort();
    report.findings.push(Tally::of(missing).finding(
        "rag_missing",
        "Live documents missing from RAG",
        "These documents have no chunks, so chat cannot cite them. A document with an empty \
         body has none by nature; otherwise a full RAG reindex embeds them (paid embedding \
         calls, unless cached).",
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_all_findings_but_caps_samples() {
        let tally = Tally::of((0..100).map(|n| n.to_string()));
        assert_eq!(tally.count, 100);
        assert_eq!(tally.samples.len(), CONSISTENCY_SAMPLE_LIMIT);
    }
}
