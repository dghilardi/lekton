use std::collections::HashMap;
use std::path::Path;

use regex::Regex;

use crate::hash::compute_file_hash;
use crate::models::{AttachmentInfo, LocalFileRef};

/// Extract local file references from markdown content.
///
/// Detects `![alt](path)`, `[text](path)`, and `<img src="path">` patterns.
/// Filters out external URLs, anchors, absolute paths, and already-rewritten
/// Lekton asset URLs.
pub fn extract_local_file_refs(markdown: &str, md_file_dir: &Path) -> Vec<LocalFileRef> {
    let md_link_re = Regex::new(r#"!?\[[^\]]*\]\(([^)\s]+)"#).unwrap();
    let html_img_re = Regex::new(r#"<img[^>]+src=["']([^"']+)["']"#).unwrap();

    let mut seen = HashMap::new();
    let mut refs = Vec::new();

    let all_captures = md_link_re
        .captures_iter(markdown)
        .chain(html_img_re.captures_iter(markdown));

    for cap in all_captures {
        let raw_path = cap[1].to_string();

        if raw_path.starts_with("http://")
            || raw_path.starts_with("https://")
            || raw_path.starts_with("mailto:")
            || raw_path.starts_with('#')
            || raw_path.starts_with('/')
            || raw_path.starts_with("/api/v1/")
        {
            continue;
        }

        // Skip markdown files — inter-document links resolved server-side
        if raw_path.ends_with(".md")
            || raw_path.ends_with(".mdx")
            || raw_path.ends_with(".markdown")
        {
            continue;
        }

        if seen.contains_key(&raw_path) {
            continue;
        }
        seen.insert(raw_path.clone(), true);

        let disk_path = md_file_dir.join(&raw_path);
        refs.push(LocalFileRef {
            raw_path,
            disk_path,
        });
    }

    refs
}

/// Build attachment info for a document's local file references.
/// Skips files that don't exist or exceed the size limit (with warnings).
pub fn resolve_attachments(
    refs: &[LocalFileRef],
    doc_slug: &str,
    max_size_bytes: u64,
) -> Vec<AttachmentInfo> {
    let mut attachments = Vec::new();

    for file_ref in refs {
        let disk_path = match file_ref.disk_path.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                eprintln!(
                    "  Warning: referenced file not found: {} (skipping)",
                    file_ref.raw_path
                );
                continue;
            }
        };

        let metadata = match std::fs::metadata(&disk_path) {
            Ok(m) => m,
            Err(_) => {
                eprintln!(
                    "  Warning: cannot read file: {} (skipping)",
                    file_ref.raw_path
                );
                continue;
            }
        };

        let size_bytes = metadata.len();
        if size_bytes > max_size_bytes {
            eprintln!(
                "  Warning: file too large ({:.1} MB > {:.1} MB limit): {} (skipping)",
                size_bytes as f64 / (1024.0 * 1024.0),
                max_size_bytes as f64 / (1024.0 * 1024.0),
                file_ref.raw_path,
            );
            continue;
        }

        let data = match std::fs::read(&disk_path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!(
                    "  Warning: failed to read {}: {} (skipping)",
                    file_ref.raw_path, e
                );
                continue;
            }
        };

        let content_hash = compute_file_hash(&data);
        let filename = disk_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let asset_key = format!("attachments/{}/{}", doc_slug, filename);

        let content_type = mime_guess::from_path(&disk_path)
            .first_or_octet_stream()
            .to_string();

        attachments.push(AttachmentInfo {
            raw_path: file_ref.raw_path.clone(),
            disk_path,
            content_hash,
            asset_key,
            size_bytes,
            content_type,
        });
    }

    attachments
}

