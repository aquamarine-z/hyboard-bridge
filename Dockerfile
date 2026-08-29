# ==============================================================================
# Multi-stage Dockerfile for hyboard-bridge
# Ultra-lightweight static build based on Rust Alpine (Final image < 15MB)
# ==============================================================================

# --- Stage 1: Build & Optimize ---
FROM rust:alpine AS builder

# Install build dependencies
RUN apk add --no-cache musl-dev pkgconfig openssl-dev

WORKDIR /app

# Cache dependencies by copying manifest files first
COPY Cargo.toml Cargo.lock ./

# Create dummy src to build dependencies cache
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src target/release/deps/hyboard_bridge* target/release/hyboard-bridge*

# Copy actual source code
COPY src ./src

# Build release binary with full optimizations
RUN cargo build --release

# --- Stage 2: Minimal Runtime ---
FROM alpine:3.20

# Install ca-certificates and tzdata for TLS validation and correct timestamps
RUN apk --no-cache add ca-certificates tzdata

WORKDIR /app

# Copy compiled binary from builder
COPY --from=builder /app/target/release/hyboard-bridge /usr/local/bin/hyboard-bridge

# Set default environment
ENV RUST_LOG=info \
    HYSTERIA_API=http://hysteria:7654

# Expose HTTP Auth webhook port
EXPOSE 9999

ENTRYPOINT ["/usr/local/bin/hyboard-bridge"]
