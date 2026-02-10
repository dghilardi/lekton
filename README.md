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
-   [MongoDB](https://www.mongodb.com/) (running instance)
-   S3-compatible storage (e.g., [MinIO](https://min.io/) for local development)

### Installation

```bash
# Clone the repository
git clone https://github.com/dghilardi/lekton.git
cd lekton

# Install Node dependencies (DaisyUI)
npm install

# Run the development server
cargo leptos watch
```

The application will be available at `http://127.0.0.1:3000`.

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

## 🎨 Customizability

Lekton is designed to be **highly customizable without recompilation**.

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

In Docker deployments, mount your custom CSS as a volume:

```bash
docker run -v ./my-custom.css:/app/public/custom.css lekton
```

## 🏗️ Architecture

Lekton follows a Headless CMS architecture:
-   **Storage Layer:** S3 for content, MongoDB for metadata.
-   **Service Layer:** Axum handles auth, ingestion, and search scoped by user roles.
-   **Presentation Layer:** Leptos for high-performance rendering.

## 📝 License

Distributed under the MIT License. See `LICENSE` for more information.
