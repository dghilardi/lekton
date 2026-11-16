FROM rust:1.99-bookworm AS chef

RUN cargo install cargo-chef --version 0.1.78 --locked

WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json --package lekton-sync

COPY . .
RUN cargo build --release --package lekton-sync

FROM gcr.io/distroless/cc-debian12:nonroot AS runtime-base

WORKDIR /app

ENTRYPOINT ["/usr/local/bin/lekton-sync"]
CMD ["--help"]

FROM runtime-base AS runtime
COPY --from=builder /app/target/release/lekton-sync /usr/local/bin/lekton-sync

# Both release images use the same static CLI artifact, without recompiling it.
FROM runtime-base AS release-runtime
COPY --from=release-artifacts --chmod=755 /bin-x86_64-unknown-linux-musl/lekton-sync /usr/local/bin/lekton-sync

FROM debian:bookworm-slim AS ci-base

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

ENTRYPOINT ["/usr/local/bin/lekton-sync"]
CMD ["--help"]

FROM ci-base AS release-ci
COPY --from=release-artifacts --chmod=755 /bin-x86_64-unknown-linux-musl/lekton-sync /usr/local/bin/lekton-sync

# Preserve the default source build and the existing `--target ci` interface.
FROM ci-base AS ci
COPY --from=builder /app/target/release/lekton-sync /usr/local/bin/lekton-sync
