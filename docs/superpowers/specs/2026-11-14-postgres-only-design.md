# Postgres-only backend: design

Status: draft for review.
Base: `main` after #64 (backup export/restore) merges. Written against `99a95ec`
plus the backup branch's code, which this spec assumes is merged.
Author: session of 2026-11-14.

## 1. Problem

The backend is SQLite-only: `sqlx` is built with the `sqlite` feature, `SqlitePool`
appears 108 times across 13 source files, and the schema is created by a
hand-written `init_db` (`backend_rust/src/lib.rs:138-560`) that runs `PRAGMA
table_info` guards, `ALTER TABLE ... ADD COLUMN`, `CAST(... AS TEXT)` column
migrations and a `PRAGMA legacy_alter_table` dance.

Two things are wanted:

1. **A managed database.** SQLite on a `local-path` PVC with `replicas: 1` and a
   `Recreate` strategy means the app is down for every deploy and the backup story
   is "restic copies a raw file, and WAL is deliberately off because that copy
   would otherwise be silently stale" (`main.rs:57-77`).
2. **More than one replica.** A multi-tenant SaaS cannot scale horizontally against
   a single-writer file.

The deployment target already exists and is already run for other workloads:
CloudNativePG 18.6 in `k8s-titan`, namespace `databases`, cluster `postgres`,
which hosts coder and authentik today.

## 2. Non-goals

These are not oversights. Each was considered and cut.

