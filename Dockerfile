# Multi-stage Dockerfile for DeskDispatch Rust task server

# Stage 1: Builder
FROM rust:1.99-slim-bookworm AS builder

WORKDIR /usr/src/app

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Pre-build dependencies to leverage Docker layer caching
COPY Cargo.toml ./
COPY crates/deskdispatch-protocol/Cargo.toml ./crates/deskdispatch-protocol/
COPY crates/worker-simulator/Cargo.toml ./crates/worker-simulator/
RUN mkdir -p src crates/deskdispatch-protocol/src crates/worker-simulator/src && \
    echo "fn main() {}" > src/main.rs && \
    echo "fn main() {}" > crates/worker-simulator/src/main.rs && \
    touch src/lib.rs && \
    touch crates/deskdispatch-protocol/src/lib.rs && \
    cargo build --release

# Copy workspace crates, source code, templates, static assets, and database migrations
COPY crates ./crates
COPY src ./src
COPY templates ./templates
COPY static ./static
COPY migrations ./migrations

# Update timestamps on source files to force recompilation of workspace crates and build final binaries
RUN touch src/main.rs src/lib.rs crates/deskdispatch-protocol/src/lib.rs crates/worker-simulator/src/main.rs && \
    cargo build --release --workspace && \
    mkdir -p /app/bin && \
    cp target/release/deskdispatch /app/bin/deskdispatch && \
    cp target/release/worker-simulator /app/bin/worker-simulator

# Stage 2: Runner
FROM debian:bookworm-slim AS runner

# Install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Create non-root user
RUN groupadd -g 10001 appgroup && \
    useradd -u 10001 -g appgroup -s /bin/false appuser

# Copy the compiled binaries from the builder stage
COPY --from=builder /app/bin/deskdispatch /app/deskdispatch
COPY --from=builder /app/bin/worker-simulator /app/worker-simulator

USER appuser:appgroup

# Set default environment variables
ENV BIND_ADDRESS=0.0.0.0:3000 \
    APP_ENV=production

EXPOSE 3000

HEALTHCHECK --interval=5s --timeout=3s --start-period=5s --retries=3 \
  CMD ["/app/deskdispatch", "healthcheck"]

ENTRYPOINT ["/app/deskdispatch"]
