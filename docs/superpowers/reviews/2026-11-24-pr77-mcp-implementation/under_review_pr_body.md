The MCP agent interface, end to end: a token an agent can hold, a staging area a token can write to and cannot approve, and an MCP server mounted inside the existing Rocket app exposing eleven tools.

21 commits, 79 files, +10718/−312. It is large because the four prerequisites are folded in — each was a branch of its own and all four had to land before the agent surface made sense. **Read it in the order below; that is the order the code depends on.**

The last three commits are a review round on the agent surface and one schema change it uncovered; they are [their own section](#review-round--the-findings-on-top-of-the-reviewed-head).

---

## ⚠️ Deploy order — read this before applying anything

**One. The frontend image must roll before the backend image.** `refactor(backend): money arrives as a string or the request is refused` (`7b639bb`) makes the API **refuse JSON numbers** for money fields. The browser's CSV importer posted JSON numbers until `5ccb11b`, which is also in this PR. New backend + old frontend = transaction creation and CSV import fail with `invalid type: integer 10, expected a decimal string`.

This is recorded at the mechanism itself — `db_types.rs:139-141`, above `strict_decimal_json` — so it survives the PR being closed. Both halves are in one PR now, which is what makes the ordering easy to miss: one merge, two images, and they do not roll atomically.

**Two. Run `backend_rust/scripts/audit_asset_symbols.sh` against the live database before the deploy that carries this.** Migration `0006` refuses to apply to a database that already holds one symbol twice in one portfolio, and `sqlx::migrate!` runs before Rocket serves a request — so a raising migration is a crashloop with the reason in a container log. The script runs the same predicate in advance, exits 1 and names the rows, and exits 0 when the migration will apply.

CI cannot tell you this. Its database is empty.

---

## WS0 — two prerequisites, no behavior change

**`8344cfe` one insert path for assets and transactions.** Assets and transactions were inserted from several places with subtly different validation, which is how the ISIN gap and the transaction-type gap below were able to exist at all. Everything now goes through `services::assets::insert` / `services::transactions::insert`.

**`5ccb11b` money leaves the browser as a decimal string.** `parseFloat` is how `9.99` became a value the tax engine could not trust. The write path now carries decimal strings end to end and never touches a float. `parseDecimal` returns the exact string that will be sent, canonicalized as text.

## WS1 — personal access tokens

**`bf65929` / `e13600a`.** Tokens an agent can hold, minted and revoked from a new Access screen.

The design point: **a token is not a session.** `AuthFairing` accepts either, but a token carries scopes and a session does not, and conflating them would make the scope model decorative. Expiry is opt-in; a 90-day staleness nudge exists because a token nobody has used in a quarter is usually a token nobody remembers and someone should look at.

`users.token_version` is **not** what kills one — bumping it invalidates sessions, and tokens deliberately survive `logout-all` (`pat.rs`'s module comment, and the spec's revocation section). Revocation is the answer to a leaked token: `DELETE /api/tokens/<id>`, per token.

## WS2 — the agent surface

### Staging lifecycle — `9b37549`

A token writes a *proposal*, never a transaction. `stage_import` stores rows plus a human-readable preview under a handle; nothing is applied until a **session** (not a token) approves it. Migration `0004_agent_staged_batch.sql`.

The preview is stored, not derived. If it were computed at render time, what a human read and what the agent was shown could differ, and the approval would be an approval of something else.

### `apply` — `2a7df1c`

The one write the approval gate authorises, and the rules that keep it single-use. An approved batch cannot be applied twice; a discarded batch cannot be applied at all.

### Session routes + panel — `8dd932e`

`/api/staged-batches` (list), `/api/staged-batches/<handle>` (the stored preview, rendered verbatim), `/api/staged-batches/<handle>/approve`. These are session-only by construction — they are under `/api`, so `AuthFairing` requires a session and a token cannot reach them.

### The `/mcp` mount — `4a0b515`

`rmcp`'s `StreamableHttpService` mounted as a Rocket route, stateless, no Node.js anywhere in the runtime. Proven by a spike branch before any of this was written.

Four things about it are load-bearing and were each learned the hard way:

- **It is mounted at the root, not under `/api`.** Adding it to the `/api` mount list served it at `/api/mcp` inside `AuthFairing`'s prefix, which answered 404 — the only way that mistake announces itself. There is now a test that no mounted route escapes the prefix rule.
- **`/mcp` authenticates itself, bearer only.** The session cookie is deliberately not consulted: `Credential::Session` passes every scope, so honouring one would make the scope model decorative. `SELF_AUTHENTICATED` is a separate list from `PUBLIC_NAMESPACES` on purpose — "public by decision" and "must check itself" are opposite promises, and folding them lets the next route be waved through.
- **The bearer token is consumed at the door and never forwarded to `rmcp`.** A tool cannot leak a credential it never sees.
- **A refused tool call is a tool result, not a protocol error.** `not_approved`, `expired`, `scope_missing`, `portfolio_not_found` come back as `CallToolResult::error` carrying a JSON `code`. `gate()` returns `Option<CallToolResult>` rather than `Result` because `Err(McpError)` invites an infinite identical retry, and a missing scope never fixes itself.

### The eleven tools — `b363dc2`

```
read     list_portfolios  get_portfolio  list_transactions  get_tax_summary
         get_performance  get_transaction_pnl  lookup_asset
import   stage_import     apply_staged   discard_staged     get_staged
```

**Every tool calls the same function the corresponding `/api` route calls.** A rule enforced only in a tool is a rule an `/api` caller need not follow, which makes it not a rule.

Two deliberate removals from `get_performance`: the engine returns `volatility`, `sharpe_ratio` and `beta` as the constants `"0.0"`, `"0.0"` and `"1.0"`, and `realized_pnl`/`unrealized_pnl` alongside them. The three constants stay in the response but the tool description **names** them as non-measurements; the two PNL fields are stripped outright. A field that is present gets quoted — prose loses to the number.

`CurrencyService` moved behind an `Arc` (`97c5c04`) so the mount can build its server in a `'static` closure. One instance means one cache and one rate limiter; a second `CurrencyService::new` would double the request rate against a rate-limited provider.

---

## Review round — the findings on top of the reviewed head

`250ccdf` · `e4bde8c` · `2692a69`, on top of the reviewed `7d3075d`. Sixteen findings; all addressed, one of them wrong, and the wrong one is recorded below because the wrong answer is the useful part.

### `250ccdf` — a lying preview, an unspendable approval, an unvalidated target

- **The base currency was a literal.** `resolve_asset` defaulted an asset's currency to `"USD"`, so a staged import into a EUR portfolio priced every row in the wrong currency and stored it. `resolve_portfolio` now reads the base currency off the row and threads it through; the mutation that proves the fix is a test that fails the moment the literal comes back.
- **An approval could be granted on an already-expired batch** — `approve` had no expiry predicate — and then refused at apply. A human clicked approve, the write never happened, and nothing said why. The UPDATE now requires `expires_at > now()`, the 409 carries `expired` plus a truthful `detail`, and the panel repeats the server's text rather than inventing its own.
- **`validate` did not look at `target`, `symbol` or `asset_type` at all.** Length limits added (`MAX_NAME_LEN`, `MAX_IDENTIFIER_LEN`, `MAX_CURRENCY_LEN`) behind a new `TooLong` that shares the existing `out_of_range` code rather than minting a second one for the same promise.
- **The pending-batch cap was a `SELECT`-then-`INSERT` race**, now serialised by `pg_advisory_xact_lock(user_id)` in the same transaction.
- **`/mcp` forwarded framing headers** into the streamable-HTTP bridge; `content-length` and `transfer-encoding` are stripped.
- **The finding that was wrong.** `get_staged` is gated on `Scope::Import` and the review said it should be `Read`. It should not: an import-only token has to poll its own batch after `stage_import`, and there is a test (`get_staged_reports_what_a_human_actually_approved`) that fails if you make the change. `read_only_hint` describes *state change*; scope describes *the job the token was minted for*. They are not the same axis and should not be aligned.

### `e4bde8c` — a portfolio backstop, and a type that cannot name an error `apply` cannot produce

`Target::New` had no bound: an `import` token could mint one portfolio per apply, forever, and every one of them is a row in every portfolio listing from then on. `MAX_PORTFOLIOS_PER_USER = 10`, checked at staging so a human is never asked to approve a batch that cannot land, and again inside the transaction that creates the row — the count can move in between, and that second one is the one that decides.

**Ten is documented as arbitrary at the constant, because it is.** It is not derived from a measurement, a spec limit, or a resource ceiling — nothing in the app degrades at eleven. It is a backstop above any plausible personal portfolio count, and any number in the same order of magnitude would do.

`ApplyError::Rejected` used to carry the whole `StageError`, which meant it claimed `apply` could answer `too_many_pending` or `database` from re-validating a stored payload. It could not — those are facts about the moment of staging, not about the document — so the arm existed only to be unreachable. `validate` now returns a narrower `ValidationError` (every way a document can be wrong) and `apply` takes that, so the impossible state cannot be named. Narrowing the type rather than commenting it is the point: a comment goes stale the day someone adds a check to `validate` that can actually fail, and a test now walks every variant and refuses any staging-only code. Wire codes unchanged.

### `2692a69` — asset identity becomes a constraint, and the detector it makes unreachable is deleted

`assets (portfolio_id, symbol)` was unique in software and nowhere else. `services::assets::insert` and `agent::staging::resolve_asset` each `SELECT`ed the uppercased symbol and `INSERT`ed if absent, with a window in the middle and no constraint underneath, so two concurrent writers could both find nothing and both write. The consequence is not cosmetic: a split asset row is a split cost basis, and every report reads one of the halves. Migration `0006` adds `uq_assets_portfolio_symbol` on `(portfolio_id, upper(symbol))`, auditing and refusing first.

Three things in that commit are the finding rather than the fix:

- **No normalization step, deliberately.** The obvious `UPDATE assets SET symbol = upper(symbol)`, copying what `0005` did to `transactions.type`, is wrong here: `historical_prices` is keyed by the symbol string and not by `asset_id`, and every read path looks prices up by the asset's symbol — so rewriting an asset's symbol silently detaches its price history. No error, just an empty chart. The fix was to move the *lookups* to `upper(symbol) = $2` and leave the data alone.
- **`backup/detail.rs` computed `duplicate_symbol` from two producers, and the migration makes the database half unreachable** — not unlikely, unreachable: the migration refuses to apply to data that already has a duplicate and the index prevents new ones, so no sequence of writes leaves the detector anything to see. `owned_detail`'s `GROUP BY symbol` subquery is deleted rather than left dormant, because a detector that cannot fire is a reader's false premise. The flag stays live on the bundle side, where a bundle is caller-supplied JSON and nothing about the index constrains it until the insert runs.
- **That left a 500.** Restoring a bundle that puts one symbol in one portfolio twice died mid-transaction on a raw index violation — a server error for a bad upload. `backup::validate` now refuses it naming the asset path, upper-cased to match the index, and **per-portfolio rather than bundle-wide** because holding one security in a taxable account and a wrapper is ordinary, not damage.

The three tests that failed on the way were not test rot and were not routed around. Two seeded duplicate `assets` rows to exercise the dead detector — the index now refuses the seed insert itself, which is the fixture telling the reader the state is gone — and moved to the bundle side, where the case is still reachable. The third pinned both producers setting the flag; no restorable bundle can make them agree any more, so it asserts the opposite with the reason attached.

One of the new schema tests passed **vacuously** at first: `assets.name` is `NOT NULL`, so every insert in it failed on that instead of on the index, and it reported success while proving nothing. It now asserts the error names `uq_assets_portfolio_symbol`. That guard is the reason to distrust any assertion of the shape `assert!(result.is_err())`.

## Migrations

`0002` transaction-type CHECK · `0003` `api_token` · `0004` `agent_staged_batch` · `0005` split dedupe index made case-insensitive · `0006` `uq_assets_portfolio_symbol`.

`0005` closes a hole `0002` documented itself: the CHECK is `upper(btrim(type))` but the partial index keyed on `type = 'SPLIT'`, so a `'split'` row satisfied the CHECK and escaped the dedupe. The index is the *only* dedupe — `apply_split_events` has no read-based check of its own — and a double-applied split rescales cost basis without saying so.

`0005` is DROP-then-CREATE, which is not atomic. Acceptable only because `sqlx::migrate!` runs before Rocket serves a request and the live deployment is `replicas: 1` with `Recreate`. **At two replicas that window is a real race**; the migration says so out loud.

`0006` audits before it creates, for the same reason `0002` does: a unique index cannot be added to a table that already violates it, and a migration that raises is an outage rather than a refusal. `scripts/audit_asset_symbols.sh` runs the identical predicate ahead of the deploy. It also reports existing ISIN collisions and says plainly that it does not constrain them — `isin` has the same software-only gap and no index, and closing it needs the same audit-first shape.

## Access control

`access.rs` remains the single chokepoint. `check_access_gate.sh` fails the build if an `/api` data route takes no `AuthedUser` or calls no `require*`. Each gate has its own test that runs the real script against a throwaway tree and mutates it — a gate is only as good as the mutation that proves it can fail.

## Verification

- **Backend: 664 tests, 0 failed.** All six gates pass (`access_gate`, `pricing_seam`, `backup_portability`, `upload_path`, `migrations`, `sqlx_cache`) on a freshly migrated database. `cargo clippy --all-targets`: 0 errors, no warning in a touched file. `cargo sqlx prepare --check`: clean — and it caught the stale cache twice during the review round, because two changed queries were compile-time macros; the image builds from that cache, so `.sqlx/` is part of the change.
- **Frontend: 361 tests, 0 failed.** `tsc --noEmit` clean. `eslint src`: 0 errors, 0 warnings.
- Migrations 0001–**0006** apply to an empty database; `uq_split_asset_date` reads `WHERE (upper(btrim(type)) = 'SPLIT'::text)` and `uq_assets_portfolio_symbol` reads `(portfolio_id, upper(symbol))`.
- `0006` was also checked in the failure direction: against a database holding `auditdup` beside `AUDITDUP` it refuses and names the portfolio, the ids and both spellings, and applies once the rows are merged. `check_migrations.sh` fails with the index dropped.
- `openapi.json` and the generated `api.d.ts` regenerated from this tree: 23 paths, no dangling `$ref`. `/mcp` is absent from the spec by design — it is not an `/api` route.
- CI on the pushed head: all eight checks pass, including `build-and-test` (which applies `0006` to a fresh service database) and `docker-build-backend` (which compiles offline from the committed `.sqlx/`).

## Not in this PR

- **backup/restore** of staged batches — out of scope by decision.
- **Multi-currency** — separate workstream.
- **A unique index on `assets (portfolio_id, upper(isin))`.** Same gap as the symbol one, no constraint behind the software check, and `audit_asset_symbols.sh` reports existing collisions without fixing them.
- The cluster side of the Postgres cutover (`docs/superpowers/specs/2026-11-14-postgres-gitops-followup.md`) lives in `k8s-titan` and must be applied **before** this image rolls: the build refuses to start without `DATABASE_URL`, and `replicas: 1` with `Recreate` makes a badly ordered apply an outage. Confirm `.spec.strategy.type` on the live deployment rather than assuming which rollout behaviour you are about to cause.


