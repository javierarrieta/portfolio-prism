# MCP Agent Interface Design

- **Status:** approved design, not yet implemented
- **Date:** 2026-11-18
- **Builds on:** `2026-11-04-oidc-identity-isolation-design.md` (the auth chokepoint), `2026-11-13-backup-export-restore-design.md` and `2026-11-16-import-preview-design.md` (preview-before-write), `2026-07-29-csv-import-design.md` (the browser CSV importer whose rules this formalizes)
- **Supersedes nothing.** No existing endpoint changes behaviour except where Workstream 0 says so explicitly.

## Overview

Add a Model Context Protocol server to Portfolio Prism so an AI agent can answer questions
about a user's portfolios and record data into them, including importing a whole portfolio
from a broker statement the agent has read.

The MCP server is **a module of the existing backend, not a separate program**. It is a
second HTTP listener inside the `backend_rust` process speaking MCP streamable HTTP,
authenticated by a new scoped personal access token. There is no Node.js anywhere in the
design, and nothing runs on the user's workstation.

No agent tool call writes a row directly. Every agent write creates a staged batch, and a
staged batch becomes a write only after a human has approved it in the web UI, over a
session cookie. The agent can stage, re-render, discard, and — once a human has approved —
apply. The backend cannot prove that a human read a preview, and this spec does not claim it
does; it guarantees two things instead. The preview exists somewhere the agent did not
write, and no token can move a batch out of `staged` on its own.

## Why this shape

Three constraints decided the architecture, and each was chosen deliberately rather than
fallen into:

1. **The model normalizes the broker statement.** There is no parser in the loop for broker
   dialects; the agent reads the statement and emits structured rows. That is the whole
   reason the write path is staged: a model that misreads `1.5` as `15` produces a
   *plausible* portfolio and a wrong tax report, and nothing downstream would notice.
   The deterministic gate therefore sits **after** the model, in the backend.
2. **No Node.** The concern is supply-chain exposure and code execution at install time.
   `rmcp` is the official Rust SDK (`modelcontextprotocol/rust-sdk`), v3.11.0 as of
   2026-11-17, Apache-2.0, MSRV 1.88, implementing the `2026-07-28` spec revision. The
   local toolchain is `rustc 1.98.1`; the binding floor is the build image's
   `rust:1.92-slim` (`backend_rust/Dockerfile:4`), which clears that MSRV. Cost:
   `cargo tree -p rmcp -e normal` lists 43 direct dependencies, against the 449 packages
   already in `backend_rust/Cargo.lock`, under the same lockfile and audit posture as the
   rest of the crate.
3. **The chokepoint stays the chokepoint.** `access.rs` is the only module permitted to
   resolve a caller-supplied id, and `check_access_gate.sh` enforces it. Any design that
   let an agent write rows without going through it would make that gate decorative.

## Success criteria

- An agent with a read-only token can answer "what is my realized gain in 2025?" and
  "which lots are oldest?" without any credential capable of writing a row.
- An agent handed a broker statement can stage a full portfolio import, show a preview
  that names what it holds, and — after a human has approved that preview in the web UI —
  apply it, with exact counts, exact decimal values, and duplicates flagged rather than
  silently dropped.
- A staged batch cannot be applied before a human approves it, cannot be applied twice,
  cannot be applied by a different user, and cannot be applied after it expires.
- A personal access token cannot call the interactive `/api` surface at all. Not denied by
  a scope check — structurally unable, because that surface never reads an
  `Authorization` header.
- `cargo test`, `check_access_gate.sh`, and the new agent gate all pass, and the new gate
  has a mutation test proving it can fail.

## Requirements

1. Read access to portfolios, assets, transactions, tax summaries, the dated tax report,
   performance, and sell P&L, through MCP tools.
2. Create access to portfolios, assets, and transactions, only through a staged batch that a
   human has approved in the web UI. Updating and deleting stay in the web UI.
3. Bulk import of a full portfolio (portfolios, assets, transactions) from rows the agent
   normalized out of a broker statement, with the same preview discipline.
4. Scoped, individually revocable credentials, self-service from the web UI.
5. The existing session-cookie auth, the OIDC flow, and every current `/api` route keep
   working unchanged.
6. The whole feature is off unless explicitly configured on. An existing deployment must
   not grow a public MCP endpoint by upgrading.
7. The preview reaches the human complete. A preview that the client truncates in transit is
   not a control — see *The client, and the preview's size budget*.

### Scale

The primary user's largest portfolio is roughly 20 assets with 1–2 transactions each, so a
real batch is 20–40 rows. Larger statements are expected later, from other people; they are
not specified here. Nothing in this design is sized for volume. The display caps, the 16 KB
preview budget and the 30-second deadline are guards that stop a future large import from
becoming an incident, not machinery the current user needs — and a batch of 40 rows is why
there is no chunking, no progress protocol, and no asynchronous job in this design. Building
for volume that does not exist yet is out of scope on the same argument that removed
`stage_edit`.

## Non-goals

- **`backup/*` is not exposed.** No export, no preview, no restore. There is no
  `backup` scope, so it is not a policy an agent can forget — it is a capability that
  cannot be minted. `restore` with `mode=replace` deletes everything a user owns and stays
  in the web UI.
- **No broker-statement parser, and no knowledge of statement formats.** No CSV dialect
  mapping, no PDF extraction, no per-broker configuration, no encoding sniffing, no date-format
  auto-detection. The boundary is total: **the client owns turning a statement into rows; the
  server owns saying precisely what a valid row is.** A broker's column names, decimal comma
  and `DD.MM.YYYY` dates are the agent's problem. Giving the server a second normalization
  path would create two places where a number can be wrong, and the one nobody is reviewing.
  What the server owes the agent instead is a schema that cannot be guessed at and errors that
  can be acted on — see *What the server owes the agent*.
- **No stdio transport.** Streamable HTTP only. A stdio shim for clients that cannot speak
  HTTP is a separate decision with its own spec.
- **No MCP resources, prompts, sampling, or subscriptions.** Tools only.
- **No multi-replica session sharing.** The session manager is in-memory; see *Sessions*.
- **No apply from the web UI.** The staged-batches screen lists batches and renders the
  stored preview; its only write is the approval itself. The human does not edit rows there
  and the browser never writes a portfolio row. The agent reads the preview in the chat
  *and* in that screen — the screen is the copy the model did not write.
- **No update or delete from the agent.** The agent creates; it never revises or removes.
  This is not a policy an agent can forget — there is no tool that does it. The reason is
  scope, not safety: no requirement asks for it, and every update or delete needs a
  caller-supplied id, which would widen `access.rs`, the gate script's job, and the
  isolation matrix for a capability nothing has asked for. Reinstatement needs a reason
  first, and an update tool would need the same approval discipline as a create.
- **No `asset_type` constraint.** See Workstream 0, item 5.

## Decisions and rationale

**The MCP server is a module, not a process.** One deployable, one image, no version skew
against the API it fronts, and no second process holding a credential on a laptop. Tools
call `engines::` and `services::` directly — no HTTP hop, no second serializer, money
stays `rust_decimal` end to end.

