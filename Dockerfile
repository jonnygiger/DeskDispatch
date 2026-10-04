# Multi-stage Dockerfile for DeskDispatch Rust task server

# Stage 1: Builder
FROM rust:1.85-slim-bookworm AS builder

WORKDIR /usr/src/app

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy dependency files and source code
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY templates ./templates
COPY static ./static
COPY migrations ./migrations

# Build the release binary
RUN cargo build --release --ignore-rust-version

# Stage 2: Runner
FROM debian:bookworm-slim AS runner

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Copy the compiled binary from the builder stage
COPY --from=builder /usr/src/app/target/release/app /app/deskdispatch

# Set default environment variables
ENV BIND_ADDRESS=0.0.0.0:3000 \
    APP_ENV=production

EXPOSE 3000

ENTRYPOINT ["/app/deskdispatch"]
