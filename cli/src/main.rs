use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use clap::Parser;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

// ── CLI args ──────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "lekton-sync",
    about = "Sync markdown documents to a Lekton instance",
    long_about = "Scans a directory for markdown files, reads their front matter, \
                  calls the Lekton sync API to compute the delta, then uploads \
                  only the documents that have changed.\n\n\
                  Required environment variables:\n  \
                  LEKTON_TOKEN  Service token for authentication\n  \
                  LEKTON_URL    Base URL of the Lekton server (or set 'url' in .lekton.yml)"
)]
struct Args {
    /// Root directory to scan for markdown files
    #[arg(default_value = ".")]
    root: PathBuf,

    /// Archive documents present in Lekton but not found locally
    #[arg(long)]
    archive_missing: bool,

    /// Show what would be done without making any changes
    #[arg(long)]
    dry_run: bool,

    /// Path to config file (defaults to .lekton.yml in root)
    #[arg(long)]
    config: Option<PathBuf>,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
}

// ── Config / front matter ─────────────────────────────────────────────────────

/// `.lekton.yml` project-level configuration.
#[derive(Deserialize, Default)]
struct LektonConfig {
    /// Base URL of the Lekton server (can also be set via LEKTON_URL env var)
    url: Option<String>,
    /// Default access level applied when a document has no `access_level` in its front matter
    #[serde(default)]
    default_access_level: Option<String>,
    /// Default service_owner applied when a document has no `service_owner` in its front matter
    #[serde(default)]
    default_service_owner: Option<String>,
    /// Slug prefix prepended to every document slug (e.g. "protocols/my-service")
    #[serde(default)]
    slug_prefix: Option<String>,
    /// Archive documents not found locally (can be overridden by --archive-missing flag)
    #[serde(default)]
    archive_missing: Option<bool>,
}

/// YAML front matter parsed from the top of each `.md` file.
#[derive(Deserialize, Default)]
struct FrontMatter {
    slug: Option<String>,
    title: Option<String>,
    access_level: Option<String>,
    service_owner: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    parent_slug: Option<String>,
    order: Option<i32>,
    is_hidden: Option<bool>,
    /// Must be `true` for the file to be synced to Lekton.
    #[serde(rename = "lekton-import", default)]
    lekton_import: bool,
}

// ── API types ─────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct SyncRequest {
    service_token: String,
    documents: Vec<SyncDocEntry>,
    archive_missing: bool,
}

#[derive(Serialize)]
struct SyncDocEntry {
    slug: String,
    content_hash: String,
}

#[derive(Deserialize)]
struct SyncResponse {
    to_upload: Vec<String>,
    to_archive: Vec<String>,
    unchanged: Vec<String>,
}

#[derive(Serialize)]
struct IngestRequest {
    service_token: String,
    slug: String,
    title: String,
    content: String,
    access_level: String,
    service_owner: String,
    tags: Vec<String>,
    parent_slug: Option<String>,
    order: i32,
    is_hidden: bool,
}

#[derive(Deserialize)]
struct IngestResponse {
    changed: bool,
}

// ── Document scanning ─────────────────────────────────────────────────────────

struct DocumentInfo {
    slug: String,
    title: String,
    content: String,
    content_hash: String,
    access_level: String,
    service_owner: String,
    tags: Vec<String>,
    parent_slug: Option<String>,
    order: i32,
    is_hidden: bool,
}

/// Compute the same `sha256:<base64url>` hash format the server uses.
fn compute_hash(content: &str) -> String {
    let hash = Sha256::digest(content.as_bytes());
    format!("sha256:{}", URL_SAFE_NO_PAD.encode(hash))
}