**`rmcp`'s HTTP transport is a tower service, so it cannot mount into Rocket.** PR #228
reshaped it as `StreamableHttpService` specifically so it works with any tower-compatible
server, and PR #642 then removed the axum dependency because axum was never used. Rocket
0.5 is not tower-based. The chosen realization is a **second listener in the same
process**: at launch, spawn a `hyper` server on a second port serving the tower service.
`hyper 1.10.1`, `hyper-util 0.1.20`, `http 1.4.2` and `tower 0.5.3` are already in
`Cargo.lock` via reqwest — the `0.6.11` in that lock belongs to `tower-http`, not `tower`.
None of the four is a direct dependency of `backend_rust/Cargo.toml` today, so adding `rmcp`
means adding them to the manifest with the features the transport needs, at whatever
versions its resolution forces; whether `rmcp` needs `tower` ≥ 0.6 is a verification item,
not an assumption. Rejected: hand-writing the streamable-HTTP transport on rmcp's
`Transport` trait, which would mean owning protocol negotiation, session ids and SSE.
Mounting the tower service inside Rocket is **not** rejected — see *Verification items*. The
grounds this document originally gave for rejecting it (hand-rolling SSE through a Rocket
`Responder`) do not apply: `2026-07-28` clients are served statelessly, and
`json_response(true)` replies with a plain JSON body.

**Two listeners, two credential types.** The Rocket API accepts only session cookies; the
agent listener accepts only bearer tokens. Neither reads the other's header. An agent
cannot call a write endpoint on `/api`, because those handlers do not look at bearer
tokens — and by the same construction a human approval cannot be forged from the agent
side, because `POST /api/staged-batches/<handle>/approve` is reachable only with a session
cookie the agent never holds (see *The approval gate*). It also requires no change to the 21
existing route handlers.

**Staging is server-side state, not a client-side promise.** A staged batch is a row in
Postgres holding the canonicalized payload and the rendered preview. A handle is an opaque
random string, not the row id. A client cannot bypass the gate by omitting a step:
`apply_staged` accepts only a handle whose row is `status='approved'`, and only a session
cookie can put it there. It also survives an MCP session dropping.

**Token expiry is opt-in, not mandatory.** Expiry protects against a forgotten credential
sitting in a config file; it does nothing against the risk this feature actually creates,
which is a model staging wrong rows. Mandatory expiry on a token that lives in an agent
client's config produces a broken session at an inconvenient moment and teaches the owner
to mint with their eyes shut. The controls that matter are scopes, revocation, and
visibility of what exists — so the UI shows staleness instead of the server enforcing a
calendar.

**Minting is self-service in the UI, and the backend owns the lifecycle.** `auth::pat`
implements every operation; the API routes and the listener both call it; nothing else
implements any of them.

---

## Workstream 0 — prerequisites: transaction type, the number boundary, and the write seam

Ships and deploys **before** the agent module, because it changes live API behaviour.

Two findings from the current schema motivate this. `transactions.type` is `TEXT` with no
`CHECK` and there is no Rust enum: `create_transaction` (`api_routes/transactions.rs:239`)
does `tx.r#type.to_uppercase()`, compares against `"SPLIT"`, and stores the result. Any
other string passes — `"DIVIDEND"` is stored and then **silently ignored**:
`tax_engine.rs:665-771` is `if BUY / else if SELL / else if SPLIT` with no terminal `else`,
and `stats_engine.rs:250-256` is the same shape, so a junk type produces no lot and no
position change. Invisible tax data is worse than a wrong lot, because nothing looks wrong.
The closed set already exists as a constant: `backup/mod.rs:222` `KNOWN_TRANSACTION_TYPES`. Separately, `transactions.quantity`, `price` and `fee` are `TEXT`, not `NUMERIC`;
the app preserves exactness by storing `Decimal::to_string()`, which is correct — but it
means a value that entered as a JSON *number* has already passed through `f64` before
becoming a `Decimal`.

1. **`TxType` enum.** `#[serde(rename_all = "UPPERCASE")] enum TxType { Buy, Sell, Split }`
   with `Display` — serde matches the style name exactly, so the lowercase spelling is a
   compile error. Defined in `models.rs`; `schemas::TransactionCreate.r#type` (`schemas.rs:18`)
   becomes `TxType`, and the `to_uppercase()` call disappears because the accepted wire values
   are exactly `BUY|SELL|SPLIT`. Anything else is a serde `422`. **The read side stays
   `String`**: `models::Transaction.r#type` and `schemas::TransactionOut.r#type` are `FromRow`
   decode targets for rows the `CHECK` below still accepts in any case, and converting them
   would fail to decode every legacy lowercase row. No frontend change is required for the
   type: `csvValidator.ts:98-100` already rejects everything outside `BUY|SELL|SPLIT`.
2. **Migration `0002_transaction_type_check.sql`.** Normalize existing rows with
   `upper(btrim(type))` — case and surrounding whitespace are not semantic changes. Then
   **abort, listing the offending values**, if anything is still outside the set; rewriting
   `DIVIDEND` to `BUY` would be inventing tax data. A `.sql` file cannot abort and list
   without plpgsql, so the check is a `DO $$ … RAISE EXCEPTION … END $$;` block, and because
   sqlx runs each file in one transaction the normalization rolls back with the abort. Then
   add `CHECK (upper(btrim(type)) IN ('BUY','SELL','SPLIT'))`. Case-insensitive on purpose:
   rows written before this migration may hold `buy`, and the `CHECK` must accept them
   without a data rewrite.

   Note the case-sensitivity triangle this leaves. The partial index
   `uq_split_asset_date` is `WHERE type = 'SPLIT'` (`migrations/0001_init.sql:89-91`) and is
   case-sensitive, and `scripts/check_migrations.sh:169-175` pins that predicate literally.
   A `'split'` row would satisfy the `CHECK` yet escape the split dedupe, so the pre-flight
   audit must also look for `(asset_id, day)` duplicates among `SPLIT` rows after
   normalization. The only lowercase writer in the tree is a test seed (`src/access.rs:260`).

   `sqlx::migrate!` runs at startup, so a failing migration means the app does not boot
   (`main.rs:74-76` → `refuse()` → exit 1 → crashloop). The audit query
   (`SELECT DISTINCT type FROM transactions WHERE upper(btrim(type)) NOT IN ('BUY','SELL','SPLIT')`)
   is run by the operator from `backend_rust/scripts/audit_transaction_types.sh` before the
   deploy that carries `0002`: exit `0` clean, `1` out-of-set rows found (printing
   `SELECT DISTINCT type`), `2` could not run. The abort itself is proven by
   `tests/migration_type_check_test.rs`, which loads migrations at runtime with
   `sqlx::migrate::Migrator::new`, applies `0001`, inserts a `'DIVIDEND'` row, applies
   `0002`, and asserts the error names the value. It cannot use `test_db::test_pool`, which
   runs the embedded `sqlx::migrate!` before the test body, and it cannot live in
   `check_migrations.sh`, which inspects a database and cannot observe a migration aborting.
3. **The bundle schema already constrains this set — verify, do not add.**
   `docs/backup-schema/v1.json:134-142` enumerates `type` from `KNOWN_TRANSACTION_TYPES`,
   `backup/mod.rs:467` refuses an out-of-set type during `validate()` — so a junk type is
   already a `422` at preview, not a mid-restore `CHECK` violation — and
   `backup/schema.rs:49-56` pins the artifact against the constant. The remaining work is
   the database `CHECK` and confirming no shipped bundle fixture violates it.
   `backup/restore.rs:365-371` inserts the same column; `services/currency_service.rs:139`
   inserts the literal `'SPLIT'` and is unaffected.
