use std::path::Path;

pub const SUMMARY_RECOMMENDED_MIN_CHARS: usize = 50;
pub const SUMMARY_RECOMMENDED_MAX_CHARS: usize = 200;

/// Derive a slug from the file path relative to root (strips the `.md` extension).
/// `index.md` and `README.md` map to their parent directory slug.
/// e.g., `docs/guides/intro.md` → `docs/guides/intro`
///       `docs/guides/index.md` → `docs/guides`
pub fn slug_from_path(file: &Path, root: &Path) -> String {
    let relative = file.strip_prefix(root).unwrap_or(file);
    if is_index_file(relative) {
        return relative
            .parent()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
    }
    let without_ext = relative.with_extension("");
    without_ext.to_string_lossy().replace('\\', "/")
}

/// Returns true if the path filename is `index.md` or `README.md` (case-insensitive).
pub fn is_index_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|f| f.to_str())
        .map(|f| matches!(f.to_lowercase().as_str(), "readme.md" | "index.md"))
        .unwrap_or(false)
}

/// Derive the source_path (relative file path with extension) from an absolute path.
/// e.g., `<root>/docs/guides/intro.md` → `docs/guides/intro.md`
pub fn source_path_from_file(file: &Path, root: &Path) -> String {
    let relative = file.strip_prefix(root).unwrap_or(file);
    relative.to_string_lossy().replace('\\', "/")
}

/// Normalise a title into a URL-safe slug segment.
///
/// Rules: lowercase, collapse any run of non-alphanumeric characters into a
/// single `-`, strip leading/trailing `-`.
///
/// e.g., `"Guidelines & Best Practices!"` → `"guidelines-best-practices"`
pub fn slug_from_title(title: &str) -> String {
    let mut result = String::with_capacity(title.len());
    let mut prev_sep = true; // start true so leading separators are dropped
    for c in title.chars() {
        if c.is_alphanumeric() {
            result.extend(c.to_lowercase());
            prev_sep = false;
        } else if !prev_sep {
            result.push('-');
            prev_sep = true;
        }
    }
    result.trim_end_matches('-').to_string()
}

