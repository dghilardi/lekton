# Lekton — Piano d'Azione: Sicurezza, Qualità e Documentazione

> Generato il 2026-03-06 dall'analisi completa del codebase (branch `next`).
> Ogni task include file coinvolti, descrizione del problema e intervento richiesto.

---

## Priorità 1 — Critiche (pre-produzione)

### 1.1 Rendere JWT_SECRET obbligatorio in produzione

**Problema:** `src/main.rs:107-113` — se `JWT_SECRET` non è impostato, il server parte
con la chiave hardcoded `"dev-insecure-secret-change-in-production!!"`. Chiunque conosca
questa stringa può forgiare JWT e impersonare qualsiasi utente, incluso admin.

**Intervento:**
- In modalità non-demo, richiedere `JWT_SECRET` come variabile d'ambiente obbligatoria.
- Se manca, il server deve rifiutarsi di partire con un messaggio chiaro.
- Mantenere il fallback insicuro solo se `DEMO_MODE=true`.

**File:** `src/main.rs`

---

### 1.2 Rendere SERVICE_TOKEN obbligatorio in produzione

**Problema:** `src/main.rs:103-104` — `SERVICE_TOKEN` ha default `"dev-token"`. Permette
a chiunque di usare gli endpoint di ingest, schema e asset senza autenticazione reale.

**Intervento:**
- Stessa logica di 1.1: obbligatorio se non in demo mode.
- Validare lunghezza minima (es. 32 caratteri) per evitare token deboli.

**File:** `src/main.rs`

---

### 1.3 Bloccare DEMO_MODE in produzione

**Problema:** `src/auth/demo_auth.rs:23-45` — credenziali hardcoded (`admin`/`admin`,
`demo`/`demo`, `public`/`public`). Solo un warning in `src/main.rs:33-39`, nessun
enforcement.

**Intervento:**
- Se `DEMO_MODE=true` e non esiste `ALLOW_DEMO_IN_PRODUCTION=true`, il server deve
  rifiutarsi di partire.
- Loggare un warning aggiuntivo se entrambe le variabili sono impostate.

**File:** `src/main.rs`

---

### 1.4 Aggiungere flag `Secure` a tutti i cookie

**Problema:** Nessun cookie di autenticazione ha il flag `Secure`. I token vengono
trasmessi anche su connessioni HTTP non cifrate.

**Cookie coinvolti:**

| Cookie | File | Righe |
|--------|------|-------|
| Access token | `src/auth/extractor.rs` | 98-105 |
| Refresh token | `src/auth/extractor.rs` | 109-115 |
| Auth state (CSRF) | `src/auth/extractor.rs` | 119-126 |
| Demo session | `src/auth/demo_auth.rs` | 89-93 |

**Intervento:**
- Aggiungere `.secure(true)` a tutti i cookie builder.
- Considerare un flag di configurazione `INSECURE_COOKIES=true` per sviluppo locale
  su HTTP (default: `false`).

---

### 1.5 Aggiornare versione Cargo.toml

**Problema:** `Cargo.toml` dichiara `version = "0.1.0"` ma il CHANGELOG è a `0.4.0`.
Mismatch di 3 versioni minor.

**Intervento:**
- Aggiornare `version = "0.4.0"` in `Cargo.toml`.

**File:** `Cargo.toml`

---

### 1.6 Aggiornare IMPLEMENTATION_ROADMAP.md

**Problema:** 8 feature implementate sono ancora marcate come `[ ]` (non completate).

| Feature | Phase |
|---------|-------|
| Nested Sidebar Navigation | 1 |
| Table of Contents | 1 |
| Breadcrumbs | 1 |
| Meilisearch Integration | 2 |
| Interactive Search Modal | 2 |
| OIDC Authentication | 5 |
| Tiptap Editor | 5 |
| RBAC Enforcement | 5 |

**Intervento:**
- Marcare tutte come `[x]`.
- Aggiungere sezione Phase 5 se mancante, documentando auth, admin API e asset registry.

