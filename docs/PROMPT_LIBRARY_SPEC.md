# Lekton Prompt Library — Specifica Tecnica MVP

> Stato: proposta tecnica
> Data: 2026-04-10
> Obiettivo: introdurre una Prompt Library come nuovo content type in Lekton, versionato in git, pubblicato via CI/CD, visibile via UI e consumabile tramite MCP con controllo accessi coerente con l'RBAC esistente.

---

## 1. Obiettivi

La Prompt Library deve permettere di:

- versionare i prompt in git;
- pubblicarli tramite pipeline CI/CD senza passare dall'editor web;
- renderli consultabili da utenti aziendali tramite UI;
- renderli utilizzabili dagli agent e dai client MCP tramite tool dedicati;
- applicare filtro per `access_level` e supporto a `draft`;
- mantenere storico e idempotenza come per i documenti.

Non è obiettivo dell'MVP:

- eseguire prompt lato server;
- introdurre un motore workflow per approval multi-step;
- supportare policy di permesso separate tra "vedere" e "usare" il prompt;
- fare semantic retrieval avanzato dedicato ai prompt.

---

## 2. Principi Architetturali

### 2.1 Content type separato

I prompt non devono essere modellati come `Document`.

Motivazioni:

- hanno campi specifici non presenti nella documentazione;
- hanno ciclo di vita diverso;
- richiedono tool MCP e UI dedicate;
- in futuro potrebbero avere variabili tipizzate, esempi, compatibilità modello, approval esplicita e analytics d'uso.

La Prompt Library riusa l'infrastruttura di Lekton ma non il modello dati documentale.

### 2.2 Riuso massimo dell'infrastruttura esistente

Si riusano i seguenti pattern già presenti nel progetto:

- hashing dei contenuti e dei metadati;
- idempotenza ingest;
- sync CI/CD basato su hash;
- storage su S3;
- metadata store su MongoDB;
- versioning storico;
- access control per `access_level`;
- enforcement nel server MCP tramite `UserContext`.

### 2.3 Governance esplicita

Ogni prompt deve avere:

- owner esplicito;
- stato esplicito;
- validazione strutturale in ingest;
- review tramite pull request nel repository sorgente;
- divieto di contenere segreti o token hardcoded.

---

## 3. Modello Dati

### 3.1 Entity `Prompt`

Nuova collection MongoDB: `prompts`.

```json
{
  "slug": "engineering/review-rfc",
  "name": "Review RFC",
  "description": "Prompt per revisionare una RFC tecnica",
  "s3_key": "prompts/engineering_review-rfc.yaml",
  "access_level": "architect",
  "status": "active",
  "owner": "platform-team",
  "last_updated": "ISO8601",
  "tags": ["architecture", "review"],
  "variables": [
    {
      "name": "rfc_text",
      "description": "Contenuto della RFC",
      "required": true
    }
  ],
  "content_hash": "sha256:...",
  "metadata_hash": "sha256:...",
  "is_archived": false
}
```

### 3.2 Campi obbligatori MVP

- `slug`
- `name`
- `description`
- `prompt_body`
- `access_level`
- `status`
- `owner`
- `last_updated`
- `tags`
- `content_hash`
- `metadata_hash`
- `is_archived`

Nota: `prompt_body` non deve essere duplicato in MongoDB. Nell'MVP viene salvato nel blob source su S3, come per i documenti. MongoDB contiene metadati e chiavi di lookup.

### 3.3 Campi opzionali MVP

- `variables`
- `examples`
- `target_model`
- `output_style`
- `reviewed_by`
- `approved_at`
- `publish_to_mcp`
- `default_primary`
- `context_cost`

Questi campi possono essere ammessi nel formato sorgente fin da subito, ma l'applicazione non deve dipenderne per il corretto funzionamento dell'MVP.

### 3.3.1 Metadata di esposizione MCP

Alcuni prompt possono essere resi disponibili direttamente come prompt MCP "pubblicati", senza passare per un recupero esplicito via tool.

Campi proposti:

- `publish_to_mcp: bool`
- `default_primary: bool`
- `context_cost: low | medium | high`

Semantica:

- `publish_to_mcp`: il prompt può entrare nel set dei prompt esposti direttamente al client MCP;
- `default_primary`: il prompt è incluso di default nel contesto utente, salvo override utente;
- `context_cost`: stima grossolana del peso contestuale del prompt, usata per warning UX.

Vincoli raccomandati:

- `default_primary = true` implica `publish_to_mcp = true`;
- un prompt non pubblicato non può essere selezionato come primary di default;
- `context_cost` default a `medium` se non valorizzato.

### 3.4 Enum `PromptStatus`

Valori ammessi:

- `draft`
- `active`
- `deprecated`

Regole:

- `draft`: visibile solo a chi ha permessi draft sul relativo access level;
- `active`: visibile agli utenti con `can_read`;
- `deprecated`: visibile come `active`, ma marcato come deprecato in UI e MCP.

### 3.5 Entity `PromptVersion`

Nuova collection MongoDB: `prompt_versions`.

```json
{
  "id": "uuid-v4",
  "slug": "engineering/review-rfc",
  "version": 3,
  "content_hash": "sha256:...",
  "s3_key": "prompts/history/engineering_review-rfc/3.yaml",
  "updated_by": "platform-prompts-token",
  "created_at": "ISO8601"
}
```

Comportamento:

- la prima ingest non crea una versione;
- una nuova versione viene creata quando cambia il contenuto del prompt;
- una modifica solo metadata non crea storico del body, ma aggiorna il record principale e `metadata_hash`.

### 3.6 Entity `UserPromptPreference`

Nuova collection MongoDB: `user_prompt_preferences`.

```json
{
  "id": "uuid-v4",
  "user_id": "references users.id",
  "prompt_slug": "engineering/review-rfc",
  "is_favorite": true,
  "is_hidden": false,
  "created_at": "ISO8601",
  "updated_at": "ISO8601"
}
```

Semantica:

- `is_favorite`: il prompt deve essere incluso nel contesto utente, se visibile e pubblicabile;
- `is_hidden`: il prompt non deve essere incluso tra i primary di default dell'utente;
- le preferenze non modificano RBAC e non possono rendere visibile un prompt non autorizzato.

Nota:

- preferiti e hidden non devono stare nel file versionato in git, perché sono scelte utente e non metadata del prompt.

---

## 4. Formato Sorgente in Git

### 4.1 Directory

Repository sorgente:

```text
prompts/
  engineering/
    review-rfc.yaml
  support/
    summarize-ticket.yaml
```

### 4.2 Formato file raccomandato

Formato raccomandato per l'MVP: YAML.

Motivazioni:

- più adatto del Markdown a dati strutturati;
- semplifica variabili, esempi e campi futuri;
- evita di dover distinguere tra body documentale e metadata frontmatter.

### 4.3 Esempio file prompt

```yaml
slug: engineering/review-rfc
name: Review RFC
description: Prompt per revisionare una RFC tecnica
access_level: architect
status: active
owner: platform-team
tags:
  - architecture
  - review
variables:
  - name: rfc_text
    description: Contenuto completo della RFC
    required: true
publish_to_mcp: true
default_primary: true
context_cost: medium
prompt_body: |
  Agisci come principal engineer.
  Analizza questa RFC e restituisci:
  1. rischi principali
  2. assunzioni deboli
  3. punti non chiari
  4. suggerimenti di miglioramento

  RFC:
  {{rfc_text}}
```

### 4.4 Regole di validazione

L'ingest deve rifiutare il file se:

- manca un campo obbligatorio;
- `slug` non è valido;
- `access_level` non esiste;
- `status` non è uno dei valori ammessi;
- `prompt_body` è vuoto;
- `variables[].name` è duplicato;
- il file contiene chiavi sconosciute se è attiva la modalità strict.

Si raccomanda una modalità `strict` abilitata di default in CI.

---

## 5. RBAC e Visibilità

### 5.1 Strategia MVP

