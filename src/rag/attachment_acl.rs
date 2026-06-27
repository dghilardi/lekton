//! Derivation of the access levels an attachment's RAG chunks should carry.
//!
//! An attachment inherits the access levels of the documents that reference it,
//! but only from those that are actually visible: published (non-draft) and not
//! archived. Drafts are excluded so an attachment never leaks through a
//! draft-only reference; archived documents grant no access.
//!
//! The result is the **set** of distinct access levels (a chunk matches when its
//! list intersects the caller's levels), so an attachment shared by a `firmware`
//! document and a `cloud` document is visible to either audience but not to a
//! third, unrelated one.

/// The fields of a referencing document needed to decide attachment visibility.
pub struct ReferencingDoc<'a> {
    pub access_level: &'a str,
    pub is_draft: bool,
    pub is_archived: bool,
}

/// Compute the sorted, de-duplicated access levels for an attachment from its
/// referencing documents. Returns an empty vec when no referencing document is
/// currently visible — such chunks match no caller and are effectively hidden
/// until a referencing document is published.
pub fn attachment_access_levels(referencing: &[ReferencingDoc]) -> Vec<String> {
    let mut set = std::collections::BTreeSet::new();
    for doc in referencing {
        if !doc.is_draft && !doc.is_archived {
            set.insert(doc.access_level.to_string());
        }
    }
    set.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(access_level: &str, is_draft: bool, is_archived: bool) -> ReferencingDoc<'_> {
        ReferencingDoc {
            access_level,
            is_draft,
            is_archived,
        }
    }

    #[test]
    fn unions_distinct_levels_from_published_docs() {
        let refs = [doc("firmware", false, false), doc("cloud", false, false)];
        assert_eq!(attachment_access_levels(&refs), vec!["cloud", "firmware"]);
    }

    #[test]
    fn deduplicates_repeated_levels() {
        let refs = [doc("cloud", false, false), doc("cloud", false, false)];
        assert_eq!(attachment_access_levels(&refs), vec!["cloud"]);
    }

    #[test]
    fn excludes_drafts_and_archived() {
        let refs = [
            doc("firmware", true, false), // draft → excluded
            doc("cloud", false, true),    // archived → excluded
            doc("desktop-pc", false, false),
        ];
        assert_eq!(attachment_access_levels(&refs), vec!["desktop-pc"]);
    }

    #[test]
    fn empty_when_no_visible_referencing_doc() {
        let refs = [doc("firmware", true, false)];
        assert!(attachment_access_levels(&refs).is_empty());
        assert!(attachment_access_levels(&[]).is_empty());
    }
}