**File:** `docs/IMPLEMENTATION_ROADMAP.md`

---

## Priorità 2 — Alte (breve termine)

### 2.1 Eliminare `.unwrap()` / `.expect()` dal codice di produzione

**Problema:** 12 istanze di panic potenziale in codice non-test.

| File | Righe | Contesto |
|------|-------|----------|
| `src/main.rs` | 42, 55, 81, 246, 249 | Inizializzazione server, env var |
| `src/search/tenant_token.rs` | 94, 95, 138, 147, 162, 180, 183 | Parsing risposta Meilisearch, base64 |
| `src/app.rs` | 258, 263, 267 | Header construction in demo logout |
| `src/app.rs` | 426 | `.expect()` su context Leptos |

**Intervento:**
- Sostituire con `?` operator, `.map_err()`, o `.ok_or_else()`.
- Per `main.rs` startup: usare `Result` e propagare errori.
- Per `tenant_token.rs`: restituire `AppError` invece di panic.
- Per `app.rs` context: usare `.ok()` con fallback o messaggio utente.

---

### 2.2 Aggiungere limiti dimensione upload

**Problema:** Nessun limite su file upload → DoS via disk/memory exhaustion.

| Endpoint | File | Righe |
|----------|------|-------|
| POST `/api/v1/upload-image` | `src/api/upload.rs` | 17-72 |
| POST `/api/v1/assets` (upload) | `src/api/assets.rs` | 44-125 |
| POST `/api/v1/assets` (editor) | `src/api/assets.rs` | 172-215 |

**Intervento:**
- Usare `axum::extract::DefaultBodyLimit` o `tower_http::limit::RequestBodyLimitLayer`.
- Limite suggerito: 50 MB per asset, 10 MB per immagini.
- Restituire `413 Payload Too Large` se superato.

---

### 2.3 Mascherare errori interni nelle risposte API

**Problema:** `src/api/errors.rs:17-25` — messaggi di errore MongoDB e S3 vengono
restituiti al client. Possono rivelare struttura interna (nomi collection, bucket, ecc.).

**Intervento:**
- Restituire messaggi generici al client: `"Internal server error"`.
- Loggare il dettaglio server-side con `tracing::error!`.
- Mantenere i messaggi dettagliati solo per `AppError::BadRequest` e `AppError::Auth`.

**File:** `src/api/errors.rs`

---

### 2.4 Aggiungere rate limiting

**Problema:** Nessun rate limiting su alcun endpoint. Brute force e DoS possibili.

**Intervento:**
- Aggiungere `tower-governor` come dipendenza.
- Applicare rate limiting globale (es. 100 req/min per IP).
- Rate limiting più stretto su endpoint sensibili:
  - `/auth/login`, `/auth/callback` → 10 req/min
  - `/api/v1/ingest` → 30 req/min
  - `/api/v1/upload-image`, `/api/v1/assets` → 20 req/min

**File:** `src/main.rs`, `Cargo.toml`

---

### 2.5 Configurare CORS esplicito

**Problema:** Nessuna policy CORS configurata. Non è chiaro se l'API debba essere
accessibile da altri domini.

**Intervento:**
- Aggiungere `tower_http::cors::CorsLayer` con configurazione esplicita.
- Default: solo same-origin. Configurabile via `CORS_ALLOWED_ORIGINS`.
- Impostare `Access-Control-Allow-Credentials: true` se servono cookie cross-origin.

**File:** `src/main.rs`

---

## Priorità 3 — Medie (medio termine)

### 3.1 Spezzare `app.rs` in moduli

**Problema:** `src/app.rs` è 1315 righe con 10+ componenti, server functions, routing,
AppState e JavaScript inline. Difficile da navigare e mantenere.

**Intervento proposto:**