4. **Money arrives as a JSON string; the deserializer rejects JSON numbers.** Add
   `db_types::strict_decimal_json` (string-only, `^-?\d+(\.\d{1,12})?$`, error names the
   fix) and apply it to the three `TransactionCreate` fields and the agent DTOs. **Do not
   modify `db_types::decimal_json`** — that is the *response* serializer at roughly 55 sites
   in `schemas.rs`, and its `visit_f64` arm is load-bearing for `TaxLot` and
   `AssetTaxSummary` round-trips. Scale is capped at 12 decimal places as a validation rule,
   not inferred from a column, because the column is `TEXT` and will hold anything.

   **This breaks the browser CSV importer, which must move first.**
   `frontend/src/utils/decimal.ts` is a *read-side* normalizer: it turns decimal strings
   into JS numbers (`:16-23`, `:97-104`) and sends nothing. The manual form already sends
   strings (`PortfolioDetail.tsx:15-21,171-177`), but the CSV importer sends JSON numbers —
   `csvValidator.ts:187-189` types `quantity`/`price`/`fee` as `number` and
   `CsvImportModal.tsx:170-176` posts them. A prerequisite frontend PR makes `parseNumeric`
   return a canonical decimal string, updates `ValidatedRow`, the dedup key at
   `CsvImportModal.tsx:163`, `test/mocks/handlers.ts:105-111`, the assertions in
   `csvValidator.test.ts`, and the contract comment at `frontend/src/types.ts:8` that
   currently permits either form. Land and deploy it before the backend rejects numbers.

   **The `TEXT` columns are not to be changed to `NUMERIC`.** `tests/decimal_migration_test.rs`
   documents why — a binary float cannot hold 9.99, and cost basis drifting by a rounding
   unit per row is wrong on tax figures — and `scripts/check_migrations.sh` fails the build
   if any of those columns is not `text`. The fix is at the boundary, never in the storage
   type.
5. **`asset_type` stays free text.** Same shape, but no engine reads it. Bundle validation
   and the bundle schema already constrain it (`backup/mod.rs:431-434`, `v1.json:6-15`),
   which is the only reason that is safe. Constraining it now would be unrelated refactoring
   with a migration attached. Add a roadmap entry to `AGENTS.md` → *Roadmap / Current
   Focus*: `asset_type` is unconstrained at the database, and constraining it needs an
   inventory of live values first.
6. **Extract the shared write services.** This is the prerequisite the rest of the design
   had assumed but never named. `services/` currently holds only `currency_service.rs`;
   transaction and asset inserts live inline in `api_routes/transactions.rs`. Extract
   `services::transactions::insert(pool, &mut tx, asset_id, &TxCreate)` from
   `api_routes/transactions.rs:239-262` — type canonicalisation, the SPLIT quantity/price
   rule (`:240-243`), fee zeroing for SPLIT (`:244`), the 1900-01-01 … 2100-01-01 bound
   (`:248-252`), the INSERT (`:254-262`) — and `services::assets::insert(pool, &mut tx,
   portfolio_id, &AssetCreate)` from `:31-96` (ISIN shape check, duplicate-ISIN conflict,
   currency detection, INSERT). Both take `&mut tx`, so a caller can compose them inside one
   transaction. The Rocket handlers call them with no behaviour change; `staging::apply`
   calls them inside the same transaction as the single-use flip. They become the only code
   in the repo that inserts a transaction or an asset outside `backup::restore`.

   **Ship it as its own `refactor/` PR in this workstream.** It is independently
   revertible, the existing suite is the proof it changed nothing, and Workstream 2 cannot
   be built before it exists.

   **Required check, in the same PR:** `check_backup_portability.sh` asserts that inserts
   live in `src/backup/`. If the extraction moves any INSERT out of that directory, that
   gate's coverage shrinks silently. The PR must show the gate's inspected-file list before
   and after, and extend its scope to the new directory in the same commit if anything
   moved. `check_access_gate.sh` is unaffected — Rule A is file-level and the handlers keep
   their `require*` calls — but `tests/access_gate_test.rs:87` pins the inspected-handler
   count, so any new route file moves that number too.

Existing validation that the agent module **reuses rather than re-invents**: the SPLIT
quantity/price rules and the 1900-01-01 … 2100-01-01 date bound already in
`create_transaction`.

---

## Workstream 1 — personal access tokens

### The invariant

`resolve_caller` (`auth/guard.rs`) already owns "turn a request into an `AuthedUser`", is
already the only path shared by `AuthFairing` and the `/auth/me` guard, and already keeps
`Ok(None)` (deny) distinct from `Err` (503). It gains a `CredentialKind` parameter —
`SessionOnly` from the fairing, `TokenOnly` from the agent listener. Both branches converge
on a new `user_by_id(pool, uid)` extracted from today's guard, which keeps the `is_active`
check and the `token_version` comparison in one place. A token yields a `uid` and falls
through to that same lookup, so a deactivated account loses its tokens for free.

`AuthedUser` gains a `credential` field: `Credential::Session` or
`Credential::Token { id, scopes }`. `require_scope(&caller, Scope::Import)` returns ok for a
session credential, and for a token credential only when that scope is present. There are
two scopes, `read` and `import`; the enum has no `Write` variant, because the agent creates
rows and does nothing else.

### Schema — `0003_api_token.sql`

```
api_token(
  id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  user_id      BIGINT NOT NULL REFERENCES users(id),
  secret_hash  TEXT NOT NULL UNIQUE,
  label        TEXT NOT NULL,
  scopes       TEXT[] NOT NULL,
  expires_at   TIMESTAMPTZ,          -- NULL = no expiry
  last_used_at TIMESTAMPTZ,
  revoked_at   TIMESTAMPTZ,
  created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
)
CREATE INDEX api_token_user_idx ON api_token (user_id);
```

Secret: `pp_` + 32 random bytes, base62. Stored as SHA-256 hex, returned exactly once.
32 bytes is 256 bits of entropy at the source, which makes online guessing impractical, so
the agent listener adds no attempt throttling; do not add throttling later without stating a
reason in a new spec. Base62 changes only the encoded length (≈43 characters), not the
entropy.

**Scopes:** `read` and `import`. No `backup` scope exists, and there is no `write` scope:
the agent only ever creates rows, so `import` already covers every write it can attempt, and
a third scope would name nothing. Scopes are a ceiling on what a token may do; they are not
a record of who is sitting in front of it. The human step is the approval state, not a
scope — see *The approval gate*. Staging requires `import`, so a `read`-only token cannot
create a staged batch at all; there is deliberately no preview-only path that writes a row
under a read credential.

**Expiry** is enforced in `pat::lookup` with one comparison, the same shape as the session
`exp` check already in `guard.rs`. Default at mint is no expiry. Enforced at lookup only:
no valid token means no tool call, so a dead credential cannot apply a staged batch.
`logout-all` bumps `users.token_version` and kills sessions; tokens deliberately survive
it, because a token is not a browser session — revocation is the answer to a leaked token.

**`last_used_at`** updates at most once a minute per token, fire-and-forget, so tool calls
do not each pay a write.

### `auth/pat.rs` — the only implementation of every operation

- `mint(pool, user_id, label, scopes, expires_at) -> Minted` — stores only the hash; the
  plaintext exists in the return value and nowhere else in the codebase.
- `list(pool, user_id) -> Vec<TokenInfo>` — metadata only; the query never selects a hash
  into a response type.
- `revoke(pool, user_id, token_id)` — sets `revoked_at`, effective on the next call because
  every request looks the row up live.
- `lookup(pool, secret) -> Option<AuthedUser>` — the agent listener's only entry point.

### `api_routes/tokens.rs` — session-authenticated

- `POST /api/tokens` — `{label, scopes[], expires_in_days?}` → `{id, …, secret}`
- `GET /api/tokens` — the list, no secret, no hash
- `DELETE /api/tokens/<id>` — revoke

