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
2. **No Node.** The concern is supply-chain exposure and install-time code execution, the
   npm incident class that bites hardest. `rmcp` is the official Rust SDK
   (`modelcontextprotocol/rust-sdk`), v3.11.0 as of 2026-11-17, Apache-2.0, MSRV 1.88,
   implementing the `2026-07-28` spec revision. The toolchain here is `rustc 1.98.1`.
   Its cost is honest and bounded: ~43 direct dependencies, against the 449 packages
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

## Non-goals

- **`backup/*` is not exposed.** No export, no preview, no restore. There is no
  `backup` scope, so it is not a policy an agent can forget — it is a capability that
  cannot be minted. `restore` with `mode=replace` deletes everything a user owns and stays
  in the web UI.
- **No broker-statement parser.** No CSV dialect mapping, no PDF extraction. The agent
  supplies structured rows.
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
`hyper 1.10.1`, `hyper-util 0.1.20`, `tower 0.6.11` and `http 1.4.2` are already in
`Cargo.lock` via reqwest, so **no new HTTP framework is added** — `rmcp` is the only new
dependency. Rejected: a Rocket↔tower request adapter (hand-rolling SSE through a Rocket
`Responder` fights the framework) and hand-writing the streamable-HTTP transport on
rmcp's `Transport` trait (would mean owning protocol negotiation, session ids and SSE).

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

## Workstream 0 — prerequisite: close the transaction-type and number boundary

Ships and deploys **before** the agent module, because it changes live API behaviour.

Two findings from the current schema motivate this. `transactions.type` is `TEXT` with no
`CHECK` and there is no Rust enum: `create_transaction` (`api_routes/transactions.rs:238`)
does `tx.r#type.to_uppercase()`, compares against `"SPLIT"`, and stores the result. Any
other string passes — `"DIVIDEND"` is stored and the tax engine reads it as a
positive-quantity lot, because the only three literals in `src/` are `BUY`, `SELL` and
`SPLIT`. Separately, `transactions.quantity`, `price` and `fee` are `TEXT`, not `NUMERIC`;
the app preserves exactness by storing `Decimal::to_string()`, which is correct — but it
means a value that entered as a JSON *number* has already passed through `f64` before
becoming a `Decimal`.

1. **`TxType` enum.** In `models.rs`: `#[serde(rename_all = "uppercase")] enum TxType {
   Buy, Sell, Split }` with `Display`. `TransactionCreate.type` becomes `TxType`; the
   `to_uppercase()` call disappears because the value is canonical by construction.
   Anything else is a serde `422`. No frontend change is required:
   `csvValidator.ts:99` already rejects everything outside `BUY|SELL|SPLIT`, so this
   formalizes the contract the browser importer has always enforced.
2. **Migration `0002_transaction_type_check.sql`.** Normalize existing rows with
   `upper(btrim(type))` — case and surrounding whitespace are not semantic changes. Then
   **abort, listing the offending values**, if anything is still outside the set; rewriting
   `DIVIDEND` to `BUY` would be inventing tax data. Then add
   `CHECK (upper(btrim(type)) IN ('BUY','SELL','SPLIT'))`. Case-insensitive on purpose: it
   keeps the engine's existing `.to_uppercase()` comparisons honest without a data
   rewrite. `sqlx::migrate!` runs at startup, so a failing migration means the app does not
   boot — the audit query
   (`SELECT DISTINCT type FROM transactions WHERE upper(btrim(type)) NOT IN ('BUY','SELL','SPLIT')`)
   is a pre-flight step in the runbook, and `check_migrations.sh` gains a case proving it
   fails on a junk value.
3. **The bundle schema gets the same closed set.** `backup/restore.rs:367` inserts the same
   column, so `docs/backup-schema/v1.json` must constrain `type` — otherwise a junk type
   is a `CHECK` violation mid-restore instead of a `422` at preview. Confirm no shipped
   bundle fixture violates it. `services/currency_service.rs:138` inserts the literal
   `'SPLIT'` and is unaffected.
