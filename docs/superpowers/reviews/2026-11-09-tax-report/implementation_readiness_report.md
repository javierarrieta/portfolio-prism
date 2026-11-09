# Stage 9 — Implementation Readiness (three reviewers, DetailedRange 415–689)

*   **Target:** `docs/superpowers/specs/2026-11-09-tax-report-and-statement-design.md`, lines 415–689
*   **Reviewers:** `egm-implementation-readiness`, `egm-codebase-details`, `egm-zoom-out`
*   **Date:** 2026-11-09
*   **Note on method:** `egm-codebase-details` and the stage-9 `egm-zoom-out` both hit the
    30-minute timeout mid-investigation at 71 turns / ~116 tool calls. Both were resumed with
    forked context and instructed to write up only what they had verified. Items they did not
    finish are marked UNVERIFIED below. `egm-implementation-readiness` completed normally.

## Verdicts

*   **`egm-implementation-readiness`: not implementable as written.** Four of six slices cannot
    be completed without a design decision the document does not supply; three of six do not
    leave the tree green. The failures are not in the analysis — the engine section is the
    best-specified part — they are in the slice assignment: changes are filed under the file that
    will exist at the end, not the slice that must compile on Monday.
*   **`egm-zoom-out`: keep stage 8's split, reject it as drawn.** Four concrete errors, below.
*   **`egm-codebase-details`: 9 blockers, 11 majors, and 13 files the plan must add and omits.**

## Blockers — the plan does not compile

### S1 — Slice 0 does not compile
File 2 tells the implementer to "register the report guard from file 3 in managed state";
`report_guard.rs` is a **slice-4** file. Deferring that sentence hits the next blocker.

### S2 — P2 is not expressible as written, and there is no migration mechanism to express it in
File 2: "Add `UNIQUE(symbol, date)` to `historical_prices`, with a one-off dedupe." Verified:
`lib.rs:170-174` creates the table with no PK and no UNIQUE; `:176` adds a **non-unique** index.
**There is no `migrations/` directory and no `sqlx::migrate!`** — every schema change in this repo
is hand-written raw SQL inside `init_db` (`lib.rs:118-206`) plus bespoke helpers
`migrate_decimal_columns` (`:399-416`) and `add_owner_id_column` (`:236-238`). SQLite has no
`ALTER TABLE … ADD CONSTRAINT UNIQUE`, and editing the `CREATE TABLE` text is a **no-op on every
existing database**, including the owner's.

Must say: dedupe → `CREATE UNIQUE INDEX IF NOT EXISTS`, written as an idempotent step in
`init_db`, with a **stated tie-break for which duplicate survives** (the table has no write
timestamp, so the only deterministic choice is `max(rowid)` — and that choice *is* a figure
decision, since Yahoo revises closes around corporate actions). Must also state ordering against
`migrate_decimal_columns`, which renames that same table's `close_price` on a dedicated
connection with `PRAGMA legacy_alter_table = ON` (`:408-416`). `lib.rs` propagates migration
errors with `?`, so a bad dedupe means **the app refuses to boot**. The heading "migrations" must
become "idempotent `init_db` DDL" or a reader will look for a mechanism that does not exist.
The repo already has the convention for proving this — `tests/decimal_migration_test.rs` builds a
legacy schema and asserts the migration — and no migration test is listed.

### S3 — P4 is described backwards, and its blast radius names no file
File 1 says both `get_price` and `get_historical_prices` change to return a result type.
Verified: `get_historical_prices` **already returns `Result`** (`currency_service.rs:406-411`);
its defect is five internal paths that swallow failure and return the DB fallback as `Ok`
(`:429`, `:442`, `:449`, `:467`, `:502`). Only `get_price` (`:267`) returns a bare `Decimal` with
`Decimal::ZERO` on failure.

Hard consumers of the signature change that the 25-file list never names: `stats_engine.rs:139`
(`get_historical_prices`), `:158` (`get_rate(...).unwrap_or(Decimal::ONE)`), `:180`
(`get_price`), and `analytics.rs:73` (`get_price`). The plan's only acknowledgement — "the
existing screen inherits the honesty as a side effect" — is a side effect that does not compile.
This is also the unanswered half of critic M-u: does the performance endpoint keep its
`.unwrap_or(Decimal::ONE)` permissiveness?