pub fn normalize_summary(summary: Option<String>) -> Option<String> {
    summary
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn warn_about_summary(source_path: &str, summary: Option<&str>) {
    match summary {
        None => eprintln!(
            "Warning: document '{source_path}' has no summary; recommended summary length is {SUMMARY_RECOMMENDED_MIN_CHARS}-{SUMMARY_RECOMMENDED_MAX_CHARS} characters"
        ),
        Some(summary) => {
            let len = summary.chars().count();
            if !(SUMMARY_RECOMMENDED_MIN_CHARS..=SUMMARY_RECOMMENDED_MAX_CHARS).contains(&len) {
                eprintln!(
                    "Warning: document '{source_path}' summary is {len} characters; recommended length is {SUMMARY_RECOMMENDED_MIN_CHARS}-{SUMMARY_RECOMMENDED_MAX_CHARS} characters"
                );
            }
        }
    }
}

pub fn prompt_slug_from_path(file: &Path, root: &Path) -> String {
    let relative = file.strip_prefix(root).unwrap_or(file);
    let without_ext = relative.with_extension("");
    without_ext.to_string_lossy().replace('\\', "/")
}

pub fn schema_name_from_dir(dir: &Path, root: &Path) -> String {
    dir.strip_prefix(root)
        .unwrap_or(dir)
        .to_string_lossy()
        .replace('\\', "/")
}

pub fn apply_prefix(prefix: Option<&str>, raw_slug: &str) -> String {
    match prefix.map(str::trim).filter(|p| !p.is_empty()) {
        Some(prefix) if raw_slug == prefix || raw_slug.starts_with(&format!("{prefix}/")) => {
            raw_slug.to_string()
        }
        Some(prefix) => format!("{prefix}/{raw_slug}"),
        None => raw_slug.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_from_path_strips_extension() {
        let root = Path::new("/docs");
        let file = Path::new("/docs/guides/intro.md");
        assert_eq!(slug_from_path(file, root), "guides/intro");
    }

    #[test]
    fn slug_from_path_readme_at_root_maps_to_empty() {
        let root = Path::new("/repo");
        let file = Path::new("/repo/README.md");
        assert_eq!(slug_from_path(file, root), "");
    }

    #[test]
    fn slug_from_path_index_at_root_maps_to_empty() {
        let root = Path::new("/repo");
        let file = Path::new("/repo/index.md");
        assert_eq!(slug_from_path(file, root), "");
    }

    #[test]
    fn slug_from_path_readme_in_subdir_maps_to_dir() {
        let root = Path::new("/repo");
        let file = Path::new("/repo/docs/operations/README.md");
        assert_eq!(slug_from_path(file, root), "docs/operations");
    }

    #[test]
    fn slug_from_path_index_in_subdir_maps_to_dir() {
        let root = Path::new("/repo");
        let file = Path::new("/repo/docs/index.md");
        assert_eq!(slug_from_path(file, root), "docs");
    }

    #[test]
    fn slug_from_path_readme_case_insensitive() {
        let root = Path::new("/repo");
        let file = Path::new("/repo/docs/Readme.MD");
        assert_eq!(slug_from_path(file, root), "docs");
    }

    #[test]
    fn source_path_from_file_preserves_extension() {
        let root = Path::new("/repo");
        let file = Path::new("/repo/docs/guides/intro.md");
        assert_eq!(source_path_from_file(file, root), "docs/guides/intro.md");
    }

    #[test]
    fn source_path_from_file_root_level() {
        let root = Path::new("/repo");
        let file = Path::new("/repo/readme.md");
        assert_eq!(source_path_from_file(file, root), "readme.md");
    }

    #[test]
    fn slug_from_title_basic() {
        assert_eq!(slug_from_title("Hello World"), "hello-world");
    }

    #[test]
    fn slug_from_title_special_chars() {
        assert_eq!(
            slug_from_title("Guidelines & Best Practices!"),
            "guidelines-best-practices"
        );
    }

    #[test]
    fn slug_from_title_numbers() {
        assert_eq!(
            slug_from_title("Chapter 1: Introduction"),
            "chapter-1-introduction"
        );
    }

    #[test]
    fn slug_from_title_empty() {
        assert_eq!(slug_from_title(""), "");
    }

    #[test]
    fn slug_from_title_only_special_chars() {
        assert_eq!(slug_from_title("---"), "");
    }

    #[test]
    fn slug_from_title_leading_trailing_separators() {
        assert_eq!(slug_from_title("  My Title  "), "my-title");
    }

    #[test]
    fn normalize_summary_none_is_none() {
        assert_eq!(normalize_summary(None), None);
    }

    #[test]
    fn normalize_summary_empty_is_none() {
        assert_eq!(normalize_summary(Some(String::new())), None);
    }

    #[test]
    fn normalize_summary_whitespace_only_is_none() {
        assert_eq!(normalize_summary(Some("   ".to_string())), None);
    }

    #[test]
    fn normalize_summary_trims_whitespace() {
        assert_eq!(
            normalize_summary(Some("  hello world  ".to_string())),
            Some("hello world".to_string())
        );
    }

    #[test]
    fn apply_prefix_adds_prefix() {
        assert_eq!(apply_prefix(Some("protocols"), "intro"), "protocols/intro");
    }

    #[test]
    fn apply_prefix_none_returns_slug() {
        assert_eq!(apply_prefix(None, "intro"), "intro");
    }

    #[test]
    fn apply_prefix_empty_prefix_returns_slug() {
        assert_eq!(apply_prefix(Some(""), "intro"), "intro");
    }

    #[test]
    fn apply_prefix_does_not_double_prefix() {
        assert_eq!(
            apply_prefix(Some("protocols"), "protocols/intro"),
            "protocols/intro"
        );
    }

    #[test]
    fn apply_prefix_does_not_double_prefix_exact_match() {
        assert_eq!(apply_prefix(Some("protocols"), "protocols"), "protocols");
    }

    #[test]
    fn slug_prefix_prepended() {
        let prefix = "protocols/my-service";
        let raw = "intro";
        let slug = apply_prefix(Some(prefix), raw);
        assert_eq!(slug, "protocols/my-service/intro");
    }
}