/// Rewrite markdown content, replacing local file paths with server asset URLs.
/// Only rewrites paths that have a corresponding attachment (i.e., file exists and
/// is within size limits).
pub fn rewrite_content(content: &str, attachments: &[AttachmentInfo]) -> String {
    let mut result = content.to_string();
    for att in attachments {
        let server_url = format!("/api/v1/assets/{}", att.asset_key);
        result = result.replace(&att.raw_path, &server_url);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn extract_refs_markdown_image() {
        let md = "# Doc\n\n![diagram](./images/arch.png)\n\nSome text.";
        let refs = extract_local_file_refs(md, Path::new("/docs"));
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].raw_path, "./images/arch.png");
    }

    #[test]
    fn extract_refs_markdown_link() {
        let md = "See [the spec](attachments/spec.pdf) for details.";
        let refs = extract_local_file_refs(md, Path::new("/docs"));
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].raw_path, "attachments/spec.pdf");
    }

    #[test]
    fn extract_refs_parent_relative() {
        let md = "![shared](../shared/logo.svg)";
        let refs = extract_local_file_refs(md, Path::new("/docs/guides"));
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].raw_path, "../shared/logo.svg");
        assert_eq!(
            refs[0].disk_path,
            PathBuf::from("/docs/guides/../shared/logo.svg")
        );
    }

    #[test]
    fn extract_refs_html_img() {
        let md = r#"Some text <img src="images/photo.jpg" alt="photo"> more text"#;
        let refs = extract_local_file_refs(md, Path::new("/docs"));
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].raw_path, "images/photo.jpg");
    }

    #[test]
    fn extract_refs_skips_external_urls() {
        let md = r#"
![ext](https://example.com/img.png)
[link](http://example.com/doc.pdf)
[mail](mailto:test@example.com)
[anchor](#section)
[abs](/absolute/path.md)
![already](/api/v1/assets/something.png)
"#;
        let refs = extract_local_file_refs(md, Path::new("/docs"));
        assert!(
            refs.is_empty(),
            "Should skip all external/absolute/anchor refs, got: {:?}",
            refs
        );
    }

    #[test]
    fn extract_refs_skips_markdown_files() {
        let md = "See [config](./020-configuration.md) and [guide](../other/guide.mdx).";
        let refs = extract_local_file_refs(md, Path::new("/docs/section"));
        assert!(
            refs.is_empty(),
            "Should skip .md/.mdx links (handled server-side), got: {:?}",
            refs
        );
    }

    #[test]
    fn extract_refs_deduplicates() {
        let md = "![a](img.png) and ![b](img.png) and [c](img.png)";
        let refs = extract_local_file_refs(md, Path::new("/docs"));
        assert_eq!(refs.len(), 1, "Should deduplicate same path");
    }

    #[test]
    fn extract_refs_multiple_different() {
        let md = "![a](one.png)\n![b](two.pdf)\n[c](three.zip)";
        let refs = extract_local_file_refs(md, Path::new("/docs"));
        assert_eq!(refs.len(), 3);
    }

    #[test]
    fn rewrite_content_replaces_paths() {
        let content = "![diagram](./images/arch.png)\n\nSee [spec](docs/spec.pdf).";
        let attachments = vec![
            AttachmentInfo {
                raw_path: "./images/arch.png".to_string(),
                disk_path: PathBuf::from("/tmp/images/arch.png"),
                content_hash: "sha256:abc".to_string(),
                asset_key: "attachments/my-doc/arch.png".to_string(),
                size_bytes: 1000,
                content_type: "image/png".to_string(),
            },
            AttachmentInfo {
                raw_path: "docs/spec.pdf".to_string(),
                disk_path: PathBuf::from("/tmp/docs/spec.pdf"),
                content_hash: "sha256:def".to_string(),
                asset_key: "attachments/my-doc/spec.pdf".to_string(),
                size_bytes: 2000,
                content_type: "application/pdf".to_string(),
            },
        ];

        let result = rewrite_content(content, &attachments);
        assert_eq!(
            result,
            "![diagram](/api/v1/assets/attachments/my-doc/arch.png)\n\nSee [spec](/api/v1/assets/attachments/my-doc/spec.pdf)."
        );
    }

    #[test]
    fn rewrite_content_no_attachments_unchanged() {
        let content = "# Hello\n\nNo attachments here.";
        let result = rewrite_content(content, &[]);
        assert_eq!(result, content);
    }
}
