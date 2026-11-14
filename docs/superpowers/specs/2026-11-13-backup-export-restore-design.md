# Portfolio Backup Export and Restore Design

**Date**: 2026-11-13
**Status**: Proposed
**Implementation plan**: not yet written. This document is the input to it; the plan is
written by the `writing-plans` skill only after this spec is approved.

**Normative for:** the bundle format and its versioning policy, the route contract, the
restore transaction's ordering and atomicity guarantee, the error-status mapping, and the
portability discipline in *Portability discipline*. Where the plan and this document
disagree, this document wins for behaviour and the plan wins for order of work.

## Overview

A signed-in user can export the portfolios they choose as a single downloadable file, and
can restore such a file back into the database — either alongside what they already have, or
replacing it.

**Problem.** There is no way to get a user's data out of this application in a form that can
be put back. The only thing resembling a backup today is the cluster's restic job, which
copies the SQLite file with a plain `restic backup /volume` — no `sqlite3 .backup`, no
checkpoint, nothing that understands the database format. `backend_rust/src/main.rs:44-58`
documents at length why that is unsafe and why WAL is deliberately not enabled until the
backup becomes SQLite-aware. That job is also, by nature, all-or-nothing: it backs up every
tenant's database, it cannot express "these two of my five portfolios", and a user cannot
ask for it.

The user-facing need is separate from the operator's need. This feature is the user's: a
file they hold, containing the portfolios they selected, that restores into any instance —
including a future one running a different database engine.

### Relationship to the Postgres work

This spec is deliberately **one of two independent features**. The second — making the
database engine selectable between SQLite and Postgres — is its own spec, and the two share
no code. They share one constraint, which this spec honours by construction rather than by
abstraction: **the bundle format is logical and engine-neutral.** It is rows, not a file
copy and not `pg_dump`.

The export is written against `SqlitePool` as everything else in this codebase is. It is
written to a stated discipline (*Portability discipline*) so that the engine port is a
mechanical change to one module rather than a rewrite. Building the engine-abstraction layer
now is out of scope: it is the other spec's central decision and guessing at it here would
be worse than leaving it clean.

## Requirements

1. A user selects which of their own portfolios to export; unselected portfolios contribute
   nothing to the file.
2. The export is a single file, downloaded by the browser. The server never stores a bundle.
3. A bundle restores into a **fresh, empty** instance and reproduces the data faithfully
   enough that the tax engine and the metrics engine produce identical results to the source
   instance.
4. Restore has two modes, chosen by the user at restore time:
   - **New copy** — always creates new portfolios; touches nothing that already exists.
   - **Replace** — deletes everything the caller owns, then inserts the bundle.
   Merge/upsert of authored data is not a mode and is out of scope.
5. Restore is atomic: a bundle that fails anywhere writes nothing.
6. Restore shows the user what it will do — counts of rows deleted and inserted — before it
   will run.
7. A bundle carries an explicit schema version, and the version definitions are persisted in
   the repository as generated, CI-checked artifacts that future contributors can read.
8. Price history is exportable as an opt-in section, off by default, and restoring it is
   disabled by a server-side flag that is off by default.
9. Every new route is tenant-scoped through `access.rs` and appears in the existing route
   inventory and isolation matrix.

## Non-goals

- **Operator disaster recovery.** Full-database backup, point-in-time recovery and retention
  belong to the deployment, not the app. On a managed Postgres they are a provider feature.
- **An in-app admin backup of all tenants.** Not requested, and it would put every tenant's
  data in one user's download.
- **Scheduled or automatic export.** The user asks for a file.
- **Server-side bundle storage.** No PVC writes, no cleanup job, and no path by which one
  tenant reads another's backup off disk.
- **Merge / dedupe of authored rows.** "Which transaction is a duplicate?" has no clean
  answer; a wrong guess silently corrupts a tax ledger.
- **Cross-instance sharing between two users.** A bundle restores into the caller's own
  space, always owned by whoever restores it.
