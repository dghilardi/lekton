# RAG: duplicate attachments and multi-document context

Date: 2026-11-17 · Base: 0.43.1 · Source: a chat test round on a production-like
deployment (hybrid search, Vertex reranker, query rewriter and analyzer on; HyDE
off) plus a code and literature review.

## Summary

| Problem | Cause | Decision |
|---|---|---|
| A `.md` attachment shows up as a source next to the page it copies | Old `lekton-sync` builds uploaded linked `.md` files as attachments | Upgrade the sync clients, clean up the orphans, then add server-side guards (below) |
| Questions spanning many documents get a partial answer | Final context of 5 chunks, at most 2 per document | Defaults raised to 10 chunks and a 20,000-character prompt budget; further steps below, to be driven by an eval set |
| A paraphrased question misses the page that answers it | Vocabulary gap, not context size | Synonyms first, contextual retrieval only if the eval set shows it is common |

## 1. Attachments that duplicate pages

### What happens

A page links to a sibling Markdown file (`[Commit guidelines](./commit-guidelines.md)`).
The intended behaviour is a link to the other page: `lekton-sync` skips `.md`,
`.mdx` and `.markdown` links when collecting attachments (since 0.24.14), and the
server resolves them to page links.

Sync clients older than 0.24.14 upload the file instead and rewrite the link to
`/api/v1/assets/attachments/<slug>/<file>.md`. The attachment is then indexed
like any other text attachment, so the chat can retrieve both the page and its
copy. The copy is usually an older revision of the page, so the two do not chunk
identically and the exact-text passage dedup does not merge them (in the
observed case about 75% of the attachment's 5-word shingles were in the page).

On the observed deployment there were 50 Markdown attachments: 28 no longer
referenced by any page (uploaded months ago, before the fix) and 22 still
referenced, uploaded as recently as late October by a sync pipeline that most
likely still runs an old client. The bug is fixed in the code; the data keeps
being produced until those pipelines upgrade.

An upgraded client leaves the `.md` links alone, so its output differs from the
stored content: the first sync after the upgrade re-ingests the affected pages
with page links.

### Plan

1. **Upgrade the sync clients** in every pipeline to a current `lekton-sync`.
   This stops new copies and, on the next sync, turns the links back into page links.
2. **Clean up** the Markdown attachments no longer referenced by any page (after
   step 1, this includes the 22). Their chunks leave the RAG index with the
   reindex's orphan sweep.
3. **Server-side guard at ingest**: reject (or ignore) attachment uploads whose
   key ends in `.md`/`.mdx`/`.markdown` when the request comes from the sync
   API, and log the client so stale pipelines are visible. A Markdown file
   carrying Lekton front matter (`lektonImport`) is a page source by definition.
4. **Consistency report finding** `markdown_attachments`: list Markdown
   attachments and whether a page references them, so the clean-up can be
   repeated and the problem cannot come back silently.

### If copies must stay (not needed today)

Only relevant if Markdown attachments become a legitimate use case:

- **Query-time near-duplicate collapse.** Over the ≤50 candidates, compare
  5-word shingles and treat two sources as one logical document when containment
  (`|A∩B| / min(|A|,|B|)`) is high on at least two chunk pairs; prefer the page
  over the attachment. Shingling with resemblance/containment is the classic
  near-duplicate technique (Broder, 1997); MinHash/LSH only approximate it for
  large collections ([Lee et al. 2021](https://arxiv.org/abs/2107.06499)). With 50
  candidates the exact comparison costs microseconds. The threshold must be
  calibrated: a stale copy scored ~0.75, below the 0.8 often suggested.
- **Ingest-time canonicalisation**: mark a text attachment `duplicate_of: <slug>`
  when it is a near-duplicate of a page with equal or wider visibility, and
  filter it at retrieval (the LlamaIndex ingestion pipeline's docstore dedup is
  the usual shape of this:
  [docs](https://developers.llamaindex.ai/python/framework/module_guides/loading/ingestion_pipeline/)).
- Rejected: embedding cosine thresholds
  ([SemDeDup](https://arxiv.org/abs/2303.09540) style; fragile, and different
  embedding prefixes for pages and attachments move the vectors) and Qdrant MMR
  (it acts on the vector branch only, before fusion and reranking).

## 2. Questions that span several documents

### Diagnosis

- The final context was 5 chunks with a per-document cap of
  `max(2, ceil(limit/3))` = 2, so an answer saw at most 5 documents and often 3.
  The 12,000-character prompt budget was about half used (chunks are ~256 tokens).
- Every slot is filled, so a weakly related chunk can take one.
- Observed answers to "recurring problems across past incidents" and "all
  steps to take a new service to production" listed fragments instead of
  synthesising; one answered honestly that no overview exists.

### Done in this change

- `rag.context_chunk_limit` default 5 → 10 (the per-document cap becomes 4).
- Context prompt budget 12,000 → 20,000 characters, enough for 10 chunks with
  headers.
- Deployments that set `context_chunk_limit` explicitly keep their value and
  must raise it themselves.

Evidence and cost: Anthropic's contextual retrieval study passes the top-20
chunks to the model ([post](https://www.anthropic.com/news/contextual-retrieval));
"Lost in the Middle" ([Liu et al. 2023](https://arxiv.org/abs/2307.03172)) shows
that information in the middle of a long context is used less well, so ordering
matters (the best chunks already come first). Cost is roughly +5k input tokens
per turn and no extra calls.

### Next steps, in order

0. **Eval set first.** 30–50 questions with expected documents, split into
   lookup, paraphrase, multi-document and page/attachment cases; baseline with
   `rag-eval` (recall@k, MRR, nDCG@k already exist). Add: distinct expected
   documents present in the final context, duplicate-source rate, off-topic
   chunk rate, context tokens and latency. For answers, pairwise LLM-as-judge
   before/after with randomised order, plus a manual check on a sample.
1. **Lower per-document cap** (2 instead of 4 at 10 chunks), if the eval set
   shows a single document still crowding multi-document answers.
2. **Route by question type.** Add a broad/aggregate class to the existing
   analyzer: broad questions get 12–15 chunks with 1–2 per document and a larger
   character budget; lookups keep the default. No extra LLM call. Adaptive-k
   ([arXiv 2506.08479](https://arxiv.org/abs/2506.08479)) reports that methods
   tuned for factoid QA do poorly on aggregation QA, where the right context
   size varies.
3. **Adaptive cut-off on reranker scores**: keep chunks up to the largest score
   gap (or while `score ≥ α·top`), minimum 3, so off-topic chunks do not get a
   slot. Only meaningful on reranker scores, not RRF scores. Adaptive-k reports
   matching fixed-k baselines with far fewer tokens.
4. **Defer** map-reduce answering, RAPTOR ([arXiv 2401.18059](https://arxiv.org/abs/2401.18059))
   and GraphRAG ([arXiv 2404.16130](https://arxiv.org/abs/2404.16130)) until
   steps 1–3 are measured: they need LLM-heavy ingest or many calls per answer.
   A maintained summary page (e.g. incident lessons learned) is a cheaper way to
   answer "across all X" questions.

## 3. Paraphrased questions

"The service loses the connection to the messaging system and stops retrying"
retrieved the message-retry settings but not the reconnection settings, which
use different words. Levers, cheapest first:

1. Domain synonyms in Meilisearch (e.g. messaging ↔ broker name,
   reconnect ↔ reconnection).
2. HyDE ([Gao et al. 2022](https://arxiv.org/abs/2212.10496)), already
   implemented and off on the observed deployment: one extra LLM call per turn.
3. Contextual retrieval at ingest: an LLM-written context prefix per chunk, for
   both embeddings and the lexical index. Anthropic reports 35% fewer retrieval
   failures (recall@20) with contextual embeddings, 49% adding contextual BM25
   and 67% with reranking, on their mixed benchmark. It needs a full reindex and
   one LLM call per chunk; the current `Title > Section` prefix is a partial form.

## Not verified

- The suggested thresholds (containment, α) are starting points, not values from
  the literature.
- Adaptive-k and GraphRAG were read from their abstracts only.
- Whether the sync pipelines producing Markdown attachments run an old
  `lekton-sync` or another client: the server does not record the client version.
