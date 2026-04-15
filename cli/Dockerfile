FROM rust:1.93-bookworm AS chef

RUN cargo install cargo-chef --locked

WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json --package lekton-sync

COPY . .
RUN cargo build --release --package lekton-sync

FROM gcr.io/distroless/cc-debian12:nonroot AS runtime

WORKDIR /app

COPY --from=builder /app/target/release/lekton-sync /usr/local/bin/lekton-sync

ENTRYPOINT ["/usr/local/bin/lekton-sync"]
CMD ["--help"]
