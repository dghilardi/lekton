# Lekton

[![License: AGPL](https://img.shields.io/github/license/dghilardi/lekton)](https://opensource.org/licenses/AGPL-3.0)
[![Rust](https://img.shields.io/badge/rust-stable-brightgreen.svg)](https://www.rust-lang.org)

**Lekton** is a high-performance, dynamic Internal Developer Portal (IDP) designed to replace static documentation generators. Built with Rust, it prioritizes speed, granular security (RBAC), and a seamless developer experience.

## 🚀 Vision

Lekton decouples content from code, allowing microservices to push their documentation and API schemas (OpenAPI/AsyncAPI) to a centralized, searchable, and secure hub. No more full site rebuilds for a typo fix.

## ✨ Key Features

-   **Dynamic Ingestion:** CI/CD integration for live documentation updates.
-   **Granular RBAC:** Server-level Role-Based Access Control for sensitive documents.
-   **Unified Schema Registry:** Support for OpenAPI (Swagger), AsyncAPI, and JSON Schema with versioning.
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
-   [Node.js](https://nodejs.org/) (for DaisyUI)
-   [Docker](https://www.docker.com/) & [Docker Compose](https://docs.docker.com/compose/)

### Quick Start with Docker Compose (Recommended)

The easiest way to run Lekton with all dependencies:

```bash
# Clone the repository
git clone https://github.com/dghilardi/lekton.git
cd lekton

# Install Node dependencies (DaisyUI)
npm install

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
1. Install Node.js dependencies (DaisyUI for Tailwind CSS)
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

#### Manual Setup

If you prefer to set up manually:

**1. Install Node.js dependencies**

```bash
# Install DaisyUI and other frontend dependencies
npm install
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
MONGODB_URI=mongodb://localhost:27017
MONGODB_DATABASE=lekton

# S3 Storage Configuration (use credentials from garage-init output)
S3_BUCKET=lekton-docs
S3_ENDPOINT=http://localhost:3900
AWS_ACCESS_KEY_ID=GK6dcd28a916458f75d62f0720
AWS_SECRET_ACCESS_KEY=893fa79f053d67be65237fdc5d2a8521df5dc0a27858f991ffa72c1ba3470291
AWS_REGION=garage

# Service Token for API ingestion
SERVICE_TOKEN=demo-ingest-token

# Enable demo auth mode (bypasses OIDC)
DEMO_MODE=true

# Logging
RUST_LOG=lekton=info,tower_http=info

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
- Run `npm install` to install Node.js dependencies
- If that doesn't work, delete `node_modules` and `package-lock.json`, then run `npm install` again

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
# Unit tests
cargo test --features ssr --lib

# Integration tests (single-threaded to avoid container conflicts)
cargo test --features ssr --test '*' -- --test-threads=1

# E2E tests — server must be running on :3000 first
source .env
MONGODB_DATABASE=lekton_e2e SERVICE_TOKEN=test-token RATE_LIMIT_BURST=1000 DEMO_MODE=true \
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

Lekton is configured via environment variables:

| Variable            | Description                          | Default                          |
| ------------------- | ------------------------------------ | -------------------------------- |
| `MONGODB_URI`       | MongoDB connection string            | `mongodb://localhost:27017`      |
| `MONGODB_DATABASE`  | MongoDB database name                | `lekton`                         |
| `S3_BUCKET`         | S3 bucket name                       | *(required)*                     |
| `S3_ENDPOINT`       | Custom S3 endpoint (MinIO, etc.)     | *(AWS default)*                  |
| `AWS_REGION`        | AWS region                           | *(from AWS config)*              |
| `SERVICE_TOKEN`     | Token for CI/CD ingestion API        | `dev-token`                      |
| `OIDC_ISSUER_URL`   | OIDC identity provider URL           | *(required for auth)*            |
| `OIDC_CLIENT_ID`    | OIDC client ID                       | *(required for auth)*            |
| `OIDC_CLIENT_SECRET`| OIDC client secret                   | *(required for auth)*            |
| `OIDC_REDIRECT_URI` | OIDC callback redirect URI           | *(required for auth)*            |
| `RUST_LOG`          | Log level filter                     | `lekton=info,tower_http=info`    |

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

### Ingestion

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `POST` | `/api/v1/ingest` | Service token | Create/update a document |
| `POST` | `/api/v1/schemas` | Service token | Create/update a schema |
| `POST` | `/api/v1/schemas/sync` | Service token | Compute schema delta / archive missing versions |
| `POST` | `/api/v1/upload/{*key}` | Service token | Upload an asset |

### Search

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `GET` | `/api/v1/search?q=...` | Public (scoped) | Search documents |

### Admin

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `GET` | `/api/v1/admin/access-levels` | Admin | List all access levels |
| `POST` | `/api/v1/admin/access-levels` | Admin | Create an access level |
| `PUT` | `/api/v1/admin/access-levels/{name}` | Admin | Update an access level |
| `DELETE` | `/api/v1/admin/access-levels/{name}` | Admin | Delete an access level |
| `GET` | `/api/v1/admin/users` | Admin | List all users |
| `GET` | `/api/v1/admin/user-permissions/{user_id}` | Admin | List user permissions |
| `POST` | `/api/v1/admin/user-permissions` | Admin | Grant/update a permission |
| `DELETE` | `/api/v1/admin/user-permissions/{user_id}/{level}` | Admin | Revoke a permission |

## Demo Mode

Set `DEMO_MODE=true` to enable built-in demo authentication without an external
OAuth/OIDC provider. This creates three predefined users:

| Username | Password | Role |
|----------|----------|------|
| `admin` | `admin` | Admin (full access) |
| `demo` | `demo` | Regular authenticated user |
| `public` | `public` | Public-level access only |

Demo mode is intended for local development and evaluation only. In production,
configure a real OIDC or OAuth2 provider via `AUTH_PROVIDER_*` environment variables.

## Architecture

Lekton follows a Headless CMS architecture:
-   **Storage Layer:** S3 for content, MongoDB for metadata.
-   **Service Layer:** Axum handles auth, ingestion, and search scoped by user roles.
-   **Presentation Layer:** Leptos for high-performance rendering.

## License

Distributed under the GNU GPL v3 License. See [LICENSE](LICENSE) for more information.
