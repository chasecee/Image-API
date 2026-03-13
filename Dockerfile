# ── Build stage ──────────────────────────────────────────────────────────────
FROM rust:slim AS builder

# System deps for image crate (libjpeg, libpng, etc. are pure-Rust; only need
# pkg-config + libssl-dev if TLS is pulled in; none needed here)
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy the whole workspace
COPY Cargo.toml Cargo.lock* ./
COPY crates/ crates/

# Build only the api binary in release mode
RUN cargo build --release --bin api

# ── Runtime stage ─────────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

# The image crate uses pure-Rust decoders, no native libs needed.
# We do want ca-certificates in case future deps need it.
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the compiled binary
COPY --from=builder /build/target/release/api /app/api

# Copy static assets that the binary serves at runtime
COPY frontend/ /app/frontend/
COPY test_images/ /app/test_images/

ENV PORT=3000
EXPOSE 3000

CMD ["/app/api"]