### S4 — The guard is unreachable from the code meant to decrement it
File 3: "the budget is a counter decremented by the pricing layer before each outbound call."
Verified: `CurrencyService::new()` (`:131-143`) takes no arguments and holds no guard reference;
it is constructed at `main.rs:56` and **20 further sites** across `tests/common/mod.rs`,
`tests/cors_test.rs`, `currency_service.rs`, `tax_engine.rs` and `stats_engine.rs`. A guard in
Rocket managed state is invisible to `fetch_with_retry` (`:157`) unless the constructor changes —
i.e. all 24 sites. Either say so, or delete file 3 as stage 8 recommends.

Related: `build_rocket(pool, currency_service, cors, auth)` (`lib.rs:420-426`, `.manage` at
`:433-435`) is called from `main.rs:63` and `tests/common/mod.rs:46,56,65`. Adding
`.manage(report_guard)` either changes that signature (→ `main.rs` + `tests/common/mod.rs` +
`cors_test.rs`, none listed) or constructs a fresh guard per Rocket instance, which changes what
the isolation test can prove. The plan picks neither, and never mentions the `pub mod
report_guard;` declaration `lib.rs` needs.

### S5 — Slice 2 fails CI at its own finish line
File 4 (slice 2) carries the `TaxLot` fix, but `TaxLot` is defined in `schemas.rs:89-102`
(file 5, **slice 4**), and `schemas::TaxLot` is registered in `openapi.rs:35` and appears in the
committed contract (`docs/openapi/openapi.json:1081-1109`), which CI diffs byte-for-byte
(`ci.yml:29-36`). File 12 (regenerate the spec) is slice 4. So slice 2 both fails CI and ships an
**undeclared change to a live endpoint's response shape** — in the document whose Goal 7 promises
no such change. The plan invites it: "`TaxLot` is corrected in place rather than worked around."

Mitigating fact: the buy id the plan says is missing already exists as `InternalLot.source_tx_id`
(`tax_engine.rs:366`) and `LotMatch.buy_transaction_id` (`schemas.rs:134`); only `TaxLot` lacks
the field. That makes it a field addition, not new plumbing.

### S6 — Three existing tests pin the buggy `TaxLot` equality and no entry names them
`tests/analytics_test.rs:51-52` (`original_qty == "100.0"`, `remaining_qty == "100.0"`);
`frontend/src/test/components/PortfolioDetail.test.tsx:184-185` (`original_qty: 50,
remaining_qty: 50`). `PortfolioDetail.tsx` renders the two columns from the one value
(headers `:322-323`, cells `:333-334`) and is not in the list. File 11 names only
`sell_pnl_test.rs`, which contains **no** `original_qty` assertion at all.

### S7 — File 8's hermetic claim is impossible for the dated-FX path, and P4 inverts the mechanism the only hermetic pricing test depends on
`get_rate` (`:220`) **takes no `pool` parameter and has no database read path at all** — its only
storage is the in-process `HashMap` at `:125`. There is no FX table; only `historical_prices` and
`split_sync` exist. Seeding cannot produce a past FX rate, so a dated multi-currency test must
reach Yahoo.

Worse, and the sharpest finding in the review: the existing hermetic pattern works **because**
`get_historical_prices` is network-first and falls back to the DB — `analytics_test.rs:169-171`
seeds prices with the comment *"so the engine uses them instead of a live Yahoo fetch"* — and
**P4 is precisely the change that turns that fallback into an error.** Slice 4 inverts the
mechanism its own test depends on, and file 8's seeding instruction then produces 503s instead of
prices. Separately, seeding every date (as `analytics_test.rs:171-183` does) leaves no calendar
gaps, so the "non-trading `to` resolves to the prior trading day" case needs a gapped seed —
which is the case the network-first path will try to fetch.

Also unwritable as named: file 8's "an unreachable provider returns 503" — `CurrencyService::new()`
takes no configuration and the Yahoo URL is hardcoded four times (`:236`, `:303`, `:421`, `:574`),
so there is no seam to make it unreachable. File 9's "slot is free after the first request
completes or panics" needs a handler that panics on command, and a panicking handler under
Rocket's local `Client` unwinds the test thread rather than producing a response (UNVERIFIED —
reasoned from harness shape, not executed). "Second concurrent report returns 429" needs two
overlapping dispatches with a stall seam the plan does not name.

