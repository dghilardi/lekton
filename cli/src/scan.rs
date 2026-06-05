use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use walkdir::WalkDir;

use crate::attachment::{extract_local_file_refs, resolve_attachments, rewrite_content};
use crate::config::PromptFile;
use crate::config::{LektonConfig, SchemaManifestFile};
use crate::hash::{
    compute_hash, compute_metadata_hash, compute_prompt_metadata_hash, compute_schema_metadata_hash,
};
use crate::models::{DocumentInfo, PromptInfo, ScannedDoc, SchemaInfo};
use crate::slug::{
    apply_prefix, is_index_file, normalize_summary, prompt_slug_from_path, schema_name_from_dir,
    slug_from_path, slug_from_title, source_path_from_file, warn_about_summary,
};

pub fn scan_documents(
    root: &Path,
    config: &LektonConfig,
    verbose: bool,
) -> Result<HashMap<String, DocumentInfo>> {
    let max_attachment_size_bytes =
        (config.max_attachment_size_mb.unwrap_or(10) as u64) * 1024 * 1024;
    let mut scanned: Vec<ScannedDoc> = Vec::new();

    for entry in WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_file()
                && e.path().extension().and_then(|ext| ext.to_str()) == Some("md")
        })
    {
        let path = entry.path();
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        let (fm, body) = crate::front_matter::parse_front_matter(&source);

        if !fm.lekton_import {
            if verbose {
                eprintln!("Skipping {} (missing lekton-import: true)", path.display());
            }
            continue;
        }

        let source_path = source_path_from_file(path, root);
        let path_derived = slug_from_path(path, root);

        let slug_raw = if let Some(ref explicit) = fm.slug {
            explicit.clone()
        } else if is_index_file(path) {
            // README.md / index.md always maps to the directory slug, ignoring the title
            path_derived.clone()
        } else if let Some(ref title) = fm.title {
            let title_slug = slug_from_title(title);
            if let Some((parent_dir, _)) = path_derived.rsplit_once('/') {
                format!("{parent_dir}/{title_slug}")
            } else {
                title_slug
            }
        } else {
            path_derived.clone()
        };

        let legacy_slug_raw = fm.slug.clone().unwrap_or_else(|| path_derived.clone());

        let parsed_parent = if let Some(p) = fm.parent_slug {
            Some(p)
        } else if let Some((parent, _)) = slug_raw.rsplit_once('/') {
            Some(parent.to_string())
        } else {
            None
        };

        let slug = match &config.slug_prefix {
            Some(prefix) if !prefix.is_empty() => format!("{prefix}/{slug_raw}"),
            _ => slug_raw.clone(),
        };

        let legacy_slug_full = match &config.slug_prefix {
            Some(prefix) if !prefix.is_empty() => format!("{prefix}/{legacy_slug_raw}"),
            _ => legacy_slug_raw.clone(),
        };
        let legacy_slug = if legacy_slug_full != slug {
            Some(legacy_slug_full)
        } else {
            None
        };

        let parent_slug = match &config.slug_prefix {
            Some(prefix) if !prefix.is_empty() => match parsed_parent {
                Some(ref p) => Some(format!("{prefix}/{p}")),
                None => Some(prefix.clone()),
            },
            _ => parsed_parent,
        };

        let title = fm.title.unwrap_or_else(|| {
            slug.rsplit_once('/')
                .map(|(_, s)| s)
                .unwrap_or(&slug)
                .replace('-', " ")
                .split_whitespace()
                .map(|w| {
                    let mut c = w.chars();
                    match c.next() {
                        None => String::new(),
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        });
        let summary = normalize_summary(fm.summary);
        warn_about_summary(&source_path, summary.as_deref());
        let access_level = fm
            .access_level
            .or_else(|| config.default_access_level.clone())
            .unwrap_or_else(|| "public".to_string());
        let service_owner = fm
            .service_owner
            .or_else(|| config.default_service_owner.clone())
            .unwrap_or_default();
        let tags = fm.tags.unwrap_or_default();
        let is_hidden = fm.is_hidden.unwrap_or(false);

        let md_file_dir = path.parent().unwrap_or(root);
        let local_refs = extract_local_file_refs(&body, md_file_dir);
        let attachments = resolve_attachments(&local_refs, &slug, max_attachment_size_bytes);

        let rewritten_content = rewrite_content(&body, &attachments);
        let content_hash = compute_hash(&rewritten_content);

        scanned.push(ScannedDoc {
            source_path,
            slug,
            legacy_slug,
            title,
            summary,
            content: body,
            rewritten_content,
            content_hash,
            metadata_hash: String::new(),
            access_level,
            service_owner,
            tags,
            parent_slug,
            explicit_order: fm.order,
            is_hidden,
            attachments,
        });
    }

    // Assign implicit order based on alphabetical filename position
    {
        let mut groups: HashMap<Option<String>, Vec<usize>> = HashMap::new();
        for (i, doc) in scanned.iter().enumerate() {
            groups.entry(doc.parent_slug.clone()).or_default().push(i);
        }

        for indices in groups.values() {
            let mut sorted: Vec<usize> = indices.clone();
            sorted.sort_by(|&a, &b| {
                let fa = scanned[a]
                    .source_path
                    .rsplit_once('/')
                    .map(|(_, f)| f)
                    .unwrap_or(&scanned[a].source_path);
                let fb = scanned[b]
                    .source_path
                    .rsplit_once('/')
                    .map(|(_, f)| f)
                    .unwrap_or(&scanned[b].source_path);
                fa.cmp(fb)
            });

            let mut implicit = 0i32;
            for &idx in &sorted {
                if scanned[idx].explicit_order.is_none() {
                    scanned[idx].explicit_order = Some(implicit);
                }
                implicit += 1;
            }
        }
    }

    let mut docs: HashMap<String, DocumentInfo> = HashMap::new();
    for mut doc in scanned {
        let order = doc.explicit_order.unwrap_or(0);
        doc.metadata_hash = compute_metadata_hash(
            &doc.title,
            doc.summary.as_deref(),
            &doc.access_level,
            &doc.service_owner,
            &doc.tags,
            doc.parent_slug.as_deref(),
            order,
            doc.is_hidden,
        );
        docs.insert(
            doc.source_path.clone(),
            DocumentInfo {
                source_path: doc.source_path,
                slug: doc.slug,
                legacy_slug: doc.legacy_slug,
                title: doc.title,
                summary: doc.summary,
                content: doc.content,
                rewritten_content: doc.rewritten_content,
                content_hash: doc.content_hash,
                metadata_hash: doc.metadata_hash,
                access_level: doc.access_level,
                service_owner: doc.service_owner,
                tags: doc.tags,
                parent_slug: doc.parent_slug,
                order,
                is_hidden: doc.is_hidden,
                attachments: doc.attachments,
            },
        );
    }

    Ok(docs)
}

pub fn scan_prompts(root: &Path, config: &LektonConfig) -> Result<HashMap<String, PromptInfo>> {
    let prompts_dir = config
        .prompts_dir
        .as_deref()
        .filter(|p| !p.is_empty())
        .unwrap_or("prompts");
    let prompt_root = root.join(prompts_dir);

    if !prompt_root.exists() {
        return Ok(HashMap::new());
    }

    let prompt_prefix = config.prompt_slug_prefix.as_deref().or(Some("prompts"));
    let mut prompts = HashMap::new();

    for entry in WalkDir::new(&prompt_root)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_file()
                && matches!(
                    e.path().extension().and_then(|ext| ext.to_str()),
                    Some("yaml") | Some("yml")
                )
        })
    {
        let path = entry.path();
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let prompt_file: PromptFile = serde_yaml::from_str(&source)
            .with_context(|| format!("Failed to parse prompt YAML {}", path.display()))?;

        if prompt_file.lekton_import == Some(false) {
            continue;
        }

        let raw_slug = prompt_file
            .slug
            .clone()
            .unwrap_or_else(|| prompt_slug_from_path(path, &prompt_root));
        let slug = apply_prefix(prompt_prefix, &raw_slug);
        let name = prompt_file.name.clone().unwrap_or_else(|| {
            raw_slug
                .split('/')
                .last()
                .unwrap_or(&raw_slug)
                .replace('-', " ")
        });
        let description = prompt_file.description.clone().context(format!(
            "Prompt '{}' is missing 'description'",
            path.display()
        ))?;
        let prompt_body = prompt_file.prompt_body.clone().context(format!(
            "Prompt '{}' is missing 'prompt_body'",
            path.display()
        ))?;
        let access_level = prompt_file
            .access_level
            .clone()
            .or_else(|| config.default_access_level.clone())
            .unwrap_or_else(|| "public".to_string());
        let owner = prompt_file
            .owner
            .clone()
            .or_else(|| config.default_service_owner.clone())
            .unwrap_or_default();
        if owner.trim().is_empty() {
            bail!(
                "Prompt '{}' is missing 'owner' and no default_service_owner is configured",
                path.display()
            );
        }
        let status = prompt_file
            .status
            .clone()
            .unwrap_or_else(|| "active".to_string());
        let tags = prompt_file.tags.clone().unwrap_or_default();
        let variables = prompt_file.variables.clone().unwrap_or_default();
        let publish_to_mcp = prompt_file.publish_to_mcp.unwrap_or(false);
        let default_primary = prompt_file.default_primary.unwrap_or(false);
        let context_cost = prompt_file
            .context_cost
            .clone()
            .unwrap_or_else(|| "medium".to_string());

        let content_hash = compute_hash(&prompt_body);
        let metadata_hash = compute_prompt_metadata_hash(
            &name,
            &description,
            &access_level,
            &status,
            &owner,
            &tags,
            &variables,
            publish_to_mcp,
            default_primary,
            &context_cost,
        );

        prompts.insert(
            slug.clone(),
            PromptInfo {
                slug,
                name,
                description,
                prompt_body,
                content_hash,
                metadata_hash,
                access_level,
                status,
                owner,
                tags,
                variables,
                publish_to_mcp,
                default_primary,
                context_cost,
            },
        );
    }

    Ok(prompts)
}