These are ordinary `/api` routes, so `AuthFairing` covers them and `check_access_gate.sh`
sees them. Two consequences, stated now rather than discovered in CI:

- `DELETE /api/tokens/<id>` resolves a caller-supplied id, so it goes through
  `access.rs`: a new `require_own_token(pool, user_id, token_id, Access::Owner)` beside the
  existing `require*` functions, because that module's stated job is being the only place
  a caller-supplied id resolves.
- `POST` and `GET` carry the existing
  `// access-gate: global-config (no caller-supplied id)` marker, which the script prints
  whenever it honours it.
- `tests/route_coverage_test.rs` holds a `PROTECTED_ROUTES` inventory and fails on a mounted
  `/api` route missing from it or from the OpenAPI document, so all three routes must be
  added there — `POST` and `GET` with no resource level (they check no caller-supplied id)
  and `DELETE` at `Owner`.

### The Access screen

A view in the existing nav rail (`frontend/src/App.tsx:657`), component
`AccessTokensPanel.tsx`, shaped like `BackupPanel.tsx`: list with label / scopes / created
/ last used / expires, a mint form with scope checkboxes and an expiry select, and a
one-time reveal modal with a copy button and an explicit "this is shown once" line.
Anything idle more than 90 days is marked **unused for 90 days** — the nudge that gets
acted on, costing one `WHERE last_used_at < now() - interval '90 days'` in the list query.
Revoke is a plain confirm, not a typed word: a token is revocable and re-mintable, unlike
`mode=replace`.

Contract flows the normal way: `utoipa` annotations, then
`cargo run --bin backend_rust -- --print-openapi > docs/openapi/openapi.json`, then
`npm run generate-types` in `frontend/`.

---

## Workstream 2 — the agent module

### Layout — `backend_rust/src/agent/`

- `protocol.rs` — request/response DTOs for every tool. `rust_decimal` for every money and
  quantity field, `serde` with `deny_unknown_fields`, `schemars` derives so the tool input
  schema and the deserialization are the same type.
- `staging.rs` — the staged-batch lifecycle: validate a proposal, persist it, hand out a
  handle, expire it, apply it. Owns `agent_staged_batch` and nothing else.
- `tools.rs` — the `ServerHandler` and the `#[tool]` handlers. Each resolves its caller,
  calls `require*` before touching a row, then calls `services::` for writes (extracted in
  Workstream 0, item 6) and `engines::` for reads. There is no second write path: if a tool
  needs an INSERT and no `services::` function performs it, that function is written first.
- `listener.rs` — binds the streamable-HTTP tower service on the second port at launch.

Two things outside `agent/` are prerequisites, not follow-ups, because nothing can be applied
without them:

- `api_routes/staged_batches.rs` — the three session-authenticated routes in *The approval
  gate*. Ordinary `/api` handlers: `AuthedUser`, a `require*` call before touching a row,
  entries in the `lib.rs` mount list and the `openapi.rs` inventory, and the usual
  `PROTECTED_ROUTES` and `check_access_gate.sh` obligations.
- `frontend/src/components/StagedBatchesPanel.tsx` — the list, the rendered preview, and the
  Approve button. It renders the stored `preview` JSONB; it does not recompute it, so what a
  human reads is byte-for-byte what the agent was shown. Like `backup`, its view must render
  outside the `currentPortfolio ? …` branch in `App.tsx`, or it is a blank page whenever no
  portfolio is selected.

Off by default: `listener.rs` binds only when `AGENT_LISTEN` is set, following the
precedent at `lib.rs:270` where the OIDC routes are mounted only when there is a provider
to talk to.

### Schema — `0004_agent_staged_batch.sql`

```
agent_staged_batch(
  id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  handle     TEXT NOT NULL UNIQUE,        -- 32 random bytes, not the row id
  user_id    BIGINT NOT NULL REFERENCES users(id),
  payload    JSONB NOT NULL,              -- canonicalized proposal
  preview    JSONB NOT NULL,              -- what the human was shown
  status     TEXT NOT NULL DEFAULT 'staged'
             CHECK (status IN ('staged','approved','applied','discarded')),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  expires_at TIMESTAMPTZ NOT NULL,
  approved_at TIMESTAMPTZ,          -- the human act; NULL until a session approves
  approved_by BIGINT,               -- users(id) of the session that approved it
  applied_at TIMESTAMPTZ
)
CREATE INDEX agent_staged_batch_user_status_idx ON agent_staged_batch (user_id, status);
```

**Handle rules.** Bound to its `user_id`, and resolved through
`access.rs::require_own_staged_batch` — a caller-supplied id resolving to a row goes
through the chokepoint, same rule as tokens. Lifetime: `AGENT_BATCH_TTL_MINUTES` (default
60). `apply_staged` compares `expires_at` and rejects an expired handle. When a user stages
a new batch, the same transaction deletes that user's rows with `status IN ('staged',
'approved') AND expires_at < now()`. There is no scheduled job, so rows belonging to a user
who never stages again remain until that user stages again.

### The approval gate

The staging gate is a control only if the step that turns a proposal into a write is one the
agent cannot perform. That step is therefore **not a scope**: a token holding a scope can
use it whenever it likes, which is the hole a scope-shaped control leaves open. It is a state
transition only a session cookie can cause.

| Step | Surface | Credential | Effect |
|---|---|---|---|
| `stage_import` | agent listener | PAT with `import` | inserts a row at `status='staged'`, returns handle and preview |
| `GET /api/staged-batches` | `/api` | session cookie | lists batches: handle, counts, status, expiry |
| `GET /api/staged-batches/<handle>` | `/api` | session cookie | renders the stored `preview` JSONB verbatim |
| `POST /api/staged-batches/<handle>/approve` | `/api` | session cookie | `staged → approved`; records `approved_at`, `approved_by` |
| `apply_staged` | agent listener | PAT with `import` | `approved → applied`, writes the rows |
| `discard_staged` | agent listener | PAT with `import` | `→ discarded`; no approval required |

Three consequences, stated here rather than discovered in implementation:

- **The web UI never writes a portfolio row.** Its only write is the approval. That is what
  the non-goal means, and it is why the screen is a prerequisite rather than a deferred
  convenience: without it, nothing can ever be applied.
- **No nonce is handed to the agent.** The agent does not carry a code relayed from the
  human, because a code that travelled through the model is evidence of nothing. It calls
  `apply_staged` and the server checks the row's state; the agent may poll `get_staged`
  until it changes.
- **Six new `/api` routes in total** — three token routes in Workstream 1, three
  staged-batch routes here — move the cardinality pins in `tests/route_coverage_test.rs`
  at `:169` and `:291` from 21 to 27, and `with_level` at `:304` from 15 to 18, counting
  `GET /api/staged-batches/<handle>` and its `approve` as `Owner`-level and the list route
  as level-less. `access.rs` also gains `require_own_staged_batch`, which widens its
  documented remit beyond portfolio, asset and transaction; say so in its header.

**Single use is a conditional update inside the write transaction**, not a step before it:

```
BEGIN;
UPDATE agent_staged_batch
   SET status='applied', applied_at=now()
 WHERE handle=$1 AND user_id=$2 AND status='approved'
 RETURNING id;
