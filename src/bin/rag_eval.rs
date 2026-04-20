//! Offline retrieval evaluation harness.
//!
//! Reads a JSONL eval set, runs `ChatService::retrieve_only` against an
//! already-indexed Qdrant collection, and reports Recall@k, MRR and nDCG@k for
//! both the pre-rerank and post-rerank candidate sets so the impact of the
//! cross-encoder reranker can be measured directly.
//!
//! Usage:
//!   cargo run --bin rag-eval --features ssr --no-default-features -- \
//!       --queries eval/queries.jsonl [--top-k 10] [--json-output out.json]
//!
//! The eval set is JSON Lines, one record per line:
//!   {"id":"Q1","query":"how do I configure X","expected_doc_slugs":["docs/x"]}
//!
//! Configuration is loaded the same way the server loads it (config files +
//! `LKN__*` env vars). The runner needs `rag.qdrant_url`, `rag.embedding_url`,
//! `rag.embedding_model`, `rag.embedding_dimensions`, and `rag.chat_model` at a
//! minimum; analyzer/HyDE/reranker/Meilisearch are picked up if configured and
//! contribute to the measured numbers.

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
    use std::collections::HashSet;
    use std::fs::File;
    use std::io::{BufRead, BufReader, Write};
    use std::path::PathBuf;

    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    use lekton::auth::models::{AuthenticatedUser, UserContext};
    use lekton::config::AppConfig;
    use lekton::rag::eval::RagEvalContext;
    use lekton::rag::vectorstore::VectorSearchResult;

    /// One record from the JSONL eval set.
    #[derive(Debug, Deserialize)]
    struct EvalQuery {
        #[serde(default)]
        id: Option<String>,
        query: String,
        /// At least one document slug that should appear in the top-k.
        #[serde(default)]
        expected_doc_slugs: Vec<String>,
        /// Optional: specific Qdrant point ids that should appear in the
        /// top-k. Use only when chunk ids are stable across reindexes.
        #[serde(default)]
        expected_chunk_ids: Vec<String>,
    }

    #[derive(Debug, Default, Clone, Serialize)]
    struct Metrics {
        recall_at_k: f64,
        mrr: f64,
        ndcg_at_k: f64,
    }

    #[derive(Debug, Serialize)]
    struct PerQueryReport {
        id: String,
        query: String,
        expected_doc_slugs: Vec<String>,
        expected_chunk_ids: Vec<String>,
        retrieved_pre: Vec<RetrievedChunk>,
        retrieved_post: Vec<RetrievedChunk>,
        metrics_pre: Metrics,
        metrics_post: Metrics,
    }

    #[derive(Debug, Serialize)]
    struct RetrievedChunk {
        rank: usize,
        point_id: String,
        document_slug: String,
        score: f32,
        relevant: bool,
    }

    #[derive(Debug, Serialize)]
    struct RunReport {
        top_k: usize,
        queries: Vec<PerQueryReport>,
        aggregate_pre: Metrics,
        aggregate_post: Metrics,
    }

    struct Args {
        queries: PathBuf,
        top_k: usize,
        json_output: Option<PathBuf>,
    }

    pub async fn run() -> Result<(), String> {
        // Default log filter keeps the metrics readable; users can override via
        // RUST_LOG=lekton::rag=debug to see the full retrieval trace.
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "warn,lekton::rag=info".into()),
            )
            .init();

        let args = parse_args()?;

        // Install the rustls crypto provider once, like main.rs does. Required
        // before any TLS connection is opened (Qdrant, embedding API, etc.).
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let config = AppConfig::load().map_err(|e| format!("config load failed: {e}"))?;
        let ctx = RagEvalContext::from_config(&config)
            .await
            .map_err(|e| format!("RagEvalContext init failed: {e}"))?;

        let queries = read_jsonl(&args.queries)?;
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

    fn parse_args() -> Result<Args, String> {
        let mut queries: Option<PathBuf> = None;
        let mut top_k: usize = 10;
        let mut json_output: Option<PathBuf> = None;

        let mut iter = std::env::args().skip(1);
        while let Some(a) = iter.next() {
            match a.as_str() {
                "--queries" | "-q" => {
                    queries = Some(PathBuf::from(
                        iter.next()
                            .ok_or_else(|| "--queries requires a path".to_string())?,
                    ));
                }
                "--top-k" | "-k" => {
                    let v = iter
                        .next()
                        .ok_or_else(|| "--top-k requires a number".to_string())?;
                    top_k = v.parse().map_err(|e| format!("invalid --top-k: {e}"))?;
                }
                "--json-output" | "-o" => {
                    json_output =
                        Some(PathBuf::from(iter.next().ok_or_else(|| {
                            "--json-output requires a path".to_string()
                        })?));
                }
                "-h" | "--help" => {
                    print_help();
                    std::process::exit(0);
                }
                other => return Err(format!("unknown argument: {other}")),
            }
        }

        let queries = queries.ok_or_else(|| {
            "missing required argument --queries <path/to/queries.jsonl>".to_string()
        })?;
        if top_k == 0 {
            return Err("--top-k must be >= 1".into());
        }
        Ok(Args {
            queries,
            top_k,
            json_output,
        })
    }

    fn print_help() {
        eprintln!(
            "rag-eval — offline retrieval metrics for the Lekton RAG pipeline\n\n\
             USAGE:\n  \
             cargo run --bin rag-eval --features ssr --no-default-features -- \\\n    \
                 --queries <path.jsonl> [--top-k N] [--json-output path.json]\n\n\
             ARGS:\n  \
             --queries, -q     JSONL eval set (required)\n  \
             --top-k, -k       Top-k cutoff for metrics (default: 10)\n  \
             --json-output, -o Write a full JSON report to the given path\n  \
             --help, -h        Print this message\n\n\
             EVAL RECORD (one per line):\n  \
             {{\"id\":\"Q1\",\"query\":\"...\",\"expected_doc_slugs\":[\"docs/x\"]}}"
        );
    }

    fn read_jsonl(path: &PathBuf) -> Result<Vec<EvalQuery>, String> {
        let file = File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        let mut out = Vec::new();
        for (n, line) in BufReader::new(file).lines().enumerate() {
            let line = line.map_err(|e| format!("read error on line {}: {e}", n + 1))?;
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let q: EvalQuery = serde_json::from_str(trimmed)
                .map_err(|e| format!("parse error on line {}: {e}", n + 1))?;
            if q.expected_doc_slugs.is_empty() && q.expected_chunk_ids.is_empty() {
                return Err(format!(
                    "line {}: query must specify at least one of expected_doc_slugs or expected_chunk_ids",
                    n + 1
                ));
            }
            out.push(q);
        }
        Ok(out)
    }

    fn admin_context() -> UserContext {
        UserContext {
            user: AuthenticatedUser {
                user_id: "rag-eval".into(),
                email: "rag-eval@local".into(),
                name: Some("rag-eval".into()),
                is_admin: true,
            },
            permissions: Vec::new(),
        }
    }

    struct ScoredCandidates {
        retrieved: Vec<RetrievedChunk>,
        metrics: Metrics,
    }

    /// Score one candidate list against the expected set. A chunk is relevant
    /// when its `point_id` is in `expected_chunk_ids` OR its `document_slug`
    /// is in `expected_doc_slugs`. Recall is computed at the *slug* level (so
    /// returning two chunks of the same expected doc does not double-count)
    /// when only slugs are given; otherwise it is computed over the union of
    /// expected slugs and ids.
    fn score(
        candidates: &[VectorSearchResult],
        query: &EvalQuery,
        top_k: usize,
    ) -> ScoredCandidates {
        let expected_slugs: HashSet<&str> = query
            .expected_doc_slugs
            .iter()
            .map(|s| s.as_str())
            .collect();
        let expected_ids: HashSet<&str> = query
            .expected_chunk_ids
            .iter()
            .map(|s| s.as_str())
            .collect();

        let topk: Vec<&VectorSearchResult> = candidates.iter().take(top_k).collect();

        let mut retrieved = Vec::with_capacity(topk.len());
        let mut covered_slugs: HashSet<&str> = HashSet::new();
        let mut covered_ids: HashSet<&str> = HashSet::new();
        let mut first_relevant_rank: Option<usize> = None;
        let mut dcg: f64 = 0.0;

        for (i, c) in topk.iter().enumerate() {
            let rank = i + 1;
            let id_match = !c.point_id.is_empty() && expected_ids.contains(c.point_id.as_str());
            let slug_match = expected_slugs.contains(c.document_slug.as_str());
            let relevant = id_match || slug_match;

            if relevant {
                if first_relevant_rank.is_none() {
                    first_relevant_rank = Some(rank);
                }
                if id_match {
                    covered_ids.insert(c.point_id.as_str());
                }
                if slug_match {
                    covered_slugs.insert(c.document_slug.as_str());
                }
                // Standard binary-relevance DCG with log2(rank + 1).
                dcg += 1.0 / ((rank + 1) as f64).log2();
            }

            retrieved.push(RetrievedChunk {
                rank,
                point_id: c.point_id.clone(),
                document_slug: c.document_slug.clone(),
                score: c.score,
                relevant,
            });
        }

        // Recall denominator: number of distinct expected items the eval cares
        // about. Slugs and ids are independent expectations; both contribute.
        let denom = expected_slugs.len() + expected_ids.len();
        let recall = if denom == 0 {
            0.0
        } else {
            (covered_slugs.len() + covered_ids.len()) as f64 / denom as f64
        };

        let mrr = first_relevant_rank.map(|r| 1.0 / r as f64).unwrap_or(0.0);

        // IDCG: optimal ordering puts up to min(denom, top_k) relevant docs at
        // the front. Each contributes 1 / log2(rank + 1).
        let ideal_hits = denom.min(top_k);
        let idcg: f64 = (1..=ideal_hits)
            .map(|r| 1.0 / ((r + 1) as f64).log2())
            .sum();
        let ndcg = if idcg > 0.0 { dcg / idcg } else { 0.0 };

        ScoredCandidates {
            retrieved,
            metrics: Metrics {
                recall_at_k: recall,
                mrr,
                ndcg_at_k: ndcg,
            },
        }
    }

    fn mean_metrics<'a>(it: impl Iterator<Item = &'a Metrics>) -> Metrics {
        let mut count = 0_usize;
        let mut acc = Metrics::default();
        for m in it {
            acc.recall_at_k += m.recall_at_k;
            acc.mrr += m.mrr;
            acc.ndcg_at_k += m.ndcg_at_k;
            count += 1;
        }
        if count == 0 {
            return acc;
        }
        Metrics {
            recall_at_k: acc.recall_at_k / count as f64,
            mrr: acc.mrr / count as f64,
            ndcg_at_k: acc.ndcg_at_k / count as f64,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn vsr(point_id: &str, slug: &str, score: f32) -> VectorSearchResult {
            VectorSearchResult {
                point_id: point_id.into(),
                chunk_text: format!("chunk for {slug}"),
                document_slug: slug.into(),
                document_title: slug.into(),
                score,
            }
        }

        fn query(slugs: &[&str]) -> EvalQuery {
            EvalQuery {
                id: None,
                query: "irrelevant".into(),
                expected_doc_slugs: slugs.iter().map(|s| s.to_string()).collect(),
                expected_chunk_ids: vec![],
            }
        }

        #[test]
        fn perfect_retrieval_yields_full_metrics() {
            let candidates = vec![vsr("p1", "docs/a", 0.9), vsr("p2", "docs/b", 0.8)];
            let q = query(&["docs/a", "docs/b"]);
            let s = score(&candidates, &q, 10);
            assert!((s.metrics.recall_at_k - 1.0).abs() < 1e-9);
            assert!((s.metrics.mrr - 1.0).abs() < 1e-9);
            assert!((s.metrics.ndcg_at_k - 1.0).abs() < 1e-9);
        }

        #[test]
        fn empty_retrieval_yields_zero_metrics() {
            let candidates: Vec<VectorSearchResult> = vec![];
            let q = query(&["docs/a"]);
            let s = score(&candidates, &q, 10);
            assert_eq!(s.metrics.recall_at_k, 0.0);
            assert_eq!(s.metrics.mrr, 0.0);
            assert_eq!(s.metrics.ndcg_at_k, 0.0);
        }

        #[test]
        fn first_relevant_at_rank_two_gives_mrr_half() {
            let candidates = vec![vsr("p0", "docs/x", 0.9), vsr("p1", "docs/a", 0.8)];
            let q = query(&["docs/a"]);
            let s = score(&candidates, &q, 10);
            assert!((s.metrics.mrr - 0.5).abs() < 1e-9);
            assert!((s.metrics.recall_at_k - 1.0).abs() < 1e-9);
        }

        #[test]
        fn top_k_cutoff_is_respected() {
            let candidates = vec![
                vsr("p0", "docs/x", 0.9),
                vsr("p1", "docs/y", 0.8),
                vsr("p2", "docs/a", 0.7),
            ];
            let q = query(&["docs/a"]);
            let s = score(&candidates, &q, 2);
            assert_eq!(s.metrics.recall_at_k, 0.0);
            assert_eq!(s.metrics.mrr, 0.0);
        }

        #[test]
        fn duplicate_slug_hits_do_not_double_count() {
            let candidates = vec![
                vsr("p0", "docs/a", 0.9),
                vsr("p1", "docs/a", 0.8),
                vsr("p2", "docs/b", 0.7),
            ];
            let q = query(&["docs/a", "docs/b"]);
            let s = score(&candidates, &q, 10);
            // Two distinct expected slugs covered: recall = 1.0 (not >1.0).
            assert!((s.metrics.recall_at_k - 1.0).abs() < 1e-9);
        }
    }
}