- **Streaming or job-queue restore.** At this scale (hundreds to low thousands of rows) a
  synchronous request is correct; a queue would be speculative infrastructure.
- **The engine-selection work itself.** See *Relationship to the Postgres work*.

## Decisions and rationale

| Decision | Chosen | Rejected | Why |
|---|---|---|---|
| Bundle contents | Logical rows as JSON | `sqlite3 .backup` / `pg_dump` | Engine-native dumps are engine-specific, not tenant-scoped, cannot express portfolio selection, and need a binary in the container. |
| Assembly | New `backup` module with its own SQL | Composing existing create handlers | The create paths carry business logic (split-sync side effects, currency and ISIN validation) that would reject a legitimate restore or fire provider calls during it, and restore would be one round trip per row. |
| Structure | Nested: portfolio → assets → transactions | Flat table dump | Only `portfolios` has `owner_id` (`lib.rs:252-267`); assets and transactions are scoped transitively. Nesting makes ownership structural — a bundle cannot reference an asset outside the selected set — and makes selection need no filtering logic. |
| Identifiers in bundle | Bundle-local `ref` labels | Real database ids | There is then no field that could be mistaken for a database id. "Never trust ids from the file" stops being a rule a contributor must remember. |
| Replace scope | Everything the caller owns | Only the portfolios named in the bundle | One rule, no cross-bundle portfolio identity problem. Naming-based replace is merge semantics by another name. |
| Preview safety | Mandatory preview + typed confirmation | A one-time token issued by preview | The file is held by the browser and is byte-identical across the two calls; a token is ceremony without added safety. |
| Restore execution | Synchronous, one transaction | Background job | Scale does not justify a queue. |
| Price restore | Server flag, off by default | Open | `historical_prices` has no `owner_id`; an open price restore lets any tenant write market data every other tenant's dated reports read. |
| Format artifact | Generated JSON Schema, committed, CI-diffed | Hand-written doc, or structs only | Mirrors the existing `--print-openapi` + `ci.yml:47-48` freshness gate, which is the pattern this repo already trusts. |

## The bundle format

```json
{
  "format": "portfolio-prism-backup",
  "schema_version": 1,
  "created_at": "2026-11-13T12:00:00Z",
  "portfolios": [
    {
      "ref": "p1",
      "name": "Main",
      "description": null,
      "currency": "USD",
      "base_currency": "USD",
      "assets": [
        {
          "ref": "a1",
          "symbol": "AAPL",
          "name": "Apple Inc.",
          "asset_type": "STOCK",
          "sector": "Technology",
          "currency": "USD",
          "isin": "US0378331005",
          "transactions": [
            {
              "type": "BUY",
              "quantity": "10.0",
              "price": "150.00",
              "fee": "1.00",
              "date": "2024-01-15T00:00:00Z"
            }
          ]
        }
      ]
    }
  ],
  "prices": [
    { "symbol": "AAPL", "date": "2024-01-15", "close_price": "150.00" }
  ]
}
```

### Field rules

- `format` — literal `"portfolio-prism-backup"`. A file that is not a bundle fails here with
  a clear message rather than at the first missing field.
- `schema_version` — integer. See *Versioning*.
- `created_at` — RFC 3339 UTC, informational. Excluded from the round-trip equality test.
- `portfolios[]` — one entry per selected portfolio. `ref` is a bundle-local label
  (`p1`, `p2`, …), unique within the bundle, so validation can refuse two rows that
  claim the same label. It is never a database id, it is never persisted, and the write
  path never reads one: the parent/child link comes from the bundle's nesting, and each
  stored link comes from the freshly allocated id of the insert that made the row.
- `assets[]` — nested in its portfolio. `ref` as above (`a1`, …).
- `transactions[]` — nested in its asset. No `ref`: nothing references a transaction.
- `prices[]` — optional, absent unless requested at export time. Flat, keyed
  `(symbol, date)`, because price rows are global and belong to no portfolio.