```
src/
├── app_state.rs          ← AppState, FromRef derives
├── app.rs                ← solo shell(), App (routing), imports
├── components/
│   ├── mod.rs
│   ├── layout.rs         ← Layout, NavigationItem, NavigationTree, RuntimeCustomCss
│   ├── search_modal.rs   ← SearchModal
│   └── user_menu.rs      ← UserMenu
├── pages/
│   ├── mod.rs
│   ├── home.rs           ← HomePage
│   ├── doc.rs            ← DocPage, Breadcrumbs, TableOfContents
│   └── login.rs          ← LoginPage (con JS estratto in public/js/login.js)
└── server/
    ├── mod.rs
    ├── documents.rs      ← server functions per documenti e navigazione
    └── auth.rs           ← server functions per auth (GetCurrentUser, ecc.)
```

---

### 3.2 Estrarre JavaScript inline

**Problema:** `src/app.rs:729-754` — script raw nel `view!` macro per la login page.
Non type-safe, non testabile, non CSP-compliant.

**Intervento:**
- Creare `public/js/login.js` con la logica del form.
- Referenziare con `<script src="/js/login.js"></script>`.
- Oppure: riscrivere come event handler Leptos puro (preferibile).

---

### 3.3 Estrarre mock condivisi per i test

**Problema:** `MockStorage`, `MockAssetRepo` e simili sono duplicati in 4+ file:
- `src/api/schemas.rs:314-419`
- `src/api/assets.rs:414-506`
- `src/api/ingest.rs:147-267`
- `src/db/user_repository.rs:279-434`

**Intervento:**
- Creare `src/test_utils.rs` (gated con `#[cfg(test)]`).
- Spostare tutte le implementazioni mock condivise.
- Importare nei singoli moduli di test.

---

### 3.4 Unificare link extraction

**Problema:** Due implementazioni diverse per estrarre link:
- `src/rendering/links.rs:9-33` — parser pulldown-cmark (robusto, AST-based).
- `src/editor/component.rs:128-147` — split naive su `href="` (fragile, può rompere
  su attributi con virgolette diverse o HTML malformato).

**Intervento:**
- Eliminare la versione naive in `editor/component.rs`.
- Riutilizzare la funzione da `rendering/links.rs` adattandola per accettare
  sia markdown che HTML, oppure usare un parser HTML (es. `scraper` crate).

---

### 3.5 Migliorare path traversal validation

**Problema:** `src/api/assets.rs:59-72` — check su `..` e `/` iniziale, ma manca
una whitelist di caratteri ammessi. Possibili bypass con encoding.

**Intervento:**
- Aggiungere regex whitelist: `^[a-zA-Z0-9._/-]+$`.
- Rifiutare caratteri speciali, unicode, e sequenze encoded.
- Normalizzare il path prima della validazione.

---

### 3.6 Aggiungere verifica firma JWKS per id_token OIDC

**Problema:** `src/auth/provider.rs:13-18` — il commento ammette che l'id_token
viene decodificato senza verificare la firma JWKS. Dipende solo da TLS.

**Intervento:**
- Usare il crate `openidconnect` per verificare la firma con le chiavi JWKS
  scaricate dal discovery endpoint.
- Aggiungere cache delle chiavi JWKS con TTL (es. 1 ora).

---

### 3.7 Rendere atomico ingest + backlink update

**Problema:** `src/api/ingest.rs:100-101` — upsert documento e update backlinks
sono due operazioni separate. Due ingest concorrenti sullo stesso slug possono
causare backlinks inconsistenti.

**Intervento:**
- Usare MongoDB transaction (session) per rendere atomica l'operazione.
- Oppure: usare `findOneAndUpdate` con pipeline di aggregazione.
- Come minimo: aggiungere un lock applicativo per slug.

---

## Priorità 4 — Basse (miglioramenti futuri)

### 4.1 Aggiungere newtype wrapper per ID semantici

**Problema:** Slug, UserId, AccessLevelName, S3Key, ServiceToken sono tutti `String`.
Facile confondere parametri in funzioni con molti argomenti stringa.