L'MVP riusa esattamente gli `access_level` già presenti per la documentazione.

Questo implica:

- ogni prompt ha un `access_level`;
- la visibilità viene calcolata tramite `UserContext`;
- non viene introdotto un nuovo tipo di permesso.

### 5.2 Policy di lettura

Utente admin:

- vede tutti i prompt, inclusi i draft.

Utente non admin:

- vede prompt `active` e `deprecated` se ha `can_read` per il relativo `access_level`;
- vede prompt `draft` solo se ha `can_read_draft` per il relativo `access_level`.

### 5.3 Policy di scrittura

Nell'MVP non è prevista scrittura via UI.

La scrittura avviene tramite:

- git + PR nel repository sorgente;
- pipeline CI/CD;
- token di servizio con scope dedicati ai prompt.

### 5.4 Possibili estensioni future

Non incluse nell'MVP:

- `can_use_prompt`;
- `can_manage_prompt`;
- approval esplicita a livello applicativo;
- visibilità per gruppo oltre al solo `access_level`.

### 5.5 Preferenze utente e composizione del contesto

Il contesto utente per i prompt pubblicati deve essere calcolato come unione di:

- prompt con `publish_to_mcp = true` e `default_primary = true`, non nascosti dall'utente;
- prompt con `publish_to_mcp = true` e `is_favorite = true` nelle preferenze utente.

Regole:

- deduplica per `slug`;
- filtro sempre per RBAC e stato;
- un prompt `is_hidden = true` deve essere escluso dal set primary di default;
- un prompt preferito continua a essere incluso anche se non è primary di default, purché non venga reso invisibile da RBAC;
- se un prompt perde `publish_to_mcp = true`, non deve più essere incluso nel contesto anche se era favorito.

---

## 6. Persistenza e Repository

### 6.1 Nuovi moduli previsti

Nuovi moduli Rust suggeriti:

- `src/db/prompt_models.rs`
- `src/db/prompt_repository.rs`
- `src/db/prompt_version_repository.rs`

I nomi sono coerenti con le convenzioni già presenti nel repo.

### 6.2 Repository `PromptRepository`

Operazioni minime:

- `create_or_update`
- `find_by_slug`
- `find_by_slug_prefix`
- `list_by_access_levels`
- `search_metadata`
- `set_archived`
- `delete_by_slug` solo se mai richiesto in futuro

### 6.3 Repository `PromptVersionRepository`

Operazioni minime:

- `create`
- `list_by_slug`
- `next_version_number`

### 6.4 Storage su S3

Chiavi raccomandate:

- corrente: `prompts/<slug-normalizzato>.yaml`
- storico: `prompts/history/<slug-normalizzato>/<version>.yaml`

Il blob caricato deve preservare il payload sorgente normalizzato, così da poter recuperare il file completo per `get_prompt`.

---

## 7. Ingest e Sync

### 7.1 Strategia

Si introducono endpoint dedicati, separati da quelli documentali:

- `POST /api/v1/prompts/ingest`
- `POST /api/v1/prompts/sync`

Motivazioni:

- evita branching complesso negli endpoint documentali;
- mantiene chiaro il dominio;
- riduce rischio di regressione sulla documentazione.

### 7.2 Request `PromptIngestRequest`

```json
{
  "service_token": "raw-token",
  "slug": "engineering/review-rfc",
  "name": "Review RFC",
  "description": "Prompt per revisionare una RFC tecnica",
  "prompt_body": "Agisci come principal engineer...",
  "access_level": "architect",
  "status": "active",
  "owner": "platform-team",
  "tags": ["architecture", "review"],
  "variables": [
    {
      "name": "rfc_text",
      "description": "Contenuto RFC",
      "required": true
    }
  ]
}
```

### 7.3 Comportamento ingest

Il flusso deve essere equivalente a `process_ingest` dei documenti:

