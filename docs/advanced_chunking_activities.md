# Piano di miglioramento del chunking RAG

Questo documento raccoglie le attività proposte per migliorare la fase di chunking della pipeline RAG di Lekton, ordinate per rapporto impatto/sforzo. Estende e razionalizza la proposta di issue [#6](https://github.com/dghilardi/lekton/issues/6) e ENH-004, integrando tecniche state-of-the-art valutate rispetto al caso d'uso (IDP con documentazione tecnica markdown).

## Stato attuale (sintesi)

Già live nel pipeline:
- HyDE (`hyde_url`/`hyde_model`) con fallback grazioso
- Query analyzer con decomposizione in sub-query (`analyzer_url`)
- Cross-encoder reranker (Jina/Infinity/Cohere-compatible)
- Hybrid search con RRF (Qdrant + Meilisearch)
- Query rewriting per follow-up
- Cache embedding su MongoDB

Stato chunking:
- `src/rag/splitter.rs` usa `text_splitter::MarkdownSplitter::new(512)` (caratteri, costante)
- Nessun overlap, nessuna metadata di struttura
- `ChunkPayload` in Qdrant: `chunk_text`, `document_slug`, `document_title`, `access_level`, `is_draft`, `tags`, `chunk_index`
- `embedding_text == display_text`
- Nessun anchor di sezione, nessun parent-document

## Principio guida

Ogni intervento sul chunking richiede un reindex completo. Conviene **bundlare in pochi PR coerenti** gli interventi che cambiano payload o sizing, e tenere separati gli interventi che lavorano a query-time o aggiungono solo metadata opzionali.

---

## Tier 0 — Prerequisito: misurazione

Prima di muovere il chunking, serve un metro per dire se è migliorato. Senza, ogni decisione successiva è opinione.

### 0.1 Mini eval set + harness offline — impatto alto, sforzo basso/medio
Costruire un set di 30-80 coppie (query, doc/sezione attesa) rappresentative dei pattern di query reali (configurazione, troubleshooting, "come si fa X", confronto tra versioni). Aggiungere un comando `cargo run --bin rag-eval` che, dato un eval set, calcola **Recall@k**, **MRR** e **nDCG@10** sul retrieval pre-rerank e post-rerank.

- File nuovi: `src/bin/rag_eval.rs`, `eval/queries.jsonl`
- Riusa `RetrievalPipeline` esistente, non duplica logica
- Permette confronti A/B tra configurazioni di chunking senza dover deployare

### 0.2 Logging strutturato dei round di retrieval — impatto medio, sforzo basso
Loggare per ogni query: `query_text`, `rewritten_query`, `analyzer_classification`, `chunk_ids` recuperati per ogni sub-query, score pre/post-rerank. Già in parte presente, va consolidato e reso filtrabile per session id, così da poter triare casi di fallimento reali.

---

## Tier 1 — Alto impatto, basso costo

### 1.1 Atomicità di code fence e tabelle — impatto alto, sforzo basso
I blocchi ` ``` ` e le tabelle markdown sono la fonte più frequente di chunk inutili in documentazione tecnica: spezzati a metà perdono semantica e formattazione. `text-splitter` li rispetta solo se entrano nella size; va aggiunto un guard che, se un blocco supera la size, lo emette come chunk oversize anziché tagliarlo.

- File: `src/rag/splitter.rs`
- Implementazione: pre-scan dei range di code fence / tabelle, blacklist dei punti di split che cadono al loro interno
- Test: code fence di 3KB, tabella con 50 righe, fence annidate in lista

### 1.2 Token-aware sizing + overlap (unifica ENH-004) — impatto alto, sforzo basso
Convergere su token (cl100k_base via `tiktoken-rs`) come unità di misura, con `chunk_size_tokens` (default 256) e `chunk_overlap_tokens` (default 64) configurabili in `RagConfig`. **Scartare** la modalità a caratteri proposta nel ticket #6 (`chunk_min_chars`/`chunk_max_chars`/`chunk_overlap_chars`): avere due unità di misura è solo debito, e `chunk_min` non serve in pratica (le code di documento corte vanno bene come sono).

- File: `Cargo.toml` (feature `tiktoken-rs` su `text-splitter`), `src/config.rs`, `src/rag/splitter.rs`, `src/rag/service.rs`, `src/rag/reindex.rs`
- Richiede reindex
- Coordinare con 1.3 e 1.4 in un unico PR (stesso reindex)

### 1.3 `SplitChunk` tipato con section metadata — impatto alto, sforzo basso
Sostituire `Vec<String>` con `Vec<SplitChunk>` che porta:
- `text: String`
- `section_path: Vec<String>` — es. `["Architecture", "Storage Layer"]`
- `section_anchor: String` — es. `architecture-storage-layer`
- `char_offset: usize`, `byte_offset: usize`
- `chunk_index: u32`

`block_kind` (paragraph/list/table/code) **rimandato**: aggiunge complessità senza un consumer concreto attuale. Si introduce quando serve (filtro UI, rerank-by-block).

- File: `src/rag/splitter.rs`, `src/rag/vectorstore.rs` (estendere `ChunkPayload`)
- Abilita ENH-001 al livello sezione "gratis"

### 1.4 `embedding_text` ≠ `display_text` — impatto alto, sforzo basso
Per ogni chunk, costruire un `embedding_text` arricchito con `Document title > H2 > H3\n\n{chunk}`. Il `display_text` resta pulito per l'iniezione nel prompt e per l'UI. Cambiamento minimo nel codice, beneficio sostanziale sul recall di chunk altrimenti ambigui (un chunk dentro "Configuration" senza heading è poco distinguibile da uno dentro "Deployment").

- File: `src/rag/service.rs` (alimentazione embedder), `src/rag/vectorstore.rs` (separare i due campi nel payload, oppure tenere solo `display_text` e ricostruire l'embedding al volo)
- Mantenere solo `display_text` in payload; l'`embedding_text` non serve a query-time

### 1.5 Two-pass heading-aware splitter — impatto medio-alto, sforzo basso-medio
Primo pass: tagliare per heading di livello configurabile (default H1+H2). Secondo pass: applicare il `MarkdownSplitter` token-based solo alle sezioni che superano `chunk_size_tokens`. **Aggiungere merge** di sezioni adiacenti più piccole di una soglia (es. `min_section_tokens = 64`), altrimenti documenti con molte H3 corte producono chunk inutili.

- File: `src/rag/splitter.rs`
- Coordinare con 1.3 (le sezioni nutrono `section_path`/`section_anchor`)

> **Bundle suggerito per il primo PR di chunking**: 1.1 + 1.2 + 1.3 + 1.4 + 1.5 in un unico PR coerente, un solo reindex, una sola riga di CHANGELOG.

---

## Tier 2 — Alto impatto, costo medio

### 2.1 Source references a livello di sezione (riallineamento ENH-001) — impatto alto, sforzo medio
Una volta presenti `section_anchor` e `section_path` nel payload, `ENH-001` può emettere citazioni come `slug#section-anchor` con titolo della sezione, anziché solo del documento. Da rivedere la spec di ENH-001 per consumare i nuovi metadata invece di tornare al solo titolo del documento.

- File: spec ENH-001, `src/rag/chat.rs` (`ChatEvent::Sources`), modello `SourceReference`
- Frontend: linkare `#anchor` per scroll-to-section

### 2.2 Parent-document / auto-merging retrieval — impatto alto, sforzo medio
Embeddare chunk piccoli (256 token) per precision, ma in fase di context-building per l'LLM restituire il **parent** (la sezione intera, o l'unione dei chunk fratelli) per dare contesto. Ben combinato con il reranker già presente: si rerank-a sui chunk piccoli, si espande al parent solo per il top-K finale.

