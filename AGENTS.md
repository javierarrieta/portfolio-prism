# Agent Instructions

## Overview
Portfolio Prism: Full-stack app for portfolio management and tax lot (FIFO/LIFO/Hybrid) calculations.

## Architecture
- **Backend**: Rust (Rocket) in `backend_rust/`. Uses **Postgres** (see *Database* below) via SQLx.
- **Frontend**: React + Vite in `frontend/`.

## Commands

### Run Everything
From root:
```bash
./start.sh
```

### Backend
- **Local database**: `nix-shell -p postgresql_18 --run 'bash scripts/dev-db.sh'`
  (from `backend_rust/`). Prints the `DATABASE_URL` to export. Stop with
  `scripts/dev-db-stop.sh`. There is no SQLite fallback: the engine is Postgres
  only, and the tests run against the same engine as production.
- **Start API**: `cargo run` (run from `backend_rust/`). In development this needs
  `AUTH_MODE=disabled` in the environment — `./start.sh` exports it. Without it the
  backend refuses to start, naming what is missing.
- **Test**: `cargo test` (run from `backend_rust/`)
- **Update OpenAPI Spec**: When API routes or schemas change, regenerate and save:
  ```bash
  cd backend_rust
  cargo run --bin backend_rust -- --print-openapi > ../docs/openapi/openapi.json
  ```
  `--print-openapi` is checked before configuration, the database, and the network,
  so it needs no database, no running server, and no OIDC configuration. The
  previous procedure started the real server and curled it; that server now refuses
  to launch without OIDC configuration, so the old command no longer works.

### Frontend
- **Dev Server**: `npm run dev` (run from `frontend/`)
- **Lint**: `npm run lint` (run from `frontend/`)

## Key Context
- **Authentication**: OIDC via Authentik. `backend_rust/src/auth/` holds the flow
  (login, callback, session cookie, guard); `backend_rust/src/access.rs` is the
  single chokepoint every data route goes through. New `/api` routes must take
  `AuthedUser` and call a `require*` function — `bash backend_rust/scripts/check_access_gate.sh`
  fails the build if one does not. `AUTH_MODE=disabled` exists only in debug builds;
  the release image refuses to start without OIDC configuration.
- **API Prefix**: All routes are under `/api`.
- **Tax Logic**: `backend_rust/src/engines/tax_engine.rs` contains the core `TaxLotEngine` supporting `FIFO`, `LIFO`, and `HYBRID` (using `hybrid_threshold_days`) strategies.
- **Database**: Postgres, and only Postgres — there is no engine flag and no SQLite
  support in the crate. `DATABASE_URL` has **no default**: `main` refuses to start
  without it and names the variable (`main.rs`). In development that is
  `postgres://postgres@127.0.0.1:55432/prism`, the URL `backend_rust/scripts/dev-db.sh`
  prints; `start.sh` exports it if the environment does not already set one. Schema comes
  from `backend_rust/migrations/` via `sqlx::migrate!` at startup, not from any
  create-if-not-exists code. There is no `backend/` directory. The cutover procedure is
  `docs/postgres-cutover.md`. **`k8s/configmap.yaml` still ships a `sqlite:` DSN on
  purpose** — a live deployment consumes that manifest, and changing it is an ordered
  cutover step (database and Secret first, image last), not a commit; see the runbook and
  `docs/superpowers/specs/2026-11-14-postgres-gitops-followup.md`. The image itself no
  longer ships one: `backend_rust/Dockerfile` has no `DATABASE_URL` and no SQLite runtime
  dependency, so a container without an injected URL exits and names the variable.
- **Metrics Engine**: `backend_rust/src/engines/stats_engine.rs` computes the daily value
  series, cumulative **TWR**, portfolio value and beta-adjusted exposure. Volatility,
  Sharpe Ratio and Portfolio Beta are **not** computed — the response carries them as the
  constants `"0.0"`, `"0.0"` and `"1.0"` (`stats_engine.rs:108-110` and `:323-325`), and
  `stats_engine.rs:383` asserts the Sharpe is `"0.0"`. The correlation matrix is returned
  empty for the same reason. Do not write a test or a doc that treats those four as real
  measurements without changing the engine first.
- **macOS Tip**: Use `127.0.0.1` instead of `localhost` for the API to avoid IPv6 resolution issues.

## Branching Workflow

- **Always create a new branch** for each feature, fix, or change. Never work on `main`.
- Name branches descriptively using the `type/scope` convention:
  - `feat/...` — new features or functionality
  - `fix/...` — bug fixes
  - `chore/...` — configuration, tooling, docs
  - `refactor/...` — code refactoring with no behavior change
- After finishing work, commit your changes. The user will create the PR and merge manually.

## Testing

