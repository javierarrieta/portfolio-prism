# Corrected PR body for #77

Written 2026-11-24. Paste-ready replacement for the body of PR #77.

Six statements in the shipped body were **false about the tree the PR carries** — not stale,
wrong — and the review treats a false claim as a finding. Each is corrected below, in the
section where it appears. The corrections are marked `CORRECTED:` so the diff against the
old body is auditable; delete the markers when pasting.

Counts are for head `34482ba` on `fix/pr77-review-findings` (the review-fix commit on top of
`df258e4`). If the fix commit is not merged into this branch, subtract one commit and use
23 / 81 / +11225 / −318.

---

The MCP agent interface, end to end: a token an agent can hold, a staging area a token can
write to and cannot approve, and an MCP server mounted inside the existing Rocket app
exposing eleven tools.

24 commits, 90 files, +12481/−335. It is large because the four prerequisites are folded in
— each was a branch of its own and all four had to land before the agent surface made sense.
**Read it in the order below; that is the order the code depends on.**

The last four commits are a review round on the agent surface, one schema change it
uncovered, a second review round on that, and `fix(agent): close the PR #77 review findings
that were code`. They are
[their own section](#review-round--the-findings-on-top-of-the-reviewed-head).

> **CORRECTED (was: "21 commits, 79 files, +10718/−312").** The body described a tree two
> commits behind its own head: `37bcdd7` and `df258e4` were pushed and are not mentioned
> anywhere in it. Anyone reading the body and then reading the diff found two commits the
> body does not know about.

---

## ⚠️ Deploy order — read this before applying anything

**One. The frontend image must roll before the backend image.**
`refactor(backend): money arrives as a string or the request is refused` (`7b639bb`) makes
the API **refuse JSON numbers** for money fields. The browser's CSV importer posted JSON
numbers until `5ccb11b`, which is also in this PR. New backend + old frontend = transaction
creation and CSV import fail with `invalid type: integer 10, expected a decimal string`.

This is recorded at the mechanism itself — `db_types.rs`, above `strict_decimal_json` — so it
survives the PR being closed. Both halves are in one PR now, which is what makes the ordering
easy to miss: one merge, two images, and they do not roll atomically.

**How to actually get that order**, because the pipeline does not do it for you: images
publish only from `release.yml` on a `v*` tag, and one release run pushes `:latest` for
**both** images at the same moment, while `k8s/app-deployment.yaml` pins the backend to a
digest and the frontend to `:latest`. So: release, then apply the **frontend** tag change
alone (the backend is digest-pinned and does not move), confirm the new panels degrade
cleanly against the old backend, then bump the backend digest and apply.

> **CORRECTED.** The old body asserted the order as a fact about deployment without saying
> that nothing in the repository produces it. `docs/postgres-cutover.md` now carries the
> procedure.

**Two. Run the pre-flight audits against the live database before the deploy that carries
this.** Migration `0006` refuses to apply to a database that already holds one symbol twice
in one portfolio, and `sqlx::migrate!` runs before Rocket serves a request — so a raising
migration is a crashloop with the reason in a container log. Two scripts run the same
predicates in advance:

| Script | Guards | Exit codes |
| --- | --- | --- |
| `backend_rust/scripts/audit_asset_symbols.sh` | `0006`'s `uq_assets_portfolio_symbol` | `0` clean, `1` duplicates found, `2` cannot tell |
| `backend_rust/scripts/audit_transaction_types.sh` | `0002`'s `CHECK` on `transactions.type` | `0` clean, `1` rows would be refused, `2` cannot tell |

Neither runs inside the backend pod — the runtime image is `debian:bookworm-slim` plus
`ca-certificates`, so there is no `psql` in it. Take the DSN from the Secret the deployment
already uses and run them from a host that has `psql`
(`nix-shell -p postgresql_18 --run 'bash backend_rust/scripts/audit_asset_symbols.sh'`),
port-forwarding the database service if it is cluster-only. **Exit `2` means "unknown", not
"clean".**

CI cannot tell you any of this. Its database is empty.

> **CORRECTED.** The old body named one script and implied it could just be run. It named no
> location, no DSN source, and no `psql` problem, and it never mentioned the second audit at
> all — `audit_transaction_types.sh` existed in the tree with no documented execution path.

---

## WS0 — two prerequisites, no behavior change

**`8344cfe` one insert path for assets and transactions.** Assets and transactions were
inserted from several places with subtly different validation, which is how the ISIN gap and
the transaction-type gap below were able to exist at all. Everything now goes through
`services::assets::insert` / `services::transactions::insert`.

**`5ccb11b` money leaves the browser as a decimal string.** `parseFloat` is how `9.99`
became a value the tax engine could not trust. The write path now carries decimal strings end
to end and never touches a float. `parseDecimal` returns the exact string that will be sent,
canonicalized as text, and now refuses what the backend's `is_canonical_decimal` refuses —
the scale and total-digit caps are shared, so a cell that parses here is a cell the API
accepts.

## WS1 — personal access tokens

**`bf65929` / `e13600a`.** Tokens an agent can hold, minted and revoked from a new Access
screen.

The design point: **a token is not a session.** A token carries scopes and a session does not,
and conflating them would make the scope model decorative. Expiry is opt-in; a 90-day
staleness nudge exists because a token nobody has used in a quarter is usually a token nobody
remembers and someone should look at.

Mechanically: `AuthFairing` resolves a **session**; the bearer token is resolved separately,
at the door that accepts it. They are not two arms of one fairing, and the old sentence here
said they were.

> **CORRECTED (was: "`AuthFairing` accepts either").** It does not. `auth/guard.rs` reads the
> session cookie; PAT lookup happens on the paths that opt into it. The distinction is what
> makes "`/mcp` does not consult the cookie" a testable claim rather than a contradiction of
> this paragraph.

`users.token_version` is **not** what kills one — bumping it invalidates sessions, and tokens
deliberately survive `logout-all` (`pat.rs`'s module comment, and the spec's revocation
section). Revocation is the answer to a leaked token, and there are now two answers:
`DELETE /api/tokens/<id>` per token, and **`POST /api/tokens/revoke-all`**, which revokes
every live token the caller owns in one call. Session-only, like the rest of that file.

> **Added.** Before this, the only response to a leaked token was row-by-row deletion in a
> UI, while a minted token could be given an expiry 10 years out and survive sign-out,
> logout-everything, and a password change. The incident lever did not exist.

## WS2 — the agent surface

### Staging lifecycle — `9b37549`

A token writes a *proposal*, never a transaction. `stage_import` stores rows plus a
human-readable preview under a handle; nothing is applied until a **session** (not a token)
approves it. Migration `0004_agent_staged_batch.sql`.

The preview is stored, not derived. If it were computed at render time, what a human read and
what the agent was shown could differ, and the approval would be an approval of something
else.

The preview now carries **money ranges** per asset and in the totals
(`quantity_min/max`, `price_min/max`, `fee_max`). It used to carry counts, which meant a
human approved "3 rows" without ever seeing that the third row was 900 units at 82.34. An
approval of a count is not an approval of a transaction.

### `apply` — `2a7df1c`

The one write the approval gate authorises, and the rules that keep it single-use. An approved
batch cannot be applied twice; a discarded batch cannot be applied at all.

Two locks, and they do different jobs:

- `FOR UPDATE` on the batch row, for the whole transaction — no second writer interleaves with
  an apply.
- `pg_advisory_xact_lock(user_id)` — the portfolio cap is counted inside the transaction that
  creates the row, and two batches are two rows, so the row lock cannot serialise that count.
  Without the user lock, two approved `Target::New` batches both counted 9 and both created a
  portfolio at the cap.

Both are pinned by tests that **force the contention from outside** — hold the lock yourself
and show `apply` waits. A `tokio::join!` race was written first and thrown away: it passed
with the advisory lock deleted, and a test that cannot fail is not a test.

### Session routes + panel — `8dd932e`

`/api/staged-batches` (list), `/api/staged-batches/<handle>` (the stored preview, rendered
verbatim), `/api/staged-batches/<handle>/approve`. These are session-only by construction —
they are under `/api`, so `AuthFairing` requires a session and a token cannot reach them.

### The `/mcp` mount — `4a0b515`

`rmcp`'s `StreamableHttpService` mounted as a Rocket route, stateless, no Node.js anywhere in
the runtime. Proven by a spike branch before any of this was written.

Five things about it are load-bearing and were each learned the hard way:

- **It is mounted at the root, not under `/api`.** Adding it to the `/api` mount list served
  it at `/api/mcp` inside `AuthFairing`'s prefix, which answered 404 — the only way that
  mistake announces itself. There is now a test that no mounted route escapes the prefix rule.
  The mount is controlled by `AGENT_MCP_MOUNT` (default on; falsey values unmount it), which
  is the kill switch for the surface without a redeploy of code.
- **`/mcp` authenticates itself, bearer only.** The session cookie is deliberately not
  consulted: `Credential::Session` passes every scope, so honouring one would make the scope
  model decorative. `SELF_AUTHENTICATED` is a separate list from `PUBLIC_NAMESPACES` on
  purpose — "public by decision" and "must check itself" are opposite promises, and folding
  them lets the next route be waved through. The test for this now runs against the **OIDC**
  builder with a real, decryptable `pp_session` attached and no bearer; the earlier version
  ran in `AuthMode::Disabled`, the one mode where a cookie is never read, so it could not
  have failed.
- **The bearer token is consumed at the door and never forwarded to `rmcp`.** A tool cannot
  leak a credential it never sees.
- **A refused tool call is a tool result, not a protocol error.** `not_approved`, `expired`,
  `scope_missing`, `portfolio_not_found` come back as `CallToolResult::error` carrying a JSON
  `code`. `gate()` returns `Option<CallToolResult>` rather than `Result` because
  `Err(McpError)` invites an infinite identical retry, and a missing scope never fixes itself.
- **The route bounds itself.** 1 MiB body cap, at most 3 requests in flight (403-beyond → 503
  with `Retry-After`), and a 120 s wall clock per request (504). Each tool call is a full
  tax-engine run on the blocking pool, and the pool is 5 connections; without these, one
  client with a script could hold the whole backend open.

> **Added.** None of the three bounds existed. The 503/504 split is deliberate: "go away and
> come back" and "your request died" are different instructions to an agent.

**And the thing the first version of this PR got wrong: the route was unreachable.** Rocket
served `/mcp`; `frontend/nginx.conf` proxied `/api`, `/api-docs` and `/auth` and nothing
else, so the SPA fallback answered the POST with HTML. Every test that built the Rocket app
in-process passed. `location /mcp` is now in the config, and `check_upload_path.sh` grew a
third rule that fails the build when the block is missing or when the proxy's
`client_max_body_size` for `/mcp` is narrower than the route's own cap — so the pairing is
checked, not assumed.

### The eleven tools — `b363dc2`

```
read     list_portfolios  get_portfolio  list_transactions  get_tax_summary
         get_performance  get_transaction_pnl  lookup_asset
import   stage_import     apply_staged   discard_staged     get_staged
```

**Where a tool has an `/api` equivalent, it calls the same function the route calls** —
`list_portfolios`, `get_portfolio`, `get_tax_summary`, `get_performance`, `lookup_asset`,
`stage_import` and `apply_staged` all go through the same service function, so a rule
enforced there is enforced here. A rule enforced only in a tool is a rule an `/api` caller
need not follow, which makes it not a rule.

Four tools have **no `/api` equivalent**, and saying so is the honest version:
`list_transactions` (the route has no pagination and sorts by date only; the tool paginates
and sorts `date DESC, id DESC`), `get_transaction_pnl`, `discard_staged` and `get_staged`
(the staged-batch routes are session-only by design, so a token-facing tool cannot share
them). `src/agent/tools.rs` uses runtime `query_as::<_>` rather than the compile-time macros,
so these four are checked at run time, not at compile time.

> **CORRECTED (was: "Every tool calls the same function the corresponding `/api` route
> calls").** True for seven of eleven. The blanket sentence was the claim that made
> `src/agent` outside the access gate look safe.

That last gap is now closed by the gate rather than by prose: `check_access_gate.sh` rule D
requires every `#[tool]` function in `src/agent/tools.rs` to call `.gate(`, and a stripped
gate fails the build. Before this the script watched `src/api_routes` and nothing watched the
agent surface.

Two deliberate removals from `get_performance`: the engine returns `volatility`,
`sharpe_ratio` and `beta` as the constants `"0.0"`, `"0.0"` and `"1.0"`, and
`realized_pnl`/`unrealized_pnl` alongside them. The three constants stay in the response but
the tool description **names** them as non-measurements; the two PNL fields are stripped
outright. A field that is present gets quoted — prose loses to the number.

`CurrencyService` moved behind an `Arc` (`97c5c04`) so the mount can build its server in a
`'static` closure. One instance means one cache and one rate limiter; a second
`CurrencyService::new` would double the request rate against a rate-limited provider.

---

## Review round — the findings on top of the reviewed head

`250ccdf` · `e4bde8c` · `2692a69` · `37bcdd7` · `df258e4` · `34482ba`, on top of the reviewed
`7d3075d`.

### `250ccdf` — a lying preview, an unspendable approval, an unvalidated target

*(unchanged from the shipped body — every claim in this subsection was checked and holds)*

### `e4bde8c` — a portfolio backstop, and a type that cannot name an error `apply` cannot produce

*(unchanged — holds)*

### `2692a69` — asset identity becomes a constraint, and the detector it makes unreachable is deleted

*(unchanged, with one addition below)*

The `duplicate_symbol` flag's remaining producer is now **upper-cased**, matching `validate`
and the `upper(symbol)` index. It used to compare case-sensitively, which meant the one
bundle shape `validate` refuses — `MSFT` beside `msft` — was the shape the flag missed. The
flag is a **tripwire, not a message to a human**: `preview` validates before it plans, so a
bundle that would set it is refused with a 422 before a plan exists, and if it ever fires on
a real request the ordering guarantee has gone. `src/backup/detail.rs` says all of this at the
flag now.

### `37bcdd7` / `df258e4` — the review round on the review round

Body cap wired, an approval that could never apply, and the manual transaction form
canonicalizing money the way the importer does.

### `34482ba` — the findings from the adversarial review that were code

`/mcp` reachability and its gate rule, `AGENT_MCP_MOUNT` / `AGENT_BATCH_TTL_MINUTES` becoming
real configuration, the user-level advisory lock in `apply`, money ranges in the preview,
`pat::lookup` filtering expiry in SQL with `now()` instead of comparing against the
application clock, `POST /api/tokens/revoke-all`, the `/mcp` body/concurrency/time bounds,
access-gate rule D, the case-insensitive bundle flag, the restore decimal-scale check, and
the two clippy warnings in test files this PR adds.

---

## Migrations

`0002` transaction-type CHECK · `0003` `api_token` · `0004` `agent_staged_batch` · `0005`
split dedupe index made case-insensitive · `0006` `uq_assets_portfolio_symbol`.

`0005` closes a hole `0002` documented itself: the CHECK is `upper(btrim(type))` but the
partial index keyed on `type = 'SPLIT'`, so a `'split'` row satisfied the CHECK and escaped
the dedupe. The index is the *only* dedupe — `apply_split_events` has no read-based check of
its own — and a double-applied split rescales cost basis without saying so.

`0005` is DROP-then-CREATE. **The shipped body called that non-atomic and leaned on
`replicas: 1` + `Recreate` for safety. That is false, and the migration file cannot be edited
to say so** — sqlx stores a checksum per applied migration and refuses to start when a file
no longer matches, and the live database applied `0001`–`0006` on 2026-11-15, so every file
in `migrations/` is frozen. The correction lives in `docs/postgres-cutover.md`.

What is actually true: sqlx wraps each migration in a transaction unless the file's first line
is `-- no-transaction`, and none of ours carry it. Measured on a scratch database — a
concurrent writer stayed blocked 4005 ms behind an uncommitted `DROP INDEX` and proceeded
only when it committed. `DROP INDEX` takes `ACCESS EXCLUSIVE`, so there is no window at one
replica or at ten. The real constraint is that the rebuild holds that lock for the duration of
the `CREATE UNIQUE INDEX`, which is a sizing question on a big table, not a correctness one.

`0006` audits before it creates, for the same reason `0002` does. It also reports existing
ISIN collisions and says plainly that it does not constrain them.

**Rolling back past `0006` is not clean**, and the shipped body did not mention it. `0006`
deliberately does not rewrite `assets.symbol` (`historical_prices` is keyed by the symbol
string), while the write path moved from `symbol = $2` to `upper(symbol) = $2`. Roll the
image back alone and a row stored as `auditdup` is invisible to the old binary's
case-sensitive lookup — it tries to insert a duplicate and dies on the raw index violation
where it used to write a silent duplicate. `docs/postgres-cutover.md` has the two options.

## Access control

`access.rs` remains the single chokepoint. `check_access_gate.sh` fails the build if an `/api`
data route takes no `AuthedUser` or calls no `require*`, **and** — rule D — if any `#[tool]`
in `src/agent/tools.rs` does not call `.gate(`. Each rule has its own test that runs the real
script against a throwaway tree and mutates it; a gate is only as good as the mutation that
proves it can fail.

## Verification

- **Backend: 685 tests, 0 failed.** All six gates pass (`access_gate`, `pricing_seam`,
  `backup_portability`, `upload_path`, `migrations`, `sqlx_cache`) on a freshly migrated
  database. `cargo clippy --all-targets`: 0 errors, and no warning in a file this PR adds —
  the two that were there (`tests/pat_test.rs`, `tests/apply_test.rs`) are fixed; the
  remaining warnings are in files this PR does not touch. `cargo sqlx prepare --check`:
  clean.
- **Frontend: 363 tests, 0 failed.** `tsc --noEmit` clean. `eslint src`: 0 errors, 0 warnings.
- Migrations 0001–**0006** apply to an empty database; `uq_split_asset_date` reads
  `WHERE (upper(btrim(type)) = 'SPLIT'::text)` and `uq_assets_portfolio_symbol` reads
  `(portfolio_id, upper(symbol))`.
- `0006` was checked in the failure direction: against a database holding `auditdup` beside
  `AUDITDUP` it refuses and names the portfolio, the ids and both spellings, and applies once
  the rows are merged. `check_migrations.sh` fails with the index dropped.
- New gates, each with a mutation that proves it can fail: a missing `location /mcp` → exit 1;
  a `/mcp` cap narrower than the route's → exit 1; a `#[tool]` with its `.gate(` stripped →
  exit 1; the advisory lock removed from `apply` → the locking test fails; the frontend
  decimal caps removed → the transport test fails.
- `openapi.json` and the generated `api.d.ts` regenerated from this tree: 24 paths, no dangling
  `$ref`. `/mcp` is absent from the spec by design — it is not an `/api` route.
- CI on the pushed head: all eight checks pass, including `build-and-test` (which applies
  `0006` to a fresh service database) and `docker-build-backend` (which compiles offline from
  the committed `.sqlx/`).

> **CORRECTED (was: "664 tests" / "361 tests" / "no warning in a touched file").** The counts
> were from an earlier head and the clippy claim was false on the head it described.

## Not in this PR

- **backup/restore** of staged batches — out of scope by decision.
- **Multi-currency** — separate workstream.
- **A unique index on `assets (portfolio_id, upper(isin))`.** Same gap as the symbol one, no
  constraint behind the software check, and `audit_asset_symbols.sh` reports existing
  collisions without fixing them.
- ~~The cluster side of the Postgres cutover~~ — **already done.** The cutover was executed on
  2026-11-15 against `portfolio.titan.arrieta.eu` and `docs/postgres-cutover.md` carries the
  execution record. The shipped body still described it as work that "must be applied
  **before** this image rolls", which reads as a prerequisite on this PR and is not one.
  What is left of that thread is in `docs/superpowers/specs/2026-11-14-postgres-gitops-followup.md`
  and lives in `k8s-titan`.

> **CORRECTED.** A stale prerequisite is worse than no prerequisite: a reviewer spends time on
> a gate that has already been passed, and stops trusting the rest of the list.
