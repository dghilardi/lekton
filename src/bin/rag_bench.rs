//! Multi-config RAG benchmark with automated document ingest.
//!
//! For each config file in `<bench-dir>/configs/`, this tool:
//! 1. Creates a fresh, isolated Qdrant collection.
//! 2. Ingests every Markdown document from `<bench-dir>/docs/`.
//! 3. Runs all queries from the queries JSONL file.
//! 4. Writes a per-config JSON report to `<output-dir>/<config-name>.json`.
//! 5. After all configs, writes a comparative Markdown report.
//! 6. Drops the ephemeral collections (unless `--keep-collections` is set).
//!
//! # Usage
//!
//! ```sh
//! cargo run --bin rag-bench --features ssr --no-default-features -- \
//!     --bench-dir eval [--top-k 10] [--output-dir eval/reports]
//! ```
//!
//! # Config file format (TOML, self-sufficient)
//!
//! ```toml
//! name        = "baseline"
//! description = "Dense vector search, chunk_size=256"
//!
//! qdrant_url          = "http://localhost:6334"
//! embedding_url       = "http://localhost:11434/v1"
//! embedding_model     = "nomic-embed-text"
//! embedding_dimensions = 768
//! embedding_api_key   = ""
//! chunk_size_tokens   = 256
//! chunk_overlap_tokens = 64
//!
//! # optional — reranker disabled when empty
//! reranker_url = ""
//!
//! # optional — hybrid search disabled when false or meilisearch_url is empty
//! hybrid_search_enabled = false
//! # meilisearch_url   = "http://localhost:7700"
//! # meilisearch_api_key = ""
//! ```
//!
//! # Document file naming
//!
//! Each `.md` file in the `docs/` directory becomes a document.
//! The slug is derived from the filename (without extension).
//! The title is taken from the first `# Heading` in the file, or falls back
//! to the slug with hyphens replaced by spaces.

#[cfg(not(feature = "ssr"))]
fn main() {
    eprintln!("rag-bench requires the `ssr` feature");
    std::process::exit(1);
}

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    if let Err(e) = ssr::run().await {
        eprintln!("rag-bench failed: {e}");
        std::process::exit(1);
    }
}

#[cfg(feature = "ssr")]
mod ssr {
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    use chrono::Utc;
    use clap::Parser;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    use lekton::config::{ChatStepConfig, LlmConfig, LlmStepConfig, RagConfig, SearchConfig};
    use lekton::rag::eval::{
        admin_context, mean_metrics, read_queries, score, EvalQuery, Metrics, RagEvalContext,
        RetrievedChunk,
    };
    use lekton::rag::service::{DefaultRagService, RagService};
    use lekton::rag::vectorstore::{QdrantVectorStore, VectorStore};

    // ── CLI ───────────────────────────────────────────────────────────────────

