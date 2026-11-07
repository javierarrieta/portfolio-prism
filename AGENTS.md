# Agent Instructions

## Overview
Portfolio Prism: Full-stack app for portfolio management and tax lot (FIFO/LIFO/Hybrid) calculations.

## Architecture
- **Backend**: Rust (Rocket) in `backend_rust/`. Uses SQLite (`backend/portfolio.db`) via SQLx.
- **Frontend**: React + Vite in `frontend/`.

## Commands

### Run Everything
From root:
```bash
./start.sh
```

### Backend
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
  so it needs no `portfolio.db`, no running server, and no OIDC configuration. The
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
- **Database**: Persistent storage is in `backend/portfolio.db`.
- **Metrics Engine**: `backend_rust/src/engines/stats_engine.rs` computes TWR, Volatility, Sharpe Ratio, and Portfolio Beta (vs SPY).
- **macOS Tip**: Use `127.0.0.1` instead of `localhost` for the API to avoid IPv6 resolution issues.

## Branching Workflow

- **Always create a new branch** for each feature, fix, or change. Never work on `main`.
- Name branches descriptively using the `type/scope` convention:
  - `feat/...` — new features or functionality
  - `fix/...` — bug fixes
  - `chore/...` — configuration, tooling, docs
  - `refactor/...` — code refactoring with no behavior change
- After finishing work, commit your changes. The user will create the PR and merge manually.

## Roadmap / Current Focus
- **Multi-Currency Support**: Implementing asset currency tracking and exchange rate conversion for tax and stats (see `docs/implementation_plan.md`).
