use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use clap::Parser;
use regex::Regex;
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
    /// Maximum attachment file size in MB (default: 10)
    #[serde(default)]
    max_attachment_size_mb: Option<u32>,
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

#[derive(Serialize)]
struct CheckHashesRequest {
    service_token: String,
    entries: Vec<CheckHashEntry>,
}

#[derive(Serialize)]
struct CheckHashEntry {
    key: String,
    content_hash: String,
}

#[derive(Deserialize)]
struct CheckHashesResponse {
    to_upload: Vec<String>,
}

// ── Attachment types ──────────────────────────────────────────────────────────

/// A local file reference found in a markdown document.
#[derive(Debug, Clone)]
struct LocalFileRef {
    /// The raw path as it appears in the markdown (e.g., "./images/arch.png")
    raw_path: String,
    /// The resolved absolute path on disk
    disk_path: PathBuf,
}

/// A resolved attachment ready for upload.
#[derive(Debug, Clone)]
struct AttachmentInfo {
    /// The raw relative path as it appears in markdown
    raw_path: String,
    /// Resolved absolute path on disk
    disk_path: PathBuf,
    /// SHA-256 content hash of the file bytes
    content_hash: String,
    /// The asset key on the server: "attachments/{doc-slug}/{filename}"
    asset_key: String,
    /// File size in bytes
    size_bytes: u64,
    /// MIME content type
    content_type: String,
}

// ── Document scanning ─────────────────────────────────────────────────────────

struct DocumentInfo {
    slug: String,
    title: String,
    /// Original markdown body (local file refs intact)
    content: String,
    /// Rewritten markdown body (local file refs replaced with server URLs).
    /// Populated after attachment processing; initially same as `content`.
    rewritten_content: String,
    content_hash: String,
    access_level: String,
    service_owner: String,
    tags: Vec<String>,
    parent_slug: Option<String>,
    order: i32,
    is_hidden: bool,
    /// Attachments found in this document
    attachments: Vec<AttachmentInfo>,
}

/// Compute the same `sha256:<base64url>` hash format the server uses.
fn compute_hash(content: &str) -> String {
    let hash = Sha256::digest(content.as_bytes());
    format!("sha256:{}", URL_SAFE_NO_PAD.encode(hash))
}

/// Compute SHA-256 content hash for a file's bytes.
fn compute_file_hash(data: &[u8]) -> String {
    let hash = Sha256::digest(data);
    format!("sha256:{}", URL_SAFE_NO_PAD.encode(hash))
}