- **Full gate list and the OIDC walkthrough**: `docs/auth-testing.md`. It is the
  entry point for verifying the auth work, including the two things CI cannot check
  (a real login, and whether the provider ends its own session on logout). Its §0 is the
  whole gate set in CI's order and it brings the database up first, because `cargo test`
  creates one real database per test and **panics** without `DATABASE_URL` — there is no
  file to fall back to and no in-memory mode. If §0 and the **Backend gates** bullet below
  ever disagree, the bullet below is what CI enforces.
- **Backend gates**, in the order CI enforces them. `build-and-test` runs every one but
  the last; `cargo build --release` is the `release-build` job:
  `sqlx migrate run --no-dotenv`, `cargo sqlx prepare --check -- --all-targets`,
  `cargo test`, `bash scripts/check_access_gate.sh`, `bash scripts/check_pricing_seam.sh`,
  `bash scripts/check_backup_portability.sh`, `bash scripts/check_upload_path.sh`,
  `bash scripts/check_migrations.sh`,
  `bash scripts/check_sqlx_cache.sh`, `cargo clippy --all-targets`,
  `cargo build --release` — all from `backend_rust/`.
  **Five of the eleven need `DATABASE_URL`** — migrate, prepare, `cargo test` (which creates
  one real database per test, `src/test_db.rs`), `check_migrations.sh` and
  `check_sqlx_cache.sh`. `scripts/dev-db.sh` prints the URL to export. The four static
  gates do not read it, and neither do the two compiles: with no `DATABASE_URL` a
  `sqlx::query!` macro resolves from the committed `.sqlx/` prepare cache, which is also
  how `backend_rust/Dockerfile` builds with no database in reach.
  `check_migrations.sh` inspects a **database**, not `migrations/` — it passes just as
  happily against a stale schema — so it means something only after `sqlx migrate run` on
  a database nobody has touched since. In CI that is the service's own `prism`, empty
  until the migrate step. Until Task 9 landed, `check_migrations.sh` and
  `check_sqlx_cache.sh` ran in no job at all: they existed, they had proved they can
  fail, and nobody was watching. Both now fail rather than skip when their tool is
  missing (`check_sqlx_cache.sh` refuses if `sqlx` or `cargo-sqlx` is not on PATH).
  Each gate script has its own test (`tests/access_gate_test.rs`,
  `tests/backup_portability_test.rs`) that runs the real script against a throwaway
  tree; a gate is only as good as the mutation that proves it can fail.
  **Scope of the compile-time checking**: `src/` is checked against the schema at compile
  time; **`tests/` is not** — roughly 124 runtime `sqlx::query`/`query_as` call sites
  across 23 files still go to the server unprepared, so a wrong column in a test query
  is a run-time failure, not a compile error. Do not read "compile-time checked queries"
  as universal coverage.
- **Image build compiles from the prepare cache**: `backend_rust/Dockerfile` copies
  `.sqlx/` and sets `SQLX_OFFLINE=true` **in the builder stage only**, so `docker build`
  never reaches a database. That is only safe while `check_sqlx_cache.sh` runs in CI;
  unwire that gate and the image ships a binary built against a snapshot nobody checks.
  `migrations/` must stay in the build context too — `sqlx::migrate!` embeds it at
  compile time.
- **Provider smoke (no browser)**: `bash scripts/auth_smoke.sh` from `backend_rust/`,
  with `DATABASE_URL` (Postgres — `scripts/dev-db.sh` prints one; a `sqlite:` URL is
  refused) plus `OIDC_ISSUER`/`OIDC_CLIENT_ID`/`OIDC_CLIENT_SECRET`/`OIDC_REDIRECT_URI`
  set.
- **Local OIDC harness**: `docker-compose.auth-test.yml` with `.auth-test.env`
  (see `.auth-test.env.example`). Not validated — the authoring workspace has no docker.

## Roadmap / Current Focus
- **Postgres cutover, the deferred half**: the application side is merged (engine, tests,
  gates, runbook). The cluster side is
  `docs/superpowers/specs/2026-11-14-postgres-gitops-followup.md` — a CNPG `Database`,
  `DatabaseRole` and Secret in the `k8s-titan` repo, applied **in that order** before the
  image rolls, because the build refuses to start without `DATABASE_URL` and `replicas: 1`
  with `Recreate` turns a badly ordered apply into an outage — but `k8s/app-deployment.yaml`
  declares no `spec.strategy`, so confirm `.spec.strategy.type` on the live deployment
  before the apply rather than assuming which rollout behaviour you are about to cause. The
  data procedure an operator follows on the day is `docs/postgres-cutover.md`.
- **Multi-Currency Support**: Implementing asset currency tracking and exchange rate conversion for tax and stats (see `docs/implementation_plan.md`).