- Richiede: chunk con `parent_id` (la sezione di appartenenza) in payload
- File: `src/rag/vectorstore.rs`, `src/rag/chat.rs` (espansione post-rerank)
- Tradeoff: aumenta i token nel context dell'LLM. Configurabile via `expand_to_parent: bool` o budget di token

### 2.3 Configurabilità della ricetta di chunking per `access_level` o `tags` — impatto medio, sforzo basso-medio
Diversi tipi di documento meritano sizing diversi: API reference (chunk piccoli, alta precision), tutorial narrativi (chunk grandi, contesto narrativo). Permettere override per tag o cartella.

- File: `RagConfig` (mappa `tag -> ChunkConfig`), `src/rag/service.rs`
- Da fare solo se l'eval mostra che una single config non funziona per tutti i corpus

---

## Tier 3 — Alto impatto, costo maggiore

### 3.1 Contextual Retrieval (Anthropic, sett. 2024) — impatto alto, costo medio-alto
Per ogni chunk, una chiamata LLM "small" genera 50-100 token di contesto situazionale ("questa sezione descrive il fallback dello storage layer in caso di indisponibilità di S3"), che vengono prependuti all'`embedding_text`. Studio Anthropic: -35% / -49% sul tasso di fallimento del retrieval. L'infrastruttura è già pronta: si aggiunge un `context_url`/`context_model` analogo a `hyde_url`/`analyzer_url`, e si fa il lavoro a indicizzazione (costo una tantum per documento).

- File: nuovo `src/rag/contextualizer.rs`, `RagConfig`, hook in `src/rag/service.rs` durante l'ingestion
- Tradeoff: latenza/costo di ingestion aumentano (1 chiamata LLM per chunk). Mitigabile con batch e con un modello piccolo locale
- Ortogonale agli altri Tier — può essere abilitato/disabilitato via config