1. valida il token di servizio;
2. valida lo slug;
3. valida `access_level`;
4. valida `status`;
5. calcola `content_hash` sul `prompt_body`;
6. calcola `metadata_hash` sui metadati strutturali;
7. confronta con l'eventuale prompt esistente;
8. se nulla è cambiato, restituisce `changed = false`;
9. se cambia il body, archivia il blob precedente in history e crea `PromptVersion`;
10. salva il blob corrente su S3;
11. aggiorna il record in MongoDB;
12. aggiorna eventuale indice di ricerca metadata/full-text.

### 7.4 Request `PromptSyncRequest`

```json
{
  "service_token": "raw-token",
  "prompts": [
    {
      "slug": "engineering/review-rfc",
      "content_hash": "sha256:...",
      "metadata_hash": "sha256:..."
    }
  ],
  "archive_missing": true
}
```

### 7.5 Comportamento sync

Identico pattern del sync documentale:

- ritorna `to_upload`;
- ritorna `to_archive`;
- ritorna `unchanged`;
- può archiviare automaticamente gli slug mancanti dallo scope del token.

### 7.6 Scope token

L'MVP deve poter riusare `ServiceTokenRepository` e il modello di scope esistente.

Esempi:

- `prompts/*`
- `prompts/engineering/*`
- `prompts/support/*`

Raccomandazione:

- mantenere namespace slug coerente per distinguere chiaramente prompt e documenti;
- usare scope separati per evitare che un token documentale possa pubblicare prompt per errore.

---

## 8. Ricerca

### 8.1 MVP

Ricerca prompt iniziale:

- metadata search su nome, descrizione, tag, slug;
- opzionalmente full-text sul `prompt_body`.

Non è necessario introdurre embeddings dedicati nell'MVP.

### 8.2 Post-MVP

Possibili estensioni:

- semantic search solo sui prompt;
- ranking basato su uso/feedback;
- filtri per `target_model`, owner, stato.

---

## 9. MCP

### 9.1 Obiettivo

Il server MCP deve esporre i prompt come strumenti dedicati e separati dai documenti.

### 9.2 Tool MCP MVP

- `list_prompts`
- `get_prompt`
- `search_prompts`
- `get_context_prompts`

### 9.3 Tool `list_prompts`

Restituisce un elenco leggero dei prompt visibili all'utente:

- `slug`
- `name`
- `description`
- `tags`
- `access_level`
- `status`
- `owner`

### 9.4 Tool `get_prompt`

Input:

- `slug`

Output:

- tutti i metadati del prompt;
- `prompt_body`;
- eventuali `variables`.

### 9.5 Tool `search_prompts`

Input:

- `query`
- `limit`

Output:

- prompt rilevanti per slug, name, description, tag e opzionalmente body.

### 9.6 Enforcement

I tool MCP devono:

- recuperare `UserContext` dal request context, come già avviene per i documenti;
- filtrare risultati in base a `access_level` e `status`;
- non esporre slug non autorizzati.

### 9.6.1 Prompt pubblicati nel contesto

Oltre ai tool di libreria, il sistema può esporre un sottoinsieme di prompt direttamente come "prompt contestuali" del client MCP.

Questo sottoinsieme non è un dominio distinto: è una modalità di esposizione di `Prompt`.

Pipeline logica:

1. il prompt viene definito e versionato in git;
2. `publish_to_mcp` stabilisce se è eleggibile alla pubblicazione contestuale;
3. `default_primary` stabilisce se entra di default nel contesto;
4. le preferenze utente (`is_favorite`, `is_hidden`) rifiniscono il set finale;
5. RBAC filtra il risultato finale.

### 9.6.2 Tool `get_context_prompts`

Tool suggerito per costruire il contesto utente effettivo.

Output:

- elenco dei prompt da includere nel contesto;
- motivo di inclusione, ad esempio `default_primary` o `favorite`;
- stima del peso contestuale complessivo;
- eventuali warning.

Esempio di output:

```json
{
  "prompts": [
    {
      "slug": "engineering/review-rfc",
      "reason": "default_primary",
      "context_cost": "medium"
    },
    {
      "slug": "engineering/code-review",
      "reason": "favorite",
      "context_cost": "low"
    }
  ],
  "estimated_context_cost": "medium",
  "warnings": [
    "Selected prompts may add significant context overhead"
  ]
}
```

