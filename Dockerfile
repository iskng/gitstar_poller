# Build stage
FROM rust:1.75-bookworm as builder

# Install dependencies for building
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Create app directory
WORKDIR /app

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Copy source code
COPY src ./src

# Build release binary
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 -s /bin/bash appuser

# Copy binary from builder
COPY --from=builder /app/target/release/github-stars-server /usr/local/bin/

# Change ownership
RUN chown appuser:appuser /usr/local/bin/github-stars-server

# Switch to non-root user
USER appuser

# Expose ports
EXPOSE 8080

# Health check
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

# Set default environment variables
ENV RUST_LOG="warn,github_stars_server=info"
ENV HEALTH_PORT=8080

# Run the binary
ENTRYPOINT ["github-stars-server"]