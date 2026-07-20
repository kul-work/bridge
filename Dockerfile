# Stage 1: Build the Rust binary
ARG RUST_VERSION=1.93.1
FROM rust:${RUST_VERSION} AS builder
ARG BRIDGE_BUILD_GIT_SHA=unknown

WORKDIR /app

# Copy manifest files
COPY Cargo.toml Cargo.lock ./

# Copy source code
COPY src ./src

# Copy compile-time embedded assets
COPY migrations ./migrations
COPY templates ./templates

# Build the application
RUN BRIDGE_BUILD_GIT_SHA="${BRIDGE_BUILD_GIT_SHA}" cargo build --release

# Stage 2: Runtime image
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the binary from builder
COPY --from=builder /app/target/release/bridge .

# Copy runtime assets
COPY migrations ./migrations

# TLS root certs for managed Postgres (Neon/Supabase) SSL connections.
# database.rs sets ssl_root_cert to ./certs/isrgrootx1.pem for neon.tech hosts
# (and prod-ca-2021.crt otherwise) when the URL uses sslmode=require, so these
# must exist in the runtime image. Copy only the CA roots, NOT the provider
# service-account JSONs in certs/.
COPY certs/isrgrootx1.pem certs/prod-ca-2021.crt ./certs/

# Set environment variables
ENV PORT=3000
ARG ENVIRONMENT=production
ENV RUST_LOG=${ENVIRONMENT:+bridge=info,axum=info}
ENV RUST_LOG=${RUST_LOG:-bridge=debug,axum=info}

EXPOSE 3000

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:3000/health || exit 1

CMD ["./bridge"]
