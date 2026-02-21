# Lekton

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
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

```bash
cargo test --features ssr
```

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

## 🏗️ Architecture

Lekton follows a Headless CMS architecture:
-   **Storage Layer:** S3 for content, MongoDB for metadata.
-   **Service Layer:** Axum handles auth, ingestion, and search scoped by user roles.
-   **Presentation Layer:** Leptos for high-performance rendering.

## 📝 License

Distributed under the MIT License. See `LICENSE` for more information.