4. **Money arrives as a string, enforced.** A deserializer used by both the agent DTOs and
   `TransactionCreate` accepts a JSON string matching `^-?\d+(\.\d{1,12})?$` and rejects a
   JSON number outright, with an error naming the fix. `frontend/src/utils/decimal.ts`
   already sends strings. Scale is capped at 12 decimal places as a validation rule, not
   inferred from a column, because the column is `TEXT` and will hold anything.

   **The `TEXT` columns are not to be changed to `NUMERIC`.** `tests/decimal_migration_test.rs`
   documents why — a binary float cannot hold 9.99, and cost basis drifting by a rounding
   unit per row is wrong on tax figures — and `scripts/check_migrations.sh` fails the build
   if any of those columns is not `text`. The fix is at the boundary, never in the storage
   type.
5. **`asset_type` stays free text.** Same shape, but nothing computes on it — no engine
   reads it. Constraining it now would be unrelated refactoring with a migration attached.
   It gets a roadmap note.

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
256 bits of entropy means there is no guessing attack, so no attempt throttling is added —
stated so nobody adds one later.

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
  calls `require*` before touching a row, then calls the same `engines::` and
  `services::` functions the Rocket handlers call.
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
precedent at `lib.rs:268` where the OIDC routes are mounted only when there is a provider
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
- Dates are strict ISO `YYYY-MM-DD`, resolved to UTC midnight, bounded 1900-01-01 …
  2100-01-01 as `create_transaction` already does. The browser importer's multi-format
  auto-detection exists because a human sits in front of dropdowns; the agent has no
  dropdowns.
- ISIN, when present, passes a mod-10 checksum implemented in Rust. `isinValidator.ts`
  lives in the browser and this path cannot depend on it. A bad ISIN is a named rejection,
  not a silently unmatched asset.

### The preview

Follows `2026-11-16-import-preview-design.md`: exact totals always, display bounded,
anomalies flagged and never blocking.

Per portfolio: name, currency, asset count, transaction count, date span. Per security:
symbol, ISIN, counts, span. An explicit **matched vs will-auto-create** split for assets.
Anomaly list: unknown symbol, duplicate-looking row (same date + asset + type + quantity +
price), currency differing from the portfolio base currency, future-dated, unusual price
scale, an assumed currency. Display caps always read "showing 25 of 1,204".

### Tool inventory

| Tool | Scope | Notes |
|---|---|---|
| `list_portfolios` | read | id, name, currency, asset and transaction counts |
| `get_portfolio` | read | portfolio plus its assets with symbol / ISIN / currency |
| `list_transactions` | read | filters: portfolio, asset, type, date range; paginated |
| `get_tax_summary` | read | the tax summary, or the dated report with `from`/`to` |
| `get_performance` | read | daily value series and cumulative TWR. Volatility, Sharpe and beta are **not** measurements — `stats_engine.rs:108-110` returns constants, and the tool description says so |
| `get_transaction_pnl` | read | the sell P&L breakdown |
| `lookup_asset` | read | symbol / ISIN resolution, for normalizing before staging |
| `stage_import` | import | the only staging entry point; single-row entry is a batch of one |
| `apply_staged` | import | writes a batch a session has already approved; returns `not_approved` until then |
| `discard_staged` | import | discards without writing; no approval required |
| `get_staged` | import | re-renders the stored preview, so the agent can report what a human approved |

Every tool sets `readOnlyHint`; none sets `destructiveHint`, because no agent tool deletes
anything. A client that honours the annotations prompts on its own; the server-side gate
does not depend on it.

### `stage_import`

```
{ target: { new: {name, base_currency} } | { existing: <portfolio_id> },
  assets: [ { symbol, name?, isin?, asset_type, currency?,
              transactions: [ { type, date, quantity, price, fee } ] } ] }
```

- **New versus merge.** `target.new` creates the portfolio. `target.existing` merges, and
  there exact-duplicate rows are flagged in the preview and skipped on apply — the same
  rule `csvValidator.ts` applies to browser CSV imports, so re-importing an updated
  statement does not double the ledger. Matching by portfolio *name* is not offered: two
  portfolios named "Depot" is a real thing and guessing wrong writes into the wrong one.
- **Asset matching.** ISIN first, then symbol plus currency within the target. Missing
  assets auto-create. Currency defaults to the portfolio's `base_currency` and the preview
  flags the assumption — deliberately different from the browser importer, which hardcodes
  USD, and wrong for a EUR statement.