-- zero rows: ROLLBACK, return already_applied (or not_approved / expired);
-- nothing else in the transaction runs.
<inserts>;
COMMIT;
```

Carrying `user_id` in the same statement makes ownership and state transition one check.
Committing the flip before the inserts would leave a batch marked `applied` with zero rows
written if the write then failed — including on the 30-second per-call deadline, which
equals the pool's `statement_timeout` — and there would be no way to retry it.

### Validation, deterministic and server-side

`deny_unknown_fields` on every DTO. Then:

- Transaction type is the closed `TxType` from Workstream 0.
- Money and quantity match `^-?\d+(\.\d{1,12})?$` and arrive as strings. A comma decimal
  or a thousands separator is rejected with an error that says why and shows the fix —
  those strings would otherwise be stored verbatim in a `TEXT` column and only fail on
  read, and a German broker statement is exactly what produces `142,30`.
- `quantity > 0`, `price >= 0`, `fee >= 0`. Zero price is legitimate rather than junk:
  `2026-11-16-import-preview-design.md` treats zero-price rows as the valid output of a
  stock-dividend flow, which is why they are accepted and flagged, never rejected.
- Dates are strict ISO `YYYY-MM-DD`, stored as `TIMESTAMPTZ` at UTC midnight. The
  1900-01-01 … 2100-01-01 bound is reused from `create_transaction`, but the **format is
  not**: the interactive endpoint takes `DateTime<Utc>` and deliberately rejects a bare date
  (`schemas.rs:25`, pinned by `tests/date_boundary_test.rs:46-57` — "a bare date must not
  silently become midnight UTC"). The agent DTO needs its own date type; the two paths
  cannot share one deserializer. The browser importer's multi-format
  auto-detection exists because a human sits in front of dropdowns; the agent has no
  dropdowns.
- **The model is the parser, so the preview is the only check.** Nothing server-side can
  distinguish a correct `1234.56` from a misread `1.234,56`, and a decimal-comma misparse is
  a 1000× error on a tax figure. At the current scale every row fits in the preview, so a
  misparse is visible to a human before apply — which is why the preview renders rows and not
  only totals, and why the 16 KB budget is a safety property rather than a performance one.
  The server's contribution is that the wire format cannot lie: money is a strict decimal
  string, dates are strict `YYYY-MM-DD`, so anything that reaches the preview has already
  survived validation.
- ISIN, when present, passes a mod-10 checksum. A Rust validator already exists —
  `api_routes::lookup::is_valid_isin` (`lookup.rs:186`), shape only, already used by the REST
  lookup route and by bundle validation (`backup/mod.rs:442`) — so it is strengthened in
  place rather than duplicated; a second rule would make the agent reject rows the API
  accepts. Adding the checksum also tightens lookup and restore for bundles that pass today,
  which is a behaviour change to call out in that PR. A bad ISIN is a named rejection,
  not a silently unmatched asset.

### What the server owes the agent

Dropping format knowledge on the client side is only safe if the server's side of that
boundary is exact. Three obligations, all cheap:

- **The schema cannot be guessed at.** Field names, strict types, required-versus-optional
  spelled out, and one complete worked example inside the tool description. In pi the default
  exposure is `codemode`, so the description is often the only documentation the model ever
  sees; a field whose meaning lives in a wiki page is a field that will be sent wrong.
- **Every rejection is actionable in one turn.** The message names the field, states the
  expected form, and states the fix — because the agent gets one round trip to correct before
  a human sees a failure instead of a preview. `"invalid string"` is a defect even when the
  status code is right. For the decimal case specifically: *expected a decimal string such as
  `"1234.56"`; the value contains a comma — if the source uses a decimal comma, replace it
  with a period before sending*. Same rule for a bare date, an unknown type, and an ISIN that
  fails its checksum.
- **Say what the server cannot catch.** A misparse that is *well-formed* — `1.234,56` read as
  `1.23` — passes the strict decimal rule, the date rule and every other check, because the
  bytes are valid. Nothing server-side detects it. The only control is a human reading the
  rendered preview, which is a real control at 40 rows where every row is visible and a weak
  one at 1,200 rows where they are not. That is the honest limit of "the model normalizes", and
  it is stated here rather than left for someone to discover.

### The preview

Follows `2026-11-16-import-preview-design.md`: exact totals always, display bounded,
anomalies flagged and never blocking.

Rendered by `backup::detail` and its `flag` codes, not by a second renderer with its own
vocabulary: `staging` hands that module a one-portfolio sub-bundle and stores what comes
back. The two genuinely new flags (`assumed_currency`, `duplicate_candidate`) are added to
`detail::flag` rather than invented at the call site. If `detail_from_bundle` turns out to be
coupled to whole-bundle shape, reuse its `PlanSide`/`PlanAsset` types and flag codes behind a
thin adapter instead, and record that here rather than forking the vocabulary.

Per portfolio: name, currency, asset count, transaction count, date span. Per security:
symbol, ISIN, counts, span. An explicit **matched vs will-auto-create** split for assets.
Anomaly list, each with a computable rule: `unknown_symbol` (no asset matches the ISIN, or
the symbol plus currency, within the target); `duplicate_candidate` (same date, asset, type,
quantity, price and fee as another row in this batch, compared as `Decimal`, never as
strings); `currency_differs` (row currency differs from the portfolio base currency);
`assumed_currency` (currency defaulted rather than supplied); `future_dated` (date later
than today); `price_out_of_range` (price more than 10× above or 1/10 below the other rows of
the same asset in this batch, and only when that asset has at least two other rows). When a
list is truncated the label reads `showing 25 of 1,204`.

### The client, and the preview's size budget

The client is a coding agent, not a chat window. **pi** is the primary client and Hermes
Agent the secondary. That is a hard constraint, because the preview *is* the human control
surface and it travels through this software.

pi's documented behaviour (`docs/mcp.md`):

- **Text results over 20 KB reach the model with their middle removed**, around a
  `…N chars truncated…` marker, with the full text written to a temporary file on the user's
  disk. A preview larger than that is not a preview: the human reads a mangled version with
  no signal that anything is missing. **The rendered preview therefore has a 16 KB budget**,
  deliberately below the threshold. If a batch would exceed it, `stage_import` returns the
  totals, the anomaly list, as many rows as fit, and the handle, with the instruction that
  the complete preview is in the Access screen. This is a second, independent reason the
  out-of-band copy exists: it is the only copy that cannot be truncated in transit.
- Streamable HTTP only; the legacy SSE transport is **rejected** — an `sse` entry is
  refused outright. That matches the stateless spike: `json_response(true)` keeps replies
  plain JSON and nothing depends on a server-initiated stream.
- Per-request `timeout` defaults to **60 seconds**, reset by progress notifications. The
  server's 30-second per-call deadline stays deliberately below it, so the agent receives a
  named server error rather than a client-side give-up. Do not raise the server deadline
  above the client's.
- Bearer tokens come from an environment variable or a header, which is exactly the PAT
  model: `pi mcp add prism --url https://<host>/mcp --bearer-token-env-var PRISM_PAT`.
- Tools register as `mcp__prism__<tool>`, and the default exposure is `codemode`, where
  tools are not declared to the model but discovered by search. Tool descriptions therefore
  carry the entire discovery burden — which is why `get_performance`'s description must name
  the constants it does not measure rather than leave the model to notice.

**Unverified for Hermes Agent:** whether it supports MCP at all, and if so its transport,
timeout and truncation behaviour. Until that is checked the acceptance gate is a run against
pi, and Hermes is an open item rather than an assumed-compatible client.

### Tool inventory