### S8 — File 13's premise is false: nothing imports the generated types
`frontend/src/types/api.d.ts` exists and `npm run generate-types` exists (`package.json:14`), but
a repo-wide grep for importers returns **only the package.json script line — zero importers**.
Every renderer uses hand-written types in `frontend/src/types.ts` (`TaxLot` :42,
`AssetTaxSummary` :51, `TaxSummary` :89, `LotMatch` :97, `SellPnlBreakdown` :111). Regenerating
`api.d.ts` changes nothing. File 13 is a no-op unless `types.ts` is added or the renderers are
repointed, which is a separate refactor no file describes.

### S9 — Slice 3 cannot compile its own payload
File 6's slice-3 work is "build `AssetTotals` and rows server-side", but `AssetTotals` /
`ReportRow` / `ReportResponse` are file 5, **slice 4**; the same sentence also says "acquire the
per-user report slot", which is file 3, **slice 4**. The plan gives no per-slice split of file 6's
bullets. And `analytics.rs:70` calls `get_price` per asset, so slice 3's `Network? no` is false
unless the implementer decides the payload is temporarily half-null — which makes the typed
payload a lie for one slice and the CSV's summable-columns header describe a shape that exists
only briefly.

## Blockers — stage 8's split, stress-tested

### S10 — Doc A as specified does not compile
Stage 8 sized Doc A at ~5 files. P4's `get_price` change breaks `analytics.rs:73` (Doc B's file 6)
and `stats_engine.rs:180` (in no list at all). Doc A's honest size is 7–8 files, two of which are
outside its own boundary.

### S11 — The elapsed-deadline replacement reintroduces the failure the guard existed to prevent
Stage 8's wording — "stops attempting further assets once elapsed time approaches the cutoff,
**returning what it collected**" — is a partial tax report, the thing Goal 5 and "no partial file
is produced" forbid, and worse if the artifact becomes a file the owner files. It must fail closed:
deadline exceeded → existing 503 + `retryable: true`, no partial payload. Two constraints stage 8
did not state: the deadline cannot preempt an in-flight outbound call (≈33 s worst case per
unreachable host), so it must sit near 20 s against the ingress-nginx 60 s default; and it can
only be checked *between* assets, which bounds anything only if the FX work is already warm.

### S12 — "One FX range per pair" is not achievable from the existing call site
`tax_engine.rs:403` calls `get_rate(pair, tx.date)` per transaction. A cache keyed by pair cannot
know the span to fetch from one date. Doc A must choose: (a) fetch a fixed wide window per pair on
first miss and hit it for every date lookup — which also makes the *undated* screen cheaper,
strengthening stage 8's own anti-guard case; or (b) the route pre-resolves a rate table and passes
it into `replay_at` — critic C2's fix, a fourth engine change belonging in Doc B. Stage 8 asserted
the cache-key change alone makes the work set structurally bounded. It does not.

Supporting detail: `get_historical_prices` (`:406-424`) **already** builds
`chart/{symbol}?period1={ts}&period2={ts}&interval=1d`, already writes the series into
`historical_prices` (`:488-497`) and already merges DB and provider rows (`:505-516`). The plan's
"add a per-currency-pair range fetch" reads as net-new when the real new work is a **per-pair
series cache** — the existing cache value type is `Decimal` keyed by `(pair, date)` (`:125`), so
"version the key" understates a value-type change to a dated series. And that path has no cache at
all today. Separately, `get_price` hardcodes `let today = Utc::now().date_naive()` (`:268`), so a
past-close lookup cannot be built on it at all; the plan must say which function it wraps.

### S13 — Deleting the guard does not delete the RFC amendment, which lives in a 26th file
The amended non-goal text is in the predecessor (`...-concepts.md:230-237`), not only in the
design's Reconciliation paragraph. The surviving replacement is still both things the original
non-goal excluded — a cache layer and a timeout knob beyond the work-set shrink — so the
amendment changes subject rather than disappearing, moves to Doc A, and requires editing the
concepts doc. No file list accounts for that file.

### S14 — Doc A has no test, and file 11 is a no-op
Every fixture in the repo is single-currency: `tests/common/mod.rs:120` hardcodes `currency =
'USD'`, `seed_portfolio` sets `base_currency` to the same string, `sell_pnl_test.rs:67` asserts
`fx_applied == false`, and `tax_engine.rs`'s harness passes `"USD", "USD"`. So file 11's
"expectations that assume FX conversion at the latest rate are updated as part of P1" describes
expectations that do not exist, and **nothing in the suite can distinguish a correct dated-FX
implementation from the current broken one**. Doc A must *add* the first FX test — and cannot do
so hermetically unless it lands FX on `historical_prices` under `EURUSD=X`. That is the single
most important omission in stage 8.

