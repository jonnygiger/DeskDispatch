# Multi-stage Dockerfile for DeskDispatch Rust task server

# Stage 1: Builder
FROM rust:1.85-slim-bookworm AS builder

WORKDIR /usr/src/app

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace crates, dependency files, and source code
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY src ./src
COPY templates ./templates
COPY static ./static
COPY migrations ./migrations

# Build the release binary
RUN cargo build --release --locked

# Stage 2: Runner
FROM debian:bookworm-slim AS runner

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN groupadd -g 10001 appgroup && \
    useradd -u 10001 -g appgroup -s /bin/false appuser

# Copy the compiled binary from the builder stage
COPY --from=builder /usr/src/app/target/release/deskdispatch /app/deskdispatch

USER appuser:appgroup

# Set default environment variables
ENV BIND_ADDRESS=0.0.0.0:3000 \
    APP_ENV=production

EXPOSE 3000

HEALTHCHECK --interval=10s --timeout=5s --start-period=10s --retries=3 \
  CMD ["/app/deskdispatch", "healthcheck"]

ENTRYPOINT ["/app/deskdispatch"]
