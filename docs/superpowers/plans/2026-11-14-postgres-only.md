# Postgres-only backend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace SQLite with Postgres as the only database engine, with the schema under versioned migrations and the data path unchanged for users.

**Architecture:** One engine, Postgres, with the schema in numbered `.sql` files compiled into the binary by `sqlx::migrate!`. The hand-written `init_db` and its `PRAGMA`-based startup migrations are deleted. Queries keep their `?`-free Postgres placeholders and move to compile-time-checked macros with a committed prepare cache. Existing SQLite data arrives through the backup bundle (#64), not through a converter.

**Tech Stack:** Rust 2024, Rocket 0.5, sqlx 0.8 (`postgres`), `rust_decimal` (unchanged), PostgreSQL 18.x (production 18.6 on CloudNativePG, local 18.7 via nix-shell, CI `postgres:18`).

**Spec:** `docs/superpowers/specs/2026-11-14-postgres-only-design.md` — every task below argues from it; §-references in this plan are that file's sections.

## Global Constraints

- **Postgres only.** No engine flag, no dialect abstraction, no `sqlite` feature in `Cargo.toml`. SQLite is a historical fact, not a supported engine (§2).
- **Money stays `TEXT`.** No `NUMERIC`/`DECIMAL` column in this work (§3).
- **Dates are native.** `TIMESTAMPTZ` for timestamps, `DATE` for `historical_prices.date` (§3).
- **Ids are `BIGINT GENERATED ALWAYS AS IDENTITY`, Rust `i64`.** `sqlx-postgres` refuses `int8`→`i32` (§3, §5.2).
- **Foreign keys are enforced.** Do not add `PRAGMA`-equivalent workarounds; do not add `ON DELETE CASCADE` (§4, §5.1).
- **Test output must be pristine** — no warnings, no ignored tests left behind.
- **Any task that changes a route regenerates `docs/openapi/openapi.json` and `frontend/src/types/api.d.ts` in the same commit** (`cd backend_rust && cargo run --bin backend_rust -- --print-openapi > ../docs/openapi/openapi.json`, then `cd frontend && npm run generate-types`).
- **Branch per feature; never work on `main`.** Commits are local; do not push or open a PR without explicit operator confirmation.
- **Commands run from `backend_rust/`** unless a step says otherwise.
- **Local database:** `bash scripts/dev-db.sh` (Task 1). `DATABASE_URL` must be exported in every shell that builds or tests.

## Review Focus

The spec's own tests will not cover these; each line gets a test in the task that owns the code.

1. **Orphan rows in pre-existing SQLite data.** A transaction whose `asset_id` no longer exists is invisible to the nested bundle and is silently dropped at cutover (§5.1). Expected behavior: the cutover audit reports the count and a non-zero count stops the cutover — Task 6.
2. **Two restores running at once under `SERIALIZABLE`.** Postgres answers `40001`, SQLite answered `SQLITE_BUSY`; the client must still see `503` with `Retry-After`, never a 500 (§5.4) — Task 7.
3. **A stale `.sqlx` prepare cache.** It compiles and lies: the query is checked against the schema as it was when the cache was generated (§8). Expected behavior: CI fails before compilation, naming the drifted query — Task 8, Task 9.
4. **A transaction date that is not a full RFC 3339 timestamp.** `TIMESTAMPTZ` will reject what `TEXT` accepted, and the CSV import path is where that arrives. Expected behavior: a `422` naming the row, not a 500 — Task 4.
5. **Pool exhaustion during a long export.** Export holds a connection for its whole duration (#64 follow-up 3); `statement_timeout` bounds any single statement inside that hold but cannot bound the hold itself, which is a sequence of fast statements — and the bundle caps do not bound it either, since `export::cap_error` checks them on the already-built bundle (§10). Expected behavior: `503` with `Retry-After` after `acquire_timeout`, not a hung request — Task 7.

---

### Task 1: The local database script

**Files:**
- Create: `backend_rust/scripts/dev-db.sh`
- Create: `backend_rust/scripts/dev-db-stop.sh`
- Modify: `AGENTS.md` (the *Commands → Backend* list only; the *Backend gates* section is Task 9's, and *Architecture* / *Database* are Task 3's)

**Interfaces:**
- Consumes: nothing.
- Produces: a running PostgreSQL 18.x on `127.0.0.1:55432`, database `prism`, role `postgres` with `CREATEDB`, and the exported `DATABASE_URL=postgres://postgres@127.0.0.1:55432/prism`. Every later task assumes these exact values.

- [ ] **Step 1: Write the start script**

```bash
#!/usr/bin/env bash
# Local PostgreSQL for development and tests. Postgres-only since the engine
# switch: there is no SQLite path to fall back to.
#
# nix-shell rather than a container because rootless podman cannot create a
# usable user namespace in this workspace (spec §9.2: the kernel refuses a
# multi-range uid_map even with the setuid helper installed). nixpkgs'
# postgresql_18 is 18.7; production is 18.6. Same major, and nothing in the
# schema depends on the difference.
set -euo pipefail
PORT="${PRISM_PG_PORT:-55432}"
DATA="${PRISM_PG_DIR:-${XDG_RUNTIME_DIR:-/tmp}/prism-pg}"

if [ -s "$DATA/PG_VERSION" ] && pg_ctl -D "$DATA" status >/dev/null 2>&1; then
  echo "already running on 127.0.0.1:$PORT (data: $DATA)"
else
  command -v initdb >/dev/null || { echo "initdb not on PATH — run inside: nix-shell -p postgresql_18 --run 'bash scripts/dev-db.sh'"; exit 1; }
  [ -s "$DATA/PG_VERSION" ] || { mkdir -p "$DATA"; initdb -D "$DATA" -U postgres -A trust --no-sync; }
  mkdir -p "$DATA/sock"
  pg_ctl -D "$DATA" -o "-p $PORT -k $DATA/sock -c listen_addresses=127.0.0.1" \
         -l "$DATA/log" -w start
fi

URI="postgres://postgres@127.0.0.1:$PORT/postgres"

# `command -v initdb` cannot tell the two toolchains apart: the image's 18.6 is on
# PATH and its server will open a cluster built by 18.7 (spec §9.1). Ask what is
# actually answering instead.
V=$(psql "$URI" -tAc 'show server_version')
case "$V" in
  18.*) ;;
  *) echo "need PostgreSQL 18.x, got $V — run inside: nix-shell -p postgresql_18 --run 'bash scripts/dev-db.sh'"; exit 1;;
esac
echo "server $V"

psql "$URI" -qc "alter role postgres createdb"
if [ "$(psql "$URI" -tAc "select 1 from pg_database where datname='prism'")" != "1" ]; then
  psql "$URI" -qc "create database prism"
fi

echo "DATABASE_URL=postgres://postgres@127.0.0.1:$PORT/prism"
```

- [ ] **Step 2: Write the stop script**

```bash
#!/usr/bin/env bash
set -euo pipefail
DATA="${PRISM_PG_DIR:-${XDG_RUNTIME_DIR:-/tmp}/prism-pg}"
pg_ctl -D "$DATA" status >/dev/null 2>&1 || exit 0
pg_ctl -D "$DATA" -m fast -w stop
```

- [ ] **Step 3: Make both executable and prove the cycle**

```bash
chmod +x backend_rust/scripts/dev-db.sh backend_rust/scripts/dev-db-stop.sh
nix-shell -p postgresql_18 --run 'bash backend_rust/scripts/dev-db.sh'
psql "postgres://postgres@127.0.0.1:55432/prism" -tAc 'select current_setting($$server_version$$)'
nix-shell -p postgresql_18 --run 'bash backend_rust/scripts/dev-db-stop.sh'
```

Expected: the start line prints `server 18.7` and `DATABASE_URL=...`, the stop exits 0, and a **second** stop prints nothing and exits 0. Run the start script **twice** — the second run must print `already running` rather than failing.

The version guard is not decoration: the `postgresql` baked into the workspace image is 18.6 and its `initdb` is broken (spec §9.1), and running this script without `nix-shell` against an existing cluster would otherwise start 18.6 on an 18.7 data directory in silence.

- [ ] **Step 4: Document it**

In `AGENTS.md`, under *Commands → Backend*, add:

```
- **Local database**: `nix-shell -p postgresql_18 --run 'bash scripts/dev-db.sh'`
  (from `backend_rust/`). Prints the `DATABASE_URL` to export. Stop with
  `scripts/dev-db-stop.sh`. There is no SQLite fallback: the engine is Postgres
  only, and the tests run against the same engine as production.
```

- [ ] **Step 5: Commit**

```bash
git add backend_rust/scripts/dev-db.sh backend_rust/scripts/dev-db-stop.sh AGENTS.md
git commit -m "chore(db): local postgres 18 harness for the postgres-only switch"
```

---

### Task 2: The schema as a migration

**Files:**
- Create: `backend_rust/migrations/0001_init.sql`
- Create: `backend_rust/scripts/check_migrations.sh`

**Interfaces:**
- Consumes: the running database from Task 1.
- Produces: `migrations/0001_init.sql`, applied by `sqlx migrate` in Task 3 and by `sqlx::migrate!` at runtime. Table and column names here are what every later query is written against.

Policy, from spec §7, because it is the kind of rule that gets broken by accident
later: `0001_init.sql` is the whole schema and there is no historical replay — the
data arrives as a bundle into a fresh database, so there is nothing to migrate
incrementally. Every change after this is a new numbered file. **Never edit a
shipped migration**; `_sqlx_migrations` is the ledger and the migrator creates it.

- [ ] **Step 1: Write the migration**

Copy the DDL from spec §4 verbatim, with one change to make after Step 3: the `split_sync` key.

```sql
-- 0001_init.sql
CREATE TABLE users (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    issuer        TEXT NOT NULL,
    subj          TEXT NOT NULL,
    email         TEXT,
    name          TEXT,
    is_active     BOOLEAN NOT NULL DEFAULT TRUE,
    token_version BIGINT NOT NULL DEFAULT 0,
    created_at    TIMESTAMPTZ NOT NULL,
    last_login_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX uq_users_issuer_subj ON users (issuer, subj);

CREATE TABLE portfolios (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    owner_id      BIGINT REFERENCES users(id),
    name          TEXT NOT NULL,
    description   TEXT,
    currency      TEXT NOT NULL DEFAULT 'USD',
    base_currency TEXT NOT NULL DEFAULT 'USD'
);
CREATE INDEX portfolios_owner_idx ON portfolios (owner_id);

CREATE TABLE assets (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    portfolio_id BIGINT NOT NULL REFERENCES portfolios(id),
    symbol       TEXT NOT NULL,
    name         TEXT NOT NULL,
    asset_type   TEXT NOT NULL,
    sector       TEXT,
    currency     TEXT NOT NULL DEFAULT 'USD',
    isin         TEXT
);
CREATE INDEX assets_portfolio_idx ON assets (portfolio_id);

CREATE TABLE transactions (
    id       BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    asset_id BIGINT NOT NULL REFERENCES assets(id),
    type     TEXT NOT NULL,
    quantity TEXT NOT NULL,
    price    TEXT NOT NULL,
    fee      TEXT NOT NULL,
    date     TIMESTAMPTZ NOT NULL
);
CREATE INDEX transactions_asset_date_idx ON transactions (asset_id, date);

-- Split dedupe, carried over from SQLite's partial unique index of the same name.
-- apply_split_events has no read-based check; the conflict IS the dedupe, so
-- without this index bug #50 (doubled split adjustments, corrupted cost basis)
-- returns silently. Expression-wrapped because date_trunc('day', …) is STABLE
-- and unindexable; AT TIME ZONE 'UTC' is IMMUTABLE. Key = the UTC calendar day.
CREATE UNIQUE INDEX uq_split_asset_date
    ON transactions (asset_id, ((date at time zone 'UTC')::date))
    WHERE type = 'SPLIT';

CREATE TABLE historical_prices (
    symbol      TEXT NOT NULL,
    date        DATE NOT NULL,
    close_price TEXT NOT NULL,
    PRIMARY KEY (symbol, date)
);

CREATE TABLE split_sync (
    asset_id       BIGINT NOT NULL REFERENCES assets(id),
    last_synced_at DATE NOT NULL,
    covered_from   DATE,
    PRIMARY KEY (asset_id)
);
```

- [ ] **Step 2: Confirm `split_sync`'s key against the code**

The spec flags this as the most reshaped table (§4 note). Read what the SQLite code actually leaves behind — `migrate_split_sync_key` and `enforce_split_uniqueness` in `backend_rust/src/lib.rs`, and the readers in `currency_service.rs`.

Resolved before this plan ran, on a workspace with no `portfolio.db` to inspect: the key **is `asset_id` alone**. Evidence: no `effective_date` column exists anywhere in the tree; `currency_service.rs:916` does a `fetch_optional` on `WHERE asset_id = ?`, which would raise `MultipleRowsFound` under a composite key; `transactions.rs:175` and `backup/restore.rs:262` delete by `asset_id` alone. Re-check that chain rather than trusting it blindly, and say in your report whether the deployed production database could still differ — it is the only real authority, and that check belongs in Task 10's cutover runbook.

Do not confuse this with `uq_split_asset_date`, which is on `transactions`, not `split_sync`; an earlier draft of the spec note conflated them.

- [ ] **Step 2b: Confirm the split-dedupe index is in the migration**

`uq_split_asset_date` is the one constraint the spec's §4 draft omitted and the app depends on. `apply_split_events` (`currency_service.rs:120-140`) inserts with `ON CONFLICT DO NOTHING` and has no read-based check of its own, so if the index is missing nothing conflicts and duplicated splits corrupt cost basis in silence. It must be in `0001_init.sql` — not a later migration: `0001` has been applied to nothing but your throwaway database, so §7's no-editing-shipped-migrations rule does not apply to it yet.

- [ ] **Step 3: Apply it to a throwaway database**

```bash
nix-shell -p postgresql_18 --run 'bash backend_rust/scripts/dev-db.sh'
psql "postgres://postgres@127.0.0.1:55432/postgres" -qc "create database prism_migrations"
nix-shell -p sqlx-cli --run 'sqlx migrate run --no-dotenv \
  --database-url "postgres://postgres@127.0.0.1:55432/prism_migrations" \
  --source backend_rust/migrations'
```

Measured against sqlx-cli 0.9, not copied from the first draft of this step: the flag is `--source`, **not** `--migrations`, and the CLI reads a `.env` unless you pass `--no-dotenv`. `createdb` is not usable here — the client in this workspace (18.6) has no `-d/--dbname`, so a URI in the first position is not accepted; create the database with `psql -qc "create database …"` instead.

Expected: `Applied 1/migrate init` (that is the real line sqlx-cli 0.9 prints, not `Applied 0001_init.sql`). If `sqlx` CLI is absent, `cargo install sqlx-cli --no-default-features --features postgres` once, or use `nix-shell -p sqlx-cli`.

- [ ] **Step 4: Write the shape gate**

A Rust test cannot do this yet: `Cargo.toml` still carries the `sqlite` feature
until Task 3, so `sqlx::PgConnection` does not exist. The gate is a shell script
over `psql`, which is why it is `scripts/check_migrations.sh` and not a test file.

```bash
#!/usr/bin/env bash
# The migration is the schema. This pins the column types the rest of the codebase
# is written against, so 0001_init.sql cannot drift silently into something the
# queries disagree with. Shell rather than a Rust test because it has to run
# during the engine switch, before sqlx's postgres feature exists.
set -euo pipefail
: "${DATABASE_URL:?run scripts/dev-db.sh and export DATABASE_URL}"
want() { # table column type
  local t
  t=$(psql "$DATABASE_URL" -tAc \
    "select data_type from information_schema.columns
      where table_schema='public' and table_name='$1' and column_name='$2'")
  [ "$t" = "$3" ] || { echo "FAIL $1.$2 is '$t', expected '$3'"; exit 1; }
  echo "ok   $1.$2 = $3"
}
want portfolios         id            bigint
want transactions       id            bigint
want transactions       date          "timestamp with time zone"
want transactions       price         text
want historical_prices  date          date
want historical_prices  close_price   text
want users              is_active     boolean

has_index() { # index name — presence only, so a renamed index still fails
  psql "$DATABASE_URL" -tAc \
    "select count(*) from pg_indexes
      where schemaname='public' and indexname='$1'" | tr -d '[:space:]'
}
[ "$(has_index uq_split_asset_date)" = "1" ] \
  || { echo "FAIL uq_split_asset_date missing — split dedupe is enforced by this index alone (bug #50)"; exit 1; }
echo "ok   uq_split_asset_date present"
echo "schema shape ok"
```

Identity is checked separately, and against `pg_attribute`, because `information_schema` reports `GENERATED BY DEFAULT AS IDENTITY` as an identity too — and `BY DEFAULT` accepts a supplied id, which is the exact property this check exists to protect:

```bash
psql "$DATABASE_URL" -tAc \
  "select attidentity from pg_attribute
     where attrelid='public.portfolios'::regclass and attname='id'" | grep -qx a \
  || { echo "FAIL portfolios.id must be GENERATED ALWAYS AS IDENTITY (attidentity 'a')"; exit 1; }
```

That matters: identity refuses a supplied id, which is the SQL expression of the bundle rule that ids are never trusted from the file.

The gate must also, and this is what makes it a gate rather than a smoke test:

- assert **every** column's type across all six tables, and NOT NULL on every column the migration declares `NOT NULL`. A seven-column sample let `transactions.price` become nullable without a word.
- assert `_sqlx_migrations` has exactly one row per `*.sql` in the migrations directory (default `$(dirname "$0")/../migrations`, overridable with `MIGRATIONS_DIR`), naming both counts when it fails. The gate inspects a database it did not build; this is what stops it going green against a stale schema.
- pass `-w` to `psql`, so a password prompt cannot hang a non-tty job, and fail loudly with `command -v psql` so a missing binary is reported as a missing binary.

- [ ] **Step 5: Run it, and run it once with a deliberate break**

```bash
DATABASE_URL="postgres://postgres@127.0.0.1:55432/prism_migrations" bash backend_rust/scripts/check_migrations.sh
```

Expected: a line of `ok` per column, `ok uq_split_asset_date present`, and `schema shape ok`. Then break it, on fresh databases, and record every failure with its real output.

The mutation this step originally prescribed — `date TIMESTAMPTZ` → `date TEXT` — **cannot produce the expected gate message**, and that is worth knowing rather than working around: `uq_split_asset_date` is an expression index over that column, so changing its type makes the migration itself refuse to apply (`function pg_catalog.timezone(unknown, text) does not exist`) and the gate never reaches its own verdict. It still fails closed, and the schema turns out to pin its own types for free. Use mutations that apply, and say so in the report:

1. `historical_prices.date DATE` → `TIMESTAMPTZ` (a column nothing else depends on) → expect the type `FAIL` line.
2. `transactions.price TEXT` → `NUMERIC(18,8)` → expect the money-is-TEXT `FAIL` line.
3. Drop `GENERATED ALWAYS AS IDENTITY` from one id → expect the identity `FAIL`.
4. Delete the `uq_split_asset_date` index → expect the index `FAIL`. This is the mutation that matters most: a gate that checked only tables and column types would have shipped the schema that reopens bug #50.
5. Weaken the index three ways — make it non-unique, drop the `WHERE type = 'SPLIT'` predicate, and change the expression off the UTC day — and confirm each is caught separately. The expression break must be `ON transactions (asset_id, (date AT TIME ZONE 'UTC'))` with the `::date` removed: still IMMUTABLE, still applies, and it reaches the expression assertion. Keying the break off `transactions.date` does not work — the type check fails first and the index check never runs.
6. Change one id column to `GENERATED BY DEFAULT AS IDENTITY` → expect the identity `FAIL`. This is the mutation the first draft of this step could not detect.

Revert the migration after each. A gate that has never been seen to fail is not a gate.

- [ ] **Step 6: Commit**

```bash
git add backend_rust/migrations backend_rust/scripts/check_migrations.sh
git commit -m "feat(db): postgres schema as migration 0001, with a shape gate"
```

---

### Task 3: The engine switch

This is one atomic change: `sqlx`'s `sqlite` and `postgres` features cannot both serve the same `Pool` type, so the crate does not compile until every reference moves. It is the largest task in the plan and it is deliberately not split — a half-converted tree that compiles is worse than one commit that is reviewed as a whole.

**Files:**
- Modify: `backend_rust/Cargo.toml:9`
- Create: `backend_rust/src/db.rs`
- Modify: `backend_rust/src/lib.rs` (delete `init_db` and the startup migrations, `:138-560`)
- Modify: `backend_rust/src/main.rs:53-88`
- Modify: `AGENTS.md` (the *Architecture* line and the *Database* bullet under *Key Context*)
- Modify: the 13 files that reference `SqlitePool` — `access.rs`, `api_routes/{analytics,portfolios,transactions}.rs`, `auth/{guard,mod,oidc}.rs`, `backup/{export,restore}.rs`, `engines/{stats_engine,tax_engine}.rs`, `lib.rs`, `services/currency_service.rs`
- Modify: `backend_rust/src/engines/stats_engine.rs:433,539`, `backend_rust/src/services/currency_service.rs:997,1469` (`INSERT OR REPLACE`)
- Test: the existing suite, unchanged in intent

**Interfaces:**
- Consumes: `migrations/0001_init.sql` (Task 2).
- Produces: `pub type DbPool = sqlx::PgPool;` and `pub async fn connect(url: &str) -> Result<DbPool, sqlx::Error>;` in `src/db.rs`. Every later task writes `DbPool`, never `PgPool` directly, so a future change stays one file.

- [ ] **Step 1: Switch the dependency and add the alias module**

`Cargo.toml:9` becomes:

```toml
sqlx = { version = "0.8", features = ["runtime-tokio", "postgres", "chrono", "migrate"] }
```

`src/db.rs`:

```rust
//! The single place the engine is named. Everything else takes `DbPool`.
use sqlx::postgres::{PgPoolOptions, PgPoolingOptions};
use sqlx::Pool;

pub type DbPool = Pool<sqlx::postgres::Postgres>;

/// Build the pool the way `main` does. `acquire_timeout` is what a saturated pool
/// turns into: a 503 with Retry-After, not a hung request (spec §5.4, §10).
pub async fn connect(url: &str) -> Result<DbPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(5)
        .min_connections(0)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .max_lifetime(std::time::Duration::from_secs(4 * 60 * 60))
        .connect(url)
        .await
}
```

Add `pub mod db;` to `src/lib.rs`.

- [ ] **Step 2: Rename the pool type everywhere**

```bash
cd backend_rust
grep -rl "SqlitePool" src | xargs sed -i 's/sqlx::sqlite::SqlitePool/backend_rust::db::DbPool/g; s/\bSqlitePool\b/DbPool/g'
grep -rn "SqlitePool" src | wc -l   # expect 0
```

Inside `src/` itself the `backend_rust::` prefix is wrong; fix those to `crate::db::DbPool`. Check with `cargo check` and correct the handful of paths it names.

- [ ] **Step 3: Convert placeholders statement by statement**

`?` is a runtime error under the Postgres driver, not a compile error, so this step is verified by the tests, not the compiler. Convert every `?` to `$n` by its position **within one statement**:

```bash
python3 - <<'PY'
import re, pathlib
# Convert `?` to $n per SQL statement. A statement is one string literal passed to
# sqlx::query / query_as; placeholders never span statements, so numbering restarts
# at each literal. Comments are stripped first so a `?` in prose is not counted.
pat = re.compile(r'(sqlx::query(?:_as)?(?:::<[^>]*>)?\s*\(\s*)("(?:[^"\\]|\\.)*")', re.S)
def fix(m):
    head, lit = m.group(1), m.group(2)
    n = 0
    out, i = [], 0
    while i < len(lit):
        if lit[i] == '?' and not lit[i-1] == '\\':
            n += 1; out.append(f'${n}')
        else:
            out.append(lit[i])
        i += 1
    return head + ''.join(out)
for p in pathlib.Path('src').rglob('*.rs'):
    s = p.read_text()
    new = pat.sub(fix, s)
    if new != s:
        p.write_text(new)
        print(p)
PY
grep -rn 'sqlx::query' src | grep -c '?'   # expect 0 outside comments
```

Then read every hit `grep -n '?' src/**/*.rs` still reports and confirm each is prose in a comment, not a live placeholder. The script cannot know the difference; you can.

- [ ] **Step 4: Replace `INSERT OR REPLACE`**

`stats_engine.rs:433` and `:539` become:

```rust
sqlx::query(
    "INSERT INTO historical_prices (symbol, date, close_price) VALUES ($1, $2, $3) \
     ON CONFLICT (symbol, date) DO UPDATE SET close_price = EXCLUDED.close_price",
)
```

`currency_service.rs:997` becomes (this `ON CONFLICT` target was a guess in the first draft of this plan; Task 2 confirmed `PRIMARY KEY (asset_id)` against the readers and the delete sites):

```rust
"INSERT INTO split_sync (asset_id, last_synced_at, covered_from) VALUES ($1, $2, $3) \
 ON CONFLICT (asset_id) DO UPDATE SET last_synced_at = EXCLUDED.last_synced_at, \
                                      covered_from   = EXCLUDED.covered_from"
```

`currency_service.rs:1469` becomes the same with `covered_from` left out of both the column list and the `DO UPDATE` — leaving it in the update clause would null out a value the two-column call does not know about, which is exactly the silent-drop behaviour `INSERT OR REPLACE` had and this port exists to remove.

- [ ] **Step 5: Delete `init_db` and wire the migrator**

Delete `init_db`, `column_exists`, `migrate_decimal_columns`, `migrate_split_key`, `migrate_split_sync_key`, `enforce_price_uniqueness`, `enforce_split_uniqueness`, `add_owner_id_column` from `src/lib.rs` (`:138-560`). In `main.rs`, replace the `SqliteConnectOptions` block, the `busy_timeout` commentary, the WAL/restic commentary and the `init_db` call with:

```rust
let database_url = std::env::var("DATABASE_URL")
    .ok()
    .filter(|s| !s.trim().is_empty())
    .unwrap_or_else(|| refuse(anyhow::anyhow!(
        "DATABASE_URL is required. Postgres-only: there is no SQLite fallback. \
         Local: nix-shell -p postgresql_18 --run 'bash scripts/dev-db.sh'"
    )));

let pool = db::connect(&database_url)
    .await
    .map_err(|e| refuse(anyhow::anyhow!("cannot connect to DATABASE_URL: {e}")))?;

// Migrations run on one connection, before the pool serves anything else. This
// replaces init_db, whose PRAGMA table_info guards were a hand-rolled migrator
// and the source of the split_sync bug documented at lib.rs:513-519.
sqlx::migrate!("./migrations")
    .run(&pool)
    .await
    .map_err(|e| refuse(anyhow::anyhow!("migrations failed: {e}")))?;
```

`refuse` returns `ExitCode`, so adjust each call site to `return Err(...)`/`std::process::exit` to match the existing `main` shape rather than inventing a new one.

**The bootstrap owner step survives — the first draft of this step omitted it, and the implementer caught it.** Deleting `init_db` deletes the only creator of the placeholder user (`issuer='local'`, `subj='bootstrap'`) and of `backfill_ownership`, and three things depend on them: `auth/mod.rs::finalise_auth` in `AUTH_MODE=disabled` looks that row up and bails if it is absent (spec §10 requires `AUTH_MODE=disabled` to keep working), `tests/common::setup_db` relies on it because `DEFAULT_USER = 1` **is** that row across 16 test files, and `tests/startup_test.rs` asserts the orphan-refusal rule through it. Keep the identity half as an explicit post-migration step:

```rust
// The bootstrap owner is data, not schema, so it cannot live in a migration
// (and 0001 is frozen). Idempotent, and a no-op on a fresh database.
db::ensure_bootstrap_owner(&pool, &cfg)
    .await
    .map_err(|e| refuse(anyhow::anyhow!("bootstrap owner step failed: {e}")))?;
```

Conditions on it: it runs **after** `sqlx::migrate!`, never before; it stays idempotent; it preserves the refusal rule exactly (orphans present and no bootstrap owner means refuse to start — not a warning); it is quiet when it changes nothing, because it runs on every boot; and its doc comment names both duties and the three dependents, because the next reader will ask why a Postgres app has a bootstrap row at all. The provisional `test_pool(caller)` calls it after the `TRUNCATE … RESTART IDENTITY CASCADE`, which is what makes `DEFAULT_USER = 1` come back as 1 on every test. `startup_test.rs` points at the renamed function, and at least one assertion proves the placeholder row exists after a normal startup.

- [ ] **Step 6: Point the test harness at Postgres — per-test databases, not a shared one**

`tests/common/mod.rs:137` currently does `SqlitePool::connect(":memory:")`, and every `src/` unit test module gets its isolation the same way. A single shared database truncated between tests **cannot work**: `cargo test` runs the tests inside one binary on parallel threads, so one test's `TRUNCATE … RESTART IDENTITY CASCADE` lands while another is mid-flight and its seeded rows vanish. That is a suite that passes by scheduling luck, which is worse than one that fails.

So the isolation Task 5 was going to introduce arrives here, because the engine switch is what destroyed it. `test_pool(caller: &str)`:

1. Creates `prism_t_<pid>_<caller>` and migrates it, per test. A `prism_template` database was tried first on the assumption that migrating ~470 times would dominate the suite; it does not. `0001_init.sql` takes ~15 ms, and the measured suite is **faster without a template** (119 s per-test-migration vs 130 s template, against a 114 s SQLite baseline). The template also cost a global sweep of every `prism_t_%` database at process start, which made two concurrent `cargo test` runs destroy each other. The measurement this step asked for was made, it came back the other way, and per the instruction written here the measurement wins.
2. Refuses a duplicate `caller`. Keep a process-wide registry (`Mutex<HashSet<String>>`) and panic naming both callers. Drop-if-exists plus a repeated caller name is the same interference, just rarer, and a silent cross-test data leak is the one outcome this helper exists to make impossible.
3. `TRUNCATE … RESTART IDENTITY CASCADE`, then `db::ensure_bootstrap_owner(&pool, &cfg)`, which is what makes `DEFAULT_USER = 1` come back as 1 on every test. `RESTART IDENTITY` is load-bearing: without it the identity sequences climb across tests and any test hard-coding an id fails only when it runs second.
4. Drops its database when the test finishes: the next `create_database` in the process drops any of its own databases that now has zero backends, using `DROP DATABASE … WITH (FORCE)` with retries. A run that is killed mid-suite leaves its last few behind, and the first `create_database` of a later run reaps any harness database whose embedded pid is gone from `/proc` **and** which has zero backends, printing one line about what it took. Drop-if-exists at creation still covers a crashed run poisoning the next.

Give the same helper to the `#[cfg(test)]` modules inside `src/` as `#[cfg(test)] pub(crate)` in `src/db.rs`: `access.rs`, `engines/stats_engine.rs`, `engines/tax_engine.rs` and `services/currency_service.rs` all relied on `:memory:` and Task 5 only touches `tests/`. Check `currency_service.rs`'s split tests specifically — they hard-code `asset_id = 1` and are the ones most likely to fail confusingly rather than obviously.

Keep the signature `test_pool(caller: &str)` exactly; Task 5 builds on it.

- [ ] **Step 7: Make the suite pass**

```bash
nix-shell -p postgresql_18 --run 'bash backend_rust/scripts/dev-db.sh'
cd backend_rust && DATABASE_URL="postgres://postgres@127.0.0.1:55432/prism" cargo test
```

Expected: the suite runs against Postgres. Expect failures in three families, and fix each at its cause rather than by editing the assertion: `i32`/`i64` mismatches, date parsing, and `DEFAULT_USER` assumptions from the SQLite `id = 1` placeholder row.

On the id mismatches: **any `query_as` / `query_scalar` that binds an id to `i32` is a runtime error under `sqlx-postgres`, not a compile error** — an `int8` column will not decode into `i32`. Those row reads belong to this task, because this task cannot finish green with them in place. Task 4 keeps the *API surface*: `schemas.rs`, path parameters, and the test seed helpers. Fix the reads here; do not paper over one with a cast.

Three traps in the same family, all found by reviewing the migration against the code rather than by compiling:

- `users.is_active` is `BOOLEAN` in Postgres and `i64` in SQLite. `auth/guard.rs:82` reads it as `i64` in position 5 of `UserRow`; that becomes `bool` here, and every use of it follows.
- `users.token_version` is `BIGINT` precisely because `guard.rs:31`, `:82`, `:119` and `oidc.rs:236`, `:275`, `:316` read it as `i64`. If you find yourself widening it in Rust instead, stop — the column is the thing that was wrong.
- `split_sync.last_synced_at` is `DATE` in Postgres, and **`String` is not a decodable target for a Postgres `DATE`** (`sqlx-postgres-0.8.6/src/types/str.rs:14-24` allows TEXT/NAME/BPCHAR/VARCHAR/UNKNOWN/citext and nothing else). So the column change alone does not make the current code run: `currency_service.rs:916` and `api_routes/portfolios.rs:71,91` must decode `NaiveDate` and format `%Y-%m-%d` explicitly at the API boundary. The failure mode is a loud mismatched-types error, not the silent parse failure an earlier draft of §4 predicted — same fix, better signal.
- `currency_service.rs:919` computes `SELECT MIN(date(date))`. Rewriting that to any expression other than `((date at time zone 'UTC')::date)` — the identical expression `uq_split_asset_date` is built on — means the index stops being used *and* the split dedupe day boundary drifts away from the constraint enforcing it. Match the index expression exactly.

- [ ] **Step 8: Correct the parts of AGENTS.md that describe SQLite**

Task 1 added a *Commands* bullet saying the engine is Postgres only, which left `AGENTS.md` contradicting itself: *Architecture* still says "Uses SQLite" and the *Database* bullet under *Key Context* still documents `sqlite:../portfolio.db?mode=rwc`. This is the task that makes those lines false, so this is the task that rewrites them. Say what the code now does — `DATABASE_URL` is required and names a Postgres DSN, the schema comes from `migrations/`, and there is no `portfolio.db` — and delete the macOS `127.0.0.1`-versus-`localhost` note if it was only ever about the SQLite file path.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "feat(db): switch the engine from sqlite to postgres

Cargo feature, the pool type behind src/db.rs, \$n placeholders, INSERT OR
REPLACE to ON CONFLICT, init_db deleted in favour of sqlx::migrate!,
DATABASE_URL required. One commit because the crate cannot compile half
way."
```

---

### Task 4: `i64` ids and date parsing at the boundary

**Files:**
- Modify: `backend_rust/src/schemas.rs` (every `pub id: i32`, `portfolio_id`, `asset_id`, `owner_id`)
- Modify: `backend_rust/src/api_routes/*.rs` (path parameters `web::Path<i32>` → `web::Path<i64>`)
- Modify: `backend_rust/src/api_routes/portfolios.rs:66,85,106` (`split_sync_age`, `split_sync_ages`, `fetch_assets_for_portfolio`)
- Modify: `backend_rust/src/backup/{export,restore}.rs`
- Modify: `backend_rust/src/engines/{stats_engine,tax_engine}.rs`
- Test: `backend_rust/tests/date_boundary_test.rs` (create)

**Interfaces:**
- Consumes: Task 3's `DbPool`.
- Produces: `i64` ids in every public struct in `schemas.rs`. **The JSON is unchanged but the OpenAPI is not** — `format: int32` becomes `format: int64` for every field that moves, exactly as Task 3 discovered when five fields flipped without anyone regenerating the artifacts. Regenerate `docs/openapi/openapi.json` and `frontend/src/types/api.d.ts` in the same commit and show the diff; Step 6 proves the *JSON* is unchanged, not the spec file.

- [ ] **Step 1: Write the failing date test**

`tests/date_boundary_test.rs`:

```rust
//! `transactions.date` became TIMESTAMPTZ. What TEXT accepted, a real date
//! column can reject — and the CSV import is where odd shapes arrive. The
//! expected behaviour is a 422 naming the row, never a 500.
use backend_rust::schemas::TransactionIn;

#[test]
fn a_date_only_string_is_rejected_before_it_reaches_the_database() {
    // serde parses "2026-11-14" into a DateTime<Utc>? Decide by running it, then
    // pin the answer here either way — this test exists to make the answer
    // explicit rather than to assert a guess.
    let parsed = serde_json::from_str::<TransactionIn>(
        r#"{"asset_id":1,"type":"BUY","quantity":"1","price":"1.0","fee":"0","date":"2026-11-14"}"#,
    );
    assert!(parsed.is_err(), "a bare date must not silently become midnight UTC");
}

#[test]
fn a_full_rfc3339_timestamp_is_accepted() {
    let ok = serde_json::from_str::<TransactionIn>(
        r#"{"asset_id":1,"type":"BUY","quantity":"1","price":"1.0","fee":"0","date":"2026-11-14T00:00:00Z"}"#,
    );
    assert!(ok.is_ok(), "{:?}", ok.err());
}
```

- [ ] **Step 2: Run it and read the answer**

```bash
cd backend_rust && DATABASE_URL="postgres://postgres@127.0.0.1:55432/prism" cargo test --test date_boundary_test
```

If the first test fails because serde *does* accept `2026-11-14`, that is a decision to make explicitly, not to inherit silently: either reject date-only input at the API (add a `DateTime<Utc>` deserializer that requires a time component) or accept it and document that a bare date means midnight UTC. Record the choice in the commit message and in `docs/`.

- [ ] **Step 3: Widen the ids**

```bash
cd backend_rust
grep -rn "i32" src/schemas.rs | grep -iE "id|portfolio|asset|owner"
```

Change each to `i64`. Then the path parameters:

```bash
grep -rn "Path<i32>\|web::Path<i32>" src/api_routes
```

Then the row reads that the whole-branch review of #64 flagged — `query_as::<_, (i32,)>` against `RETURNING id`, which `sqlx-postgres` refuses because `int8` is not `i32`:

```bash
grep -rn "query_as::<_, (i32" src
grep -rn "(i32,)" src
```

- [ ] **Step 4: Fix what the compiler reports**

```bash
cd backend_rust && cargo check --all-targets 2>&1 | grep "^error" | head -20
```

Work the list. Expect `HashMap<i32, _>` in `split_sync_ages`, the `DEFAULT_USER` constant in `tests/common/mod.rs`, and the seed helpers' signatures.

- [ ] **Step 5: Run the suite**

```bash
cd backend_rust && DATABASE_URL="postgres://postgres@127.0.0.1:55432/prism" cargo test
```

Expected: PASS, no warnings.

- [ ] **Step 6: Prove the generated artifacts did not change**

```bash
cd backend_rust && cargo run --bin backend_rust -- --print-openapi > ../docs/openapi/openapi.json
cd ../frontend && npm run generate-types
cd .. && git status --porcelain docs/openapi/openapi.json frontend/src/types/api.d.ts
```

Expected: no output — `i64` and `i32` are both `integer` in JSON Schema. If there *is* a diff, read it before committing; a format change here would be a frontend-visible change and needs saying so.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor(db): ids are i64 to match int8, and the date boundary is pinned by tests"
```

---

### Task 5: Per-test databases

**Scope note — this task shrank.** Task 3 discovered that a shared database cannot work at all: `cargo test` parallelises inside a binary, so truncating between tests deletes rows another test is using. Per-test databases therefore arrived in Task 3, together with the template-database trick, the duplicate-`caller` guard, and the `src/` unit-test half (`access.rs`, `stats_engine.rs`, `tax_engine.rs`, `currency_service.rs`), which this task could not have reached because it only touches `tests/`. What is left here is consolidation, not invention: fold the duplicated helper into one place, convert the remaining `tests/*.rs` call sites, retire `setup_db(&cfg)` and `DEFAULT_USER`, and prove isolation rather than assert it. The `CREATE DATABASE` sketch below is superseded by Task 3's shipped harness — per-test freshly migrated databases, no template (see Step 6 above for the measurement), and keep the uniqueness guard.

**Files:**
- Modify: `backend_rust/tests/common/mod.rs` (`test_pool`, `seed_*`)
- Modify: every `tests/*.rs` that still calls `setup_db` or reads `DEFAULT_USER`
- Possibly modify: `backend_rust/src/test_db.rs`, if the `seed_*` helpers move there so `src/` unit tests and `tests/` share one copy

**Interfaces:**
- Consumes: `DATABASE_URL` with a role that has `CREATEDB` (Task 1 grants it).
- Produces: `pub async fn test_pool(caller: &str) -> DbPool` returning a **private, freshly migrated** database per test, named `prism_t_<pid>_<caller>` and dropped at the end of the test.

- [ ] **Step 1: Consolidate onto the shipped harness**

Task 3 shipped the real harness in `src/test_db.rs` (`test_pool(caller)`, the duplicate-`caller` panic, the crash sweep) and already converted the `src/` unit tests. What is left is deletion, not invention: fold `tests/common/mod.rs` onto it, retire `setup_db(&cfg)` and `DEFAULT_USER`, and move the `seed_*` helpers so they take the per-test pool. The `CREATE DATABASE` sketch that was here is gone — it predated Task 3 and would have replaced a working harness with a worse one.

- [ ] **Step 2: Convert the suite file by file**

Task 3 already converted most of `tests/`; finish the job. For each file still using `setup_db` or `DEFAULT_USER`: switch to `test_pool("<file>")` and the id returned by `seed_user`. Run each file alone before moving on:

```bash
DATABASE_URL="postgres://postgres@127.0.0.1:55432/prism" cargo test --test portfolios_test
```

Do not convert two files at once. A failure in a file you did not just touch is a leak from the one you did.

- [ ] **Step 3: Prove isolation**

Add to `tests/common/mod.rs`:

```rust
#[tokio::test]
async fn two_tests_do_not_see_each_others_rows() {
    let a = test_pool("iso_a").await;
    let b = test_pool("iso_b").await;
    seed_portfolio(&a, 1, "only in a", "USD").await;
    let n: (i64,) = sqlx::query_as("SELECT count(*) FROM portfolios").fetch_one(&b).await.unwrap();
    assert_eq!(n.0, 0, "test databases are not isolated");
}
```

The literal `1` is the `DEFAULT_USER` this task is retiring — use the id `seed_user` returns, or the bootstrap owner id from `ensure_bootstrap_owner`, and say which.

- [ ] **Step 4: Full suite, twice**

```bash
cd backend_rust && DATABASE_URL="postgres://postgres@127.0.0.1:55432/prism" cargo test
DATABASE_URL="postgres://postgres@127.0.0.1:55432/prism" cargo test
```

Expected: PASS both times. The second run catches a test that left a database behind and a later test reusing it.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "test(db): one migrated database per test instead of a shared sqlite cache"
```

---

### Task 6: Foreign keys, and the orphan audit

Scope note, because the spec's first draft got this wrong: `sqlx-sqlite` sets `foreign_keys=ON` on every connection by default, so `assets.portfolio_id` and `transactions.asset_id` are **already** enforced and are not behaviour changes. The two genuinely new constraints are `portfolios.owner_id → users(id)` (SQLite added the column by a bare `ALTER`, no FK) and `split_sync.asset_id → assets(id)`. Audit all four relationships anyway — the two already-enforced ones should come back clean, and that is worth confirming rather than assuming — but expect the findings to be about `owner_id`.

**Files:**
- Create: `backend_rust/tests/foreign_keys_test.rs`
- Create: `backend_rust/scripts/pre_cutover_audit.sh`
- Modify: `docs/backup-walkthrough.md` (a new section: what to run before an export)

**Interfaces:**
- Consumes: Task 5's `test_pool`.
- Produces: `scripts/pre_cutover_audit.sh`, the stop condition in the cutover runbook (Task 10).

- [ ] **Step 1: Write the failing FK tests**

```rust
//! SQLite never set `PRAGMA foreign_keys`, so every FOREIGN KEY in init_db was
//! decorative. Postgres enforces them. These tests pin the new behaviour before
//! anything in production depends on it — spec §5.1.
#[tokio::test]
async fn a_transaction_for_a_missing_asset_is_refused() {
    let pool = common::test_pool("fk_orphan").await;
    let err = sqlx::query(
        "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) \
         VALUES ($1, 'BUY', '1', '1', '0', '2026-11-14T00:00:00Z')",
    )
    .bind(999_999_i64)
    .execute(&pool)
    .await
    .expect_err("an orphan transaction must not insert");
    assert!(format!("{err:?}").contains("foreign key"), "{err:?}");
}

#[tokio::test]
async fn a_portfolio_with_assets_cannot_be_deleted_underneath_them() {
    let pool = common::test_pool("fk_delete").await;
    let pid = common::seed_portfolio(&pool, 1, "keep", "USD").await;
    common::seed_asset(&pool, pid, "VTI", "Vanguard").await;
    let err = sqlx::query("DELETE FROM portfolios WHERE id = $1")
        .bind(pid)
        .execute(&pool)
        .await
        .expect_err("cascade was deliberately not added; the app deletes in order");
    assert!(format!("{err:?}").contains("foreign key"), "{err:?}");
}
```

- [ ] **Step 2: Run them**

```bash
cd backend_rust && DATABASE_URL="postgres://postgres@127.0.0.1:55432/prism" cargo test --test foreign_keys_test
```

Expected: PASS. If the first fails, the FK is missing from `0001_init.sql` — fix the migration, do not weaken the test.

- [ ] **Step 3: Write the audit script**

`scripts/pre_cutover_audit.sh` runs against the **SQLite** file, because that is where the orphans are:

```bash
#!/usr/bin/env bash
# Pre-cutover audit. Run against the live SQLite database BEFORE the export.
#
# The bundle is nested (portfolio -> assets -> transactions), so a row whose
# parent is gone is not under any parent and is therefore never exported. Nothing
# in the export path can report it, because from the export's point of view those
# rows do not exist. This script is the only thing that sees them. Spec §5.1.
set -euo pipefail
DB="${1:?usage: pre_cutover_audit.sh /path/to/portfolio.db}"
command -v sqlite3 >/dev/null || { echo "sqlite3 required to audit the source database"; exit 1; }

q() { sqlite3 "$DB" "$1"; }
echo "orphan transactions : $(q "select count(*) from transactions t left join assets a on a.id=t.asset_id where a.id is null")"
echo "orphan assets       : $(q "select count(*) from assets a left join portfolios p on p.id=a.portfolio_id where p.id is null")"
echo "portfolios, no owner: $(q "select count(*) from portfolios p left join users u on u.id=p.owner_id where p.owner_id is not null and u.id is null")"
echo "price rows          : $(q "select count(*) from historical_prices")"
echo "transactions        : $(q "select count(*) from transactions")"
```

- [ ] **Step 4: Prove it finds an orphan**

```bash
cp portfolio.db /tmp/audit.db && sqlite3 /tmp/audit.db "insert into transactions (asset_id,type,quantity,price,fee,date) values (999999,'BUY','1','1','0','2026-11-14T00:00:00Z')"
bash backend_rust/scripts/pre_cutover_audit.sh /tmp/audit.db
```

Expected: `orphan transactions : 1`. A non-zero line is the stop condition; say so in the script's header comment, which Step 3 already does.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "test(db): foreign keys are enforced, and the orphan audit that guards the cutover"
```

---

### Task 7: Locks, timeouts, and the 503

**Files:**
- Modify: `backend_rust/src/api_routes/backup.rs` (the busy → `503` mapping)
- Modify: `backend_rust/src/backup/restore.rs` (extended busy-code classification deleted)
- Modify: `backend_rust/src/db.rs` (`statement_timeout`)
- Test: `backend_rust/tests/restore_concurrency_test.rs` (create)

**Interfaces:**
- Consumes: Task 3's `db::connect`.
- Produces: the rule `pool timeout | statement timeout | serialization failure → 503 + Retry-After`, and nothing else maps to 503.

- [ ] **Step 1: Delete the SQLite busy classification**

Remove the extended-busy-code matching added in #64 (`7facddb`) and the `busy_timeout` references. Replace the error mapping with:

```rust
// Postgres has no SQLITE_BUSY. Three conditions mean "try later" and they are the
// only things that produce a 503 here (spec §5.4):
//   * the pool had no connection within acquire_timeout
//   * statement_timeout fired
//   * SERIALIZABLE gave up after the retry policy
// Everything else is a 500, because it is a bug and a retry will not help.
fn is_retryable(err: &sqlx::Error) -> bool {
    match err {
        sqlx::Error::PoolTimedOut => true,
        sqlx::Error::Database(db) => matches!(
            db.code().as_deref(),
            Some("40001") | Some("40P01") | Some("55P03") // serialization, deadlock, lock wait
        ),
        _ => false,
    }
}
```

- [ ] **Step 2: Set `statement_timeout` on the connection**

In `db::connect`, add:

```rust
        // A runaway analytics query must not pin a pool connection forever; export
        // holds one for its whole duration (#64 follow-up 3) and this is the backstop.
        .after_connect(|conn, _meta| Box::pin(async move {
            sqlx::query("SET statement_timeout = '30s'").execute(conn).await.map(|_| ())
        }))
```

- [ ] **Step 3: Write the concurrency test the SQLite engine never allowed**

```rust
//! Two restores at once. Under SQLite the second waited on a file lock and then
//! got SQLITE_BUSY; under SERIALIZABLE Postgres aborts one with 40001. The client
//! contract is the same either way: 503 with Retry-After, never a 500.
use rocket::http::Status;

#[tokio::test]
async fn a_concurrent_restore_gets_503_not_500() {
    let (client, pool) = common::client().await;
    let bundle = common::bundle_with_one_portfolio(&pool).await;

    // Two restores, same bundle, fired together. `POST /api/backup/restore` in
    // copy mode takes a SERIALIZABLE transaction, so one of them must abort.
    let (a, b) = tokio::join!(
        common::post_restore(&client, &bundle),
        common::post_restore(&client, &bundle),
    );
    let mut codes = [a.0, b.0];
    codes.sort();

    // Exactly one success. If BOTH are 2xx the restore is not SERIALIZABLE, and
    // that is a finding about the engine switch, not a test to relax.
    assert_eq!(codes[1], Status::Ok, "second restore must not also succeed: {codes:?}");
    assert!(
        matches!(codes[0], Status::ServiceUnavailable),
        "the loser must be 503, got {:?} — a 500 here means the new error \
         mapping in is_retryable missed the sqlx error variant Postgres produced",
        codes[0]
    );
}
```

`common::post_restore` returns `(Status, HeaderMap)`; add it next to the existing
restore helpers in `tests/common/mod.rs` rather than duplicating the request
building. Assert `Retry-After` on the 503 in the same test — the header is part
of the contract #64 shipped and the remapping must not silently drop it.

- [ ] **Step 4: Pin the pool-timeout half of the rule**

Review Focus 5 is a saturated pool, which is hard to provoke end to end and easy
to pin at the mapping:

```rust
#[test]
fn pool_timeout_is_retryable_but_a_syntax_error_is_not() {
    // PoolTimedOut carries no database error code, so it is matched by variant.
    assert!(is_retryable(&sqlx::Error::PoolTimedOut));
    // A real database error with an unrelated code must NOT be retried: a 42601
    // syntax error retried forever is an outage with a Retry-After header on it.
    let err = sqlx::postgres::PgDatabaseError::default(); // if not constructible,
    // drive the mapping through a query that fails (SELECT from a missing table)
    // and assert is_retryable(&err) is false on the resulting sqlx::Error.
    assert!(!is_retryable(&err));
}
```

If `PgDatabaseError` is not constructible in sqlx 0.8 — it may not be — use the
comment's fallback and drive a real failing query. The point is that the false
case is exercised, not that the test is elegant.

- [ ] **Step 5: Run the suite and the gate scripts**

```bash
cd backend_rust && DATABASE_URL="postgres://postgres@127.0.0.1:55432/prism" cargo test
bash scripts/check_backup_portability.sh
```

Expected: PASS. The portability gate must still pass — it bans engine-specific constructs, and `SET statement_timeout` is portable enough to be allowed, but if the gate objects, fix the gate deliberately and say why in the commit message.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "fix(db): 503 means pool timeout, statement timeout, or serialization failure"
```

---

### Task 8: Compile-time checked queries and the prepare cache

**Files:**
- Create: `backend_rust/.sqlx/` (generated, committed)
- Modify: all 13 query files — `sqlx::query` → `sqlx::query!`, `query_as::<_, T>` → `sqlx::query_as!`
- Create: `backend_rust/scripts/check_sqlx_cache.sh`

**Interfaces:**
- Consumes: a database at head (Task 1 + Task 2 migrations).
- Produces: `.sqlx/` and the rule that `cargo sqlx prepare --check` runs before `cargo build` in CI.

- [ ] **Step 1: Convert one file and see the payoff**

Start with `src/engines/stats_engine.rs`, the heaviest SQL user. Convert its queries to macros and run:

```bash
cd backend_rust
DATABASE_URL="postgres://postgres@127.0.0.1:55432/prism" cargo check
```

Expect the compiler to reject at least the `i32`-vs-`int8` shapes this port is designed to catch. Fix them at the query, not by casting the column.

- [ ] **Step 2: Convert the remaining files, one per commit**

Order: `tax_engine.rs`, `api_routes/portfolios.rs`, `api_routes/transactions.rs`, `api_routes/analytics.rs`, `api_routes/backup.rs`, `backup/export.rs`, `backup/restore.rs`, `auth/guard.rs`, `auth/mod.rs`, `auth/oidc.rs`, `access.rs`, `services/currency_service.rs`, `lib.rs`.

Where a query is built with `format!` (the `init_db` leftovers and any dynamic table names), the macro cannot take it; leave those as runtime `sqlx::query` and add a `// macro-exempt: dynamic table name` comment so the count of remaining runtime queries is greppable and can go to zero deliberately rather than by accident.

- [ ] **Step 3: Generate and commit the cache**

```bash
cd backend_rust
nix-shell -p sqlx-cli --run 'DATABASE_URL="postgres://postgres@127.0.0.1:55432/prism" cargo sqlx prepare'
ls .sqlx | head -3
```

Expected: `query.json` files appear. Commit them. Confirm the crate compiles with **no** `DATABASE_URL` set — that is the whole point of the cache:

```bash
env -u DATABASE_URL cargo check --all-targets
```

- [ ] **Step 4: Write the freshness gate**

`scripts/check_sqlx_cache.sh`:

```bash
#!/usr/bin/env bash
# The prepare cache is checked against the schema as it was when it was
# generated. A stale cache compiles and lies, which is worse than no cache:
# CI would be green against a schema nobody runs. Spec §8.
set -euo pipefail
: "${DATABASE_URL:?run scripts/dev-db.sh and export DATABASE_URL}"
cd "$(dirname "$0")/.."
sqlx migrate run --no-dotenv || { echo "migrations must be applied before --check"; exit 1; }
cargo sqlx prepare --check -- --all-targets
```

- [ ] **Step 5: Prove the gate fails on drift**

Add a column to a copy of the schema, regenerate nothing, run the gate, and confirm it fails naming the query. Then revert. Record the exact failure text in the script's comment so the next person knows what a real failure looks like.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "build(db): compile-time checked queries with a committed prepare cache"
```

---

### Task 9: CI

**Files:**
- Modify: `.github/workflows/ci.yml` — **every job that touches the backend**, not just `build-and-test`: `build-and-test`, `e2e-smoke`, and anything else that starts the app or runs `cargo test`.
- Modify: `AGENTS.md` (gate list)

**Interfaces:**
- Consumes: `migrations/`, `.sqlx/`, `scripts/check_sqlx_cache.sh`.
- Produces: a green workflow in which no job can pass without a real Postgres at head.

**Scope note — this task owns the whole workflow.** Task 3 fixed `frontend/e2e/run.sh` to require Postgres and discovered that `e2e-smoke` (`ci.yml:251-264`) runs it with no `DATABASE_URL` and no service, so it dies demanding `initdb` on PATH. That job was not in any task's file list, which is how a green-on-merge claim gets made in error. Any job that boots the backend needs the service and the DSN; check them all rather than the two named here.

**This is also why the branch cannot be merged before this task lands.** From Task 3 onward, `test_db.rs` hard-requires `DATABASE_URL`, so `cargo test` in `build-and-test` is red until the service exists. That is deliberate and known: a branch with Task 3 but not Task 9 has a red pipeline by construction, and must not be turned into a PR until Task 9 closes it.

- [ ] **Step 1: Add the service and reorder the steps**

In the `build-and-test` job:

```yaml
    services:
      postgres:
        # Same major as production (CloudNativePG ghcr.io/cloudnative-pg/postgresql:18.6).
        # The local harness runs 18.7 via nix-shell; patch drift is accepted and
        # documented in the spec, §9.1.
        image: postgres:18
        env: { POSTGRES_PASSWORD: ci, POSTGRES_DB: prism }
        ports: ['55432:5432']
        options: >-
          --health-cmd pg_isready --health-interval 5s --health-timeout 5s --health-retries 20
```

and in `steps`, before `Run Tests`:

```yaml
      - name: Install sqlx-cli
        run: cargo install sqlx-cli --no-default-features --features postgres --locked

      - name: Migrate
        env: { DATABASE_URL: postgres://postgres:ci@127.0.0.1:55432/prism }
        run: sqlx migrate run --no-dotenv
        working-directory: backend_rust

      - name: Prepare cache is current
        # Before build, not after: a stale cache compiles and lies.
        env: { DATABASE_URL: postgres://postgres:ci@127.0.0.1:55432/prism }
        run: cargo sqlx prepare --check -- --all-targets
        working-directory: backend_rust
```

and add `DATABASE_URL` to the env of the existing `Run Tests` step.

**Then wire the two gates that exist but run nowhere.** Task 8 confirmed by grep that `ci.yml` runs `check_pricing_seam.sh`, `check_access_gate.sh` and `check_backup_portability.sh`, but **neither `check_migrations.sh` nor `check_sqlx_cache.sh`** — so a migration landing without a regenerated `.sqlx` reaches `main` green today. Both get CI steps, and each must **fail rather than skip** when its tool is missing: `check_sqlx_cache.sh` already refuses when `sqlx` or `cargo-sqlx` is absent, and a step that swallows that exit code is a gate that only looks enforced.

Then do the same for `e2e-smoke`: the same `postgres` service and `DATABASE_URL`, and fix the job description, which still says it drives "a real SQLite file". The backend runs `sqlx::migrate!` at startup, so that job needs no separate migrate step — the service database is empty and the app fills it.

**One trap when you do.** The two smokes must not share a database. `statement.smoke.mjs:100` records that the app auto-selects the first portfolio it is given, and `GET /api/portfolios` has no `ORDER BY`, so if the sell-pnl run's portfolio exists when the statement run starts, the statement renders *that* portfolio, prints a plausible document, and passes without testing its own fixture. Task 3's stopgap gives the two steps different databases (`prism` and the image's own empty `postgres`) because the runner is not promised a `psql` to `createdb` with. Prefer the real fix: one scratch database per invocation, created and dropped by `run.sh`, which also restores the isolation the throwaway SQLite file used to provide for free.

- [ ] **Step 2: Update the documented gate list**

In `AGENTS.md` under *Backend gates*, replace the list with the real order: `sqlx migrate run --no-dotenv`, `cargo sqlx prepare --check`, `cargo test`, `bash scripts/check_access_gate.sh`, `bash scripts/check_pricing_seam.sh`, `bash scripts/check_backup_portability.sh`, `bash scripts/check_migrations.sh`, `cargo clippy --all-targets`, `cargo build --release`. Note that every one of them needs `DATABASE_URL` and that `scripts/dev-db.sh` prints it. Say explicitly that `check_migrations.sh` inspects a **database**, not the migration file — it will happily pass against a stale schema — so it is only meaningful after `sqlx migrate run` on a database nobody has touched since. Until this task landed, that gate was advisory: it existed, proved itself able to fail, and ran nowhere anyone was watching.

The same bullet must also state the **scope** of the compile-time checking, because Task 8 converted `src/` only: say plainly that `src/` is compile-time checked against the schema and **`tests/` still issues runtime SQL** (~124 call sites across 23 files). Without that sentence, "compile-time checked queries" reads as universal coverage and the next person to write a test query will assume the compiler has their back.

- [ ] **Step 3: Verify locally in the same order**

```bash
nix-shell -p postgresql_18 --run 'bash backend_rust/scripts/dev-db.sh'
cd backend_rust
export DATABASE_URL="postgres://postgres@127.0.0.1:55432/prism"
sqlx migrate run && cargo sqlx prepare --check -- --all-targets \
  && cargo test && cargo clippy --all-targets && cargo build --release
```

Expected: all green, no warnings.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "ci(db): postgres service container, migrate, prepare --check, then test"
```

---

### Task 10: Cutover runbook, docs, and the deferred half

**Files:**
- Create: `docs/postgres-cutover.md`
- Modify: `docs/walkthrough.md` §3 — **`docs/backup-walkthrough.md` does not exist**; the plan named a file that was never written, and Task 6 found this while adding the pre-cutover audit section. `README.md:48` names `docs/walkthrough.md` as the manual walkthrough, and that is where the backup steps live. Rewrite its SQLite lock and `PRAGMA` steps here.
- Modify: `docs/walkthrough.md` — referenced twice from `README.md` (`:48`, `:170`), and still tells the reader to `export DATABASE_URL='sqlite:/tmp/second.db?mode=rwc'` and to query the store with `sqlite3` (`:305`, `:367-405`). Every command in it now fails. Decide deliberately whether to rewrite it or delete it and drop the README references; do not leave a README pointing at instructions that cannot run. (`docs/migration_python_to_rust.md` and `docs/implementation_plan.md` also mention SQLite, but they are dated historical records and should stay as they are.)
- Modify: `AGENTS.md` (Roadmap: the GitOps follow-up named)
- Create: `docs/superpowers/specs/2026-11-14-postgres-gitops-followup.md`

**Interfaces:**
- Consumes: everything above.
- Produces: the procedure an operator follows on the day, and the named follow-up spec so the `k8s-titan` half is not rediscovered from scratch.

- [ ] **Step 1: Write the runbook from spec §11**

Copy §11's seven steps, and make step 2 executable rather than described:

```markdown
2. **Audit for orphans.** `bash backend_rust/scripts/pre_cutover_audit.sh /path/to/portfolio.db`.
   Every count on the first three lines must be 0. A non-zero count is a stop
   condition: the bundle cannot represent a row whose parent is gone, so it will
   be dropped without the export reporting anything. Repair or accept the loss in
   writing before continuing.
```

Include the rollback note verbatim: after the restore, anything written to Postgres is in no bundle until you export again.

- [ ] **Step 2: Rewrite the walkthrough's SQLite-specific steps**

`docs/backup-walkthrough.md` currently teaches SQLite lock behaviour — the `503` from a held write lock, `PRAGMA integrity_check`, the rollback-journal explanation. Replace each with the Postgres equivalent, and re-run the whole walkthrough against the local cluster, changing nothing but the commands as you go. A walkthrough that has not been executed since the engine changed is a walkthrough that is wrong.

- [ ] **Step 3: Write the follow-up spec stub**

`docs/superpowers/specs/2026-11-14-postgres-gitops-followup.md` — spec §14 verbatim, plus the deploy-ordering warning as its own numbered section, because it is the part that causes an outage if it is missed: the Postgres build refuses to start without `DATABASE_URL`, so the `Database`, `DatabaseRole` and Secret must exist before the image rolls out, and with `replicas: 1` and `Recreate` a badly ordered apply means no serving replica.

- [ ] **Step 4: Full gate run**

```bash
cd backend_rust && export DATABASE_URL="postgres://postgres@127.0.0.1:55432/prism"
sqlx migrate run && cargo sqlx prepare --check -- --all-targets \
  && cargo test && bash scripts/check_access_gate.sh && bash scripts/check_pricing_seam.sh \
  && bash scripts/check_backup_portability.sh && cargo clippy --all-targets && cargo build --release
cd ../frontend && npm run lint && npx vitest run
```

Expected: everything green, no warnings, `260` frontend tests passing.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "docs(db): cutover runbook, rewritten walkthrough, named GitOps follow-up"
```

---

## Deferred, with source

Not in this plan, each with where it came from:

- The `k8s-titan` half — Task 10 writes the stub; the work is its own change in that repo (spec §14).
- `stats_engine.rs:108-110,323-325` returns volatility, Sharpe, beta and correlation as **constants**. Pre-existing; the metrics engine is the heaviest SQL user, so fixing it after the port is cheaper than during (spec §15).
- Multi-replica `CurrencyService` cache and rate limiter — the reason more than one replica is not free (spec §15).
- Fold the export's N+1 per-asset reads into one join (#64 follow-up 3).
- No abort/timeout on frontend `apiFetch` (#64 follow-up 6).
- `create_asset` accepts any uppercased string (#64 follow-up 1).
- `NUMERIC` for money, if it is ever wanted. `TEXT` was a decision, not an oversight (spec §3).
