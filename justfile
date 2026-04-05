# Lekton task runner
# Usage: just <recipe>
# Requires: just (https://github.com/casey/just)

set dotenv-load := true

# Show available recipes
default:
    @just --list

# ── Development ───────────────────────────────────────────────────────────────

# Start dev server with hot-reload (loads .env automatically)
dev:
    cargo leptos watch

# Check compilation for both SSR and WASM targets
check:
    cargo check --features ssr
    cargo check --features hydrate

# ── Tests ─────────────────────────────────────────────────────────────────────

# Run unit tests (fast, no Docker required)
test:
    cargo test --features ssr --lib

# Run integration tests (requires Docker for MongoDB, MinIO, Meilisearch)
test-integration:
    cargo test --features ssr --test '*' -- --test-threads=1

# Run e2e tests — starts the server if it is not already running on :3000
test-e2e *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail

    E2E_SERVER_STARTED=false

    # cargo-leptos spawns the actual binary as a child process, so we can't
    # reliably kill by the cargo-leptos PID. Instead we track by port and kill
    # whatever is bound to :3000 on exit.
    _cleanup() {
        if [ "$E2E_SERVER_STARTED" = "true" ]; then
            echo "Stopping e2e server..."
            lsof -ti :3000 | xargs kill -9 2>/dev/null || true
        fi
    }
    trap _cleanup EXIT

    if curl -sf http://localhost:3000/ > /dev/null 2>&1; then
        echo "Server already running on :3000, reusing."
    else
        echo "Starting server for e2e tests..."
        # Use a separate DB so e2e data doesn't pollute the dev database.
        # All other vars (S3, Meilisearch, etc.) come from .env via dotenv-load.
        LKN__DATABASE__NAME=lekton_e2e \
        LKN__AUTH__SERVICE_TOKEN=test-token \
        LKN__SERVER__RATE_LIMIT_BURST=1000 \
        LKN__AUTH__DEMO_MODE=true \
            cargo leptos serve > /tmp/lekton-e2e-server.log 2>&1 &
        CARGO_LEPTOS_PID=$!
        E2E_SERVER_STARTED=true

        echo "Waiting for server to be ready (building + starting)..."
        for i in $(seq 1 60); do
            if curl -sf http://localhost:3000/ > /dev/null 2>&1; then
                echo "Server ready after $((i * 2))s."
                break
            fi
            # If cargo-leptos itself died the build failed; bail early
            if ! kill -0 "$CARGO_LEPTOS_PID" 2>/dev/null; then
                echo "Build/start failed. Log:"
                tail -20 /tmp/lekton-e2e-server.log
                exit 1
            fi
            sleep 2
        done

        if ! curl -sf http://localhost:3000/ > /dev/null 2>&1; then
            echo "Server did not become ready in time. Log:"
            tail -20 /tmp/lekton-e2e-server.log
            exit 1
        fi
    fi

    # Pass test-specific overrides to the playwright process too — global-setup.ts
    # reads SERVICE_TOKEN from the environment, and dotenv-load may have set it
    # to the dev value from .env.
    LKN__AUTH__SERVICE_TOKEN=test-token npx playwright test {{ ARGS }}

# Open the Playwright UI for interactive test debugging
test-e2e-ui:
    #!/usr/bin/env bash
    set -euo pipefail

    E2E_SERVER_STARTED=false

    _cleanup() {
        if [ "$E2E_SERVER_STARTED" = "true" ]; then
            echo "Stopping e2e server..."
            lsof -ti :3000 | xargs kill -9 2>/dev/null || true
        fi
    }
    trap _cleanup EXIT

    if curl -sf http://localhost:3000/ > /dev/null 2>&1; then
        echo "Server already running on :3000, reusing."
    else
        echo "Starting server for e2e tests..."
        LKN__DATABASE__NAME=lekton_e2e \
        LKN__AUTH__SERVICE_TOKEN=test-token \
        LKN__SERVER__RATE_LIMIT_BURST=1000 \
        LKN__AUTH__DEMO_MODE=true \
            cargo leptos serve > /tmp/lekton-e2e-server.log 2>&1 &
        CARGO_LEPTOS_PID=$!
        E2E_SERVER_STARTED=true

        echo "Waiting for server to be ready..."
        for i in $(seq 1 60); do
            if curl -sf http://localhost:3000/ > /dev/null 2>&1; then
                echo "Server ready after $((i * 2))s."
                break
            fi
            if ! kill -0 "$CARGO_LEPTOS_PID" 2>/dev/null; then
                echo "Build/start failed. Log:"
                tail -20 /tmp/lekton-e2e-server.log
                exit 1
            fi
            sleep 2
        done
    fi

    LKN__AUTH__SERVICE_TOKEN=test-token npx playwright test --ui

# Run all test suites in sequence: unit → integration → e2e
test-all: test test-integration test-e2e

# ── Utilities ─────────────────────────────────────────────────────────────────

# Show e2e server log (useful when test-e2e fails at startup)
e2e-logs:
    @cat /tmp/lekton-e2e-server.log 2>/dev/null || echo "No e2e server log found."

# Install Playwright browsers (run once after npm install)
playwright-install:
    npx playwright install --with-deps chromium