/// Extract local file references from markdown content.
///
/// Detects `![alt](path)`, `[text](path)`, and `<img src="path">` patterns.
/// Filters out external URLs, anchors, absolute paths, and already-rewritten
/// Lekton asset URLs.
fn extract_local_file_refs(markdown: &str, md_file_dir: &Path) -> Vec<LocalFileRef> {
    // Markdown images/links: ![alt](path) or [text](path)
    // We use a single pattern that optionally matches the leading !
    let md_link_re = Regex::new(r#"!?\[[^\]]*\]\(([^)\s]+)"#).unwrap();
    // HTML img tags: <img src="path"> or <img src='path'>
    let html_img_re = Regex::new(r#"<img[^>]+src=["']([^"']+)["']"#).unwrap();

    let mut seen = HashMap::new();
    let mut refs = Vec::new();

    let all_captures = md_link_re
        .captures_iter(markdown)
        .chain(html_img_re.captures_iter(markdown));

    for cap in all_captures {
        let raw_path = cap[1].to_string();

        // Skip external URLs, anchors, absolute paths, and already-rewritten paths
        if raw_path.starts_with("http://")
            || raw_path.starts_with("https://")
            || raw_path.starts_with("mailto:")
            || raw_path.starts_with('#')
            || raw_path.starts_with('/')
            || raw_path.starts_with("/api/v1/")
        {
            continue;
        }

        // Deduplicate by raw path
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
fn resolve_attachments(
    refs: &[LocalFileRef],
    doc_slug: &str,
    max_size_bytes: u64,
) -> Vec<AttachmentInfo> {
    let mut attachments = Vec::new();

    for file_ref in refs {
        // Canonicalize to resolve ../
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
fn rewrite_content(content: &str, attachments: &[AttachmentInfo]) -> String {
    let mut result = content.to_string();
    for att in attachments {
        // Simple string replacement: replace the raw_path with the server URL.
        // This works because we're replacing exact path strings.
        let server_url = format!("/api/v1/assets/{}", att.asset_key);
        result = result.replace(&att.raw_path, &server_url);
    }
    result
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
/// Files without `lekton-import: true` in their front matter are skipped.
/// Also extracts local file references and builds attachment info.
fn scan_documents(root: &Path, config: &LektonConfig) -> Result<HashMap<String, DocumentInfo>> {
    let max_attachment_size_bytes =
        (config.max_attachment_size_mb.unwrap_or(10) as u64) * 1024 * 1024;
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

        // Extract local file references and build attachments
        let md_file_dir = path.parent().unwrap_or(root);
        let local_refs = extract_local_file_refs(&body, md_file_dir);
        let attachments = resolve_attachments(&local_refs, &slug, max_attachment_size_bytes);

        // Rewrite content: replace local paths with server asset URLs
        let rewritten_content = rewrite_content(&body, &attachments);
        // Hash the rewritten content for consistent sync comparison
        let content_hash = compute_hash(&rewritten_content);

        docs.insert(
            slug.clone(),
            DocumentInfo {
                slug,
                title,
                content: body,
                rewritten_content,
                content_hash,
                access_level,
                service_owner,
                tags,
                parent_slug,
                order,
                is_hidden,
                attachments,
            },
        );
    }

    Ok(docs)
}

// ── HTTP helpers ──────────────────────────────────────────────────────────

const MAX_RETRIES: u32 = 5;
const INITIAL_BACKOFF_MS: u64 = 500;

/// Sleep with exponential backoff if the response is 429 (Too Many Requests).
/// Returns `true` if the caller should retry, `false` if retries are exhausted
/// or the response was not 429.
async fn backoff_on_429(response: &reqwest::Response, attempt: &mut u32, backoff_ms: &mut u64) -> bool {
    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS && *attempt < MAX_RETRIES {
        *attempt += 1;
        eprintln!(
            "  rate limited, retrying in {}ms (attempt {}/{})",
            backoff_ms, *attempt, MAX_RETRIES,
        );
        tokio::time::sleep(std::time::Duration::from_millis(*backoff_ms)).await;
        *backoff_ms *= 2;
        true
    } else {
        false
    }
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

    // ── Collect attachments for ALL documents ────────────────────────────────
    // We check hashes for every attachment regardless of whether the document
    // body changed, so that replacing a PDF/image with new content is detected
    // even when the markdown (which only references the file by URL) is unchanged.
    let mut all_attachments: HashMap<String, &AttachmentInfo> = HashMap::new();
    let total_attachment_count: usize = docs.values().map(|d| d.attachments.len()).sum();

    for doc in docs.values() {
        for att in &doc.attachments {
            all_attachments.entry(att.asset_key.clone()).or_insert(att);
        }
    }

    if total_attachment_count > 0 {
        println!(
            "Found {} attachment(s) across all documents ({} unique to check)",
            total_attachment_count,
            all_attachments.len(),
        );
    }

    // ── Dry run: show plan and exit ───────────────────────────────────────────
    if args.dry_run {
        if !all_attachments.is_empty() {
            println!("\nWould upload attachments:");
            for (key, att) in &all_attachments {
                println!(
                    "  + {} ({:.1} KB)",
                    key,
                    att.size_bytes as f64 / 1024.0,
                );
            }
        }
        if !sync_result.to_upload.is_empty() {
            println!("\nWould upload documents:");
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

    // ── Upload attachments ────────────────────────────────────────────────────
    let mut attachments_uploaded = 0usize;
    let mut attachment_errors = 0usize;

    if !all_attachments.is_empty() {
        // Check which attachments need uploading (hash-based dedup)
        let check_url = format!("{base_url}/api/v1/assets/check-hashes");
        let entries: Vec<CheckHashEntry> = all_attachments
            .values()
            .map(|att| CheckHashEntry {
                key: att.asset_key.clone(),
                content_hash: att.content_hash.clone(),
            })
            .collect();

        if args.verbose {
            eprintln!("POST {check_url} ({} entries)", entries.len());
        }

        let check_resp = client
            .post(&check_url)
            .json(&CheckHashesRequest {
                service_token: token.clone(),
                entries,
            })
            .send()
            .await
            .context("Failed to call check-hashes API")?;

        if !check_resp.status().is_success() {
            let status = check_resp.status();
            let body = check_resp.text().await.unwrap_or_default();
            bail!("Check-hashes API returned {status}: {body}");
        }

        let check_result: CheckHashesResponse = check_resp
            .json()
            .await
            .context("Failed to parse check-hashes response")?;

        let to_upload_set: std::collections::HashSet<&str> =
            check_result.to_upload.iter().map(|s| s.as_str()).collect();

        let unchanged_count = all_attachments.len() - to_upload_set.len();
        if unchanged_count > 0 {
            println!("{unchanged_count} attachment(s) unchanged, {} to upload", to_upload_set.len());
        }

        // Upload each attachment that needs it
        for key in &check_result.to_upload {
            let Some(att) = all_attachments.get(key.as_str()) else {
                continue;
            };

            let data = match std::fs::read(&att.disk_path) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("  error: failed to read {}: {e}", att.raw_path);
                    attachment_errors += 1;
                    continue;
                }
            };

            if args.verbose {
                eprintln!(
                    "  uploading: {} ({:.1} KB)",
                    key,
                    data.len() as f64 / 1024.0,
                );
            }

            let upload_url = format!("{base_url}/api/v1/assets/{key}");
            let file_name = att.disk_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "file".to_string());
            let content_type_str = att.content_type.clone();

            let mut attempt = 0u32;
            let mut backoff_ms = INITIAL_BACKOFF_MS;
            let upload_result = loop {
                let file_part = reqwest::multipart::Part::bytes(data.clone())
                    .file_name(file_name.clone())
                    .mime_str(&content_type_str)
                    .unwrap_or_else(|_| reqwest::multipart::Part::bytes(vec![]));
                let form = reqwest::multipart::Form::new()
                    .text("service_token", token.clone())
                    .part("file", file_part);

                match client.put(&upload_url).multipart(form).send().await {
                    Ok(r) if backoff_on_429(&r, &mut attempt, &mut backoff_ms).await => continue,
                    other => break other,
                }
            };

            match upload_result {
                Ok(r) if r.status().is_success() => {
                    attachments_uploaded += 1;
                    println!("  attachment: {key}");
                }
                Ok(r) => {
                    let status = r.status();
                    let body = r.text().await.unwrap_or_default();
                    eprintln!("  error: attachment {key}: HTTP {status} — {body}");
                    attachment_errors += 1;
                }
                Err(e) => {
                    eprintln!("  error: attachment {key}: {e}");
                    attachment_errors += 1;
                }
            }
        }
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

        let ingest_body = IngestRequest {
            service_token: token.clone(),
            slug: doc.slug.clone(),
            title: doc.title.clone(),
            content: doc.rewritten_content.clone(),
            access_level: doc.access_level.clone(),
            service_owner: doc.service_owner.clone(),
            tags: doc.tags.clone(),
            parent_slug: doc.parent_slug.clone(),
            order: doc.order,
            is_hidden: doc.is_hidden,
        };

        let mut attempt = 0u32;
        let mut backoff_ms = INITIAL_BACKOFF_MS;
        let result = loop {
            match client.post(&ingest_url).json(&ingest_body).send().await {
                Ok(r) if backoff_on_429(&r, &mut attempt, &mut backoff_ms).await => continue,
                other => break other,
            }
        };

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
    if attachments_uploaded > 0 || attachment_errors > 0 {
        println!(
            "\nAttachments: {attachments_uploaded} uploaded, {} errors",
            attachment_errors,
        );
    }
    println!(
        "Documents: {uploaded} uploaded, {} unchanged, {} archived",
        sync_result.unchanged.len(),
        sync_result.to_archive.len(),
    );

    let total_errors = errors + attachment_errors;
    if total_errors > 0 {
        bail!("{total_errors} upload(s) failed");
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

    // ── Attachment extraction tests ───────────────────────────────────────────

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
        assert_eq!(refs[0].disk_path, PathBuf::from("/docs/guides/../shared/logo.svg"));
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
        assert!(refs.is_empty(), "Should skip all external/absolute/anchor refs, got: {:?}", refs);
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

    // ── Content rewriting tests ───────────────────────────────────────────────

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

    // ── Scan with attachments test ────────────────────────────────────────────

    #[test]
    fn scan_detects_attachments() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();

        // Create an image file
        let img_dir = dir.path().join("images");
        std::fs::create_dir(&img_dir).unwrap();
        std::fs::File::create(img_dir.join("logo.png"))
            .unwrap()
            .write_all(b"fake png data")
            .unwrap();

        // Create a markdown file referencing the image
        let md_path = dir.path().join("guide.md");
        std::fs::File::create(&md_path)
            .unwrap()
            .write_all(
                b"---\ntitle: Guide\nlekton-import: true\n---\n# Guide\n\n![logo](images/logo.png)\n",
            )
            .unwrap();

        let docs = scan_documents(dir.path(), &LektonConfig::default()).unwrap();
        assert_eq!(docs.len(), 1);
        let doc = docs.values().next().unwrap();
        assert_eq!(doc.attachments.len(), 1);
        assert_eq!(doc.attachments[0].asset_key, "attachments/guide/logo.png");
        assert!(doc.rewritten_content.contains("/api/v1/assets/attachments/guide/logo.png"));
        // Original content should still have the local path
        assert!(doc.content.contains("images/logo.png"));
    }
}
