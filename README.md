# Lekton

[![License: AGPL](https://img.shields.io/github/license/dghilardi/lekton)](https://opensource.org/licenses/AGPL-3.0)
[![Rust](https://img.shields.io/badge/rust-stable-brightgreen.svg)](https://www.rust-lang.org)

**Lekton** is a high-performance, dynamic Internal Developer Portal (IDP) designed to replace static documentation generators. Built with Rust, it prioritizes speed, granular security (RBAC), and a seamless developer experience.

## 🚀 Vision

Lekton decouples content from code, allowing microservices to push their documentation and API schemas (OpenAPI/AsyncAPI) to a centralized, searchable, and secure hub. No more full site rebuilds for a typo fix.

## ✨ Key Features

-   **Dynamic Ingestion:** CI/CD integration for live documentation updates via the [`lekton-sync`](cli/README.md) CLI.
-   **Granular RBAC:** Server-level Role-Based Access Control for sensitive documents.
-   **Unified Schema Registry:** Support for OpenAPI (Swagger), AsyncAPI, and JSON Schema with versioning.
-   **Ask the Docs (RAG):** Retrieval-augmented chat over your documentation, with hybrid search, HyDE, query rewriting, and cross-encoder reranking (optional, see [RAG setup](#optional-rag-enhancements)).
-   **MCP Server:** Exposes docs, schemas, and prompts as [Model Context Protocol](https://modelcontextprotocol.io/) tools at `/mcp`, authenticated via Personal Access Tokens.
-   **Prompt Library:** Versioned, RBAC-aware prompt registry for sharing reusable LLM prompts across teams.
-   **Blazing Fast:** SSR powered by Leptos and Axum.
-   **Modern UI:** Tailwind CSS v4 and DaisyUI 5 for a rich, responsive design system.
-   **Intelligent Search:** Powered by Meilisearch with tenancy protection.
-   **Interactive Editing:** Integrated WYSIWYG/Markdown editor with link validation.
-   **Runtime Customizable:** Override styles via `public/custom.css` — no recompilation needed.

## 🛠️ Technology Stack

-   **Frontend:** [Leptos](https://leptos.dev/) (Rust, SSR/Hydration)
-   **Backend:** [Axum](https://github.com/tokio-rs/axum) (Rust API)
-   **Styling:** [Tailwind CSS v4](https://tailwindcss.com/) + [DaisyUI 5](https://daisyui.com/)
-   **Build Tool:** [cargo-leptos](https://github.com/leptos-rs/cargo-leptos)
-   **Database:** [MongoDB](https://www.mongodb.com/) (Metadata & RBAC)
-   **Storage:** S3 Compatible (Markdown & Schema artifacts)
-   **Search:** [Meilisearch](https://www.meilisearch.com/)

## 🚀 Getting Started

### Prerequisites

-   [Rust](https://rustup.rs/) (stable toolchain)
-   [cargo-leptos](https://github.com/leptos-rs/cargo-leptos): `cargo install cargo-leptos --locked`
-   [Node.js](https://nodejs.org/) — required for Mermaid diagram assets (`npm ci` before building). **If you only need to check the Rust backend** (no UI assets), you can skip Node.js by disabling the default `mermaid` feature: `cargo check --no-default-features --features ssr`.
-   [Docker](https://www.docker.com/) & [Docker Compose](https://docs.docker.com/compose/)

### Quick Start with Docker Compose (Recommended)

The easiest way to run Lekton with all dependencies:

```bash
# Clone the repository
git clone https://github.com/dghilardi/lekton.git
cd lekton

# Install Node dependencies (DaisyUI, Mermaid)
npm ci

# Start all services (MongoDB, Garage S3, and Lekton)
docker-compose up
```

The application will be available at `http://localhost:3000`.

**What's included:**
- MongoDB 7 for metadata storage
- Garage S3-compatible storage for documents
- Lekton application with demo auth mode
- Automatic initialization of Garage (bucket creation, API keys)

### Development Mode (Cargo + Docker)

For faster development without rebuilding Docker containers, you can run the Rust application with `cargo` while keeping MongoDB and S3 storage in Docker.

#### Quick Setup (Recommended)

Use the setup script to automatically start dependencies and create your `.env` file:

```bash
# The setup script will install npm dependencies, start Docker services, and create .env
./scripts/setup-dev-env.sh
```

This script will:
1. Install Node.js dependencies (DaisyUI for Tailwind CSS and Mermaid rendering)
2. Start MongoDB and Garage S3 in Docker
3. Initialize Garage (create bucket and API keys)
4. Extract credentials automatically
5. Create a `.env` file with the correct configuration

Then just run:
```bash
cargo leptos watch
```

#### Optional RAG Enhancements

The development setup can also enable the new RAG retrieval features introduced in this branch. They are all optional and disabled by default.

**Available options**
- Hybrid search: fuse Qdrant vector results with Meilisearch BM25 via RRF by setting `LKN__RAG__HYBRID_SEARCH_ENABLED=true`
- Cross-encoder reranker: start `infinity` and set `LKN__RAG__RERANKER_URL=http://localhost:7997/rerank`
- Query decomposition: pull a lightweight local model and set `LKN__RAG__ANALYZER_MODEL=phi3:mini`
- HyDE: pull a lightweight local model and set `LKN__RAG__HYDE_MODEL=phi3:mini`
- Query rewriting for follow-up questions: set `LKN__RAG__REWRITE_MODEL=phi3:mini`

**Supporting services**
```bash
# Optional cross-encoder reranker service (~600 MB model download on first start)
docker-compose up -d infinity

# Optional local model for analyzer / HyDE / rewrite
ollama pull phi3:mini
```

The setup script and `.env.example` already include commented examples for these variables, so the shortest path is to uncomment only the blocks you want to try.

#### Evaluating Retrieval Quality

Once the corpus is indexed, the `rag-eval` binary measures how well the retrieval pipeline answers a known set of queries. It calls the same retrieval path used by the chat (`ChatService::retrieve_only`) and reports Recall@k, MRR and nDCG@k for both pre-rerank and post-rerank candidates, so the impact of the cross-encoder reranker is directly visible.

```bash
cargo run --bin rag-eval --features ssr --no-default-features -- \
    --queries eval/queries.jsonl --top-k 10
# Optional JSON dump for diffing across runs:
#   --json-output reports/run-$(date +%s).json
```

The included `eval/queries.jsonl` is a starter set of twelve queries against the demo corpus; replace it with 30-50 queries representative of your production documentation before drawing conclusions. Each record is a single JSON line: `{"id":"Q01","query":"...","expected_doc_slugs":["slug-a"]}`.

Set `RUST_LOG=lekton::rag=debug` to additionally see per-sub-query and pre-rerank chunk ids in the trace, filterable by `session_id` for triaging individual queries.

#### Manual Setup

If you prefer to set up manually:

**1. Install Node.js dependencies**

```bash
# Install DaisyUI, Mermaid, and other frontend dependencies
npm ci
```

**2. Start dependencies only**

```bash
# Start MongoDB and Garage in the background
docker-compose up -d mongodb garage garage-init
```

Wait for `garage-init` to complete (check with `docker-compose logs garage-init`). It will output credentials like:

```
Access Key ID: GK6dcd28a916458f75d62f0720
Secret Access Key: 893fa79f053d67be65237fdc5d2a8521df5dc0a27858f991ffa72c1ba3470291
```

**3. Create a `.env` file**

Create a `.env` file in the project root with these variables:

```bash
# MongoDB Configuration
LKN__DATABASE__URI=mongodb://localhost:27017
LKN__DATABASE__NAME=lekton

# S3 Storage Configuration (use credentials from garage-init output)
LKN__STORAGE__BUCKET=lekton-docs
LKN__STORAGE__ENDPOINT=http://localhost:3900
AWS_ACCESS_KEY_ID=GK6dcd28a916458f75d62f0720
AWS_SECRET_ACCESS_KEY=893fa79f053d67be65237fdc5d2a8521df5dc0a27858f991ffa72c1ba3470291
AWS_REGION=garage

# Service Token for API ingestion
LKN__AUTH__SERVICE_TOKEN=demo-ingest-token

# Enable demo auth mode (bypasses OIDC)
LKN__AUTH__DEMO_MODE=true

# Logging
LKN__SERVER__LOG_FILTER=lekton=info,tower_http=info

# Leptos configuration
LEPTOS_SITE_ADDR=127.0.0.1:3000
```

**Important:** Replace `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` with the actual credentials output by `garage-init`.

**4. Run with cargo**

You have two options:

**Option A: Using environment variables from `.env` manually**
```bash
# Export environment variables
export $(cat .env | xargs)

# Run the development server
cargo leptos watch
```

**Option B: Using a tool like `dotenv-cli` or `just`**

With `dotenv-cli`:
```bash
# Install dotenv-cli
cargo install dotenv-cli

# Run with auto-loaded .env
dotenv cargo leptos watch
```

With `just` (if you have a justfile):
```bash
just dev  # (if configured to load .env)
```

The application will be available at `http://127.0.0.1:3000` with hot-reload enabled.

**5. Stop dependencies when done**

```bash
docker-compose down
```

### Troubleshooting

**Problem: "Can't resolve 'daisyui'" or Tailwind CSS errors**
- Run `npm ci` to install Node.js dependencies
- If that doesn't work, delete `node_modules`, then run `npm ci` again

**Problem: "Mermaid assets are required but node_modules/mermaid is missing"**
- Run `npm ci` before Cargo build, check, or test commands. The `just` recipes do this automatically.

**Problem: "Failed to connect to MongoDB"**
- Ensure MongoDB is running: `docker-compose ps mongodb`
- Check if the port is already in use: `lsof -i :27017`

**Problem: "Failed to initialize S3 client"**
- Check that Garage is running: `docker-compose ps garage`
- Verify credentials in `.env` match the output from `docker-compose logs garage-init`
- Ensure S3_ENDPOINT is set to `http://localhost:3900` (not `https`)

**Problem: Garage init fails or shows errors**
- Remove volumes and restart: `docker-compose down -v && docker-compose up -d mongodb garage garage-init`
- Check Garage logs: `docker-compose logs garage`

**Problem: Port 3000 already in use**
- Check what's using the port: `lsof -i :3000`
- Either stop that process or change `LEPTOS_SITE_ADDR` in `.env` to use a different port (e.g., `127.0.0.1:3001`)

### Running Tests

The project has three test suites. A [`justfile`](./justfile) is provided for convenience — it loads `.env` automatically so you don't need to `source` it manually.

| Suite | What it tests | Requirements |
|---|---|---|
| Unit | Pure logic (no I/O) | None |
| Integration | DB, S3, search via real containers | Docker |
| E2E | Full browser flows via Playwright | Docker + built app |

#### With `just` (recommended)

```bash
# Unit tests only (fast)
just test

# Integration tests (starts testcontainers automatically)
just test-integration

# E2E tests (starts the server on :3000 if not already running)
just test-e2e

# Run a specific spec file or test name
just test-e2e e2e/search.spec.ts
just test-e2e --grep "Ctrl\+K"

# Interactive Playwright UI for debugging e2e tests
just test-e2e-ui

# All suites in sequence
just test-all
```

#### Without `just`

```bash
# Install Node dependencies required by build.rs
npm ci

# Unit tests
cargo test --features ssr --lib

# Integration tests (single-threaded to avoid container conflicts)
cargo test --features ssr --test '*' -- --test-threads=1

# E2E tests — server must be running on :3000 first
source .env
LKN__DATABASE__NAME=lekton_e2e LKN__AUTH__SERVICE_TOKEN=test-token LKN__SERVER__RATE_LIMIT_BURST=1000 LKN__AUTH__DEMO_MODE=true \
    cargo leptos serve &
npx playwright test
```

> **Tip:** Run `just e2e-logs` to inspect the server log if an e2e run fails at startup.

### Formatting

Rust code in this repository is expected to be formatted with `rustfmt` before review or merge.

```bash
# Apply formatting to the whole workspace
just fmt

# Verify formatting locally, matching CI
just fmt-check
```

GitHub Actions runs `cargo fmt --all --check` on pushes and pull requests, so unformatted Rust code will fail CI.

## ⚙️ Configuration

Lekton is configured via environment variables with the `LKN__` prefix and `__` as the nesting separator (e.g. `LKN__DATABASE__URI`). AWS credentials (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_REGION`) use their standard names directly.

For a full reference with all options and defaults, see [`config/default.toml`](config/default.toml) and the inline docs in [`src/config.rs`](src/config.rs). The `.env.example` file at the repo root is a ready-to-use template.

Common variables:

| Variable                        | Description                          | Default                          |
| ------------------------------- | ------------------------------------ | -------------------------------- |
| `LKN__DATABASE__URI`            | MongoDB connection string            | `mongodb://localhost:27017`      |
| `LKN__DATABASE__NAME`           | MongoDB database name                | `lekton`                         |
| `LKN__STORAGE__BUCKET`          | S3 bucket name                       | *(required)*                     |
| `LKN__STORAGE__ENDPOINT`        | Custom S3 endpoint (MinIO, etc.)     | *(AWS default)*                  |
| `AWS_REGION`                    | AWS region                           | *(from AWS config)*              |
| `LKN__AUTH__SERVICE_TOKEN`      | Token for CI/CD ingestion API        | *(optional)*                     |
| `LKN__AUTH__DEMO_MODE`          | Enable built-in demo auth            | `false`                          |
| `LKN__AUTH__CLIENT_ID`          | OAuth2/OIDC client ID                | *(required for auth)*            |
| `LKN__AUTH__CLIENT_SECRET`      | OAuth2/OIDC client secret            | *(required for auth)*            |
| `LKN__AUTH__REDIRECT_URI`       | OAuth2/OIDC callback URI             | *(required for auth)*            |
| `LKN__AUTH__AUTHORIZATION_ENDPOINT` | OIDC issuer / OAuth2 auth endpoint | *(required for auth)*          |
| `LKN__SERVER__LOG_FILTER`       | Log level filter                     | `lekton=info,tower_http=info`    |

## 🎨 Customizability & Theming

Lekton is designed to be **highly customizable without recompilation**. Change colors, fonts, spacing, and more by simply editing a CSS file.

### Runtime Style Injection

Edit `public/custom.css` to override any styles. This file is loaded after the main stylesheet, so your overrides take precedence:

```css
/* Override DaisyUI theme colors */
[data-theme="light"] {
  --p: 210 64% 31%;    /* primary */
  --s: 210 40% 50%;    /* secondary */
}

/* Override Lekton design tokens */
:root {
  --lekton-font-family: "Fira Code", monospace;
  --lekton-sidebar-width: 20rem;
}
```

### Pre-built Themes

Lekton comes with **3 beautiful pre-built themes** in the `examples/themes/` directory:

#### 🌙 GitHub Dark
A developer-friendly dark theme inspired by GitHub's dark mode.
```bash
cp examples/themes/github-dark.css public/custom.css
```
**Features:**
- Dark color scheme perfect for late-night coding
- GitHub-style blue accents and syntax highlighting
- Clean, minimalist design
- Optimized for code readability

#### ❄️ Nord
A beautiful arctic, north-bluish color palette.
```bash
cp examples/themes/nord.css public/custom.css
```
**Features:**
- Carefully selected frost and aurora colors
- Excellent contrast and readability
- Subtle animations and hover effects
- Popular among developers and designers

#### ☀️ Solarized Light
A precision-crafted light theme with warm tones, perfect for documentation.
```bash
cp examples/themes/solarized-light.css public/custom.css
```
**Features:**
- Warm, easy-on-the-eyes color palette
- Serif fonts for a traditional documentation feel
- High readability with scientific color selection
- Ideal for long reading sessions

### Using Themes in Docker

Mount your chosen theme as a volume:

```bash
# Using GitHub Dark theme
docker run -v ./examples/themes/github-dark.css:/app/public/custom.css lekton

# Or with Docker Compose, add to volumes:
volumes:
  - ./examples/themes/nord.css:/app/public/custom.css
```

### Creating Custom Themes

Start with one of the example themes and modify it to match your brand:

1. **Copy an example theme:**
   ```bash
   cp examples/themes/nord.css public/custom.css
   ```

2. **Edit colors using DaisyUI color variables:**
   ```css
   html[data-theme="light"] {
     --p: 220 90% 56%;     /* Primary color */
     --s: 174 60% 51%;     /* Secondary color */
     --a: 36 100% 50%;     /* Accent color */
   }
   ```

3. **Customize Lekton-specific tokens:**
   ```css
   :root {
     --lekton-font-family: "Your Font", sans-serif;
     --lekton-sidebar-width: 18rem;
     --lekton-content-max-width: 80rem;
   }
   ```

4. **Reload the page** (no compilation needed!)

### Theme Customization Reference

**DaisyUI Color Variables:**
- `--p` / `--pf` / `--pc` - Primary color (and focus/content variants)
- `--s` / `--sf` / `--sc` - Secondary color
- `--a` / `--af` / `--ac` - Accent color
- `--b1` / `--b2` / `--b3` - Background colors (base)
- `--bc` - Base content (text color)
- `--in` / `--su` / `--wa` / `--er` - Info, Success, Warning, Error

**Lekton Design Tokens:**
- `--lekton-font-family` - Main font stack
- `--lekton-sidebar-width` - Sidebar width
- `--lekton-content-max-width` - Maximum content width
- `--lekton-spacing-*` - Spacing scale (xs, sm, md, lg, xl)

For more details, see [DaisyUI Themes Documentation](https://daisyui.com/docs/themes/).

## API

Most write endpoints are driven by the [`lekton-sync`](cli/README.md) CLI in CI/CD; you rarely call them by hand. Endpoints below are the stable `v1` surface.

### Ingestion (service token)

| Method | Endpoint | Description |
|--------|----------|-------------|
| `POST` | `/api/v1/ingest` | Create/update a document |
| `POST` | `/api/v1/sync` | Compute document delta (which docs need re-upload) |
| `POST` | `/api/v1/schemas` | Create/update a schema |
| `POST` | `/api/v1/schemas/sync` | Compute schema delta / archive missing versions |
| `POST` | `/api/v1/prompts/ingest` | Create/update a prompt |
| `POST` | `/api/v1/prompts/sync` | Compute prompt delta |
| `PUT` | `/api/v1/assets/{*key}` | Upload an asset |
| `POST` | `/api/v1/assets/check-hashes` | Check which asset hashes already exist |

### Read & Search

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `GET` | `/api/v1/search?q=...` | Public (scoped) | Search documents |
| `GET` | `/api/v1/schemas` | Public (scoped) | List schemas |
| `GET` | `/api/v1/schemas/{name}?version={ver}` | Public (scoped) | Schema detail (no `version`) or artifact content |
| `GET` | `/api/v1/assets/{*key}` | Derived from referencing docs | Serve an asset |

### RAG Chat (authenticated user)

| Method | Endpoint | Description |
|--------|----------|-------------|
| `POST` | `/api/v1/rag/chat` | Streamed chat completion (SSE) over the corpus |
| `GET` | `/api/v1/rag/sessions` | List the user's chat sessions |
| `DELETE` | `/api/v1/rag/sessions/{id}` | Delete a session |
| `GET` | `/api/v1/rag/sessions/{id}/messages` | Load a session's messages |
| `POST`/`DELETE` | `/api/v1/rag/messages/{id}/feedback` | Submit / clear feedback on an answer |

### Personal Access Tokens (authenticated user)

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET`/`POST` | `/api/v1/user/pats` | List / create your PATs (used for MCP auth) |
| `PATCH`/`DELETE` | `/api/v1/user/pats/{id}` | Enable-disable / delete a PAT |

### Admin

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET`/`POST` | `/api/v1/admin/access-levels` | List / create access levels |
| `PUT`/`DELETE` | `/api/v1/admin/access-levels/{name}` | Update / delete an access level |
| `GET` | `/api/v1/admin/users`, `/api/v1/admin/users/{id}` | List users / get one |
| `PUT` | `/api/v1/admin/users/{id}/access-levels` | Set a user's granted access levels |
| `GET`/`POST` | `/api/v1/admin/service-tokens` | List / create CI/CD service tokens |
| `DELETE` | `/api/v1/admin/service-tokens/{id}` | Deactivate a service token |
| `GET` | `/api/v1/admin/pats`, `PATCH /…/pats/{id}` | List all PATs / toggle one |
| `POST` | `/api/v1/admin/{rag,search}/reindex` | Trigger a RAG / search re-index (poll `…/status`) |
| `POST` | `/api/v1/admin/schemas/reindex-endpoints` | Re-extract schema operations (poll `…/status`) |

### Auth & MCP

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/auth/login`, `/auth/callback` | OAuth2/OIDC login flow (`/api/auth/login` in demo mode) |
| `POST` | `/auth/refresh`, `/auth/logout` | Rotate refresh token / log out |
| `GET` | `/auth/me` | Current authenticated user |
| — | `/mcp` | Model Context Protocol Streamable HTTP endpoint (PAT auth) |

## Demo Mode

Set `LKN__AUTH__DEMO_MODE=true` to enable built-in demo authentication without an external
OAuth/OIDC provider. This creates three predefined users:

| Username | Password | Role |
|----------|----------|------|
| `admin` | `admin` | Admin (full access) |
| `demo` | `demo` | Regular authenticated user |
| `public` | `public` | Public-level access only |

Demo mode is intended for local development and evaluation only. In production,
configure a real OIDC or OAuth2 provider via `LKN__AUTH__*` environment variables.

## Architecture

Lekton follows a Headless CMS architecture:
-   **Storage Layer:** S3 for content, MongoDB for metadata.
-   **Service Layer:** Axum handles auth, ingestion, and search scoped by user roles.
-   **Presentation Layer:** Leptos for high-performance rendering.

## License

Distributed under the GNU AGPL v3 License. See [LICENSE](LICENSE) for more information.
