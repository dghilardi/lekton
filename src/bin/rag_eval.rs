//! Offline retrieval evaluation harness.
//!
//! Reads a JSONL eval set, runs `ChatService::retrieve_only` against an
//! already-indexed Qdrant collection, and reports Recall@k, MRR and nDCG@k
//! for both the pre-rerank and post-rerank candidate sets.
//!
//! Configuration is loaded from the standard Lekton config stack (env vars +
//! `config/lekton.toml`). The collection must already be indexed; this tool
//! does not ingest documents. Use `rag-bench` when you also need ingest and
//! multi-config comparison.
//!
//! # Usage
//!
//! ```sh
//! cargo run --bin rag-eval --features ssr --no-default-features -- \
//!     --queries eval/queries.jsonl [--top-k 10] [--json-output out.json]
//! ```

#[cfg(not(feature = "ssr"))]
fn main() {
    eprintln!("rag-eval requires the `ssr` feature");
    std::process::exit(1);
}

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    if let Err(e) = ssr::run().await {
        eprintln!("rag-eval failed: {e}");
        std::process::exit(1);
    }
}

#[cfg(feature = "ssr")]
mod ssr {
    use std::fs::File;
    use std::io::Write;
    use std::path::PathBuf;

    use clap::Parser;
    use serde::Serialize;
    use uuid::Uuid;

    use lekton::config::AppConfig;
    use lekton::rag::eval::{
        admin_context, mean_metrics, read_queries, score, Metrics, RagEvalContext, RetrievedChunk,
    };

    #[derive(Parser)]
    #[command(
        name = "rag-eval",
        about = "Offline retrieval metrics for the Lekton RAG pipeline.\n\n\
                 Requires an already-indexed Qdrant collection. \
                 Use rag-bench when you need automated ingest + multi-config comparison."
    )]
    struct Args {
        /// JSONL eval set (one query record per line).
        #[arg(short, long)]
        queries: PathBuf,

        /// Top-k cutoff for Recall@k, MRR and nDCG@k.
        #[arg(short = 'k', long, default_value_t = 10, value_parser = parse_top_k)]
        top_k: usize,

        /// Write a full JSON report to this path (optional).
        #[arg(short, long)]
        json_output: Option<PathBuf>,
    }

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
    struct RunReport {
        top_k: usize,
        queries: Vec<PerQueryReport>,
        aggregate_pre: Metrics,
        aggregate_post: Metrics,
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

    pub async fn run() -> Result<(), String> {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "warn,lekton::rag=info".into()),
            )
            .init();

        let args = Args::parse();

        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let config = AppConfig::load().map_err(|e| format!("config load failed: {e}"))?;
        let ctx = RagEvalContext::from_config(&config)
            .await
            .map_err(|e| format!("RagEvalContext init failed: {e}"))?;

        let queries = read_queries(&args.queries)?;
        if queries.is_empty() {
            return Err(format!("no queries found in {}", args.queries.display()));
        }
        eprintln!(
            "rag-eval: loaded {} queries from {}",
            queries.len(),
            args.queries.display()
        );

        let user_ctx = admin_context();
        let mut reports: Vec<PerQueryReport> = Vec::with_capacity(queries.len());

        for (idx, q) in queries.iter().enumerate() {
            let qid = q.id.clone().unwrap_or_else(|| format!("Q{:03}", idx + 1));
            let session_id = format!("rag-eval-{}", Uuid::new_v4());

            let retrieval = ctx
                .chat_service
                .retrieve_only(&user_ctx, &q.query, &[], &session_id)
                .await
                .map_err(|e| format!("retrieve_only failed for {qid}: {e}"))?;

            let pre = score(&retrieval.pre_rerank, q, args.top_k);
            let post = score(&retrieval.post_rerank, q, args.top_k);

            println!(
                "{qid:<10} pre  R@{k}={:>5.3}  MRR={:>5.3}  nDCG={:>5.3}   \
                 post R@{k}={:>5.3}  MRR={:>5.3}  nDCG={:>5.3}",
                pre.metrics.recall_at_k,
                pre.metrics.mrr,
                pre.metrics.ndcg_at_k,
                post.metrics.recall_at_k,
                post.metrics.mrr,
                post.metrics.ndcg_at_k,
                k = args.top_k,
            );

            reports.push(PerQueryReport {
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

        let aggregate_pre = mean_metrics(reports.iter().map(|r| &r.metrics_pre));
        let aggregate_post = mean_metrics(reports.iter().map(|r| &r.metrics_post));

        println!();
        println!(
            "─── aggregate over {} queries (top-k={}) ───",
            reports.len(),
            args.top_k
        );
        println!(
            "pre-rerank   R@{k}={:>5.3}  MRR={:>5.3}  nDCG={:>5.3}",
            aggregate_pre.recall_at_k,
            aggregate_pre.mrr,
            aggregate_pre.ndcg_at_k,
            k = args.top_k,
        );
        println!(
            "post-rerank  R@{k}={:>5.3}  MRR={:>5.3}  nDCG={:>5.3}",
            aggregate_post.recall_at_k,
            aggregate_post.mrr,
            aggregate_post.ndcg_at_k,
            k = args.top_k,
        );

        if let Some(path) = &args.json_output {
            let report = RunReport {
                top_k: args.top_k,
                queries: reports,
                aggregate_pre,
                aggregate_post,
            };
            let mut file = File::create(path)
                .map_err(|e| format!("failed to create {}: {e}", path.display()))?;
            serde_json::to_writer_pretty(&mut file, &report)
                .map_err(|e| format!("failed to serialize report: {e}"))?;
            file.write_all(b"\n").ok();
            eprintln!("rag-eval: wrote JSON report to {}", path.display());
        }

        Ok(())
    }
}