- **Decimals are strings** (`quantity`, `price`, `fee`, `close_price`). This matches storage
  as it is today — those columns are `TEXT` (`lib.rs:176-192`) — keeps `rust_decimal` off a
  float path, and is what Postgres `NUMERIC` binds cleanly later.
- **Two different date shapes, because the database holds two different shapes.**
  `transactions[].date` is an RFC 3339 UTC datetime string (`2024-01-15T00:00:00Z`) — the API
  field is `DateTime<Utc>` (`schemas.rs:18`) and it is stored verbatim as TEXT. `prices[].date`
  is `YYYY-MM-DD`, because `historical_prices.date` is a `DATE` column read back as
  `NaiveDate`. Normalising the two would be a change to stored data, which this feature does not
  make.
- Transaction dates must additionally fall inside **1900-01-01 to 2100-01-01**, the range
  `create_transaction` already enforces (`transactions.rs:223-229`). A restore must not accept a
  row the ordinary write path would reject.
- `asset_type` and `transaction.type` are the existing uppercase literals the engine already
  accepts. The schema enumerates them; an unknown value is a validation error, not a row that
  silently never matches.
- Absent fields are `null`, never omitted, for nullable columns. `description`, `sector` and
  `isin` are nullable.

### Versioning

The format is a persisted artifact, not an implication of the Rust structs.

- **`docs/backup-schema/v1.json`** — JSON Schema (draft 2020-12), **generated**, committed.
  It is the normative definition of `schema_version: 1`.
- Generated by `cargo run --bin backend_rust -- --print-backup-schema`, mirroring
  `--print-openapi`. Like that flag it is handled **before** configuration, the database and
  the network (`main.rs:14-18`), so CI needs no `portfolio.db` and no OIDC configuration.
- Derived from the Rust structs with `schemars`, so code and artifact cannot drift.
  `rust_decimal` and `chrono::NaiveDate` are overridden to `type: string` — which is what
  they serialize as.
- **`docs/backup-schema/README.md`** — the version register: what each version is, what
  changed from the previous one, which versions the current build accepts, and the
  deprecation policy. This is the document a contributor in two years reads.
- **`v1.json` is never edited.** A format change adds `v2.json` plus a `v1 → v2` upgrader in
  code. The build declares the versions it accepts, validates an uploaded bundle against the
  embedded schema for its declared version (`include_str!`, so the binary carries the same
  bytes as the repository file), and refuses anything outside that set with a message naming
  what it accepts.
- Restore logs the version it read and the version it upgraded from.

## Backend changes

### New module: `backend_rust/src/backup/`

| File | Responsibility |
|---|---|
| `mod.rs` | The format structs, the accepted-version set, validation entry points. |
| `export.rs` | Reading the caller's rows and assembling a bundle. All export SQL. |
| `restore.rs` | Validation, the restore transaction, the preview plan. All restore SQL. |
| `schema.rs` | JSON Schema generation for `--print-backup-schema`. |

Handlers live in `backend_rust/src/api_routes/backup.rs`, **not** in `src/backup/`, because
`scripts/check_access_gate.sh` inspects `src/api_routes` only; a handler placed elsewhere
would be invisible to Rules A and C.

### Routes

Four routes, mounted in `build_rocket` (`lib.rs:560`), each taking `AuthedUser`.

| Route | Purpose | Access |
|---|---|---|
| `GET /api/backup/export` | Produce a bundle. | `require(Viewer)` per id |
| `GET /api/backup/capabilities` | Server limits and feature flags. | authenticated only |
| `POST /api/backup/preview` | Plan for a bundle; no writes. | authenticated only |
| `POST /api/backup/restore` | Apply a bundle. | authenticated only |

**`GET /api/backup/export?portfolio_ids=1,2&include_prices=0`**
Returns `application/json` with `Content-Disposition: attachment;
filename="portfolio-prism-backup-YYYY-MM-DD.json"` and `Cache-Control: no-store` — a cached
bundle is a silent data-loss bug.