- **No live SQLite→Postgres migration.** No dump-and-load code, no dual-write, no
  shim. The backup feature (#64) *is* the migration path: export a bundle from the
  SQLite instance, restore it into Postgres. That is why the bundle format was
  built engine-neutral, and it is the reason this spec can be this size.
- **No engine selection.** No runtime switch, no dialect abstraction, no
  `Box<dyn AnyPool>`. SQLite stops being a supported engine. The reason is test
  fidelity: the operator's words were *"I am concerned of executing tests against a
  different path than production."* A dual-engine build is exactly that.
- **No GitOps work in this spec.** The `k8s-titan` half — CNPG `Database`,
  `DatabaseRole`, credential Secret, deployment env — is §14, named and scoped,
  deliberately deferred to its own change in that repo.
- **No decimal type change.** Money stays `TEXT`. See §3.
- **No ORM.** `sqlx` query strings stay query strings.

## 3. Decisions

| Decision | Chosen | Rejected | Why |
| --- | --- | --- | --- |
| Engines | Postgres only | Dual engine behind a flag | Test fidelity. A dual-dialect abstraction is a permanent tax on every query and a second path that production never runs. |
| Data migration | The backup bundle | `pg_dump`-style converter, dual write | Already built and reviewed. Engine-neutral by construction. One path, exercised by a real feature rather than a one-off script. |
| Schema management | `sqlx::migrate!` with numbered `.sql` files | Keep `init_db`, or an external migrator | `init_db`'s `PRAGMA table_info` idiom is what produced the `split_sync` bug the code itself documents (`lib.rs:513-519`: *"keep the bug while the code above it looked fixed"*). Numbered migrations compiled into the binary remove the class. An external migrator adds a binary to the image and a step that can be skipped. |
| Ids | `BIGINT GENERATED ALWAYS AS IDENTITY`, Rust `i64` | Keep `i32`, or `SERIAL` | SQLite `INTEGER` is 64-bit today, so ids already exceed `i32` in principle. `sqlx-postgres` refuses `int8`→`i32` at compile time, so keeping `i32` is not an option once queries are checked. `GENERATED ALWAYS AS IDENTITY` is standard SQL and refuses a supplied id, which matches the bundle rule that ids are never trusted from the file. |
| Money | Stays `TEXT` | `NUMERIC`/`DECIMAL` | Exact decimal-string round-trip is preserved and no rounding question ever arises. `rust_decimal` already serialises to a string in the API, so the wire format does not change. A later `NUMERIC` migration is a separate, mechanical spec. |
| Dates | `TIMESTAMPTZ` for timestamps, `DATE` for `historical_prices.date` | Keep `TEXT` | Real date ordering, range scans, and `date_trunc` for the metrics engine. Verified: transaction dates are `DateTime<Utc>` in `schemas.rs:18,32,121,...` and the price date is a `NaiveDate`, so both map cleanly with no parsing rule to invent. |
| Query safety | `sqlx::query!` macros with a committed `.sqlx` prepare cache | Keep runtime `sqlx::query` | Turns the `i32`-vs-`int8` class of bug — which the whole-branch review of #64 caught by hand — into a compile error. The cache is what makes this possible without a live database at compile time. |
| Test database | `#[sqlx::test]`, one fresh migrated database per test | Shared database with transaction rollback | Per-test databases remove the interference that made the #64 flake hard to see (shared `:memory:` cache, unlock-notify hangs). Postgres supports it natively; it needs `CREATEDB` on the test role. |
| Local dev database | `nix-shell -p postgresql_18` ephemeral cluster (18.7, same major as production) | Rootless podman running `postgres:18` | Podman was the first choice and was tried: it cannot create a usable user namespace in this pod, even after the `0.0.12` image fix. Measured in §9.2. The nix cluster was then proven end to end (§9.1). |
| CI database | Service container `postgres:18` | `postgres:16`, SQLite tests | Same major as the production image (`ghcr.io/cloudnative-pg/postgresql:18.6`). |
| Foreign keys | Enforced, as Postgres does by default | Reproduce SQLite's non-enforcement | See §5.1 — this is a real behaviour change and the largest risk in the spec. |

## 4. Target schema

`backend_rust/migrations/0001_init.sql`. Written from the DDL actually in
`init_db`, not from memory.

```sql
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

-- Split dedupe. SQLite enforced this with a partial unique index
-- (`uq_split_asset_date`, added after bug #50: a duplicated split adjustment
-- silently corrupts cost basis). `apply_split_events` has no read-based check of
-- its own — it inserts and lets the conflict do the dedupe — so dropping this
-- index reopens that bug. The expression is wrapped because `date_trunc('day', …)`
-- is STABLE and cannot be indexed; `AT TIME ZONE 'UTC'` is IMMUTABLE. The dedupe
-- key is therefore the UTC calendar day, matching what SQLite's `date(date)`
-- produced from the stored value.
CREATE UNIQUE INDEX uq_split_asset_date ON transactions (asset_id, ((date at time zone 'UTC')::date))
    WHERE type = 'SPLIT';

-- No owner_id, deliberately: same as today, and the reason price restore stays
-- flag-gated. Composite PK replaces the unique index SQLite needed.
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

Notes on what changed and why:

- **`users.token_version` is `BIGINT`, not `INTEGER`.** It is read as `i64` on every
  authenticated request (`auth/guard.rs:31`, `:82`, `:119`; `auth/oidc.rs:236`, `:275`,
  `:316`), and `sqlx-postgres` will not decode `INT4` into `i64` — an `INTEGER`
  column would fail every guard in the app with a mismatched-types error. An
  earlier draft of this section said `INTEGER`.
- **`split_sync.last_synced_at` is `DATE`, not `TIMESTAMPTZ`.** It is written as
  `Utc::now().date_naive()` (`currency_service.rs:894`, bound `:999`) and read back
  as a `String` parsed with `%Y-%m-%d` (`:916`, `:924`), then surfaced verbatim as
  `splits_last_synced_at` (`api_routes/portfolios.rs:71`, `:91`). A `timestamptz`
  comes back as `2026-11-14 00:00:00+00`, which no `%Y-%m-%d` parse accepts, and
  under `sqlx-postgres` the read fails even earlier: `String` is not a decodable
  target for a date column at all, so it is a loud mismatched-types error rather
  than the silent throttle failure first predicted. Either way the column is
  `DATE`, and the Rust side must decode `NaiveDate` and format the API string
  explicitly.

- **`split_sync`'s key.** `init_db` migrated this table's key over the course of
  several releases (`migrate_split_key`, `migrate_split_sync_key`). The Postgres
  schema states the intended end state once: `PRIMARY KEY (asset_id)`. That was
  confirmed against the code, not guessed — no `effective_date` column exists
  anywhere in the tree, `currency_service.rs:916` does a `fetch_optional` on
  `WHERE asset_id = ?` (a composite key would raise `MultipleRowsFound`), and both
  `transactions.rs:175` and `backup/restore.rs:262` delete by `asset_id` alone.
  An earlier draft of this note listed `enforce_split_uniqueness` and
  `uq_split_asset_date` as part of that history; they are not — that index is on
  `transactions` and is reproduced above.
- **`uq_split_asset_date` is load-bearing.** A schema review that only compares
  tables and column types will miss it, which is why the migration shape gate
  asserts the index exists, is unique, is partial on `type = 'SPLIT'`, and is
  expressed on the UTC day.
- **Three new indexes** (`portfolios_owner_idx`, `assets_portfolio_idx`,
  `transactions_asset_date_idx`). SQLite has none beyond the three unique indexes,
  because its rowid tables made some lookups cheap enough. Every authorisation
  check filters on `owner_id` and every portfolio read filters on `portfolio_id`;
  on Postgres those are sequential scans without an index.
- **`is_active` becomes `BOOLEAN`.** Today it is `INTEGER NOT NULL DEFAULT 1`.
- **No `ON DELETE CASCADE`.** The app deletes explicitly, in a known order, and
  restore inserts children before parents. Adding cascade would be a behaviour
  change smuggled into a port.

## 5. Behaviour changes the engine switch forces

These are not rewrites. They are places where Postgres is not SQLite and the
difference reaches the API.

### 5.1 Foreign keys: two of them are new, and those are the ones to audit

An earlier draft of this section claimed `PRAGMA foreign_keys` was never set and
that every `FOREIGN KEY` in `init_db` had therefore been decorative. **That was
wrong.** `sqlx-sqlite` sets `foreign_keys=ON` on every connection by default
(`sqlx-sqlite-0.8.6/src/options/mod.rs:183-185`), so `backup/restore.rs:258`
("`foreign_keys` is ON in this app") was the accurate statement and the draft was
not. `assets.portfolio_id` and `transactions.asset_id` are enforced today, so
making them `REFERENCES` in Postgres changes nothing.

What is genuinely new in `0001_init.sql` is two relationships SQLite never
constrained:

1. `portfolios.owner_id → users(id)`. SQLite added the column by a bare `ALTER`
   (`lib.rs:259`) with no foreign key, so an orphaned `owner_id` is possible.
2. `split_sync.asset_id → assets(id)`. Lower risk than it looks: `split_sync` is
   not part of the backup bundle at all — `backup/restore.rs` only ever `DELETE`s
   from it (`:262`) and `export.rs` never writes it — so nothing crosses the
   cutover boundary through this table.

Postgres enforces all of them always, so:

1. Writes that today create an orphan row will now fail with a constraint
   violation. That is an improvement, but it is a new 500/400 path that no test
   covers yet.
2. **Existing SQLite data may already contain orphans, and the export would drop
   them silently.** The bundle is nested — portfolio → assets → transactions — so a
   transaction whose `asset_id` no longer exists is not under any asset and is
   therefore never exported. Nothing reports it. This is now much less likely for
   `assets`/`transactions` than the first draft claimed, because those two were
   already enforced; it remains a live possibility for `owner_id`.

   **Required before cutover:** run the orphan audit (§11 step 2) against the live
   database across all four relationships and record the counts. If any are
   non-zero, the cutover does not proceed until they are either repaired or
   explicitly accepted as loss, in writing, by the operator. This is the one place
   in the spec where silent data loss is possible.

### 5.2 `i32` → `i64` everywhere an id crosses the boundary

108 `i32` occurrences, of which the id-shaped ones become `i64`: pool row reads
(`query_as::<_, (i32,)>` on `RETURNING id` — the exact pattern recorded as
follow-up 8 in #64), `Portfolio.id`, `Asset.id`, `Transaction.id`, `User.id`,
path parameters, and `HashMap<i32, _>` in `split_sync_ages`.

The JSON wire format does not change: these are numbers either way, and identity
sequences start at 1, so no JavaScript precision question arises.

### 5.3 `INSERT OR REPLACE` → `ON CONFLICT DO UPDATE`

Four sites, all identified: `stats_engine.rs:433`, `stats_engine.rs:539`
(`historical_prices`), `currency_service.rs:997`, `currency_service.rs:1469`
(`split_sync`). Each becomes an explicit `ON CONFLICT (…) DO UPDATE SET …`, which
is also more honest: `INSERT OR REPLACE` in SQLite is a delete plus an insert, so
it silently drops columns not named in the statement. The composite PK in §4 gives
`historical_prices` a real conflict target.

### 5.4 Lock and busy semantics disappear, and the 503 changes meaning

`busy_timeout(5s)`, `SQLITE_BUSY`, and the extended busy-code classification added
in #64 (`7facddb`) are SQLite concepts. Postgres does not block a reader behind a
writer, and the restore's `SERIALIZABLE` transaction reports
`40001 serialization_failure` instead.

The `503 + Retry-After` response shape stays — clients already handle it — but its
trigger becomes: pool exhaustion (timeout acquiring a connection), statement
timeout, and serialisation failure after the retry policy is exhausted. The
backup module's portability gate already forbids engine-specific constructs, so
this is a mapping change in one place, not a rewrite.

**Recorded as a follow-up in #64 and now in scope:** the restore's serialisation
behaviour under concurrent writers has never been tested. It gets a test here,
because Postgres is where it can be written.

### 5.5 `init_db` and its startup migrations are deleted

`lib.rs:138-560` goes away entirely: the `CREATE TABLE IF NOT EXISTS` set,
`column_exists`, `migrate_decimal_columns`, `migrate_split_key`,
`enforce_price_uniqueness`, `enforce_split_uniqueness`, `add_owner_id_column`,
`migrate_split_sync_key`. Their content is §4. Their *history* — the dedupe
tie-breaks, the `MAX(rowid)` choice documented at `lib.rs:420-428` — is a
one-time data concern for the live SQLite database, not something Postgres needs
to re-enact, because the data arrives through the bundle, which is already
deduped by the unique index it was exported from.

This also closes the recorded pre-existing bug: `main.rs:82-88` boots `init_db` on
a default multi-connection file-backed pool, which is a stale-schema window. With
`sqlx::migrate!` the migrator takes a single connection and runs before the pool
is used for anything else.

### 5.6 SQLite-only connect options go away

`SqliteConnectOptions`, `busy_timeout`, the WAL commentary, and the
`sqlite:../portfolio.db?mode=rwc` default all go. §10 replaces them.

## 6. Code plan

Mechanical, but wide. Order matters for reviewability.

| Step | Change | Files |
| --- | --- | --- |
| 1 | `sqlx` features: `sqlite` → `postgres`, add `migrate`, `macros`, `rust_decimal` if a `NUMERIC` column ever appears (not now) | `Cargo.toml` |
| 2 | Introduce `pub type DbPool = sqlx::PgPool;` and `pub type DbPoolOptions = sqlx::postgres::PgPoolOptions;` in one module, then replace all 108 `SqlitePool` references with the alias | 13 files |
| 3 | `?` → `$1..$n` in all 143 query sites. `grep -c '?'` gives the count; the compiler finds every miss once step 4 lands | same 13 files |
| 4 | Convert `sqlx::query` / `query_as` to `sqlx::query!` / `query_as!` incrementally, file by file, with `cargo sqlx prepare` after each file | same 13 files |
| 5 | `i32` → `i64` for ids (§5.2) | `schemas.rs`, routes, engines, `backup/` |
| 6 | `INSERT OR REPLACE` → `ON CONFLICT` (§5.3) | `stats_engine.rs`, `currency_service.rs` |
| 7 | Delete `init_db` and its helpers; call `sqlx::migrate!("./migrations")` at startup (§5.5) | `lib.rs`, `main.rs` |
| 8 | Lock/busy remap (§5.4) | `backup/restore.rs`, `api_routes/backup.rs` |
| 9 | Test harness to `#[sqlx::test]` (§9) | `tests/`, `src/` test modules |
| 10 | Regenerate `openapi.json` + `api.d.ts` if any schema type changed (`i64` ids do not change the JSON type, so likely no diff — verify, do not assume) | `docs/`, `frontend/src/` |

Step 2 before step 4 is deliberate: the alias makes the type rewrite one commit and
keeps the macro conversion diff readable, instead of two changes interleaved
across thirteen files.

## 7. Migration policy

- `backend_rust/migrations/`, numbered `NNNN_name.sql`, compiled in with
  `sqlx::migrate!`.
- **`0001_init.sql` is the whole schema.** There is no historical replay: the data
  arrives as a bundle into a fresh database, so there is nothing to migrate
  incrementally.
- Every schema change after this is a new numbered file. Never edit a shipped
  migration. `_sqlx_migrations` is the ledger; it is created by the migrator.
- The `--print-backup-schema` and `--print-openapi` flags keep working without a
  database, which is what makes the CI ordering in §9.3 possible.

## 8. Compile-time checked queries and `.sqlx`

`sqlx::query!` needs a live database at compile time, or a prepared-query cache.
The cache is the answer for CI and for anyone without Postgres running:

- `.sqlx/` is committed, generated by `cargo sqlx prepare`.
- `sqlx-data.json`'s successor in 0.8 is the `.sqlx/` directory; confirm the exact
  layout against the 0.8 docs during implementation rather than assuming the
  older filename.
- **Local workflow:** anyone editing SQL runs against the local ephemeral cluster
  (§9.1) and re-runs `cargo sqlx prepare` before committing. A stale cache is a CI
  failure, not a silent drift — that is what `--check` is for.
- **The failure mode to watch:** `cargo sqlx prepare` against a database whose
  schema is not at head produces a cache that compiles and lies. The prepare step
  therefore runs *after* `sqlx migrate run` in every script and workflow, and the
  CI job fails if `cargo sqlx prepare --check` reports drift.

## 9. Test harness

### 9.1 Local: `nix-shell -p postgresql_18` ephemeral cluster

This is the plan, not the fallback. Measured end to end in this workspace on
2026-11-14, after the podman route was tried and failed (§9.2):

```
initdb ok, version: initdb (PostgreSQL) 18.7
server up
PostgreSQL 18.7 on x86_64-pc-linux-gnu, compiled by clang version 21.1.8, 64-bit
1
1|12345678901234567890.123456789|2026-11-14 13:40:00.123456+00|2026-11-14
createdb works, db visible: 1
stopped cleanly
```

That run covers the four things this spec actually depends on: `GENERATED ALWAYS
AS IDENTITY`, `TIMESTAMPTZ` with microsecond precision, `DATE`, exact `TEXT`
decimal round-trip, and `CREATEDB` for `#[sqlx::test]`.

`backend_rust/scripts/dev-db.sh` wraps it: `initdb` into a directory under
`$XDG_RUNTIME_DIR` or `/tmp`, `pg_ctl -o "-p 55432 -k <sockdir> -c
listen_addresses=127.0.0.1"`, `create database prism_test`,
`alter role postgres createdb`, and a matching `stop`. `DATABASE_URL` is
`postgres://postgres@127.0.0.1:55432/prism_test`.

**Version note.** nixpkgs' `postgresql_18` is **18.7**; production is **18.6**
(`ghcr.io/cloudnative-pg/postgresql:18.6`) and CI's `postgres:18` will resolve to
whatever 18.x is current. Same major, patch drift. That is acceptable and it is
stated rather than hidden: nothing in §4 or §5 depends on a 18.6-vs-18.7
difference, and the alternative — pinning 18.6 locally — buys fidelity nobody has
asked for at the cost of a nix overlay.

**Do not use the `postgresql` already in the workspace image.** It is 18.6 and it
is broken for this purpose: `initdb` dies with

```
FATAL:  could not access file "dict_snowball": No such file or directory
STATEMENT:  CREATE FUNCTION dsnowball_init(INTERNAL) ... AS '$libdir/dict_snowball'
```

even though `/nix/store/9i3rafrdlicmvn7bnnzafwq715y9ap23-postgresql-18.6/lib/dict_snowball.so`
exists and `ldd` on it reports nothing missing. The nix-shell build of the same
major works, so the fault is in the image's closure, not in PostgreSQL. Worth
reporting upstream in `coder-workspaces`; not worth blocking on.

### 9.2 Podman: attempted, and it does not work in this pod

Recorded in full because the conclusion reverses what this spec said on first
draft, and because the evidence is specific enough to act on.

The image fix shipped in coder-workspaces `0.0.12` and is present in the running
workspace: `/etc/subuid` and `/etc/subgid` carry `coder:100000:65536`,
`/run/user/1000` exists, `XDG_RUNTIME_DIR` is set, and `/etc/containers/` pins
`vfs` and `cgroupfs`. Podman 5.8.7 starts and reports itself rootless.

It still cannot run a container. Two distinct faults:

1. **`/bin/newuidmap` shadows the setuid copy.** The image's `PATH` is
   `...:/bin:/usr/bin:...`, and `buildEnv` puts a plain non-setuid *symlink* at
   `/bin/newuidmap`, ahead of the setuid copy at `/usr/bin/newuidmap`
   (`-rwsr-xr-x root root`). Podman picks `/bin/newuidmap` and reports
   `should have setuid or have filecaps setuid`. Fixable in `image.nix` with
   `cp --remove-destination` into `/bin` as well.
2. **Even with the setuid helper found, the kernel refuses the mapping.** With
   `PATH` forced so `/usr/bin/newuidmap` wins, podman still fails:
   `newuidmap: write to uid_map failed: Operation not permitted`. Reproduced
   directly, outside podman: `unshare -U sleep 30` then
   `newuidmap <pid> 0 1000 1 1 100000 65536` → `EPERM`, while
   `unshare -U -r` (single self-mapping) succeeds. So single-identity user
   namespaces are allowed and multi-range `uid_map` writes are not.

`NoNewPrivs: 0`, `Seccomp: 0`, no AppArmor, `CapBnd: 00000000a80425fb` (the
default Docker bounding set, which does include `CAP_SETUID`), `/proc/self/uid_map`
is `0 0 4294967295` so this is the initial user namespace, and
`user.max_user_namespaces` is 514686. Nothing inside the container explains the
refusal, which puts the fix outside this workspace — in the pod spec or the node.

**Consequence for this spec:** rootless podman is not the local harness. If it is
ever fixed at the pod level, §9.1's script is a one-line swap and the CI
configuration does not change at all.

### 9.3 CI

GitHub Actions, service container `postgres:18`, in this order:

```
services: postgres:18  →  sqlx migrate run  →  cargo sqlx prepare --check
                        →  cargo build  →  cargo test  →  clippy
```

`--check` before build is the point: it fails on a stale cache before anything
compiles against it. The existing backup schema-freshness and portability gates
stay where they are. CI is the authoritative environment; the local harness exists
for iteration speed.

## 10. Configuration

- `DATABASE_URL` becomes **required**. No default. A missing or malformed value
  makes the app refuse to start, through the existing `StartupConfig::from_env` →
  `refuse()` path that already handles missing OIDC config the same way. The
  current `sqlite:../portfolio.db?mode=rwc` default is deleted — a default that
  silently creates a file database is the opposite of this spec's purpose.
- Pool options: `max_connections` (default 5, so two replicas stay well inside
  CNPG's default `max_connections=100`), `min_connections 0`,
  `max_connection_lifetime` a few hours, `acquire_timeout` 5 s — which is what the
  `503` in §5.4 keys off.
- `statement_timeout` set on the connection (`options=-c statement_timeout=30s` or
  per-statement), so a runaway analytics query cannot pin a pool connection
  indefinitely. The #64 review recorded that export holds a pool connection for
  its whole duration; this is the backstop.
- `AUTH_MODE=disabled` still works for local development.
- The SQLite commentary in `main.rs:57-77` about WAL and restic is deleted with
  the code it describes. It is a good comment about a dead engine.

## 11. Cutover runbook (application side)

The infrastructure half is §14. This is what happens to the data.

1. **Freeze writes.** Scale the SQLite deployment to zero, or put it in read-only
   mode. The bundle is a point-in-time snapshot per portfolio, but two snapshots
   taken at different moments are not one consistent backup.
2. **Audit for orphans** against the live SQLite database, and record the numbers:
   ```sql
   SELECT COUNT(*) FROM transactions t LEFT JOIN assets a ON a.id = t.asset_id
     WHERE a.id IS NULL;
   SELECT COUNT(*) FROM assets a LEFT JOIN portfolios p ON p.id = a.portfolio_id
     WHERE p.id IS NULL;
   SELECT COUNT(*) FROM portfolios p LEFT JOIN users u ON u.id = p.owner_id
     WHERE p.owner_id IS NOT NULL AND u.id IS NULL;
   ```
   Any non-zero count is a stop condition (§5.1).
3. **Export** a bundle covering every portfolio, with `include_prices=true`.
   Store the file outside the cluster. It is the only copy that matters.
4. **Deploy Postgres** (§14) and start the Postgres build against an empty
   database. `sqlx migrate run` creates the schema.
5. **Restore** with `apply_prices=true` and `BACKUP_ALLOW_PRICE_RESTORE=1`, then
   unset the flag. Preview first, as the feature requires.
6. **Verify counts** — row counts per table against the SQLite source, and the
   app's own reports for one portfolio against the pre-cutover numbers.
7. **Keep the SQLite PVC** until the Postgres instance has survived one backup
   cycle of its own.

Rollback after step 5 is not free: anything written to Postgres after the restore
is not in any bundle. Re-export from Postgres before declaring the cutover done.

## 12. Verification gates

All from `backend_rust/`, all must pass, all must be warning-free:

- `sqlx migrate run` then `cargo sqlx prepare --check`
- `cargo test` — including the new serialisation test (§5.4) and the FK-enforcement
  tests (§5.1)
- `bash scripts/check_access_gate.sh`
- `cargo clippy --all-targets`
- `cargo build --release`
- `bash scripts/auth_smoke.sh` (unchanged, still must pass)
- the manual walkthrough in `docs/backup-walkthrough.md`, re-run against Postgres —
  several of its steps describe SQLite lock behaviour and will need rewriting,
  not re-running

## 13. Risks

| Risk | Likelihood | Consequence | Mitigation |
| --- | --- | --- | --- |
| Orphan rows silently dropped at cutover | Unknown — unmeasured today | Silent data loss | §11 step 2 is a stop condition with recorded numbers |
| `sqlx prepare` cache drifts from head | High over time | Compiles and lies | `--check` in CI, always after `migrate run` |
| 143-site rewrite breaks a query the tests do not cover | Medium | Wrong numbers, not a crash | Step 3 is placeholder-only and reviewable as such; the macro conversion in step 4 is where the compiler starts paying |
| Podman cannot run containers in this pod | **Confirmed, not a risk any more** | No local container engine; local harness is the nix cluster instead | §9.1 is the plan and is proven. The pod-level cause is outside this workspace (§9.2). CI is authoritative either way. |
| `split_sync` live shape differs from §4 | Medium | Migration creates a table the queries disagree with | Verify against the deployed database before writing `0001` |
| FK enforcement surfaces a write path that relied on orphans | Medium | New 500s in production | §5.1 tests written before the switch, not after |

## 14. Deferred: the `k8s-titan` half

Deliberately out of this spec, recorded here so it is not rediscovered from
scratch. It is its own change in that repo, with that repo's own gates
(`make portfolio-prism-check`, `make check`) and its own PR.

- A CNPG `Database` and `DatabaseRole` for portfolio-prism on the existing
  `postgres` cluster in `databases` — the cluster already hosts coder and
  authentik, so the pattern exists to copy.
- The credential Secret, and how `DATABASE_URL` reaches the deployment.
- **Deploy ordering is the hard part and it belongs in that spec:** the Postgres
  build refuses to start without `DATABASE_URL` (§10), so the database and its
  Secret must exist before the image rolls out, and the cutover (§11) must happen
  between the two. With `replicas: 1` and `Recreate`, a badly ordered apply is a
  crash-looping app with no serving replica, not a graceful degradation.
- Whether the SQLite PVC and its restic backup are retired, and when.

## 15. Explicitly deferred, with source

- `stats_engine.rs:108-110,323-325` returns volatility, Sharpe, beta and
  correlation as **constants**. Pre-existing, unrelated to the engine, and the
  metrics engine is the heaviest SQL user in the codebase — rewriting queries
  against numbers that are hardcoded is wasted work. Fix first or separately.
- Multi-replica `CurrencyService` in-process cache and rate limiter — the reason
  more than one replica is not free.
- Fold the export's N+1 per-asset reads into one join (#64 follow-up 3).
- No abort/timeout on frontend `apiFetch` (#64 follow-up 6).
- `create_asset` accepts any uppercased string (#64 follow-up 1).