## Where stage 9 and stage 8 agree

Guard deleted; subset deleted with Alternative G and the IDOR row; P3 out with the `/pnl` precedent
as the rule; strict validation scoped to the dated path; Access rationale and file 9's collaborator
case deleted; files 16 and 22 deleted; print slice deferred; provenance moved to a review record;
P2 before P1. The print slice lifts out cleanly — files 19–23 have no inbound references.

## Entanglements the split must handle

| # | Entanglement | Consequence |
| :-- | :-- | :-- |
| 1 | File 1 is shared by nature (slices 0 and 4) | Hard dependency: Doc B's error contract is meaningless before Doc A merges. Say "Doc B is blocked on Doc A". |
| 2 | File 6 (`analytics.rs`) is edited by both documents | Must be ordered and named, or the two documents conflict on the one handler serving the screen the owner reconciles against. |
| 3 | File 4's `TaxLot` fix lives in `schemas.rs` and breaks `analytics_test.rs` | Doc B's list is missing a file it must edit, and this is a Doc B item that moves existing figures. |
| 4 | File 2 (`lib.rs`) is clean after guard deletion — purely P2 | But see S2: unstated dedupe tie-break, boot-failure risk, and `idx_historical_prices_symbol_date` (`:176`) becomes redundant. |
| 5 | File 17 (`App.tsx`) is listed for slices 1, 4, 5 — two are print-slice | After deferral it is a two-line change. |

**Worst intermediate state created by the split:** Doc A shipped with P1 + P4 and no FX test and no
seedable FX store — every multi-currency figure moves with nothing asserting it. Today's behaviour
is wrong but at least consistent with the suite.

## Dead code and untestable stubs

*   **Print deferred:** nothing goes dead in the backend. Orphaned prose: Goal 6, Alternative A,
    slices 1 and 5, the diagram's right branch, the Verification Plan's Print line, file 17's
    slice-1/5 clauses, and the "shared formatter" reuse claim.
*   **The real stub risk is frontend and comes from the combination.** File 21's `handlers.ts`
    change is the only MSW wiring in the list and exists for the statement. Print deferred **and**
    CSV client-side → files 14/15/16 survive but the dated fetch and typed-error surfacing in
    `App.tsx` ship with **no test at all**. Print deferred **and** CSV server-side → files 14/15/16
    disappear and **file 13 has no consumer**: a typed contract with no client. Decide explicitly
    whether the JSON variant stays a documented public surface (keep 7/12/13) or becomes an
    internal intermediate.
*   **After guard deletion**, the elapsed deadline has no owning file and no value; it belongs in
    file 6's asset loop plus a constant in file 1's resolver, and its semantics must be rewritten
    fail-closed.

## Additional verified mismatches (majors)

*   **`Transaction.date` is `DateTime<Utc>`, not a date** (`models.rs:35`). The plan claims both
    "no conversion is involved" and "a sell at any time on `to` is inside the window"; both cannot
    hold. `replay_at`'s `end` type and the conversion must be stated.
*   **`replay_at`'s printed signature cannot produce open-lot values.** `market_value` and the
    gain/loss fields are computed in `calculate_lots` from `current_price_base`
    (`tax_engine.rs:549-551`, `:580-590`), which needs the current price and the currency service.
    The signature omits `current_price`, `currency_service`, `strategy`, `threshold_days`.
*   **File 7 describes the wrong edit.** The `#[utoipa::path]` has no `params(...)`; utoipa derives
    query parameters from the Rocket attribute (`analytics.rs:21`), so `&<from>&<to>` documents
    itself. What file 7 must say is that the new report types be added to `components(schemas(...))`
    in `openapi.rs:26-40`, or the schema references unregistered types.
*   **File 10's "extend the auth sweep to the dated variant" has no hook.** `auth_test.rs:24-52`
    derives paths from the router and special-cases `/tax-summary` at `:46`; there is one route, so
    a dated sweep needs a second parameterised pass, not a special case. Confirmed holding: the
    three hard counts (`route_coverage_test.rs:140`, `:262`, `:275-278` = 15/15/13) and
    `to_document_path` (`:86-92`) truncates at `?`, so adding query parameters leaves them intact.
*   **File 21 mis-describes the MSW change.** `handlers.ts` already registers
    `/api/portfolios/:id/tax-summary`; MSW matches on path, so a dated request is already
    intercepted. The instruction should read "branch the existing handler on `from`/`to`".