- **Hard caps.** 25 portfolios, 200 assets, 5,000 transactions per stage, rejected with a
  named error rather than streamed. Not hypothetical: this deployment's proxy already
  refused one oversized body (`af17064`).
- **Apply.** Re-validates — the target portfolio may have been deleted between stage and
  apply — then writes in one Postgres transaction, all or nothing, and returns the created
  portfolio ids, asset ids and transaction count.

`stage_import` is the **only** staging entry point. Single-row entry from a chat — "add a
buy of 1.5 shares of NOC on 3 March" — is one `stage_import` call with one asset and one
transaction; there is no separate `stage_edit` tool, and no update or delete path at all.
The payload therefore carries no caller-supplied row ids except the staged-batch handle,
which is what keeps the `access.rs` surface and the gate script's job small.

### Sessions

`LocalSessionManager` is in-memory: MCP sessions do not survive a process restart and do
not work across replicas. The deployment is `replicas: 1`, which makes this acceptable
**only because all semantic state lives in Postgres** — a restart drops the MCP session but
not a pending staged batch. Any future change to `replicas` must revisit this.

---

## Error handling

The repo already has the right discipline and this module inherits it. `guard.rs` keeps
`Ok(None)` (deny) strictly apart from `Err(db)` (503 via `UNAVAILABLE_PATH`) because
collapsing an outage into a denial once put a signed-in user in an OIDC redirect loop.
`access.rs::deny()` answers **not-found, never forbidden**, because a 403 is an existence
oracle. Both rules carry over to agent tools.

**Every domain failure is an MCP tool error** (`is_error: true` with a structured
payload), never a JSON-RPC error: a protocol-level error can tear down the session, and
"row 47 has a comma in the price" is content the model should read and act on. JSON-RPC
errors are reserved for malformed protocol.

Payload shape, defined once in `protocol.rs`:

```
{ error: { code, message, row?, field?, hint? } }
```

Stable codes: `non_canonical_decimal`, `unknown_transaction_type`, `unknown_asset`,
`portfolio_not_found`, `stale_handle`, `already_applied`, `scope_missing`,
`batch_too_large`. `hint` exists because the consumer retries — `"money must be a string:
\"142.30\", not 142.3"` stops the thrashing a bare 422 causes. `already_applied` and
`stale_handle` are distinct because the model's correct response differs: report success
versus re-stage.

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
  rather than a `CHECK` violation mid-restore.
- **End-to-end over the real transport**: rmcp's own client
  (`StreamableHttpClientTransport`) against the listener bound to an ephemeral port —
  `initialize` → `tools/list` → `stage_import` → `apply_staged` → assert the rows exist.
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

## Sequencing

1. **Workstream 0** — `TxType`, the `CHECK` migration, the bundle-schema constraint,
   money-as-string. Merge, deploy, confirm the live database passes the audit.
2. **Workstream 1** — `0003_api_token.sql`, `auth/pat.rs`, `api_routes/tokens.rs`, the
   Access panel.
3. **Workstream 2** — `0004_agent_staged_batch.sql`, `agent/protocol.rs`,
   `agent/staging.rs`, `agent/tools.rs`, `agent/listener.rs`, the agent gate script and its
   mutation test.
4. Enable `AGENT_LISTEN` in the cluster and mint the first token.

Each workstream is independently shippable and independently revertible. Workstream 0 is
not MCP work at all — it is a live API behaviour change that MCP depends on — and it may
just as well be executed as its own plan and its own PR; nothing in Workstreams 1 or 2
waits on it beyond the enum's existence.

## Verification items to confirm during implementation

These are unknowns about external APIs, not open design questions.

- Exact `rmcp` 3.11 feature-flag names for the server and streamable-HTTP-server transports,
  and the `StreamableHttpServerConfig` fields for stateful mode and SSE keep-alive.
- Whether `hyper-util`'s `TowerToHyperService` composes with the `hyper 1.10.1` already in
  the lockfile without a version bump, or whether `rmcp` requires a newer `hyper`.
- Whether rmcp's `schemars` integration matches the `schemars 1.0` already in
  `Cargo.toml`, or pulls a second version.
- Whether `catch_unwind` around a tool call is sufficient given `tokio` task panics, or
  whether the call must run in its own spawned task.
- Whether any live row violates the `type` `CHECK` after normalization — the pre-flight
  audit answers this before deploy, not after.
