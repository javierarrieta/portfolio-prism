# syntax=docker/dockerfile:1

# --- Stage 1: Build ---
FROM rust:1.92-slim AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libsqlite3-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy only dependency files first for layer caching
COPY Cargo.toml Cargo.lock ./

# Pre-build dependencies (cached unless Cargo.toml/Cargo.lock change)
RUN cargo build --release -j $(nproc) --bin backend_rust 2>/dev/null || true

# Copy source code
COPY src/ src/

# Full build
RUN cargo build --release -j $(nproc) --bin backend_rust

# --- Stage 2: Runtime ---
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    libsqlite3-0 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /build/target/release/backend_rust /app/backend_rust

# Expose volume for persistent data. PRE-CUTOVER: the app no longer writes to a file —
# it connects to a Postgres server — so this volume is unused by the current image.
VOLUME /app/data

# PRE-CUTOVER, AND WRONG FOR THE CURRENT IMAGE. This default is a SQLite DSN and the
# backend refuses to start on one, so `docker run` without an explicit -e DATABASE_URL
# exits immediately. Left as-is on purpose: the real URL carries a host and credentials
# and belongs in the deployment's Secret, not baked into an image layer. Overriding it
# (`docker run -e DATABASE_URL=postgres://...`) is the only correct way to use this image
# until the cutover in Task 10 removes these two lines.
ENV DATABASE_URL=sqlite:///app/data/portfolio.db?mode=rwc

RUN useradd -r -s /bin/false app
RUN mkdir -p /app/data && chown app:app /app/data
USER app

EXPOSE 8000

ENTRYPOINT ["./backend_rust"]