| Tool | Scope | Notes |
|---|---|---|
| `list_portfolios` | read | id, name, currency, asset and transaction counts |
| `get_portfolio` | read | portfolio plus its assets with symbol / ISIN / currency |
| `list_transactions` | read | filters: portfolio, asset, type, date range; paginated |
| `get_tax_summary` | read | the tax summary, or the dated report with `from`/`to` |
| `get_performance` | read | weekly value series and cumulative TWR — the published series is aggregated at `stats_engine.rs:317-320`, though its items still carry a field named `daily_return`. Volatility, Sharpe, beta **and** `realized_pnl` / `unrealized_pnl` are **not** measurements: `stats_engine.rs:108-110` and `:323-325` return constants on both paths. This tool omits the two P&L fields and its description names the three constants; realized gain is answered by `get_tax_summary`, not here |
| `get_transaction_pnl` | read | the sell P&L breakdown |
| `lookup_asset` | read | symbol / ISIN resolution, for normalizing before staging |
| `stage_import` | import | the only staging entry point; single-row entry is a batch of one |
| `apply_staged` | import | writes a batch a session has already approved; returns `not_approved` until then |
| `discard_staged` | import | discards without writing; no approval required |
| `get_staged` | import | re-renders the stored preview, so the agent can report what a human approved |

The three tools that change state — `stage_import`, `apply_staged`, `discard_staged` — set
`readOnlyHint: false`; the rest set it true. None sets `destructiveHint`, because no agent
tool deletes anything. A client that honours the annotations prompts on its own; the
server-side gate does not depend on it.

### `stage_import`

```json
{
  "target": { "kind": "new", "name": "Depot", "base_currency": "EUR" },
  "assets": [
    {
      "symbol": "IWDA",
      "isin": "IE00B4L5Y983",
      "name": "iShares Core MSCI World",
      "asset_type": "ETF",
      "currency": "EUR",
      "transactions": [
        { "type": "BUY", "date": "2024-03-01",
          "quantity": "1.5", "price": "82.3400", "fee": "0.00" }
      ]
    }
  ]
}
```

The shape is the contract, so it maps to serde without interpretation:

```rust
#[serde(tag = "kind", rename_all = "lowercase", deny_all_fields)]
enum Target {
    New      { name: String, base_currency: String },
    Existing { portfolio_id: i64 },
}
```

Internally tagged with a `kind` discriminator — one field, no ambiguity for the client, and
`deny_unknown_fields` on every struct so a misspelled key is an error rather than a silently
dropped value. `Existing` carries `portfolio_id` and nothing else; `New` carries name and
currency and no id. `base_currency` and `asset_type` are required, `isin` and `name` and
`currency` optional; `transactions` must hold at least one row, because an asset with no
transactions has nothing to import and its presence is a mistake worth naming. `type` is
`TxType`, `date` is strict `YYYY-MM-DD`, and `quantity`, `price` and `fee` are strings — a
JSON number is rejected here exactly as it is at `/api` after Workstream 0.

- **New versus merge.** `target.new` creates the portfolio. `target.existing` merges, and
  there exact-duplicate rows are listed in the preview and skipped on apply. This is **new
  behaviour, not precedent**: the browser importer's dedupe (`CsvImportModal.tsx:163-168`) is
  in-memory within one run, never compares against stored rows, and keys on `fee` too.
  Matching against the database is the capability `backup::restore` deliberately refused
  (`restore.rs:22-24`: "which transaction is a duplicate?" has no clean answer, and a wrong
  guess silently corrupts a tax ledger), so it is reviewed as new code. Duplicate matching is
  `Decimal` equality over `(date, asset_id, type, quantity, price, fee)` — never string
  equality, which fails on a re-exported `10` against a stored `10.0` and would break the
  re-import guarantee in the common case. Matching by portfolio *name* is not offered: two
  portfolios named "Depot" is a real thing and guessing wrong writes into the wrong one.
- **Asset matching.** ISIN first, then symbol plus currency within the target. Missing
  assets auto-create. Currency defaults to the portfolio's `base_currency` and the preview
  flags the assumption — deliberately different from the browser importer, which hardcodes
  USD, and wrong for a EUR statement.
- **Hard caps.** 50 assets and 500 transactions per batch, from *Limits* — checked in
  `staging::validate` before anything is inserted, and rejected with a named error rather
  than streamed. An earlier draft said 200 assets and 5,000 transactions, which cannot be
  reconciled with the 16 KB preview budget, and listed a 25-portfolio cap this payload cannot
  even express: a batch targets exactly one portfolio. That a body limit is needed at all is
  not hypothetical — this deployment's proxy already refused one oversized bundle
  (`af17064`).
- **Apply.** Re-validates — the target portfolio may have been deleted between stage and
  apply — then writes in one Postgres transaction, all or nothing, and returns the created
  portfolio ids, asset ids and transaction count.

`stage_import` is the **only** staging entry point. Single-row entry from a chat — "add a
buy of 1.5 shares of NOC on 3 March" — is one `stage_import` call with one asset and one
transaction; there is no separate `stage_edit` tool, and no update or delete path at all.
The payload carries exactly two caller-supplied ids: the target portfolio id in
`target.existing`, and the staged-batch handle. Both resolve through `access.rs`, which is
what keeps the `access.rs` surface and the gate script's job small.

### Sessions

`LocalSessionManager` is in-memory: MCP sessions do not survive a process restart and do
not work across replicas. The deployment is `replicas: 1`, which makes this acceptable
**only because all semantic state lives in Postgres** — a restart drops the MCP session but
not a pending staged batch. Any future change to `replicas` must revisit this.

---

## Error handling

The repo already has the right discipline and this module inherits it. `guard.rs` keeps
`Ok(None)` (deny) strictly apart from `Err(db)`, because
collapsing an outage into a denial once put a signed-in user in an OIDC redirect loop. The
split lives in two places, not one: `AuthFairing` rewrites `Err` into a 503 at
`/_auth/unavailable` (`auth/fairing.rs:32,110`), while the `FromRequest` guard answers 500
(`guard.rs:187-190`).
`access.rs::deny()` answers **not-found, never forbidden**, because a 403 is an existence
oracle. Both rules carry over to agent tools.

**Every domain failure is an MCP tool error** (`is_error: true` with a structured
payload), never a JSON-RPC error: a protocol-level error can tear down the session, and
"row 47 has a comma in the price" is content the model should read and act on. JSON-RPC
errors are reserved for malformed protocol.

Payload shape, defined once in `protocol.rs` — this is the authoritative list of codes, and
wherever this document names one in prose it means the code in this list:

```json
{ "error": {
    "code": "non_canonical_decimal",
    "message": "assets[0].transactions[3].price must be a decimal string",
    "field": "assets[0].transactions[3].price",
    "hint": "expected a decimal string such as \"82.34\"; the value contains a comma — if the source uses a decimal comma, replace it with a period before sending"
} }
```

`field` is a path into the submitted payload, never an internal id. Stable codes:
`non_canonical_decimal`, `unknown_transaction_type`, `unknown_asset`, `portfolio_not_found`,
`not_approved`, `stale_handle`, `expired`, `already_applied`, `scope_missing`,
`batch_too_large`. `hint` exists because the consumer retries — the text above stops the
thrashing a bare 422 causes. `already_applied`, `not_approved` and `expired` are distinct
codes because the model's correct response differs for each: report success, wait for the
human, re-stage.

Never leaked: SQL text, stack traces, another user's data. Rows that do not exist yet are
named by index in the submitted payload, not by internal id.

**Fault isolation must be paid for explicitly.** The listener runs in the same process as
the API, so a panic in a tool handler would take the whole application down, not just the
MCP session. Every tool call is wrapped in `catch_unwind` with a 30-second per-call
deadline — so a hung pricing lookup cannot wedge a session — and the listener survives
both. This is the price of the in-process decision; it is cheap to pay knowingly and
expensive to discover.