GET rather than POST on purpose: `ReportControls.tsx:81` already downloads server bytes with
a plain `<a href>`, so this needs no `fetch`/`Blob`/object-URL code and the browser handles
the cookie. The selection is a short list of ids; a query string is adequate.

`require(portfolio_id, Viewer)` is called for **every** id before any data query. Note the
gate script's own header records that `WHERE portfolio_id IN (...)` slips past Rule B (the
`analytics.rs` precedent), so the enforcement here is the isolation matrix, not the script.

An empty or absent `portfolio_ids` is `422`, never an empty bundle. A user will later restore
an empty bundle, and "restore succeeded, everything vanished" is the worst outcome this
feature could produce.

**`GET /api/backup/capabilities`**
`{ "schema_versions": [1], "allow_price_restore": false, "max_bytes": 8388608,
"max_rows": 50000 }`. Lets the UI decide whether to offer the prices option and to reject an
oversized file before uploading it.

**`POST /api/backup/preview`** — body is the bundle. Performs full validation and returns the
plan without opening a write transaction:

```json
{
  "schema_version": 1,
  "mode": "replace",
  "delete": { "portfolios": 5, "assets": 31, "transactions": 842 },
  "insert": { "portfolios": 2, "assets": 9, "transactions": 140 },
  "prices": 0,
  "warnings": []
}
```

For `mode=copy` the delete counts are zero. `mode` is a query parameter on preview —
`POST /api/backup/preview?mode=replace` — so the counts shown match the mode the user selected,
and the UI re-previews when they change it.

**`POST /api/backup/restore?mode=replace|copy&apply_prices=false`** — body is the bundle.
Returns what was written and the new portfolio ids.

### Validation, then writing

Parsing and validation happen **before** a transaction is opened: every decimal parses, every
date parses in the shape its table stores, enum fields hold a known value, required fields are
present, and the total row count is under `max_rows`. Validation errors return as a list of bundle paths —
`portfolios[0].assets[2].transactions[15]: price "abc" is not a number` — capped at the
first 20 so a bad multi-megabyte file cannot produce a multi-megabyte error. The transaction
exists for consistency, not to catch typos.

### The restore transaction

Inside one `pool.begin()`, in this order:

1. If `mode=replace`: delete `transactions` → `assets` → `portfolios`, each scoped by a nested
   subquery on `owner_id`. Never a bare `DELETE`, and no caller-supplied id is involved.
2. Insert portfolios, recording `ref → new id`.
3. Insert assets using the portfolio map, extending it with `ref → new id`.
4. Insert transactions using the asset map.
5. If `apply_prices`: upsert price rows.

Commit, or roll the whole thing back. **Partial application is not a state this feature can
produce.**

**Id allocation uses `INSERT … RETURNING id`.** This is the existing house pattern —
`create_portfolio` uses `INSERT … RETURNING *` (`portfolios.rs:21-26`) — and `RETURNING` is
one of the few constructs SQLite (3.35+) and Postgres already agree on, so the restore ports
without needing `last_insert_rowid`, which would have pinned it to SQLite.

**Price rows use `ON CONFLICT (symbol, date) DO NOTHING`, not `DO UPDATE`.** Existing cached
prices stay authoritative; a bundle fills gaps only. The unique index
`uq_historical_prices_symbol_date` (`lib.rs:445`) already exists to support it. Combined with
the server flag, this narrows the poisoning surface to "dates not yet cached" rather than
"any price, any time".

### Error mapping

The rule `access.rs` already states is that a database failure is never collapsed into a
client error. Applied here:

| Condition | Status |
|---|---|
| Malformed bundle, failed validation, unknown `schema_version`, empty export selection, row or byte cap exceeded | `422` with the specific reasons |
| `apply_prices=true` while the server flag is off | `422`, never a silent ignore |
| Lock timeout (`SQLITE_BUSY` after the 5 s `busy_timeout`) | `503` with `Retry-After` |
| Any other database error | `500` |

