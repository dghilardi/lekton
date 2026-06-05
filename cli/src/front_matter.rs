use crate::config::FrontMatter;

/// Strip YAML front matter from a markdown file and return (front_matter, body).
/// The body is everything after the closing `---` delimiter.
pub fn parse_front_matter(source: &str) -> (FrontMatter, String) {
    // Strip UTF-8 BOM (U+FEFF) that some Windows editors prepend
    let source = source.strip_prefix('\u{FEFF}').unwrap_or(source);
    // Support both Unix and Windows line endings for the opening delimiter
    let after_open = if source.starts_with("---\r\n") {
        &source[5..]
    } else if source.starts_with("---\n") {
        &source[4..]
    } else {
        return (FrontMatter::default(), source.to_string());
    };

    // Find the closing "---" on its own line
    const END_MARKER: &str = "\n---";
    if let Some(idx) = after_open.find(END_MARKER) {
        let fm_str = &after_open[..idx];
        let after_marker = &after_open[idx + END_MARKER.len()..];
        let body = if after_marker.starts_with("\r\n") {
            after_marker[2..].to_string()
        } else if after_marker.starts_with('\n') {
            after_marker[1..].to_string()
        } else {
            after_marker.to_string()
        };
        let fm = serde_yaml::from_str(fm_str).unwrap_or_default();
        (fm, body)
    } else {
        (FrontMatter::default(), source.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_front_matter_unix() {
        let src = "---\ntitle: My Doc\nslug: my-doc\n---\n# Body text";
        let (fm, body) = parse_front_matter(src);
        assert_eq!(fm.title.as_deref(), Some("My Doc"));
        assert_eq!(fm.slug.as_deref(), Some("my-doc"));
        assert_eq!(body, "# Body text");
    }

    #[test]
    fn parse_front_matter_windows() {
        let src = "---\r\ntitle: Win\r\n---\r\n# Body";
        let (fm, body) = parse_front_matter(src);
        assert_eq!(fm.title.as_deref(), Some("Win"));
        assert_eq!(body, "# Body");
    }

    #[test]
    fn parse_front_matter_windows_lekton_import() {
        let src = "---\r\nlekton-import: true\r\ntitle: Win\r\n---\r\n# Body";
        let (fm, _) = parse_front_matter(src);
        assert!(
            fm.lekton_import,
            "lekton_import should be true with CRLF line endings"
        );
    }

    #[test]
    fn parse_front_matter_utf8_bom() {
        let src = "\u{FEFF}---\nlekton-import: true\ntitle: BOM Doc\n---\n# Body";
        let (fm, body) = parse_front_matter(src);
        assert!(
            fm.lekton_import,
            "lekton_import should be true despite leading BOM"
        );
        assert_eq!(fm.title.as_deref(), Some("BOM Doc"));
        assert_eq!(body, "# Body");
    }

    #[test]
    fn parse_front_matter_utf8_bom_crlf() {
        let src = "\u{FEFF}---\r\nlekton-import: true\r\ntitle: BOM CRLF\r\n---\r\n# Body";
        let (fm, _) = parse_front_matter(src);
        assert!(
            fm.lekton_import,
            "lekton_import should be true with BOM + CRLF"
        );
    }

    #[test]
    fn parse_front_matter_missing() {
        let src = "# No front matter\n\nJust a doc.";
        let (fm, body) = parse_front_matter(src);
        assert!(fm.title.is_none());
        assert_eq!(body, src);
    }

    #[test]
    fn parse_front_matter_accepts_kebab_case_keys() {
        let src = "---\naccess-level: public\nservice-owner: docs\nparent-slug: guides\nis-hidden: true\nlekton-import: true\n---\n# Body";
        let (fm, _) = parse_front_matter(src);
        assert_eq!(fm.access_level.as_deref(), Some("public"));
        assert_eq!(fm.service_owner.as_deref(), Some("docs"));
        assert_eq!(fm.parent_slug.as_deref(), Some("guides"));
        assert_eq!(fm.is_hidden, Some(true));
        assert!(fm.lekton_import);
    }

    #[test]
    fn parse_front_matter_accepts_camel_case_keys() {
        let src = "---\nsummary: Brief guide to the platform deployment workflow.\naccessLevel: developer\nserviceOwner: platform\nparentSlug: docs\nisHidden: true\nlektonImport: true\n---\n# Body";
        let (fm, _) = parse_front_matter(src);
        assert_eq!(
            fm.summary.as_deref(),
            Some("Brief guide to the platform deployment workflow.")
        );
        assert_eq!(fm.access_level.as_deref(), Some("developer"));
        assert_eq!(fm.service_owner.as_deref(), Some("platform"));
        assert_eq!(fm.parent_slug.as_deref(), Some("docs"));
        assert_eq!(fm.is_hidden, Some(true));
        assert!(fm.lekton_import);
    }
}
