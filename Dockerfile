# Multi-architecture Dockerfile for AyanomiBancho
# Supports: linux/amd64, linux/arm64 (ARMv8), linux/arm/v7 (ARMv7)

FROM --platform=$BUILDPLATFORM rust:1.85-slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    gcc \
    libc6-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/ayanomibancho

# Copy source manifests
COPY Cargo.toml Cargo.lock ./

# Create dummy source to build dependencies
RUN mkdir -p src/bin && \
    echo "fn main() {}" > src/main.rs && \
    echo "fn main() {}" > src/bin/ayanomi_bancho.rs && \
    echo "fn main() {}" > src/bin/ayanomi_web.rs && \
    echo "fn main() {}" > src/bin/ayanomi_gateway.rs && \
    echo "" > src/lib.rs && \
    cargo build --release && \
    rm -rf src

# Copy real source code and compile
COPY src ./src
RUN touch src/lib.rs src/main.rs && cargo build --release --bin ayanomibancho

# Runtime stage (minimal and optimized for ARM / x86)
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary from builder
COPY --from=builder /usr/src/ayanomibancho/target/release/ayanomibancho /app/ayanomibancho

# Copy necessary runtime assets
COPY config.toml /app/config.toml
COPY static /app/static
COPY default /app/default

# Data directory volume for SQLite databases & avatars & banners
RUN mkdir -p /app/data
VOLUME ["/app/data"]

EXPOSE 5000 5001 5002

ENV RUST_LOG=info

CMD ["/app/ayanomibancho"]