pub fn scan_schemas(root: &Path, config: &LektonConfig) -> Result<HashMap<String, SchemaInfo>> {
    let schemas_dir = config
        .schemas_dir
        .as_deref()
        .filter(|p| !p.is_empty())
        .unwrap_or("schemas");
    let schema_root = root.join(schemas_dir);

    if !schema_root.exists() {
        return Ok(HashMap::new());
    }

    let mut schemas = HashMap::new();

    for entry in WalkDir::new(&schema_root)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_file()
                && matches!(
                    e.file_name().to_str(),
                    Some("lekton.schema.yml") | Some("lekton.schema.yaml")
                )
        })
    {
        let manifest_path = entry.path();
        let manifest_dir = manifest_path.parent().unwrap_or(&schema_root);
        let source = std::fs::read_to_string(manifest_path)
            .with_context(|| format!("Failed to read {}", manifest_path.display()))?;
        let manifest: SchemaManifestFile = serde_yaml::from_str(&source)
            .with_context(|| format!("Failed to parse {}", manifest_path.display()))?;

        if manifest.schema_type.trim().is_empty() {
            bail!(
                "Schema manifest '{}' is missing 'schema_type'",
                manifest_path.display()
            );
        }

        let raw_name = manifest
            .name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| schema_name_from_dir(manifest_dir, &schema_root));
        let name = apply_prefix(config.schema_name_prefix.as_deref(), &raw_name);
        let service_owner = manifest
            .service_owner
            .clone()
            .or_else(|| config.default_service_owner.clone())
            .unwrap_or_default();
        if service_owner.trim().is_empty() {
            bail!(
                "Schema manifest '{}' is missing 'service_owner' and no default_service_owner is configured",
                manifest_path.display()
            );
        }
        let tags = manifest.tags.clone().unwrap_or_default();

        for version in manifest.versions {
            if version.file.trim().is_empty() {
                bail!(
                    "Schema manifest '{}' has a version entry without 'file'",
                    manifest_path.display()
                );
            }
            if version.version.trim().is_empty() {
                bail!(
                    "Schema manifest '{}' has a version entry without 'version'",
                    manifest_path.display()
                );
            }

            let spec_path = manifest_dir.join(&version.file);
            let content = std::fs::read_to_string(&spec_path)
                .with_context(|| format!("Failed to read {}", spec_path.display()))?;
            let access_level = version
                .access_level
                .clone()
                .or_else(|| manifest.default_access_level.clone())
                .or_else(|| config.default_access_level.clone())
                .unwrap_or_else(|| "public".to_string());
            let content_hash = compute_hash(&content);
            let metadata_hash = compute_schema_metadata_hash(&version.status, &access_level);
            let key = format!("{}@{}", name, version.version);

            schemas.insert(
                key.clone(),
                SchemaInfo {
                    key,
                    name: name.clone(),
                    schema_type: manifest.schema_type.clone(),
                    version: version.version,
                    status: version.status,
                    access_level,
                    service_owner: service_owner.clone(),
                    tags: tags.clone(),
                    content,
                    content_hash,
                    metadata_hash,
                },
            );
        }
    }

    Ok(schemas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn scan_document_accepts_camel_case_front_matter_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("guide.md");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(
                b"---\ntitle: Guide\nsummary: A concise guide for testing camel case front matter fields in document sync.\naccessLevel: public\nserviceOwner: docs\nparentSlug: handbook\nisHidden: true\nlektonImport: true\n---\n# Guide body\n",
            )
            .unwrap();
        let docs = scan_documents(dir.path(), &LektonConfig::default(), false).unwrap();
        assert_eq!(docs.len(), 1);
        let doc = docs.values().next().unwrap();
        assert_eq!(
            doc.summary.as_deref(),
            Some("A concise guide for testing camel case front matter fields in document sync.")
        );
        assert_eq!(doc.access_level, "public");
        assert_eq!(doc.service_owner, "docs");
        assert_eq!(doc.parent_slug.as_deref(), Some("handbook"));
        assert!(doc.is_hidden);
    }

    #[test]
    fn scan_skips_files_without_front_matter() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("no-fm.md");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"# No front matter\n")
            .unwrap();
        let docs = scan_documents(dir.path(), &LektonConfig::default(), false).unwrap();
        assert!(docs.is_empty());
    }

    #[test]
    fn scan_picks_up_titled_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("guide.md");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"---\ntitle: Guide\naccess_level: public\nlekton-import: true\n---\n# Guide body\n")
            .unwrap();
        let docs = scan_documents(dir.path(), &LektonConfig::default(), false).unwrap();
        assert_eq!(docs.len(), 1);
        let doc = docs.values().next().unwrap();
        assert_eq!(doc.title, "Guide");
        assert_eq!(doc.access_level, "public");
        assert!(doc.content_hash.starts_with("sha256:"));
    }

    #[test]
    fn scan_skips_file_without_lekton_import() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("readme.md");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"---\ntitle: README\n---\n# Not for Lekton\n")
            .unwrap();
        let docs = scan_documents(dir.path(), &LektonConfig::default(), false).unwrap();
        assert!(docs.is_empty());
    }

    #[test]
    fn scan_explicit_slug_with_prefix_does_not_self_reference_parent() {
        use std::fs;
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("guidelines");
        fs::create_dir_all(&sub).unwrap();
        let path = sub.join("guidelines_mcu.md");
        fs::File::create(&path)
            .unwrap()
            .write_all(
                b"---\ntitle: MCU Team Guidelines\nsummary: Guidelines for the MCU team covering build, branching, and coding standards.\nslug: guidelines\naccess_level: micro\nservice_owner: micro\nlekton-import: true\n---\n# MCU Guidelines\n",
            )
            .unwrap();

        let config = LektonConfig {
            slug_prefix: Some("micro/docs".to_string()),
            ..LektonConfig::default()
        };
        let docs = scan_documents(dir.path(), &config, false).unwrap();
        assert_eq!(docs.len(), 1);
        let doc = docs.values().next().unwrap();
        assert_eq!(doc.slug, "micro/docs/guidelines");
        assert_ne!(
            doc.parent_slug.as_deref(),
            Some("micro/docs/guidelines"),
            "parent_slug must not equal slug (self-referential)"
        );
        assert_eq!(doc.parent_slug.as_deref(), Some("micro/docs"));
    }

    #[test]
    fn scan_detects_attachments() {
        let dir = tempfile::tempdir().unwrap();

        let img_dir = dir.path().join("images");
        std::fs::create_dir(&img_dir).unwrap();
        std::fs::File::create(img_dir.join("logo.png"))
            .unwrap()
            .write_all(b"fake png data")
            .unwrap();

        let md_path = dir.path().join("guide.md");
        std::fs::File::create(&md_path)
            .unwrap()
            .write_all(
                b"---\ntitle: Guide\nlekton-import: true\n---\n# Guide\n\n![logo](images/logo.png)\n",
            )
            .unwrap();

        let docs = scan_documents(dir.path(), &LektonConfig::default(), false).unwrap();
        assert_eq!(docs.len(), 1);
        let doc = docs.values().next().unwrap();
        assert_eq!(doc.attachments.len(), 1);
        assert_eq!(doc.attachments[0].asset_key, "attachments/guide/logo.png");
        assert!(doc
            .rewritten_content
            .contains("/api/v1/assets/attachments/guide/logo.png"));
        assert!(doc.content.contains("images/logo.png"));
    }

    #[test]
    fn scan_prompts_reads_yaml_prompt_files() {
        let dir = tempfile::tempdir().unwrap();
        let prompts_dir = dir.path().join("prompts");
        std::fs::create_dir(&prompts_dir).unwrap();

        let path = prompts_dir.join("code-review.yaml");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(
                br#"name: Code Review
description: Review a patch
access_level: developer
status: active
owner: platform-team
publish_to_mcp: true
default_primary: true
context_cost: medium
prompt_body: |
  Review this diff carefully.
"#,
            )
            .unwrap();

        let prompts = scan_prompts(dir.path(), &LektonConfig::default()).unwrap();
        assert_eq!(prompts.len(), 1);
        let prompt = prompts.values().next().unwrap();
        assert_eq!(prompt.slug, "prompts/code-review");
        assert_eq!(prompt.name, "Code Review");
        assert!(prompt.publish_to_mcp);
        assert!(prompt.default_primary);
        assert!(prompt.content_hash.starts_with("sha256:"));
        assert!(prompt.metadata_hash.starts_with("sha256:"));
    }

    #[test]
    fn scan_prompts_uses_configurable_prefix_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let prompts_dir = dir.path().join("ai-prompts");
        std::fs::create_dir(&prompts_dir).unwrap();

        let path = prompts_dir.join("ops.yaml");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(
                br#"description: Ops review
prompt_body: |
  Analyze the incident.
"#,
            )
            .unwrap();

        let config = LektonConfig {
            prompts_dir: Some("ai-prompts".to_string()),
            prompt_slug_prefix: Some("company/prompts".to_string()),
            default_access_level: Some("internal".to_string()),
            default_service_owner: Some("platform-team".to_string()),
            ..LektonConfig::default()
        };

        let prompts = scan_prompts(dir.path(), &config).unwrap();
        let prompt = prompts.values().next().unwrap();
        assert_eq!(prompt.slug, "company/prompts/ops");
        assert_eq!(prompt.access_level, "internal");
        assert_eq!(prompt.owner, "platform-team");
        assert_eq!(prompt.status, "active");
        assert_eq!(prompt.context_cost, "medium");
    }

    #[test]
    fn scan_prompts_skips_prompt_with_explicit_import_false() {
        let dir = tempfile::tempdir().unwrap();
        let prompts_dir = dir.path().join("prompts");
        std::fs::create_dir(&prompts_dir).unwrap();

        let path = prompts_dir.join("hidden.yaml");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(
                br#"name: Hidden Prompt
description: Should not be imported
owner: platform-team
lekton-import: false
prompt_body: |
  Ignore me.
"#,
            )
            .unwrap();

        let prompts = scan_prompts(dir.path(), &LektonConfig::default()).unwrap();
        assert!(prompts.is_empty());
    }

    #[test]
    fn scan_prompts_requires_owner_without_default() {
        let dir = tempfile::tempdir().unwrap();
        let prompts_dir = dir.path().join("prompts");
        std::fs::create_dir(&prompts_dir).unwrap();

        let path = prompts_dir.join("missing-owner.yaml");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(
                br#"name: Missing Owner
description: No owner set
prompt_body: |
  This should fail.
"#,
            )
            .unwrap();

        let err = scan_prompts(dir.path(), &LektonConfig::default()).unwrap_err();
        assert!(err
            .to_string()
            .contains("missing 'owner' and no default_service_owner is configured"));
    }

    #[test]
    fn scan_schemas_reads_manifest_versions() {
        let dir = tempfile::tempdir().unwrap();
        let schema_dir = dir.path().join("schemas").join("payments");
        std::fs::create_dir_all(&schema_dir).unwrap();

        std::fs::File::create(schema_dir.join("openapi-v1.yaml"))
            .unwrap()
            .write_all(b"openapi: 3.0.0\ninfo:\n  title: Payments\n  version: 1.0.0\npaths: {}\n")
            .unwrap();
        std::fs::File::create(schema_dir.join("lekton.schema.yml"))
            .unwrap()
            .write_all(
                br#"name: payment-api
schema_type: openapi
service_owner: payments
default_access_level: internal
tags: [payments, api]
versions:
  - file: openapi-v1.yaml
    version: 1.0.0
    status: stable
"#,
            )
            .unwrap();

        let schemas = scan_schemas(dir.path(), &LektonConfig::default()).unwrap();
        assert_eq!(schemas.len(), 1);
        let schema = schemas.values().next().unwrap();
        assert_eq!(schema.key, "payment-api@1.0.0");
        assert_eq!(schema.name, "payment-api");
        assert_eq!(schema.access_level, "internal");
        assert_eq!(schema.service_owner, "payments");
        assert_eq!(schema.tags, vec!["payments".to_string(), "api".to_string()]);
        assert!(schema.content_hash.starts_with("sha256:"));
        assert!(schema.metadata_hash.starts_with("sha256:"));
    }

    #[test]
    fn scan_schemas_applies_prefix_and_default_owner() {
        let dir = tempfile::tempdir().unwrap();
        let schema_dir = dir.path().join("contracts").join("orders");
        std::fs::create_dir_all(&schema_dir).unwrap();

        std::fs::File::create(schema_dir.join("asyncapi-v1.yaml"))
            .unwrap()
            .write_all(b"asyncapi: 2.6.0\ninfo:\n  title: Orders\n  version: 1.0.0\nchannels: {}\n")
            .unwrap();
        std::fs::File::create(schema_dir.join("lekton.schema.yml"))
            .unwrap()
            .write_all(
                br#"schema_type: asyncapi
versions:
  - file: asyncapi-v1.yaml
    version: 1.0.0
    status: beta
    access_level: developer
"#,
            )
            .unwrap();

        let config = LektonConfig {
            schemas_dir: Some("contracts".to_string()),
            schema_name_prefix: Some("platform".to_string()),
            default_service_owner: Some("platform-team".to_string()),
            ..LektonConfig::default()
        };

        let schemas = scan_schemas(dir.path(), &config).unwrap();
        let schema = schemas.values().next().unwrap();
        assert_eq!(schema.name, "platform/orders");
        assert_eq!(schema.schema_type, "asyncapi");
        assert_eq!(schema.service_owner, "platform-team");
        assert_eq!(schema.access_level, "developer");
        assert_eq!(schema.status, "beta");
    }
}
