#![forbid(unsafe_code)]

mod api;
mod attachment;
mod config;
mod front_matter;
mod hash;
mod http;
mod models;
mod scan;
mod slug;

use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use clap::Parser;

use api::{
    CheckHashEntry, CheckHashesRequest, CheckHashesResponse, IngestRequest, IngestResponse,
    PromptIngestRequest, PromptIngestResponse, PromptSyncEntry, PromptSyncRequest,
    PromptSyncResponse, SchemaIngestRequest, SchemaIngestResponse, SchemaSyncEntry,
    SchemaSyncRequest, SchemaSyncResponse, SyncDocEntry, SyncRequest, SyncResponse,
};
use http::{
    backoff_on_429, detect_git_remote, is_interactive, prompt_and_persist_source_id,
    INITIAL_BACKOFF_MS,
};
use scan::{scan_documents, scan_prompts, scan_schemas};

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
    root: std::path::PathBuf,

    /// Archive documents present in Lekton but not found locally
    #[arg(long)]
    archive_missing: bool,

    /// Show what would be done without making any changes
    #[arg(long)]
    dry_run: bool,

    /// Path to config file (defaults to .lekton.yml in root)
    #[arg(long)]
    config: Option<std::path::PathBuf>,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
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

    let config: config::LektonConfig = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config {}", config_path.display()))?;
        serde_yaml::from_str(&content)
            .with_context(|| format!("Failed to parse config {}", config_path.display()))?
    } else {
        config::LektonConfig::default()
    };

    // ── Require source_id ────────────────────────────────────────────────────
    let source_id = match config.id.clone() {
        Some(id) => id,
        None if is_interactive() => {
            let suggestion = detect_git_remote(&args.root);
            prompt_and_persist_source_id(&config_path, suggestion.as_deref())?
        }
        None => anyhow::bail!(
            "Missing required 'id' field in .lekton.yml.\n\
             Add a stable identifier for this repository import, e.g.:\n\n  \
             id: my-org/my-repo\n\n\
             This id is used by Lekton to scope auto-archiving and resolve cross-links."
        ),
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
    let archive_missing_schemas =
        args.archive_missing || config.archive_missing_schemas.unwrap_or(false);

    // ── Scan documents ────────────────────────────────────────────────────────
    let root = args
        .root
        .canonicalize()
        .with_context(|| format!("Cannot access root path: {}", args.root.display()))?;

    if args.verbose {
        eprintln!("Scanning {}", root.display());
    }

    let docs = scan_documents(&root, &config, args.verbose)?;
    let prompts = scan_prompts(&root, &config)?;
    let schemas = scan_schemas(&root, &config)?;

    if docs.is_empty() && prompts.is_empty() && schemas.is_empty() {
        println!("No documents, prompts, or schemas found.");
        return Ok(());
    }

    println!(
        "Found {} document(s), {} prompt(s), and {} schema version(s)",
        docs.len(),
        prompts.len(),
        schemas.len()
    );

    // ── Call sync API ─────────────────────────────────────────────────────────
    let client = reqwest::Client::new();
    let sync_result = if !docs.is_empty() || archive_missing {
        let sync_url = format!("{base_url}/api/v1/sync");
        if args.verbose {
            eprintln!("POST {sync_url}");
        }

        let sync_entries: Vec<SyncDocEntry> = docs
            .values()
            .map(|d| SyncDocEntry {
                source_path: d.source_path.clone(),
                slug: d.slug.clone(),
                content_hash: d.content_hash.clone(),
                metadata_hash: d.metadata_hash.clone(),
                legacy_slug: d.legacy_slug.clone(),
            })
            .collect();

        let sync_resp = client
            .post(&sync_url)
            .json(&SyncRequest {
                service_token: token.clone(),
                source_id: source_id.clone(),
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

        let result: SyncResponse = sync_resp
            .json()
            .await
            .context("Failed to parse sync response")?;

        println!(
            "Documents: {} to upload, {} unchanged, {} to archive",
            result.to_upload.len(),
            result.unchanged.len(),
            result.to_archive.len(),
        );
        result
    } else {
        SyncResponse {
            to_upload: vec![],
            to_archive: vec![],
            unchanged: vec![],
        }
    };

    let prompt_sync_result = if !prompts.is_empty() || archive_missing {
        let sync_url = format!("{base_url}/api/v1/prompts/sync");
        if args.verbose {
            eprintln!("POST {sync_url}");
        }

        let sync_entries: Vec<PromptSyncEntry> = prompts
            .values()
            .map(|p| PromptSyncEntry {
                slug: p.slug.clone(),
                content_hash: p.content_hash.clone(),
                metadata_hash: p.metadata_hash.clone(),
            })
            .collect();

        let sync_resp = client
            .post(&sync_url)
            .json(&PromptSyncRequest {
                service_token: token.clone(),
                prompts: sync_entries,
                archive_missing,
            })
            .send()
            .await
            .context("Failed to call prompt sync API")?;

        if !sync_resp.status().is_success() {
            let status = sync_resp.status();
            let body = sync_resp.text().await.unwrap_or_default();
            bail!("Prompt sync API returned {status}: {body}");
        }

        let result: PromptSyncResponse = sync_resp
            .json()
            .await
            .context("Failed to parse prompt sync response")?;

        println!(
            "Prompt sync result: {} to upload, {} unchanged, {} to archive",
            result.to_upload.len(),
            result.unchanged.len(),
            result.to_archive.len(),
        );
        result
    } else {
        PromptSyncResponse {
            to_upload: vec![],
            to_archive: vec![],
            unchanged: vec![],
        }
    };

    let schema_sync_result = if !schemas.is_empty() || archive_missing_schemas {
        let sync_url = format!("{base_url}/api/v1/schemas/sync");
        if args.verbose {
            eprintln!("POST {sync_url}");
        }

        let sync_entries: Vec<SchemaSyncEntry> = schemas
            .values()
            .map(|schema| SchemaSyncEntry {
                name: schema.name.clone(),
                version: schema.version.clone(),
                content_hash: schema.content_hash.clone(),
                metadata_hash: schema.metadata_hash.clone(),
            })
            .collect();

        let sync_resp = client
            .post(&sync_url)
            .json(&SchemaSyncRequest {
                service_token: token.clone(),
                schemas: sync_entries,
                archive_missing: archive_missing_schemas,
            })
            .send()
            .await
            .context("Failed to call schema sync API")?;

        if !sync_resp.status().is_success() {
            let status = sync_resp.status();
            let body = sync_resp.text().await.unwrap_or_default();
            bail!("Schema sync API returned {status}: {body}");
        }

        let result: SchemaSyncResponse = sync_resp
            .json()
            .await
            .context("Failed to parse schema sync response")?;

        println!(
            "Schema sync result: {} to upload, {} unchanged, {} to archive",
            result.to_upload.len(),
            result.unchanged.len(),
            result.to_archive.len(),
        );
        result
    } else {
        SchemaSyncResponse {
            to_upload: vec![],
            to_archive: vec![],
            unchanged: vec![],
        }
    };

    // ── Collect attachments for ALL documents ────────────────────────────────
    let mut all_attachments: HashMap<String, &models::AttachmentInfo> = HashMap::new();
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
                println!("  + {} ({:.1} KB)", key, att.size_bytes as f64 / 1024.0);
            }
        }
        if !sync_result.to_upload.is_empty() {
            println!("\nWould upload documents:");
            for entry in &sync_result.to_upload {
                println!("  + {} (slug: {})", entry.source_path, entry.actual_slug);
            }
        }
        if !prompt_sync_result.to_upload.is_empty() {
            println!("\nWould upload prompts:");
            for slug in &prompt_sync_result.to_upload {
                println!("  + {slug}");
            }
        }
        if !schema_sync_result.to_upload.is_empty() {
            println!("\nWould upload schema versions:");
            for key in &schema_sync_result.to_upload {
                println!("  + {key}");
            }
        }
        if !sync_result.to_archive.is_empty() {
            println!("\nWould archive:");
            for slug in &sync_result.to_archive {
                println!("  - {slug}");
            }
        }
        if !prompt_sync_result.to_archive.is_empty() {
            println!("\nWould archive prompts:");
            for slug in &prompt_sync_result.to_archive {
                println!("  - {slug}");
            }
        }
        if !schema_sync_result.to_archive.is_empty() {
            println!("\nWould archive schema versions:");
            for key in &schema_sync_result.to_archive {
                println!("  - {key}");
            }
        }
        if !sync_result.unchanged.is_empty() && args.verbose {
            println!("\nUnchanged:");
            for source_path in &sync_result.unchanged {
                println!("  = {source_path}");
            }
        }
        if !schema_sync_result.unchanged.is_empty() && args.verbose {
            println!("\nUnchanged schema versions:");
            for key in &schema_sync_result.unchanged {
                println!("  = {key}");
            }
        }
        println!("\nDry run — no changes made.");
        return Ok(());
    }

    // ── Upload attachments ────────────────────────────────────────────────────
    let mut attachments_uploaded = 0usize;
    let mut attachment_errors = 0usize;
    let mut failed_attachment_keys: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    if !all_attachments.is_empty() {
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
            println!(
                "{unchanged_count} attachment(s) unchanged, {} to upload",
                to_upload_set.len()
            );
        }

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
            let file_name = att
                .disk_path
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
                    failed_attachment_keys.insert(key.clone());
                }
                Err(e) => {
                    eprintln!("  error: attachment {key}: {e}");
                    attachment_errors += 1;
                    failed_attachment_keys.insert(key.clone());
                }
            }
        }
    }

    // ── Upload changed documents ──────────────────────────────────────────────
    let ingest_url = format!("{base_url}/api/v1/ingest");
    let mut uploaded = 0usize;
    let mut errors = 0usize;

    for upload_entry in &sync_result.to_upload {
        let Some(doc) = docs.get(&upload_entry.source_path) else {
            eprintln!(
                "Warning: server requested upload of unknown source_path '{}', skipping",
                upload_entry.source_path
            );
            continue;
        };

        if args.verbose {
            eprintln!(
                "Uploading: {} (slug: {})",
                upload_entry.source_path, upload_entry.actual_slug
            );
        }

        let blocking_attachments: Vec<&str> = doc
            .attachments
            .iter()
            .filter(|a| failed_attachment_keys.contains(&a.asset_key))
            .map(|a| a.raw_path.as_str())
            .collect();
        if !blocking_attachments.is_empty() {
            eprintln!(
                "Error: skipping '{}' — {} attachment(s) failed to upload: {}",
                doc.source_path,
                blocking_attachments.len(),
                blocking_attachments.join(", ")
            );
            errors += 1;
            continue;
        }

        let order = match u32::try_from(doc.order) {
            Ok(v) => v,
            Err(_) => {
                eprintln!(
                    "Error: '{}' has a negative order value ({}) in its front-matter — fix or remove the 'order' field and retry",
                    doc.source_path, doc.order
                );
                continue;
            }
        };

        let ingest_body = IngestRequest {
            service_token: token.clone(),
            source_path: doc.source_path.clone(),
            source_id: source_id.clone(),
            slug: upload_entry.actual_slug.clone(),
            title: doc.title.clone(),
            summary: doc.summary.clone(),
            content: doc.rewritten_content.clone(),
            access_level: doc.access_level.clone(),
            service_owner: doc.service_owner.clone(),
            tags: doc.tags.clone(),
            parent_slug: doc.parent_slug.clone(),
            order,
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
                let ingest: IngestResponse = r.json().await.unwrap_or(IngestResponse {
                    changed: true,
                    indexed: true,
                });
                uploaded += 1;
                if args.verbose {
                    let note = if ingest.changed {
                        "updated"
                    } else {
                        "metadata only"
                    };
                    println!(
                        "  uploaded: {} (slug: {}, {note})",
                        upload_entry.source_path, upload_entry.actual_slug
                    );
                } else {
                    println!("  uploaded: {}", upload_entry.source_path);
                }
                if !ingest.indexed {
                    eprintln!(
                        "  warning: {} was saved but failed to index in search/RAG — \
                         run an admin re-index to restore search/chat",
                        upload_entry.source_path
                    );
                }
            }
            Ok(r) => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                eprintln!(
                    "  error: {}: HTTP {status} — {body}",
                    upload_entry.source_path
                );
                errors += 1;
            }
            Err(e) => {
                eprintln!("  error: {}: {e}", upload_entry.source_path);
                errors += 1;
            }
        }
    }

    // ── Upload changed prompts ───────────────────────────────────────────────
    let prompt_ingest_url = format!("{base_url}/api/v1/prompts/ingest");
    let mut prompts_uploaded = 0usize;
    let mut prompt_errors = 0usize;

    for slug in &prompt_sync_result.to_upload {
        let Some(prompt) = prompts.get(slug) else {
            eprintln!("Warning: server requested upload of unknown prompt '{slug}', skipping");
            continue;
        };

        if args.verbose {
            eprintln!("Uploading prompt: {slug}");
        }

        let ingest_body = PromptIngestRequest {
            service_token: token.clone(),
            slug: prompt.slug.clone(),
            name: prompt.name.clone(),
            description: prompt.description.clone(),
            prompt_body: prompt.prompt_body.clone(),
            access_level: prompt.access_level.clone(),
            status: prompt.status.clone(),
            owner: prompt.owner.clone(),
            tags: prompt.tags.clone(),
            variables: prompt.variables.clone(),
            publish_to_mcp: prompt.publish_to_mcp,
            default_primary: prompt.default_primary,
            context_cost: prompt.context_cost.clone(),
        };

        let mut attempt = 0u32;
        let mut backoff_ms = INITIAL_BACKOFF_MS;
        let result = loop {
            match client
                .post(&prompt_ingest_url)
                .json(&ingest_body)
                .send()
                .await
            {
                Ok(r) if backoff_on_429(&r, &mut attempt, &mut backoff_ms).await => continue,
                other => break other,
            }
        };

        match result {
            Ok(r) if r.status().is_success() => {
                let ingest: PromptIngestResponse = r
                    .json()
                    .await
                    .unwrap_or(PromptIngestResponse { changed: true });
                prompts_uploaded += 1;
                if args.verbose {
                    let note = if ingest.changed {
                        "updated"
                    } else {
                        "metadata only"
                    };
                    println!("  uploaded prompt: {slug} ({note})");
                } else {
                    println!("  uploaded prompt: {slug}");
                }
            }
            Ok(r) => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                eprintln!("  error: prompt {slug}: HTTP {status} — {body}");
                prompt_errors += 1;
            }
            Err(e) => {
                eprintln!("  error: prompt {slug}: {e}");
                prompt_errors += 1;
            }
        }
    }

    // ── Upload changed schema versions ──────────────────────────────────────
    let schema_ingest_url = format!("{base_url}/api/v1/schemas");
    let mut schemas_uploaded = 0usize;
    let mut schema_errors = 0usize;

    for key in &schema_sync_result.to_upload {
        let Some(schema) = schemas.get(key) else {
            eprintln!(
                "Warning: server requested upload of unknown schema version '{key}', skipping"
            );
            continue;
        };

        if args.verbose {
            eprintln!("Uploading schema: {key}");
        }

        let ingest_body = SchemaIngestRequest {
            service_token: token.clone(),
            name: schema.name.clone(),
            schema_type: schema.schema_type.clone(),
            version: schema.version.clone(),
            status: schema.status.clone(),
            access_level: schema.access_level.clone(),
            service_owner: schema.service_owner.clone(),
            tags: schema.tags.clone(),
            content: schema.content.clone(),
        };

        let mut attempt = 0u32;
        let mut backoff_ms = INITIAL_BACKOFF_MS;
        let result = loop {
            match client
                .post(&schema_ingest_url)
                .json(&ingest_body)
                .send()
                .await
            {
                Ok(r) if backoff_on_429(&r, &mut attempt, &mut backoff_ms).await => continue,
                other => break other,
            }
        };

        match result {
            Ok(r) if r.status().is_success() => {
                let ingest: SchemaIngestResponse = r
                    .json()
                    .await
                    .unwrap_or(SchemaIngestResponse { changed: true });
                schemas_uploaded += 1;
                if args.verbose {
                    let note = if ingest.changed {
                        "updated"
                    } else {
                        "metadata only"
                    };
                    println!("  uploaded schema: {key} ({note})");
                } else {
                    println!("  uploaded schema: {key}");
                }
            }
            Ok(r) => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                eprintln!("  error: schema {key}: HTTP {status} — {body}");
                schema_errors += 1;
            }
            Err(e) => {
                eprintln!("  error: schema {key}: {e}");
                schema_errors += 1;
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
    println!(
        "Prompts: {prompts_uploaded} uploaded, {} unchanged, {} archived",
        prompt_sync_result.unchanged.len(),
        prompt_sync_result.to_archive.len(),
    );
    println!(
        "Schemas: {schemas_uploaded} uploaded, {} unchanged, {} archived",
        schema_sync_result.unchanged.len(),
        schema_sync_result.to_archive.len(),
    );

    let total_errors = errors + attachment_errors + prompt_errors + schema_errors;
    if total_errors > 0 {
        bail!("{total_errors} upload(s) failed");
    }

    Ok(())
}