**Intervento:**
- Creare newtype: `pub struct Slug(String)`, `pub struct S3Key(String)`, ecc.
- Implementare `From`, `Display`, `AsRef<str>`, `Serialize`/`Deserialize`.
- Aggiungere validazione nel costruttore (`Slug::new()` verifica formato).

---

### 4.2 Standardizzare error handling con `From<T>`

**Problema:** `src/error.rs` ha solo `From<anyhow::Error>`. Mancano conversioni
per `mongodb::error::Error`, `serde_json::Error`, `reqwest::Error`, ecc.

**Intervento:**
- Aggiungere `impl From<T> for AppError` per ogni tipo di errore comune.
- Eliminare i `.map_err(|e| AppError::Database(...))` sparsi nel codice.
- Considerare `thiserror` `#[from]` attribute per generazione automatica.

---

### 4.3 Aggiungere `PartialEq` a `AppError`

**Problema:** Non si possono fare `assert_eq!` su errori nei test.

**Intervento:**
- Derivare `PartialEq` su `AppError`.
- Se contiene tipi non-PartialEq, implementare manualmente confrontando variante e messaggio.

---

### 4.4 Ridurre clone non necessari

**Locazioni principali:**

| File | Righe | Descrizione |
|------|-------|-------------|
| `src/editor/component.rs` | 64-104 | 8 clone su campi `old_doc` — usare destructuring con move |
| `src/api/ingest.rs` | 73-76 | Clone su `links_out` e `parent_slug` evitabili |

**Intervento:**
- Usare `let Document { access_level, service_owner, tags, .. } = old_doc;`
  per muovere i campi invece di clonare.

---

### 4.5 Usare identity reale invece di service token per audit

**Problema:** `src/api/assets.rs:272-281` — il service token del client è usato come
`uploaded_by`. Non distingue tra diversi chiamanti.

**Intervento:**
- Aggiungere header opzionale `X-Uploaded-By` o campo nel body.
- Se l'upload avviene da CI/CD, usare il nome del servizio.
- Se l'upload avviene da editor, usare l'utente autenticato.

---

### 4.6 Considerare `SameSite=Strict` per i cookie

**Problema:** Tutti i cookie usano `SameSite::Lax`. `Strict` offre protezione
CSRF più forte ma può interferire con flussi OAuth (redirect da provider esterno).

**Intervento:**
- Usare `Strict` per access token e refresh token.
- Mantenere `Lax` per auth state cookie (necessario per callback OAuth).
- Documentare la scelta.

---

### 4.7 Aggiornare REQUIREMENTS.md con modelli Phase 5

**Problema:** Il documento non include i modelli dati aggiunti in Phase 5:
- `User` collection
- `AccessLevelEntity` collection
- `UserPermission` embedded document
- `RefreshToken` collection
- `Asset` collection
- `Settings` collection

**Intervento:**
- Aggiungere sezione 5.4+ con schema di ogni nuova collection.
- Aggiornare diagramma relazioni se presente.

**File:** `docs/REQUIREMENTS.md`

---

### 4.8 Documentare Admin API e Demo Mode nel README

**Problema:** Il README non menziona:
- Endpoint admin: `GET/POST/DELETE /api/v1/admin/access-levels`,
  `GET/POST/DELETE /api/v1/admin/user-permissions`
- Modalità demo: `DEMO_MODE=true`, credenziali disponibili, limitazioni

**Intervento:**
- Aggiungere sezione "Admin API" con tabella endpoint.
- Aggiungere sezione "Demo Mode" con istruzioni e warning.

**File:** `README.md`

---

## Riepilogo

| Priorità | Task | Categoria |
|----------|------|-----------|
| **1 — Critiche** | 1.1–1.6 (6 task) | Sicurezza, Documentazione |
| **2 — Alte** | 2.1–2.5 (5 task) | Sicurezza, Robustezza |
| **3 — Medie** | 3.1–3.7 (7 task) | Refactoring, Qualità |
| **4 — Basse** | 4.1–4.8 (8 task) | Manutenibilità, Documentazione |
| **Totale** | **26 task** | |