Logging records tool name, user id, token id, batch size, outcome and the batch
fingerprint — never the payload, which is someone's financial history.

---

## Testing and gates

Everything DB-touching runs on one real Postgres database per test via `src/test_db.rs`, so
these land in the `build-and-test` job after `sqlx migrate run` and need `DATABASE_URL`
like the other five gates.

- **`protocol.rs` deserialization**, table-driven: comma decimal rejected, JSON-number
  money rejected, unknown field rejected, `"DIVIDEND"` rejected, `01/02/2025` rejected,
  `"142.30"` round-trips to exactly `142.30`.
- **`staging.rs` against a real database**: `apply_staged` before a session has approved
  returns `not_approved` and writes nothing; apply-twice returns `already_applied` and
  writes nothing the second time; expired handle rejected; a failed write after the flip
  leaves the row `approved`, not `applied` — the rollback assertion that proves the flip and
  the inserts are one transaction; payload round-trip preserves
  decimal scale byte-for-byte — assert the stored string equals the input string, which is
  the proof no float was ever involved. `tests/decimal_migration_test.rs` already asserts
  that invariant for the interactive write path; this is the same assertion for the agent
  path.
- **The approval gate is the control, so it gets its own tests.** A bearer token cannot
  reach the three `/api/staged-batches` routes at all (they are session-only, and this is
  the mirror of the two-listener invariant); a session cannot approve a handle belonging to
  another user; `approved_by` records the approving session's user and nothing else changes
  it; and no agent tool exists that can set `status='approved'`.
- **Extend `tests/isolation_test.rs`** with the agent tools: user B cannot apply A's
  handle, stage into A's portfolio, or read A's preview through `get_staged`; and user B's
  session cannot approve A's handle. The gate
  script's own comments say the isolation matrix is the real gate, not the script.
- **New gate `scripts/check_agent_gate.sh`**: every `#[tool]` handler takes the caller and
  calls `require*` before any row access. Necessary because `check_access_gate.sh` anchors
  on `^#[(get|post|...)]` route attributes in `src/api_routes` and would be blind to tool
  handlers, leaving the highest-privilege new code in the repo ungatechecked. With
  `tests/agent_gate_test.rs` running the real script against a throwaway tree, mirroring
  `tests/access_gate_test.rs`. Added to the CI gate list in `AGENTS.md` order.
- **PAT tests**: mint → lookup → revoke → lookup yields `None`; expired rejected; a
  deactivated user's token rejected; and the two-listener invariant tested directly — a
  request to `/api/portfolios/` carrying only an `Authorization` header is denied.
- **`tests/route_coverage_test.rs`** gains the three token routes and the three
  staged-batch routes in `PROTECTED_ROUTES`, so
  the published OpenAPI document and the mounted surface cannot drift apart.
- **Workstream 0 tests**: the migration aborts on a junk `type` (proven by mutation), serde
  rejects an unknown type, and a restore bundle with a junk type is a `422` at preview
  rather than a `CHECK` violation mid-restore. The write-service extraction is proven by
  the existing suite staying green — that is the whole point of landing it as its own
  `refactor/` PR — plus one new assertion that the SPLIT rules and the date bound still
  reject what they rejected before, so the seam cannot silently drop a validation on the way
  out of the handler.
- **End-to-end over the real transport**: rmcp's own client
  (`StreamableHttpClientTransport`) against the listener bound to an ephemeral port —
  `initialize` → `tools/list` → `stage_import` → approve through `/api` with a session →
  `apply_staged` → assert the rows exist. The approval step is in the path, not skipped,
  or the test proves a flow the product does not have.
- **One run against the real client**, not only rmcp's test client:
  `pi mcp add prism --url https://<host>/mcp --bearer-token-env-var PRISM_PAT`, then a full
  import through it. Third-party clients are where transport assumptions break — pi rejects
  the SSE transport outright and truncates results over 20 KB, neither of which a
  same-vendor client would ever show.
  The only test that proves the tower-service wiring rather than the logic behind it.
- **Golden preview fixture**: an anonymized broker statement → the preview JSON, asserting
  exact counts, spans and flags. Guards "exact totals are always exact".

Out of scope for tests, stated: a real OIDC provider (already manual, per
`docs/auth-testing.md`) and PDF parsing, which does not exist.

---

## Configuration

| Variable | Meaning | Default |
|---|---|---|
| `AGENT_LISTEN` | bind the agent listener on this `host:port`; unset means the module does not exist | unset |
| `AGENT_BATCH_TTL_MINUTES` | staged-batch lifetime | `60` |

`k8s/configmap.yaml` gains no `AGENT_LISTEN`; enabling the feature is an explicit operator
edit, and the ingress needs a route for the second port before the endpoint is reachable
from outside the cluster.

## Limits

Every bound this design imposes, in one place. A limit that is not in this table does not
exist, and a limit in this table is not enforced anywhere else.

| Limit | Value | Why this value | Enforced where |
|---|---|---|---|
| Rows per staged batch | 500 | A real batch is 20–40 rows; 500 is a full history for a large portfolio, and it bounds the JSONB row and the preview | `staging::validate`, before insert |
| Pending batches per user | 5 | Stops abandoned batches accumulating rows; a legitimate user does not have six imports in flight | `staging::stage`, same transaction as the insert |
| Batch lifetime | `AGENT_BATCH_TTL_MINUTES`, default 60 | A human approves in the same sitting or the batch is stale | `expires_at`, compared in `apply_staged` |
| Request body | 1 MiB | Bounds memory per call against the container's 256 Mi limit | listener, before deserialization |
| Per-call deadline | 30 s | Deliberately below pi's 60 s client timeout, so the agent gets a named server error | `tokio::time::timeout` around the handler |
| Preview payload | 16 KB | pi truncates results over 20 KB by removing the middle | measured on the rendered string, before the response is built |
| Display caps | 25 portfolios, 50 assets | Existing constants in `backup::detail`; reused, not reinvented | `backup::detail` |
| Pool | `max_connections(5)`, `acquire_timeout` 5 s, `statement_timeout` 30 s (`db.rs:20,44,70`) | **Unchanged on purpose.** The agent shares the existing pool; raising it is a separate decision with its own trade-offs | `db.rs`, existing |
| List page size | 100 default, 500 max | Keeps a read response under the same client truncation limit the preview has to respect | `agent::tools`, every listing tool |
| Model token budget | not enforceable here | The server sees one `tools/call`, not the agent's context | client side; out of scope |

**No rate limiter in v1.** The bounds above cap storage and memory, which is all a
normally-working agent can grow. A runaway loop is a different failure: it costs pool
connections and CPU, and the container is `cpu: 500m / memory: 256Mi` with `replicas: 1`.
The answer is named rather than built: revoke the token in the Access screen, which
Workstream 1 already provides, and if a loop is ever actually observed, add a per-token
token bucket then. Throttling for a loop that has not happened, on a single-user
deployment, is the same speculative volume the rest of this document refuses.

## Failure modes