### 9.7 Estensione futura `render_prompt`

Non inclusa nell'MVP, ma compatibile col modello proposto.

Input futuro:

- `slug`
- `variables`

Output futuro:

- `rendered_prompt`
- lista placeholder mancanti

Per l'MVP è sufficiente esporre il body raw.

---

## 10. UI

### 10.1 Nuova sezione applicativa

Nuova area UI: `Prompt Library`.

Percorso suggerito:

- pagina lista: `/prompts`
- pagina dettaglio: `/prompts/:slug`

### 10.2 Vista lista

Funzionalità MVP:

- ricerca testuale;
- filtro per `access_level`;
- filtro per `status`;
- elenco compatto con nome, descrizione, tag e owner;
- badge per `deprecated` e `draft`.

### 10.3 Vista dettaglio

Funzionalità MVP:

- metadati principali;
- body del prompt;
- elenco variabili;
- azione "copy";
- warning visuale se `deprecated`;
- eventuale link a storico versioni in fase successiva.

### 10.4 Scrittura via UI

Fuori scope MVP.

L'interfaccia deve essere read-only, salvo future estensioni.

### 10.5 Preferenze utente

La UI deve permettere almeno:

- aggiungere/rimuovere un prompt dai preferiti;
- nascondere/mostrare un prompt primary di default;
- visualizzare se un prompt è `published`, `default primary` o solo `library`.

Pagina o pannello suggerito:

- sezione "Context Prompts" nel profilo utente oppure nella Prompt Library.

### 10.6 Warning su eccesso di prompt contestuali

La UI deve mostrare un warning se l'utente seleziona troppi prompt contestuali.

Strategia raccomandata:

- soglia soft sul numero, ad esempio 5 prompt;
- soglia soft sul peso totale, calcolato da `context_cost`;
- nessun blocco rigido nell'MVP, ma warning esplicito su possibile degrado della qualità del contesto.

Mappatura iniziale suggerita:

- `low = 1`
- `medium = 2`
- `high = 4`

Con warning, ad esempio:

- totale >= 8: mostra warning;
- totale >= 12: mostra warning più severo.

---

## 11. Validazione e Sicurezza

### 11.1 Regole di contenuto

La pipeline deve impedire, per quanto possibile:

- presenza di segreti hardcoded;
- token API o credenziali nei prompt;
- body vuoti o quasi vuoti;
- placeholder non dichiarati, se la validazione templating è attiva.

### 11.2 Controlli minimi consigliati in CI

- schema validation del file YAML;
- lint dei placeholder;
- check access level esistente rispetto a un catalogo condiviso;
- check duplicati su `slug`.

### 11.3 Audit minimo

Il sistema deve poter rispondere a:

- chi ha pubblicato l'ultima versione;
- quando è stata pubblicata;
- quale versione precedente era attiva.

Nel MVP è sufficiente registrare:

- `updated_by` nelle versioni;
- `last_updated` nel record corrente.

### 11.4 Coerenza delle preferenze utente

Le preferenze utente devono essere trattate come best-effort e auto-riparanti.

Se un prompt:

- viene archiviato;
- perde `publish_to_mcp`;
- non è più visibile per RBAC;

allora può restare nella collection delle preferenze, ma non deve più comparire nel contesto utente effettivo.

---

## 12. Test

### 12.1 Integrazione backend

Da aggiungere test analoghi a quelli documentali:

- ingest di un nuovo prompt;
- ingest invariato con `changed = false`;
- ingest con body modificato crea `PromptVersion`;
- ingest con soli metadata modificati non crea version history del body;
- sync con `to_upload`, `unchanged`, `to_archive`;
- archiviazione dei prompt mancanti;
- enforcement access level;
- enforcement draft.
- composizione del contesto utente con primary + favorites;
- esclusione dei primary nascosti;
- esclusione dei preferiti non più pubblicabili.

### 12.2 MCP

Test minimi:

- `list_prompts` filtra correttamente per utente;
- `get_prompt` nega accesso ai prompt non autorizzati;
- `search_prompts` non restituisce risultati fuori scope.
- `get_context_prompts` include i primary di default non nascosti;
- `get_context_prompts` include i favoriti pubblicabili;
- `get_context_prompts` restituisce warning oltre soglia.

### 12.3 UI

Test minimi:

- lista prompt visibile a utente autenticato;
- prompt non autorizzati assenti;
- dettaglio prompt;
- badge `deprecated`;
- azione copia.

---

## 13. Moduli e File da Introdurre

Struttura suggerita:

```text
src/
  api/
    prompts_ingest.rs
    prompts_sync.rs
  db/
    prompt_models.rs
    prompt_repository.rs
    prompt_version_repository.rs
    user_prompt_preference_repository.rs
  mcp/
    server.rs               # esteso con tool prompt
  pages/
    prompts.rs
    profile.rs              # esteso con preferenze prompt
  components/
    prompt_list.rs
    prompt_detail.rs
    prompt_context_settings.rs
tests/
  test_prompt_ingest.rs
  test_prompt_sync.rs
  test_prompt_mcp.rs
  test_prompt_rbac.rs
  test_prompt_preferences.rs
```

Alternative possibili:

- usare `src/api/prompts.rs` per accorpare ingest e sync;
- usare `src/pages/prompt.rs` e `src/pages/prompts.rs` separati, in linea con le altre route.

---

## 14. Ordine di Implementazione

### Fase 1

- definire `Prompt` e `PromptVersion`;
- aggiungere repository Mongo;
- aggiungere test di persistenza di base.

### Fase 2

- implementare ingest prompt;
- implementare sync prompt;
- aggiungere test hash/versioning/idempotenza.

### Fase 3

- integrare RBAC lato backend;
- aggiungere ricerca metadata/full-text.

### Fase 4

- estendere MCP con i tool prompt;
- aggiungere test di autorizzazione MCP.
- implementare `get_context_prompts`.

### Fase 5

- aggiungere UI Prompt Library;
- aggiungere test e2e minimi.
- aggiungere preferiti, hidden e warning contesto.

---

## 15. Decisioni Aperte

Da confermare prima dell'implementazione:

1. Il repository dei prompt è questo stesso repo o un repo dedicato sincronizzato via CI?
2. I prompt devono essere solo consultabili o anche eseguibili/renderizzabili dalla UI nel breve termine?
3. Lo `status` deve essere completamente gestito via git o deve esistere anche una transizione lato admin UI in futuro?
4. Serve supporto placeholder esplicito nel formato MVP oppure basta `prompt_body` raw?
5. Serve ricerca sul body del prompt fin dal primo rilascio oppure basta metadata search?
6. I prompt pubblicati nel contesto MCP devono essere passati integralmente o come riferimenti risolvibili dal client?
7. Le preferenze utente devono essere sincronizzabili tra ambienti oppure restano locali all'istanza Lekton?

Raccomandazione corrente:

- repo sorgente dedicato oppure cartella dedicata in repo separato dalla documentazione operativa, se il volume crescerà;
- UI read-only nel MVP;
- stato gestito via git;
- supporto `variables` semplice già nel formato;
- ricerca metadata-first, body search opzionale.
- published prompts come sottoinsieme dei prompt di libreria;
- preferenze utente persistite in MongoDB;
- warning su peso contesto, non solo su numero di prompt.

---

## 16. Raccomandazione Finale

La Prompt Library va implementata come nuovo dominio applicativo, non come variante dei documenti.

La soluzione consigliata per l'MVP è:

- formato YAML in git;
- publish via CI/CD;
- endpoint dedicati `prompts/ingest` e `prompts/sync`;
- storage/versioning coerenti con i documenti;
- riuso degli `access_level` esistenti;
- tool MCP dedicati;
- UI read-only con lista e dettaglio.

Questo approccio minimizza il rischio architetturale, evita di sovraccaricare il dominio documentale e lascia spazio a estensioni future senza rompere il modello.