/// Strip YAML front matter from a markdown file and return (front_matter, body).
/// The body is everything after the closing `---` delimiter.
fn parse_front_matter(source: &str) -> (FrontMatter, String) {
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
        // Skip the newline after the closing ---
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

/// Derive a slug from the file path relative to root (strips the `.md` extension).
/// e.g., `docs/guides/intro.md` → `docs/guides/intro`
fn slug_from_path(file: &Path, root: &Path) -> String {
    let relative = file.strip_prefix(root).unwrap_or(file);
    let without_ext = relative.with_extension("");
    without_ext.to_string_lossy().replace('\\', "/")
}

/// Scan all `.md` files under `root` and build a map of slug → DocumentInfo.
/// Files without a `title` or `slug` in their front matter are skipped.
fn scan_documents(root: &Path, config: &LektonConfig) -> Result<HashMap<String, DocumentInfo>> {
    let mut docs = HashMap::new();

    for entry in WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_file()
                && e.path()
                    .extension()
                    .and_then(|ext| ext.to_str())
                    == Some("md")
        })
    {
        let path = entry.path();
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        let (fm, body) = parse_front_matter(&source);

        // Skip files not explicitly marked for import
        if !fm.lekton_import {
            continue;
        }

        let derived_slug = slug_from_path(path, root);
        let slug_raw = fm.slug.clone().unwrap_or_else(|| derived_slug.clone());

        let parsed_parent = if let Some(p) = fm.parent_slug {
            Some(p)
        } else {
            if let Some((parent, _)) = derived_slug.rsplit_once('/') {
                Some(parent.to_string())
            } else {
                None
            }
        };

        let slug = match &config.slug_prefix {
            Some(prefix) if !prefix.is_empty() => format!("{prefix}/{slug_raw}"),
            _ => slug_raw,
        };

        let parent_slug = match &config.slug_prefix {
            Some(prefix) if !prefix.is_empty() => {
                match parsed_parent {
                    Some(ref p) => Some(format!("{prefix}/{p}")),
                    None => Some(prefix.clone()),
                }
            }
            _ => parsed_parent,
        };

        let title = fm.title.unwrap_or_else(|| slug.clone());
        let access_level = fm
            .access_level
            .or_else(|| config.default_access_level.clone())
            .unwrap_or_else(|| "public".to_string());
        let service_owner = fm
            .service_owner
            .or_else(|| config.default_service_owner.clone())
            .unwrap_or_default();
        let tags = fm.tags.unwrap_or_default();
        let order = fm.order.unwrap_or(0);
        let is_hidden = fm.is_hidden.unwrap_or(false);
        let content_hash = compute_hash(&body);

        docs.insert(
            slug.clone(),
            DocumentInfo {
                slug,
                title,
                content: body,
                content_hash,
                access_level,
                service_owner,
                tags,
                parent_slug,
                order,
                is_hidden,
            },
        );
    }

    Ok(docs)
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // ── Load .lekton.yml ──────────────────────────────────────────────────────
    let config_path = args
        .config
        .clone()
        .unwrap_or_else(|| args.root.join(".lekton.yml"));

    let config: LektonConfig = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config {}", config_path.display()))?;
        serde_yaml::from_str(&content)
            .with_context(|| format!("Failed to parse config {}", config_path.display()))?
    } else {
        LektonConfig::default()
    };

    // ── Resolve URL and token ─────────────────────────────────────────────────
    let base_url = std::env::var("LEKTON_URL")
        .ok()
        .or_else(|| config.url.clone())
        .context("LEKTON_URL environment variable or 'url' in .lekton.yml is required")?;
    let base_url = base_url.trim_end_matches('/').to_string();

    let token =
        std::env::var("LEKTON_TOKEN").context("LEKTON_TOKEN environment variable is required")?;

    // ── Determine options ─────────────────────────────────────────────────────
    let archive_missing = args.archive_missing || config.archive_missing.unwrap_or(false);

    // ── Scan documents ────────────────────────────────────────────────────────
    let root = args
        .root
        .canonicalize()
        .with_context(|| format!("Cannot access root path: {}", args.root.display()))?;

    if args.verbose {
        eprintln!("Scanning {}", root.display());
    }

    let docs = scan_documents(&root, &config)?;

    if docs.is_empty() {
        println!("No documents found (files must have `lekton-import: true` in their YAML front matter).");
        return Ok(());
    }

    println!("Found {} document(s)", docs.len());

    // ── Call sync API ─────────────────────────────────────────────────────────
    let client = reqwest::Client::new();
    let sync_url = format!("{base_url}/api/v1/sync");

    if args.verbose {
        eprintln!("POST {sync_url}");
    }

    let sync_entries: Vec<SyncDocEntry> = docs
        .values()
        .map(|d| SyncDocEntry {
            slug: d.slug.clone(),
            content_hash: d.content_hash.clone(),
        })
        .collect();

    let sync_resp = client
        .post(&sync_url)
        .json(&SyncRequest {
            service_token: token.clone(),
            documents: sync_entries,
            archive_missing,
        })
        .send()
        .await
        .context("Failed to call sync API")?;

    if !sync_resp.status().is_success() {
        let status = sync_resp.status();
        let body = sync_resp.text().await.unwrap_or_default();
        bail!("Sync API returned {status}: {body}");
    }

    let sync_result: SyncResponse = sync_resp
        .json()
        .await
        .context("Failed to parse sync response")?;

    println!(
        "Sync result: {} to upload, {} unchanged, {} to archive",
        sync_result.to_upload.len(),
        sync_result.unchanged.len(),
        sync_result.to_archive.len(),
    );

    // ── Dry run: show plan and exit ───────────────────────────────────────────
    if args.dry_run {
        if !sync_result.to_upload.is_empty() {
            println!("\nWould upload:");
            for slug in &sync_result.to_upload {
                println!("  + {slug}");
            }
        }
        if !sync_result.to_archive.is_empty() {
            println!("\nWould archive:");
            for slug in &sync_result.to_archive {
                println!("  - {slug}");
            }
        }
        if !sync_result.unchanged.is_empty() && args.verbose {
            println!("\nUnchanged:");
            for slug in &sync_result.unchanged {
                println!("  = {slug}");
            }
        }
        println!("\nDry run — no changes made.");
        return Ok(());
    }

    // ── Upload changed documents ──────────────────────────────────────────────
    let ingest_url = format!("{base_url}/api/v1/ingest");
    let mut uploaded = 0usize;
    let mut errors = 0usize;

    for slug in &sync_result.to_upload {
        let Some(doc) = docs.get(slug) else {
            eprintln!("Warning: server requested upload of unknown slug '{slug}', skipping");
            continue;
        };

        if args.verbose {
            eprintln!("Uploading: {slug}");
        }

        let result = client
            .post(&ingest_url)
            .json(&IngestRequest {
                service_token: token.clone(),
                slug: doc.slug.clone(),
                title: doc.title.clone(),
                content: doc.content.clone(),
                access_level: doc.access_level.clone(),
                service_owner: doc.service_owner.clone(),
                tags: doc.tags.clone(),
                parent_slug: doc.parent_slug.clone(),
                order: doc.order,
                is_hidden: doc.is_hidden,
            })
            .send()
            .await;

        match result {
            Ok(r) if r.status().is_success() => {
                let ingest: IngestResponse = r.json().await.unwrap_or(IngestResponse { changed: true });
                uploaded += 1;
                if args.verbose {
                    let note = if ingest.changed { "updated" } else { "metadata only" };
                    println!("  uploaded: {slug} ({note})");
                } else {
                    println!("  uploaded: {slug}");
                }
            }
            Ok(r) => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                eprintln!("  error: {slug}: HTTP {status} — {body}");
                errors += 1;
            }
            Err(e) => {
                eprintln!("  error: {slug}: {e}");
                errors += 1;
            }
        }
    }

    // ── Summary ───────────────────────────────────────────────────────────────
    println!(
        "\nDone: {uploaded} uploaded, {} unchanged, {} archived",
        sync_result.unchanged.len(),
        sync_result.to_archive.len(),
    );

    if errors > 0 {
        bail!("{errors} upload(s) failed");
    }

    Ok(())
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_format() {
        let h = compute_hash("# Hello");
        assert!(h.starts_with("sha256:"), "hash should start with sha256:");
        assert_eq!(h.len(), "sha256:".len() + 43); // 32 bytes → 43 base64url chars (no padding)
    }

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
    fn parse_front_matter_missing() {
        let src = "# No front matter\n\nJust a doc.";
        let (fm, body) = parse_front_matter(src);
        assert!(fm.title.is_none());
        assert_eq!(body, src);
    }

    #[test]
    fn slug_from_path_strips_extension() {
        let root = Path::new("/docs");
        let file = Path::new("/docs/guides/intro.md");
        assert_eq!(slug_from_path(file, root), "guides/intro");
    }

    #[test]
    fn slug_from_path_root_file() {
        let root = Path::new("/repo");
        let file = Path::new("/repo/readme.md");
        assert_eq!(slug_from_path(file, root), "readme");
    }

    #[test]
    fn slug_prefix_prepended() {
        // Simulate the slug_prefix logic
        let prefix = "protocols/my-service";
        let raw = "intro";
        let slug = format!("{prefix}/{raw}");
        assert_eq!(slug, "protocols/my-service/intro");
    }

    #[test]
    fn scan_skips_files_without_front_matter() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("no-fm.md");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"# No front matter\n")
            .unwrap();
        let docs = scan_documents(dir.path(), &LektonConfig::default()).unwrap();
        assert!(docs.is_empty());
    }

    #[test]
    fn scan_picks_up_titled_file() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("guide.md");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"---\ntitle: Guide\naccess_level: public\nlekton-import: true\n---\n# Guide body\n")
            .unwrap();
        let docs = scan_documents(dir.path(), &LektonConfig::default()).unwrap();
        assert_eq!(docs.len(), 1);
        let doc = docs.values().next().unwrap();
        assert_eq!(doc.title, "Guide");
        assert_eq!(doc.access_level, "public");
        assert!(doc.content_hash.starts_with("sha256:"));
    }

    #[test]
    fn scan_skips_file_without_lekton_import() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("readme.md");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"---\ntitle: README\n---\n# Not for Lekton\n")
            .unwrap();
        let docs = scan_documents(dir.path(), &LektonConfig::default()).unwrap();
        assert!(docs.is_empty());
    }
}