| Failure | Effect | Containment |
|---|---|---|
| Tool handler panics | `catch_unwind` at the dispatch boundary turns it into a tool error; the listener survives | that call only; a panic inside a write transaction drops it, so it rolls back |
| Pool exhausted by agent calls | web UI requests queue, then fail `acquire_timeout` after 5 s | the 30 s deadline bounds how long one call can hold a connection; revoke the token. This is the cost of sharing a process and no flag removes it |
| Pod restarts mid-batch | staged rows survive — they are rows in Postgres; an in-flight apply is one transaction, so it either happened or did not | stateless mode means the client simply POSTs again; see *Verification items* |
| Migration `0002` aborts on a junk `type` | app refuses to start and crashloops | the pre-flight audit runs first; the abort is one transaction, so schema and data are untouched; reverting the image is enough |
| `apply_staged` called twice | second call's flip matches zero rows | returns `already_applied`, writes nothing |
| Approved but never applied | batch sits at `approved` | expires at TTL, shows as stale in the Access screen, purged on that user's next stage |
| Postgres unreachable | agent gets a named internal error | the guard's `Err` path stays distinct from a denial, so an outage never looks like a permissions problem |
| Token leaks | attacker holds a live credential until revoked | hashed at rest, so a database dump yields nothing usable; revocation takes effect on the next call |
| Rows staged into the wrong portfolio | — | `access.rs` limits resolution to the caller's own portfolios, and the preview names the target before apply |
| Model misparses a figure into valid bytes | nothing server-side catches it | a human reading the preview is the only control; see *What the server owes the agent* |
| Client truncates the preview | the human approves a partial document | the 16 KB budget makes this structurally impossible for pi; over budget the tool points at the Access screen |

## Rollout and rollback

Order matters in exactly two places: the frontend money change must deploy before the backend
that rejects numbers, and the cluster database objects must exist before the image that
requires them — that second one is `docs/postgres-cutover.md`, not this spec.

1. **Workstream 0**, as separate PRs: the write-service extraction (no behaviour change) →
   the frontend money-as-string change → deploy it → the backend strict deserializer →
   `TxType` plus migration `0002`, run only after `audit_transaction_types.sh` passes
   against the live database.
2. **Workstream 1**: `0003_api_token.sql`, `auth/pat.rs`, `/api/tokens`, the Access panel.
   Inert until someone mints a token.
3. **Workstream 2**: `0004_agent_staged_batch.sql`, the staged-batch routes and the Access
   screen's batch list, then the agent module — deployed with `AGENT_LISTEN` unset, so the
   listener never binds and no MCP endpoint exists.
4. Only then: set `AGENT_LISTEN`, expose the port, mint the first token, run the pi
   acceptance test.

**Turning it off.** Unset `AGENT_LISTEN` and roll. The listener never binds, so the endpoint
is absent rather than firewalled. Revoke every token in the Access screen. Staged-batch rows
are inert data and may stay.

**Rolling back.** Revert the image. Migrations are additive — `0003` and `0004` add tables,
`0002` adds a `CHECK` — so an older image runs against the newer schema. `0002` is the
exception worth stating: if it aborted, nothing changed; if it applied, reverting the image
does not remove the `CHECK`, deliberately, because the constraint is correct on its own
terms and does not travel back with the feature that motivated it.

**Stop condition.** If the stateless-mode spike fails, Workstream 2 stops and returns to this
document. The second-listener design is the fallback that the spike must beat, not an
assumption to discover late.

## Alternatives considered

- **Separate process or sidecar.** Rejected by decision: the server is a module of the
  existing backend. A sidecar would isolate the pool and the panic surface at the cost of a
  second deployment, a second health check, and a second place where `access.rs` could be
  bypassed.
- **TypeScript or Python SDK.** Rejected: no Node in the backend runtime, and Python would
  be a second runtime in the image for the same reason.
- **stdio transport.** Rejected for v1: the deployment is a remote cluster pod, and stdio
  serves a local client. See *Non-goals*.
- **Mount inside Rocket rather than a second listener.** Not rejected — an open spike with a
  named payoff. See *Verification items*.
- **A nonce relayed to the agent as proof of approval.** Rejected: a code that travels
  through the model is evidence of nothing, because the model needing proof is the one
  holding it. The control is a state transition only a session cookie can cause.
- **A separate `write` scope gating apply.** Rejected: it would force a second token or a
  second agent onto the confirm step, ruled out on ergonomics. The approval gate gets the
  same separation without a second credential.

## Sequencing

1. **Workstream 0** — `TxType`; the `CHECK` migration with its audit script and abort test;
   the frontend money-as-string migration; the backend strict deserializer, which cannot
   merge before that frontend change is deployed; and the write-service extraction. Each is
   its own PR.
2. **Workstream 1** — `0003_api_token.sql`, `auth/pat.rs`, `api_routes/tokens.rs`, the
   Access panel.
3. **Workstream 2** — `0004_agent_staged_batch.sql`, then the staged-batch `/api` routes and
   the Access screen's pending-batches list, which are prerequisites because nothing can be
   applied without them, then `agent/protocol.rs`, `agent/staging.rs`, `agent/tools.rs`,
   `agent/listener.rs`.
4. Enable `AGENT_LISTEN` in the cluster and mint the first token.

Workstream 0 is not MCP work at all — it is a live API behaviour change that MCP happens to
depend on, which is why it ships as separate PRs rather than as part of the agent module.
Workstream 2's plan is not started until three things are recorded in this document: the
approval mechanism (above), the shared write service (Workstream 0, item 6), and the
outcome of the stateless-mode spike in *Verification items*. Reverting Workstream 0 means
dropping the `CHECK`; rows written after deploy are unaffected because the accepted set does
not widen.

## Verification items to confirm during implementation

Some of these are questions about external crate behaviour; others are not — the
`catch_unwind` scope is an internal design choice, and the `type` audit is a query against
live data.

- Exact `rmcp` 3.11 feature-flag names for the server and streamable-HTTP-server transports,
  and the `StreamableHttpServerConfig` fields for stateful mode and SSE keep-alive.
- Whether `hyper-util`'s `TowerToHyperService` composes with the `hyper 1.10.1` already in
  the lockfile without a version bump, or whether `rmcp` requires a newer `hyper`.
- Whether rmcp's `schemars` integration resolves to the 1.2.3 already in the lock. The lock
  carries **two** versions today — 0.9.0 (`Cargo.lock:2867`, via utoipa 4.2) and 1.2.3
  (`:2879`) — against `Cargo.toml:20`'s `schemars = "1.0"`. A third copy is a blocker for
  the PR.
- Whether `rmcp` builds against `tower 0.5.3`, the version in the lock, or requires 0.6, and
  whether `hyper` / `hyper-util` need bumping.
- **Spike, before Workstream 2 is planned:** can `StreamableHttpService` be mounted as a
  Rocket route in stateless mode? Bridge one POST (method, URI, headers, buffered body in;
  status, headers, bytes out) with `legacy_session_mode(false)` and `json_response(true)`,
  and drive it with rmcp's own client through `initialize` → `tools/list` → one tool call.
  Adopt it only if there is no SSE and no `Mcp-Session-Id` anywhere, the adapter stays in one
  bounded file, `AuthFairing`'s prefix rule (`auth/fairing.rs:40`) needs no exemption, and
  `tests/route_coverage_test.rs:181 PUBLIC_NAMESPACES` admits the namespace by explicit
  amendment. If it lands, the second listener, its direct `hyper`/`tower` manifest entries,
  the k8s exposure work and the `replicas: 1` session coupling all disappear.
- Whether `catch_unwind` around a tool call is sufficient given `tokio` task panics, or
  whether the call must run in its own spawned task.
- Whether any live row violates the `type` `CHECK` after normalization — the pre-flight
  audit answers this before deploy, not after.
- Whether Hermes Agent supports MCP at all, and if so its transport, per-request timeout and
  output-truncation limit. The 16 KB preview budget is derived from pi's documented 20 KB; a
  client that truncates lower lowers the budget with it. Do not declare a second client
  supported before this is answered.
