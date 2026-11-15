# syntax=docker/dockerfile:1

# --- Stage 1: Build ---
FROM rust:1.92-slim AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libsqlite3-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Dependency files first, so a source-only change reuses whatever the registry already
# has. There used to be a `RUN cargo build --release ... 2>/dev/null || true` line here
# labelled "Pre-build dependencies (cached unless Cargo.toml/Cargo.lock change)". It
# never did that: with only these two files in the context there is no `src/lib.rs` and no
# `src/main.rs`, `Cargo.toml` declares no `[lib]` and no `[[bin]]`, so cargo stops at
# manifest parse — "no targets specified in the manifest" — before resolving a single
# dependency, and `2>/dev/null || true` hid the exit code. Verified in a directory holding
# exactly these two files: exit 101, nothing compiled, nothing cached. Dropped rather than
# repaired, because repairing it means copying `src/` before it, which is the layer it
# existed to avoid invalidating; real dependency caching wants cargo-chef or a vendored
# registry and is its own change.
COPY Cargo.toml Cargo.lock ./

# The compile-time schema source, and the reason this stage is not just `COPY src/`.
#
# `src/` is `sqlx::query!` macros, and a macro is resolved when the crate is compiled,
# against **either** a live database **or** the committed prepare cache in `.sqlx/`.
# This build has no database, so it has to be told to use the cache; without
# `SQLX_OFFLINE` the first macro fails with:
#
#   error: set `DATABASE_URL` to use query macros online, or run `cargo sqlx prepare`
#         to update the query cache
#
# `migrations/` is not optional either: `sqlx::migrate!` in `src/main.rs` embeds the
# migration files at compile time, so a context without it fails the same way.
#
# Builder stage only, on purpose. `ENV` does not cross a `FROM`, and the runtime stage
# below copies exactly one file out of this one (the binary), so the shipped image
# carries neither `SQLX_OFFLINE` nor `.sqlx/`. An offline flag in the running image
# would advertise a mode the binary does not have.
#
# What this trusts: the image compiles against `.sqlx/` without ever looking at the real
# schema, so a stale cache ships a binary whose queries no longer match production —
# the failure mode a prepare cache is famous for. That is acceptable only because
# `scripts/check_sqlx_cache.sh` is a CI step in `.github/workflows/ci.yml`
# (`build-and-test`) and a documented gate in `AGENTS.md`. Unwire that gate and this
# build is quietly compiling against a snapshot nobody checks.
ENV SQLX_OFFLINE=true
COPY .sqlx/ .sqlx/
COPY migrations/ migrations/

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