### Configuration

- **Request body limit.** Rocket's default `limits.data` is 32 KiB and a real bundle exceeds
  it with a `413`. Raised to an explicit constant (8 MiB) **in code**, in `build_rocket`,
  rather than by deployment environment variable — so it is testable, cannot be forgotten in
  a manifest, and is reported by `/capabilities`. Bounded, not unbounded: this is a public
  endpoint.
- **`BACKUP_ALLOW_PRICE_RESTORE`** (default `false`), read alongside the existing
  `StartupConfig::from_env` pattern.
- New dependency: `schemars`, with `type: string` overrides for `rust_decimal` and
  `chrono::NaiveDate`.

## Portability discipline

The engine-selection work is a separate spec. This one commits to making its share of the
job mechanical. Every SQL statement in `src/backup/` must:

- use `?` placeholders — SQLite's syntax now, and what a `sqlx` `Any`-style pool rewrites for
  Postgres. Choosing the pool type is the other spec's decision, not this one's.
- name columns explicitly; no `INSERT … VALUES` without a column list.
- use `RETURNING` for allocated ids.
- use `ON CONFLICT (cols) DO NOTHING` where an upsert is needed.
- avoid SQLite-only constructs: `PRAGMA`, `pragma_table_info`, `INSERT OR REPLACE`,
  `INSERT OR IGNORE`, `INSERT OR ROLLBACK`, `INSERT OR ABORT`, `INSERT OR FAIL`,
  `AUTOINCREMENT`, `last_insert_rowid`, `strftime`, `julianday`, `datetime()`,
  `IFNULL`, `GROUP_CONCAT`.
- store and read decimals and dates as strings, as the format already dictates.

Enforced by `backend_rust/scripts/check_backup_portability.sh`, a static grep gate in the
style of `check_access_gate.sh`, failing case-insensitively — SQLite keywords are —
on any of the banned constructs in a statement under `src/backup/`, and on any
`INSERT INTO` whose column list does not immediately follow the table name, however the
string literal wraps. Prose is not a statement: comment lines
are skipped, since the rule is about SQL the module sends to the engine, and
`restore.rs` names `PRAGMA foreign_keys` in a comment to explain a schema fact. The
expected diff when the engine spec lands is the pool type in this module's signatures and
nothing else.

## Frontend changes

A **Backup** tab in the nav (`App.tsx:540-555`), one new component `BackupPanel.tsx`, two
cards.

**Export card.** Checkbox list of the caller's portfolios. An "include price history"
checkbox, disabled with an explanation when the capability is off. A download anchor built
the way `ReportControls.tsx:81` builds its CSV href. Export is disabled with no selection.

**Restore card.** File input, following the upload shape of `CsvImportModal.tsx`. A mode
radio defaulting to **New copy**. A Preview button rendering the plan counts. The Restore
button stays disabled until a preview has returned for the current file, and `replace`
additionally requires typing `REPLACE`. The prices checkbox appears only when the capability
allows it. After a restore, the summary states what was written and links each new portfolio.

## Testing

### Backend

- `tests/route_coverage_test.rs` — the four routes added to `PROTECTED_ROUTES` with their
  `Access` level. That file already fails on a mounted route missing from the inventory or
  from the OpenAPI document, so it is the drift check.
- `tests/isolation_test.rs` — user B cannot export user A's portfolio, and cannot restore a
  bundle containing A's data into A's space. Each denial paired with an owner success, per
  that file's existing rule.