*   **P2's unique index will not exist in hand-rolled test schemas.** `currency_service.rs:722`,
    `:745` and `decimal_migration_test.rs:21`, `:86` each `CREATE TABLE historical_prices`
    themselves, bypassing `init_db`. Any P2 assertion written against them proves nothing.
*   **File 5 names the wrong serde helper.** `decimal_json` (`db_types.rs:22`) serialises a
    non-optional `Decimal`; the nullable helper is `decimal_opt_json` (`:73`, used at
    `schemas.rs:199-216`). The payload mandates "nulls are nulls" and `UNMATCHED` carries null money.
*   **Verification tooling reality.** `frontend/node_modules` does not exist, so `npm test`,
    `lint`, `typecheck` and `generate-types` all fail until a networked `npm ci`. No chromium-family
    binary exists on this machine and `playwright-core` does not download browsers, so the print
    slice is unverifiable here. `cargo`/`clippy`/`rustc` are present with a populated registry cache
    but no `backend_rust/target/`, so every cargo command is a cold full build.
    `bash scripts/check_access_gate.sh` is the only listed gate that runs unconditionally.
    **`cargo clippy` is local-only — CI never runs it.**
*   **Confirmed correct, no change needed:** `replay` private (`:390`); breakdowns discarded after
    summing (`:569`); `latest_date` from `sorted_txs.last()` (`:550`); split branch rescaling every
    lot (`:517-531`); `original_qty`/`remaining_qty` both post-decrement (`:587-588`); `get_rate`
    per transaction (`:403`); symbology `format!("{}={}")` with a range-less URL and latest-close
    selector (`:235-236`, `:257-259`); cache keyed by date and never evicted (`:125`, `:261-262`);
    `Access::Viewer` (`analytics.rs:30`); `body = serde_json::Value` producing `"schema": {}`;
    `downloadCsv` owning the BOM (`download.ts:2,20-23`); `normalize*` coercing through `Number()`
    (`decimal.ts:60,130`); `sync_splits_for_asset` count-then-insert with a 30-day throttle
    returning `Ok(0)` (`:46`, `:60`, `:556-560`); `main.rs:63` a plain `.launch()` with no tasks;
    `--print-openapi` checked before config/db/network (`main.rs:29-32`) and CI-diffable.

## Files the plan must add and omits

| Omitted file | Why it is forced |
| :-- | :-- |
| `backend_rust/src/engines/stats_engine.rs` | Calls `get_historical_prices` (:139), `get_rate` (:158), `get_price` (:180) — breaks on P4 |
| `backend_rust/tests/analytics_test.rs` | Pins `original_qty == remaining_qty` at :51-52 |
| `frontend/src/components/PortfolioDetail.tsx` | Renders the two lot columns from one value (:322-323/:333-334) |
| `frontend/src/test/components/PortfolioDetail.test.tsx` | Pins the buggy equality at :184-185 |
| `backend_rust/src/main.rs` | `build_rocket(...)` call site at :63 if managed state changes the signature |
| `backend_rust/tests/common/mod.rs` | Three `build_rocket*` sites at :46/:56/:65 |
| `backend_rust/tests/cors_test.rs` | Five `CurrencyService::new()` sites if the constructor changes |
| `backend_rust/tests/decimal_migration_test.rs` | Hand-rolled `historical_prices` DDL at :21/:86 that P2 must also cover |
| `frontend/src/types.ts` | The real hand-written types every renderer uses; `api.d.ts` has zero importers |
| `frontend/package.json` | Needed for any `e2e:statement` script |
| `frontend/src/main.tsx` | The only entry-import site for a print stylesheet (`main.tsx:3`) |
| `docs/superpowers/reviews/2026-11-09-tax-report.md` | Stage 8's provenance move; the convention is real |
| A shared CSV-cell module (or an export change to `sellPnlCsv.ts`) | `FORMULA_LEAD`/`text`/`num` are module-private at `:55,56,62` |
| `docs/superpowers/specs/...-concepts.md` | The RFC amendment at :230-237 must be edited if the guard goes |

## What to ship first (stage 9 zoom-out's independent answer)