### 3.2 Multi-representation indexing (HyDE invertito) — impatto alto, costo medio
A indicizzazione, generare 3-5 domande ipotetiche per chunk con un LLM e embeddarle puntando allo stesso `chunk_id`. Stesso effetto di HyDE ma il costo è pagato una volta in ingestion, non per ogni query. Può **sostituire** HyDE a query-time per alcuni profili di query, riducendo la latenza chat.

- File: `src/rag/representation.rs` (nuovo), `src/rag/vectorstore.rs` (più embedding per chunk_id)
- Tradeoff: storage Qdrant ×3-5; deduplicare per chunk_id in fase di retrieval
- Da valutare contro 3.1: se Contextual Retrieval già satura il guadagno, questo è marginale

### 3.3 Reindex incrementale / versioning del chunking — impatto medio, sforzo medio
Ogni cambiamento di `chunk_size_tokens`, splitter o ricetta di enrichment richiede un full reindex, che diventa costoso al crescere del corpus. Aggiungere un `chunking_version: u32` in `ChunkPayload` e un endpoint che reindexa solo i documenti il cui versioning è disallineato con la config corrente. Permette migrazioni progressive.

- File: `src/rag/reindex.rs`, `src/api/admin.rs`
- Importante quando il corpus supera qualche migliaio di documenti

---

## Tier 4 — Sperimentale, dipende da assunzioni

### 4.1 Late chunking (Jina v3 / voyage-3) — impatto potenzialmente alto, costo alto
Embeddare l'intero documento con un embedder long-context (8k+) e poi derivare gli embedding dei chunk via mean-pooling sui token di ciascuna sezione. Risolve in modo elegante il problema del contesto perso al confine dei chunk, ma richiede di cambiare embedder e impatta tutto il pipeline. **Da considerare solo quando si valuta una migrazione dell'embedder**, non come progetto a sé.

### 4.2 Proposition-based chunking — impatto incerto, costo alto
Decomporre il testo in proposizioni atomiche (claim self-contained) tramite LLM ed embeddare quelle. Ottimo per documenti densi di fatti (FAQ, troubleshooting), molto meno per narrativi/architetturali. Da valutare solo se l'eval del Tier 0 mostra un fallimento sistematico su query factoid.

### 4.3 RAPTOR / hierarchical summarization — impatto incerto, costo alto
Costruire un albero di riassunti (chunk → riassunto sezione → riassunto documento → riassunto cluster di documenti) ed embeddare tutti i livelli. Dà boost su query "panoramiche" cross-documento, ma è infrastruttura significativa (clustering, ricostruzione albero su update). Probabilmente **non vale l'investimento** finché il corpus IDP non supera centinaia di documenti correlati.

### 4.4 Embedding multi-vector (ColBERT-style) — impatto incerto, costo molto alto
Mantenere un embedding per token anziché per chunk. Recupero molto preciso ma storage e infra completamente diversi (Qdrant non supporta nativamente, serve Vespa o estensioni). **Sconsigliato** per Lekton allo stato attuale.

---

## Cosa NON fare (esplicito)

- **`chunk_min_chars`** dal ticket #6: non risolve un problema reale e introduce join innaturali tra chunk eterogenei.
- **Doppia modalità char-based + token-based**: scegliere token e basta.
- **`block_kind` upfront**: aggiungerlo solo quando un consumer lo richiede (rischio di metadata non usato per sempre).
- **Migrare a embedder long-context "perché fa figo"**: il guadagno reale arriva solo se 4.1 viene implementato; altrimenti è solo costo.
- **Implementare 3.1 e 3.2 insieme al primo round**: vanno valutati uno alla volta sull'eval set, altrimenti non si capisce chi sposta cosa.

---

## Sequenza consigliata

1. **Tier 0** (0.1, 0.2) — prerequisito misurazione, sblocca tutto il resto.
2. **Tier 1 bundle** (1.1 + 1.2 + 1.3 + 1.4 + 1.5) — un PR, un reindex, baseline nuova.
3. Misurare con l'eval set: confermare il guadagno e congelare la baseline.
4. **Tier 2.1** (riallineamento ENH-001) — sfrutta i metadata appena introdotti.
5. **Tier 2.2** (parent-document) — misurare delta vs Tier 1.
6. **Tier 3.1** (Contextual Retrieval) — il candidato a maggior impatto residuo, da abilitare via config e A/B sull'eval.
7. **Tier 3.3** (reindex incrementale) — quando il corpus rende doloroso il full reindex.
8. **Tier 3.2** (multi-representation) — solo se 3.1 non satura.
9. **Tier 4** — solo se l'eval evidenzia gap che i Tier 1-3 non hanno chiuso.

A ogni step, registrare nel CHANGELOG la metrica di riferimento (Recall@10, MRR) prima/dopo, così da avere una storia tracciabile della qualità del retrieval.