- New `tests/backup_test.rs`:
  - selection of 1 of 2 portfolios contributes nothing from the unselected one;
  - empty selection is `422`;
  - unknown `schema_version` is `422` and writes nothing;
  - oversized bundle is `422` before any write;
  - `apply_prices` is `422` when the flag is off; when on it fills gaps and does **not**
    overwrite an existing `close_price`;
  - **round-trip**: export → restore into a fresh in-memory database → export again →
    identical modulo `created_at`. This is the test that proves the format is sufficient,
    which is what backup features most often fail at;
  - **atomicity**: a bundle valid for 200 rows and invalid on row 201 leaves the database
    byte-identical;
  - **restored data still computes**: the tax engine over a restored portfolio produces the
    same result as over the original. This proves the `ref` rewiring, not merely the row
    counts;
  - `mode=replace` removes all previously-owned rows and leaves the bundle's;
    `mode=copy` leaves existing rows untouched and yields new ids.
- `bash scripts/check_backup_portability.sh` and `bash scripts/check_access_gate.sh` pass.
- A test asserting `--print-backup-schema` output parses as JSON Schema and that the accepted
  version set is exactly what `docs/backup-schema/` contains.

### Frontend

Following `AppReport.test.tsx` and `CsvImportModal.integration.test.tsx`:

- the export href carries exactly the selected ids;
- no selection means no export;
- Restore is disabled until a preview has returned for the current file;
- `replace` is disabled until `REPLACE` is typed;
- the prices option is absent when the capability reports it off.

### CI

- Regenerate `docs/openapi/openapi.json` and `frontend/src/types/api.d.ts`; both are already
  diffed for freshness (`ci.yml:47-48`, `ci.yml:133`).
- New step: regenerate `docs/backup-schema/v1.json` and diff it, identical in shape to the
  OpenAPI freshness step.
- New step: `check_backup_portability.sh`, alongside the existing access-gate step.

## Docs

- `docs/backup-schema/v1.json`, `docs/backup-schema/README.md`.
- A Backup section in `docs/walkthrough.md` for the manual run-through, including the
  restore-into-a-fresh-instance step CI cannot perform.
- `docs/openapi/openapi.json` regenerated.

## Known follow-ups for the engine-selection spec

Recorded here so they are not lost, and deliberately not solved here:

1. **Restore isolation under concurrent writes.** `mode=replace` deletes rows a concurrent
   request may be touching. On SQLite today this is moot — one writer, `replicas: 1`
   (`k8s/app-deployment.yaml:9`). Under Postgres with several replicas the right answer is
   probably a `SERIALIZABLE` restore transaction. This spec does not guess at it.
2. **Pool type in `src/backup/`.** The module must be rewired to whatever pool the engine
   spec chooses. Per *Portability discipline* that is the only expected change.
3. **`historical_prices` ownership.** It has no `owner_id` and is shared across tenants. The
   price-restore flag is a mitigation, not a model. If tenants ever contribute prices at
   all, that table needs an owner or a provenance column.
5. **The row ceiling is one number, and it is mode-blind.** `MAX_BUNDLE_ROWS` counts
   `portfolios + assets + transactions + prices` at both ends: `export` refuses a selection
   over it, and `collect` refuses a bundle over it whether or not `apply_prices` was asked
   for. Two consequences, both accepted: a price-heavy selection that used to export now
   answers `422` (the message names the row limit and offers omitting the price history as
   the remedy), and a hand-authored bundle carrying more than the ceiling in prices alone is
   refused even by a restore that would write none of them. An app-produced bundle cannot
   reach the second state, because the export side counts the same rows. If a mode-dependent
   cap is ever wanted, the place it belongs is a separate price ceiling, not a conditional on
   a query parameter.
6. **Multi-replica pricing rate limits.** `CurrencyService` holds a process-local cache and a
   `Mutex<Option<Instant>>` rate limiter (`currency_service.rs:205-206`). N replicas means N
   caches and N× the request rate against the pricing provider. Independent of this feature,
   but it is a blocker for the same `replicas: 1` removal.

## Out of scope

Merge/dedupe restore, operator or scheduled backups, server-side bundle storage,
cross-user sharing, background restore jobs, the engine-selection feature itself, and any
change to `historical_prices`' schema.
