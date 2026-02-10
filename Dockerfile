FROM rust:1.81-bookworm as builder

# Install cargo-leptos
RUN cargo install cargo-leptos --locked

# Add wasm target
RUN rustup target add wasm32-unknown-unknown

WORKDIR /app
COPY . .

# Build for release
RUN cargo leptos build --release

FROM debian:bookworm-slim
WORKDIR /app

# Copy the binary
COPY --from=builder /app/target/release/lekton /app/lekton

# Copy the site assets (pkg, style, public)
COPY --from=builder /app/target/site /app/target/site

ENV LEPTOS_SITE_ROOT="target/site"
ENV LEPTOS_SITE_ADDR="0.0.0.0:3000"

CMD ["/app/lekton"]
