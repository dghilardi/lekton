# =============================================================================
# Stage 1: Chef — prepare dependency recipe for caching
# =============================================================================
FROM rust:1.93-bookworm AS chef

RUN cargo install cargo-chef --locked && \
    cargo install cargo-leptos --locked && \
    rustup target add wasm32-unknown-unknown

WORKDIR /app

# =============================================================================
# Stage 2: Planner — generate the dependency recipe
# =============================================================================
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# =============================================================================
# Stage 3: Builder — build dependencies then the application
# =============================================================================
FROM chef AS builder

# Install Node.js for Tailwind/DaisyUI
RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - && \
    apt-get install -y nodejs

# Cache dependencies
COPY --from=planner /app/recipe.json recipe.json
COPY package.json package-lock.json ./
RUN npm ci
RUN cargo chef cook --release --recipe-path recipe.json --features ssr && \
    cargo chef cook --release --recipe-path recipe.json --target wasm32-unknown-unknown --features hydrate

# Copy source and build
COPY . .
RUN npm ci
RUN cargo leptos build --release
# Guarantee schema viewer JS bundles are in target/site/js/ even when cargo
# considers the build up-to-date and skips the public/ -> target/site/ sync.
RUN mkdir -p target/site/js && \
    cp node_modules/@scalar/api-reference/dist/browser/standalone.js target/site/js/scalar-standalone.js && \
    cp node_modules/@scalar/api-reference/dist/style.css target/site/js/scalar-style.css && \
    cp node_modules/@asyncapi/react-component/browser/standalone/index.js target/site/js/asyncapi-standalone.js && \
    cp node_modules/@asyncapi/react-component/styles/default.min.css target/site/js/asyncapi-default.min.css

# =============================================================================
# Stage 4: Runtime — minimal image with just the binary
# =============================================================================
FROM debian:bookworm-slim AS runtime

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        libssl3 \
        curl && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the server binary
COPY --from=builder /app/target/release/lekton /app/lekton

# Copy the generated site assets (JS, WASM, CSS)
COPY --from=builder /app/target/site /app/target/site

# Copy the Cargo.toml (needed by cargo-leptos for runtime config)
COPY --from=builder /app/Cargo.toml /app/Cargo.toml

# Copy public assets
COPY --from=builder /app/public /app/public

ENV LEPTOS_SITE_ADDR="0.0.0.0:3000"
ENV LEPTOS_SITE_ROOT="target/site"
EXPOSE 3000

CMD ["/app/lekton"]
