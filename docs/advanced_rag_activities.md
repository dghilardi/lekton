Attività raccomandate, in ordine di priorità

### 1. Hybrid Search con RRF — impatto alto, sforzo basso
Già avete Meilisearch **e** Qdrant, ma i risultati vengono usati in modo separato. Fondere i risultati di entrambi con **Reciprocal Rank Fusion** nel chat RAG darebbe subito una qualità migliore, combinando recall lessicale (full-text) con recall semantico (vettori). È il gain più facile perché l'infrastruttura esiste già.

### 2. Cross-encoder reranker — impatto alto, sforzo medio
Attualmente i chunk vengono restituiti nell'ordine di similarità coseno di Qdrant. Un reranker (`bge-reranker-v2-m3` in locale via Ollama) riordinerebbe i top-N chunk in base alla rilevanza query-documento effettiva, migliorando sensibilmente le risposte, specie per query tecniche.

### 3. Query Decomposition (Task 2) — impatto alto per casi d'uso IDP
Per un IDP con query tipo "confronta la config MQTT del device A e B" o "quale firmware ha introdotto la funzionalità X?", la decomposizione in sub-query parallele è molto rilevante. Il `query_rewriter.rs` esistente è una buona base da estendere.

### 4. HyDE — impatto medio, sforzo basso
Le query degli sviluppatori ("come configuro X?") hanno vocabolario diverso dalla documentazione tecnica ("Configurazione di X"). HyDE colmerebbe questo gap generando un documento ipotetico e usandone l'embedding. Semplice da aggiungere come flag opzionale.

---

**Sconsiglio per ora** Task 3 (multi-hop) e Task 5 (observability): il primo richiede un dominio documentale con dipendenze semantiche tra documenti (da verificare), il secondo è importante ma non migliora la qualità delle risposte.

Vuoi procedere con una di queste? Partirei da **Hybrid Search + RRF** perché sfrutta infrastruttura già presente.