**Doc A alone, first, standing on its own merits** — it is not a prerequisite dressed as a feature:
today a foreign-currency asset cannot be priced at all (`EUR=USD` returns no data, `:236`) and every
multi-currency realized figure is converted at today's rate (`:403` + `:257`). Order inside Doc A:
P2 (dedupe with a stated tie-break, then the unique index) → P1 (dated FX, `EURUSD=X`, per-pair
series cache with a stated fetch window, cache bound and key versioning, FX persisted to
`historical_prices` so it is seedable) → the first hermetic FX test, replacing file 11's no-op → P4
with "the undated path keeps today's permissive behaviour" stated, plus the `stats_engine.rs:180`
and `analytics.rs:73` compile fixes → `SellPnlModal.tsx` footnote → release note naming which
figures move.

**Then Doc B minimal:** `replay_at` + explicit `valuation_date` + `date <= to` before matching (no
network); then dated `tax-summary` with both-or-neither dates, collected pricing failures, a
fail-closed elapsed deadline, and **server-generated CSV via `?format=csv` on the same route**; then
`ReportControls` (two date inputs + four presets, defaulting to current-year-**to-date**, not
Dec 31, or the default request trips the future-date 422).

Server CSV is cheap and the reason is verified: auth is an HttpOnly session cookie
(`apiFetch.ts`, `auth/guard.rs:103` — the guard never consults `Authorization`), and `/api` is
same-origin in dev and prod (`nginx.conf:12`). So a plain `<a href>` download carries the session,
`curl --cookie` reproduces the file byte-for-byte (Goal 4 becomes testable for free), and
`route_coverage_test.rs`'s hard counts stay correct because no route is added — that count test is
precisely what keeps `format=csv` on the existing route instead of tempting a new one.
**UNVERIFIED:** whether utoipa 4.2 can declare two content types for one status without a
hand-written `ToResponse`, and whether Rocket 0.5's `Either` implements `Responder`; the in-repo
pattern to cite is the auth enum responder (`LoginOutcome` / `Unavailable`, `auth/routes.rs:33-52`).

**Skip:** print/statement, asset subset, guard, P3, undated-path strict validation, files 16 and 22.

**One gap stage 8 never addressed that blocks both halves:** single-parameter dates. "`to` alone means
cumulative from inception" must be stated in the route contract. It was the statement's primary case;
with the statement deferred it becomes the CSV's `realized_cumulative` column, so it is now
load-bearing for the thing that *is* shipping.

## Slice re-cut recommended by stage 9

1.  Move file 5 (payload types), file 7 (OpenAPI), files 12 and 13 into **slice 3**, so slice 3 is
    "typed dated payload, realized only, single-currency, hermetic" and compiles and regenerates
    cleanly.
2.  Move the `TaxLot` fix out of slice 2 into slice 3, with `schemas.rs`, `openapi.json`
    regeneration, `analytics_test.rs`, `PortfolioDetail.tsx` and its test, and a new assertion for
    the pre-decrement `original_qty`.
3.  Split file 6's bullets per slice — the 429/guard sentence does not belong in a slice-3 sentence.
4.  Make slice 4 "pricing + failure contract" only, and give it a **provider seam** (injectable base
    URL, or an explicitly DB-first dated lookup with the fallback rule stated) and an
    **elapsed-deadline rule** — which is also stage 8's replacement for the guard and makes file 3,
    its 429 variant and file 9's unwritable concurrency tests disappear together.
5.  Delete file 2's guard-registration sentence regardless of what happens to the guard, and give P2
    a migration test in the `decimal_migration_test.rs` shape plus a dedupe tie-break.
6.  Give slice 1 an `App.tsx` entry point and a named print hook and chrome selectors (the nav is
    `<ul className="nav-menu">` with `.nav-item` children, `App.tsx:385-396`), or defer the whole
    slice — as drawn it is the only slice whose *verification* is unrunnable.

## UNVERIFIED — outstanding checks

1.  `tests/access_gate_test.rs` — never read; may assert handler/file counts that a changed
    `analytics.rs` handler surface would break.
2.  `tests/access_test.rs` — never read; may hold tax-summary access assertions that move.
3.  Rocket local-client panic behaviour underpinning S7's panic-release claim.
4.  Whether `e2e/run.sh` supplies `AUTH_MODE=disabled`, and whether `cargo run` succeeds without it.
5.  `frontend/src/test/mocks/server.ts` and `handlers.test.ts` — the latter may assert the exact
    handler list and break when the tax-summary mock is branched.
6.  Whether `cargo test` actually passes offline here (registry cache present, `target/` absent).
7.  utoipa / Rocket content-negotiation feasibility for `?format=csv` (see above).