    #[derive(Parser)]
    #[command(
        name = "rag-bench",
        about = "Multi-config RAG benchmark with automated document ingest.\n\n\
                 For each .toml file in <bench-dir>/configs/, ingests docs, \
                 runs queries, and produces per-config and comparative reports."
    )]
    struct Args {
        /// Root benchmark directory (must contain configs/, docs/ subdirs and
        /// a queries.jsonl file, or use --queries to override the query path).
        #[arg(short, long, default_value = "eval")]
        bench_dir: PathBuf,

        /// Path to the JSONL queries file. Defaults to <bench-dir>/queries.jsonl.
        #[arg(short, long)]
        queries: Option<PathBuf>,

        /// Top-k cutoff for Recall@k, MRR and nDCG@k.
        #[arg(short = 'k', long, default_value_t = 10, value_parser = parse_top_k)]
        top_k: usize,

        /// Directory to write reports. Defaults to <bench-dir>/reports.
        #[arg(short, long)]
        output_dir: Option<PathBuf>,

        /// Keep ephemeral Qdrant collections after the run (useful for debugging).
        #[arg(long)]
        keep_collections: bool,

        /// Run only the named config(s), comma-separated. Defaults to all configs.
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
    }

    // ── Bench config ──────────────────────────────────────────────────────────

    /// A self-sufficient configuration for one benchmark variant.
    #[derive(Debug, Deserialize)]
    struct BenchConfig {
        /// Short identifier used in report filenames and the comparative table.
        name: String,
        /// Human-readable description shown in reports (optional).
        #[serde(default)]
        description: String,

        // Qdrant
        qdrant_url: String,
        /// Overrides the auto-derived collection name (`lekton_bench_<name>`).
        #[serde(default)]
        qdrant_collection: Option<String>,

        // Embedding
        embedding_url: String,
        embedding_model: String,
        embedding_dimensions: u32,
        #[serde(default)]
        embedding_api_key: String,
        #[serde(default)]
        embedding_headers: HashMap<String, String>,

        // Chunking
        #[serde(default = "default_chunk_size")]
        chunk_size_tokens: u32,
        #[serde(default = "default_chunk_overlap")]
        chunk_overlap_tokens: u32,

        // Retrieval options
        #[serde(default)]
        expand_to_parent: bool,
        #[serde(default)]
        hybrid_search_enabled: bool,
        #[serde(default)]
        meilisearch_url: String,
        #[serde(default)]
        meilisearch_api_key: String,

        // Reranker (disabled when reranker_url is empty)
        #[serde(default)]
        reranker_url: String,
        #[serde(default)]
        reranker_model: String,
        #[serde(default)]
        reranker_api_key: String,
        #[serde(default)]
        reranker_headers: HashMap<String, String>,

        // Optional LLM-based retrieval enhancement steps
        #[serde(default)]
        analyzer: Option<LlmStepConfig>,
        #[serde(default)]
        hyde: Option<LlmStepConfig>,
    }

    fn default_chunk_size() -> u32 {
        256
    }
    fn default_chunk_overlap() -> u32 {
        64
    }

    fn parse_top_k(value: &str) -> Result<usize, String> {
        let parsed = value
            .parse::<usize>()
            .map_err(|e| format!("invalid top-k value: {e}"))?;
        if parsed == 0 {
            Err("top-k must be at least 1".into())
        } else {
            Ok(parsed)
        }
    }

    impl BenchConfig {
        fn collection_name(&self) -> String {
            self.qdrant_collection.clone().unwrap_or_else(|| {
                let safe = self.name.replace([' ', '-', '/'], "_");
                format!("lekton_bench_{safe}")
            })
        }

        /// Reject obviously broken configs early with an actionable message.
        fn validate(&self) -> Result<(), String> {
            if self.name.is_empty() {
                return Err("config has empty 'name'".into());
            }
            if self.qdrant_url.is_empty() {
                return Err(format!("[{}] qdrant_url must not be empty", self.name));
            }
            if self.embedding_url.is_empty() {
                return Err(format!("[{}] embedding_url must not be empty", self.name));
            }
            if self.embedding_dimensions == 0 {
                return Err(format!("[{}] embedding_dimensions must be > 0", self.name));
            }
            if self.chunk_size_tokens == 0 {
                return Err(format!("[{}] chunk_size_tokens must be > 0", self.name));
            }
            if self.chunk_overlap_tokens >= self.chunk_size_tokens {
                return Err(format!(
                    "[{}] chunk_overlap_tokens ({}) must be < chunk_size_tokens ({})",
                    self.name, self.chunk_overlap_tokens, self.chunk_size_tokens
                ));
            }
            if self.hybrid_search_enabled && self.meilisearch_url.is_empty() {
                return Err(format!(
                    "[{}] hybrid_search_enabled = true requires meilisearch_url",
                    self.name
                ));
            }
            Ok(())
        }

        fn to_rag_config(&self) -> RagConfig {
            RagConfig {
                qdrant_url: self.qdrant_url.clone(),
                qdrant_collection: self.collection_name(),
                embedding_url: self.embedding_url.clone(),
                embedding_model: self.embedding_model.clone(),
                embedding_dimensions: self.embedding_dimensions,
                embedding_api_key: self.embedding_api_key.clone(),
                embedding_headers: self.embedding_headers.clone(),
                embedding_cache_store_text: false,
                embedding_cache_query: false,
                chunk_size_tokens: self.chunk_size_tokens,
                chunk_overlap_tokens: self.chunk_overlap_tokens,
                expand_to_parent: self.expand_to_parent,
                hybrid_search_enabled: self.hybrid_search_enabled,
                reranker_url: self.reranker_url.clone(),
                reranker_model: self.reranker_model.clone(),
                reranker_api_key: self.reranker_api_key.clone(),
                reranker_headers: self.reranker_headers.clone(),
                // LLM / chat fields are not actually exercised by the bench
                // (we only call retrieve_only, never chat completion), but
                // ChatService::from_rag_config refuses to build without a model
                // and a parseable Tera template — so we provide harmless stubs.
                llm: LlmConfig {
                    // Non-empty URL satisfies LlmProvider validation; the bench
                    // never actually calls chat completion so it's never dialled.
                    url: "http://rag-bench-stub.invalid".into(),
                    api_key: String::new(),
                    model: "rag-bench-stub".into(),
                    headers: HashMap::new(),
                    vertex_project_id: String::new(),
                    vertex_location: String::new(),
                },
                chat: ChatStepConfig {
                    model: Some("rag-bench-stub".into()),
                    url: None,
                    api_key: None,
                    headers: None,
                    vertex_project_id: None,
                    vertex_location: None,
                    system_prompt_template: "stub".into(),
                },
                analyzer: self.analyzer.clone(),
                hyde: self.hyde.clone(),
                rewriter: None,
            }
        }

        fn to_search_config(&self) -> SearchConfig {
            SearchConfig {
                url: self.meilisearch_url.clone(),
                api_key: self.meilisearch_api_key.clone(),
            }
        }
    }

    // ── Document loading ──────────────────────────────────────────────────────

    struct BenchDoc {
        slug: String,
        title: String,
        content: String,
    }

    fn load_docs(docs_dir: &Path) -> Result<Vec<BenchDoc>, String> {
        let mut docs = Vec::new();
        let entries = fs::read_dir(docs_dir)
            .map_err(|e| format!("cannot read docs dir {}: {e}", docs_dir.display()))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("directory entry error: {e}"))?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let slug = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| format!("invalid filename: {}", path.display()))?
                .to_string();
            let content = fs::read_to_string(&path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            let title = extract_title(&content).unwrap_or_else(|| slug.replace('-', " "));
            docs.push(BenchDoc {
                slug,
                title,
                content,
            });
        }

        if docs.is_empty() {
            return Err(format!("no .md files found in {}", docs_dir.display()));
        }
        docs.sort_by(|a, b| a.slug.cmp(&b.slug));
        Ok(docs)
    }

    fn extract_title(content: &str) -> Option<String> {
        content
            .lines()
            .find(|l| l.starts_with("# "))
            .map(|l| l.trim_start_matches("# ").trim().to_string())
    }

    // ── Report types ──────────────────────────────────────────────────────────

    #[derive(Debug, Serialize)]
    struct PerQueryReport {
        id: String,
        query: String,
        expected_doc_slugs: Vec<String>,
        expected_chunk_ids: Vec<String>,
        expected_text_fragments: Vec<String>,
        retrieved_pre: Vec<RetrievedChunk>,
        retrieved_post: Vec<RetrievedChunk>,
        metrics_pre: Metrics,
        metrics_post: Metrics,
    }

    #[derive(Debug, Serialize)]
    struct ConfigReport {
        config_name: String,
        description: String,
        collection: String,
        top_k: usize,
        doc_count: usize,
        queries: Vec<PerQueryReport>,
        aggregate_pre: Metrics,
        aggregate_post: Metrics,
    }

    // ── Run one config ────────────────────────────────────────────────────────

    /// Run the ingest + eval phases for one config. The caller is responsible
    /// for collection cleanup so it always runs regardless of success or error.
    async fn run_config_inner(
        bench: &BenchConfig,
        vectorstore: &QdrantVectorStore,
        docs: &[BenchDoc],
        queries: &[EvalQuery],
        top_k: usize,
    ) -> Result<ConfigReport, String> {
        let rag_config = bench.to_rag_config();
        let collection = bench.collection_name();

        // Clean start: drop any leftover collection from a previous interrupted run.
        eprintln!("[{}] cleaning collection '{collection}'…", bench.name);
        vectorstore
            .delete_collection()
            .await
            .map_err(|e| format!("[{}] delete_collection: {e}", bench.name))?;
        vectorstore
            .ensure_collection(bench.embedding_dimensions)
            .await
            .map_err(|e| format!("[{}] ensure_collection: {e}", bench.name))?;

        // Ingest all documents. Benchmark uses a fixed access_level/is_draft/tags
        // because the eval treats every doc as part of the corpus regardless of
        // production metadata; this keeps recall numbers comparable across configs.
        eprintln!("[{}] ingesting {} documents…", bench.name, docs.len());
        let rag_service = DefaultRagService::from_rag_config(&rag_config)
            .map_err(|e| format!("[{}] DefaultRagService init: {e}", bench.name))?;

        for doc in docs {
            rag_service
                .index_document(&doc.slug, &doc.title, &doc.content, "public", false, &[])
                .await
                .map_err(|e| format!("[{}] index_document '{}': {e}", bench.name, doc.slug))?;
        }
        eprintln!("[{}] ingest complete", bench.name);

        // Build retrieval context. Hybrid search needs Meilisearch; otherwise None.
        let search_config = bench.to_search_config();
        let search_opt = if bench.hybrid_search_enabled && !bench.meilisearch_url.is_empty() {
            Some(&search_config)
        } else {
            None
        };
        let ctx = RagEvalContext::from_rag_config(&rag_config, search_opt)
            .await
            .map_err(|e| format!("[{}] RagEvalContext init: {e}", bench.name))?;

        let user_ctx = admin_context();
        let mut query_reports: Vec<PerQueryReport> = Vec::with_capacity(queries.len());

        for (idx, q) in queries.iter().enumerate() {
            let qid = q.id.clone().unwrap_or_else(|| format!("Q{:03}", idx + 1));
            let session_id = format!("rag-bench-{}", Uuid::new_v4());

            let retrieval = ctx
                .chat_service
                .retrieve_only(&user_ctx, &q.query, &[], &session_id)
                .await
                .map_err(|e| format!("[{}] retrieve_only {qid}: {e}", bench.name))?;

            let pre = score(&retrieval.pre_rerank, q, top_k);
            let post = score(&retrieval.post_rerank, q, top_k);

            query_reports.push(PerQueryReport {
                id: qid,
                query: q.query.clone(),
                expected_doc_slugs: q.expected_doc_slugs.clone(),
                expected_chunk_ids: q.expected_chunk_ids.clone(),
                expected_text_fragments: q.expected_text_fragments.clone(),
                retrieved_pre: pre.retrieved,
                retrieved_post: post.retrieved,
                metrics_pre: pre.metrics,
                metrics_post: post.metrics,
            });
        }

        let aggregate_pre = mean_metrics(query_reports.iter().map(|r| &r.metrics_pre));
        let aggregate_post = mean_metrics(query_reports.iter().map(|r| &r.metrics_post));

        Ok(ConfigReport {
            config_name: bench.name.clone(),
            description: bench.description.clone(),
            collection,
            top_k,
            doc_count: docs.len(),
            queries: query_reports,
            aggregate_pre,
            aggregate_post,
        })
    }

    /// Wrap [`run_config_inner`] so the collection is always dropped on the way
    /// out — even when ingest or retrieval fails partway through.
    async fn run_config(
        bench: &BenchConfig,
        docs: &[BenchDoc],
        queries: &[EvalQuery],
        top_k: usize,
        keep_collections: bool,
    ) -> Result<ConfigReport, String> {
        bench.validate()?;
        let collection = bench.collection_name();

        let vectorstore = QdrantVectorStore::new(&bench.qdrant_url, &collection)
            .map_err(|e| format!("[{}] failed to build Qdrant client: {e}", bench.name))?;

        let result = run_config_inner(bench, &vectorstore, docs, queries, top_k).await;

        if !keep_collections {
            // Best-effort cleanup: never override the original error if we have one,
            // but warn on cleanup failure so leaked collections are visible.
            if let Err(e) = vectorstore.delete_collection().await {
                eprintln!(
                    "[{}] cleanup warning (collection may leak): {e}",
                    bench.name
                );
            } else {
                eprintln!("[{}] dropped collection '{collection}'", bench.name);
            }
        }

        result
    }

    // ── Comparative Markdown report ───────────────────────────────────────────

    fn write_comparative_report(
        output_dir: &Path,
        reports: &[ConfigReport],
        top_k: usize,
        run_at: &str,
    ) -> Result<(), String> {
        let path = output_dir.join("comparative.md");
        let mut md = String::new();

        md.push_str("# RAG Benchmark — Comparative Report\n\n");
        md.push_str(&format!("Run at: {run_at}  \n"));
        md.push_str(&format!(
            "Documents: {}  |  Queries: {}  |  Top-k: {top_k}\n\n",
            reports.first().map(|r| r.doc_count).unwrap_or(0),
            reports.first().map(|r| r.queries.len()).unwrap_or(0),
        ));

        // Aggregate table (post-rerank).
        md.push_str("## Aggregate Metrics (post-rerank)\n\n");
        md.push_str(&format!(
            "| Config | Description | Recall@{top_k} | MRR | nDCG@{top_k} |\n"
        ));
        md.push_str("|--------|-------------|-----------|-----|----------|\n");
        for r in reports {
            md.push_str(&format!(
                "| `{}` | {} | {:.3} | {:.3} | {:.3} |\n",
                r.config_name,
                markdown_table_cell(&r.description),
                r.aggregate_post.recall_at_k,
                r.aggregate_post.mrr,
                r.aggregate_post.ndcg_at_k,
            ));
        }
        md.push('\n');

        // Pre-rerank table for comparison.
        md.push_str("## Aggregate Metrics (pre-rerank)\n\n");
        md.push_str(&format!(
            "| Config | Recall@{top_k} | MRR | nDCG@{top_k} |\n"
        ));
        md.push_str("|--------|-----------|-----|----------|\n");
        for r in reports {
            md.push_str(&format!(
                "| `{}` | {:.3} | {:.3} | {:.3} |\n",
                r.config_name,
                r.aggregate_pre.recall_at_k,
                r.aggregate_pre.mrr,
                r.aggregate_pre.ndcg_at_k,
            ));
        }
        md.push('\n');

        // Per-config, per-query detail.
        md.push_str("## Per-Query Detail\n\n");
        for r in reports {
            md.push_str(&format!("### `{}`\n\n", r.config_name));
            if !r.description.is_empty() {
                md.push_str(&format!("{}\n\n", r.description));
            }
            md.push_str(&format!(
                "| ID | Query | pre R@{top_k} | pre MRR | post R@{top_k} | post MRR |\n"
            ));
            md.push_str("|-----|-------|----------|---------|----------|----------|\n");
            for q in &r.queries {
                let query_short = truncate_chars(&q.query, 60);
                md.push_str(&format!(
                    "| {} | {} | {:.3} | {:.3} | {:.3} | {:.3} |\n",
                    q.id,
                    markdown_table_cell(&query_short),
                    q.metrics_pre.recall_at_k,
                    q.metrics_pre.mrr,
                    q.metrics_post.recall_at_k,
                    q.metrics_post.mrr,
                ));
            }
            md.push('\n');
        }

        fs::write(&path, &md).map_err(|e| {
            format!(
                "failed to write comparative report to {}: {e}",
                path.display()
            )
        })?;
        eprintln!("rag-bench: comparative report → {}", path.display());
        Ok(())
    }

    fn truncate_chars(value: &str, max_chars: usize) -> String {
        let mut chars = value.chars();
        let truncated: String = chars.by_ref().take(max_chars).collect();
        if chars.next().is_some() {
            format!("{truncated}…")
        } else {
            truncated
        }
    }

    fn markdown_table_cell(value: &str) -> String {
        value.replace('|', "\\|").replace(['\r', '\n'], " ")
    }

    // ── Entry point ───────────────────────────────────────────────────────────

    pub async fn run() -> Result<(), String> {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "warn,lekton::rag=info".into()),
            )
            .init();

        let args = Args::parse();

        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let configs_dir = args.bench_dir.join("configs");
        let docs_dir = args.bench_dir.join("docs");
        let queries_path = args
            .queries
            .clone()
            .unwrap_or_else(|| args.bench_dir.join("queries.jsonl"));
        let output_dir = args
            .output_dir
            .clone()
            .unwrap_or_else(|| args.bench_dir.join("reports"));

        // Load configs.
        let mut config_files: Vec<PathBuf> = fs::read_dir(&configs_dir)
            .map_err(|e| format!("cannot read configs dir {}: {e}", configs_dir.display()))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("toml"))
            .collect();
        config_files.sort();

        if config_files.is_empty() {
            return Err(format!(
                "no .toml config files found in {}",
                configs_dir.display()
            ));
        }

        let mut configs: Vec<BenchConfig> = Vec::new();
        for path in &config_files {
            let text = fs::read_to_string(path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            let cfg: BenchConfig = toml::from_str(&text)
                .map_err(|e| format!("parse error in {}: {e}", path.display()))?;
            if !args.only.is_empty() && !args.only.contains(&cfg.name) {
                continue;
            }
            configs.push(cfg);
        }

        if configs.is_empty() {
            return Err("no configs to run (check --only filter)".into());
        }

        // Load docs and queries.
        let docs = load_docs(&docs_dir)?;
        let queries = read_queries(&queries_path)?;

        eprintln!(
            "rag-bench: {} config(s), {} doc(s), {} quer(ies), top-k={}",
            configs.len(),
            docs.len(),
            queries.len(),
            args.top_k
        );

        fs::create_dir_all(&output_dir)
            .map_err(|e| format!("cannot create output dir {}: {e}", output_dir.display()))?;

        let run_at = Utc::now().to_rfc3339();
        let mut all_reports: Vec<ConfigReport> = Vec::new();

        for bench in &configs {
            eprintln!("\n=== config: {} ===", bench.name);
            let report =
                run_config(bench, &docs, &queries, args.top_k, args.keep_collections).await?;

            // Print aggregate to stdout.
            println!(
                "{:<20} post R@{}={:.3}  MRR={:.3}  nDCG={:.3}",
                bench.name,
                args.top_k,
                report.aggregate_post.recall_at_k,
                report.aggregate_post.mrr,
                report.aggregate_post.ndcg_at_k,
            );

            // Write per-config JSON report.
            let json_path = output_dir.join(format!("{}.json", bench.name));
            let mut file = fs::File::create(&json_path)
                .map_err(|e| format!("cannot create {}: {e}", json_path.display()))?;
            serde_json::to_writer_pretty(&mut file, &report)
                .map_err(|e| format!("failed to serialise report: {e}"))?;
            eprintln!("rag-bench: wrote {}", json_path.display());

            all_reports.push(report);
        }

        // Write comparative Markdown report.
        write_comparative_report(&output_dir, &all_reports, args.top_k, &run_at)?;

        eprintln!("\nrag-bench: done — reports in {}", output_dir.display());
        Ok(())
    }
}
