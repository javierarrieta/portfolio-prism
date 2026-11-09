# Pricing Layer Truthfulness

*   **Author:** javierarrieta, with EGM review support
*   **Status:** Proposed
*   **Date:** 2026-11-09
*   **Depends on:** nothing. **Blocks:** `2026-11-09-tax-report-and-statement-design.md`
*   **Origin:** split out of the tax report design during its EGM review
    (`docs/superpowers/reviews/2026-11-09-tax-report/`), stages 8 and 9.

## Overview

Three defects in the pricing layer make past-dated figures wrong in a way that produces no
visible signal, and a fourth makes "no data" indistinguishable from "could not ask". All
four are worth fixing whether or not any report feature ships: today a foreign-currency
asset cannot be priced at all, and every multi-currency realized figure in the app is
converted at today's rate.

This document is the pricing-layer fix on its own merits. It adds no route, no frontend
surface beyond one footnote, and no new state. It moves existing figures, so it ships as
four separately reviewed changes with a release note naming which figures move.

## The Problem

1.  **Foreign-currency assets cannot be priced at all.** `CurrencyService::get_rate`
    builds the Yahoo symbol as `EUR=USD` (`currency_service.rs:235`); the live endpoint
    returns "No data found" for that form and data for `EURUSD=X`. Any asset whose
    currency differs from the portfolio's base currency fails.
2.  **`get_rate` ignores its date argument.** It keys its cache on the requested date
    (`:125`) and then returns the latest close in the series (`:257-259`), so the stored
    value looks correctly dated and is not. The URL carries no range (`:236`), so a past
    date is not in the response and no selector change can fix it. `tax_engine.rs:403`
    already calls it per transaction with past dates, so existing multi-currency figures
    are already converted at today's rate.
3.  **`historical_prices` has no unique constraint.** `lib.rs:170-174` creates the table
    with no primary key and `:176` adds a non-unique index, so `INSERT OR REPLACE` always
    inserts, every fetch appends duplicates, and "the close on date X" is an arbitrary row
    among N — the reads have no `ORDER BY`.
4.  **Failed lookups are indistinguishable from absent data.** `get_price` returns
    `Decimal::ZERO` on every failure (`:267`). `get_historical_prices` already returns a
    `Result` (`:406`) but swallows the error at five sites (`:429`, `:442`, `:449`, `:467`,
    `:502`) and hands back the database fallback as `Ok`.

A fourth problem is structural rather than behavioural: **there is no way to test any of
this.** Every fixture in the suite is single-currency — `tests/common/mod.rs:120` hardcodes
`'USD'` and `seed_portfolio` sets `base_currency` to the same string, and
`sell_pnl_test.rs:67` asserts `fx_applied == false` — so nothing in the suite can
distinguish a correct dated-FX implementation from the broken one. There is also no FX
table: rates live only in the process `HashMap` at `:125`, lost on every restart, so a
past FX rate cannot be seeded.

## Goals

*   **Goal 1 — dated FX actually dated.** `get_rate(pair, date)` returns the last rate on
    or before `date` within a bounded lookback, and says which date it used.
*   **Goal 2 — FX resolves at all.** `EURUSD=X` symbology, so a foreign-currency asset
    prices instead of failing.
*   **Goal 3 — one close per symbol per date.** `historical_prices` cannot hold two rows
    for the same `(symbol, date)`, so an as-of price is reproducible.
*   **Goal 4 — no-data is not provider-down.** A caller can distinguish "this symbol has no
    close on or before that date" from "Yahoo could not be reached".
*   **Goal 5 — the FX path is hermetically testable.** A backend test asserts a past-dated
    multi-currency conversion without reaching the network. This discharges the open item
    *"FX conversion path has no backend test"* in
    `docs/superpowers/reviews/2026-11-05-sell-pnl-breakdown.md`.
*   **Goal 6 — every figure movement is declared.** Each of the four changes ships alone,
    with a commit-message note naming which existing figures move and why.

## Non-Goals

*   **No report, payload, CSV or print view.** That is
    `2026-11-09-tax-report-and-statement-design.md`, which depends on this one.
*   **No new route and no new endpoint behaviour on the undated path.** The dated report
    needs the honest result types; the existing screen keeps its current permissive
    behaviour, stated explicitly under Open Questions.
*   **No provider failover, batching framework or concurrent fan-out.** The work set is
    shrunk instead.
*   **No split-sync redesign.** `sync_splits_for_asset` running inside read paths is a
    separate investigation with no destination in this deployment — `main.rs:63` is a plain
    `.launch()` with no Rocket tasks and `k8s/` has no CronJob or Job. Tracked as issue #50.

## Technical Plan

Four changes, in this order. The order is forced, not preferred.

### P2 first — `historical_prices` uniqueness

There is no migration mechanism in this repo: no `migrations/` directory, no
`sqlx::migrate!`. Every schema change is raw SQL inside `init_db` (`lib.rs:118-206`) plus
the bespoke helpers `migrate_decimal_columns` (`:399-416`) and `add_owner_id_column`
(`:236-238`). SQLite has no `ALTER TABLE … ADD CONSTRAINT UNIQUE`, and editing the
`CREATE TABLE` text is a no-op on every existing database, including the owner's.

The change is therefore, run idempotently from `init_db`:

1.  Dedupe existing rows. The table has no write timestamp, so the only deterministic
    tie-break is `max(rowid)` — the most recently written row survives. That choice is
    itself a figure decision, because Yahoo revises closes around corporate actions, and it
    is what makes "the close on date X" reproducible from here on.
2.  `CREATE UNIQUE INDEX IF NOT EXISTS uq_historical_prices_symbol_date ON
    historical_prices(symbol, date)`.
3.  Make the write path a real upsert (`INSERT … ON CONFLICT DO UPDATE`) instead of
    `INSERT OR REPLACE`, which today appends.

Ordering constraints: this must run after `migrate_decimal_columns`, which renames this
same table's `close_price` column on a dedicated connection with
`PRAGMA legacy_alter_table = ON` (`:408-416`). A dedupe that misses a duplicate makes the
index creation fail, `init_db` returns `Err` and **the app refuses to boot** — so this
ships with a migration test in the shape of `tests/decimal_migration_test.rs`, which
builds a legacy schema and asserts the migration. The existing plain index
`idx_historical_prices_symbol_date` (`:176`) becomes redundant and is dropped.

**Why P2 must precede P1:** P1's range fetch writes the fetched series into
`historical_prices`. Without the unique index, each fetch appends roughly 250 rows per
symbol, making the duplication this fix exists to remove actively worse.

### P1 — dated FX

Rebuild `get_rate` to request a date range with `period1` and `period2` and select the rate
on or before the requested date within a bounded lookback, using `EURUSD=X`.

The mechanism that makes "one FX range per pair" true is a **change of cache value type**,
not a key version. The cache is `HashMap<(String, String, NaiveDate), Decimal>` (`:125`) and
is never evicted. On first miss for a pair, fetch a fixed wide window and store a dated
series per pair; every date lookup then hits the series locally. This is the simpler of the
two available shapes — the alternative, having the route pre-resolve a rate table and pass it
into the engine, is a change to the reporting design's engine entry point and belongs to that
document.

Two consequences worth naming. First, this also makes the **undated** screen cheaper: today
every distinct transaction date is its own outbound call. Second, the shrink only counts if
it reaches the engine's per-transaction call site (`tax_engine.rs:403`), which is the
largest outbound consumer in the app — roughly one call per transaction on a cold cache.

Persist the fetched series into `historical_prices` under `symbol = 'EURUSD=X'`. That is
what makes Goal 5 possible: it gives FX a seedable table, cross-restart caching, and one
read path instead of two.

Bound the cache and version its key, so entries written by the current buggy behaviour
cannot be read as correct after the fix. Without a bound, once past dates become reachable
the cache grows without limit inside a 256Mi pod.

**This moves existing figures.** Every foreign-currency realized and unrealized number
changes, including the ones asserted in `backend_rust/tests/sell_pnl_test.rs`. It also makes
`SellPnlModal.tsx:283` ("at the latest available rate") false; that footnote is corrected in
the same change.

### The first hermetic FX test

Before P4, not after. A backend test that seeds `historical_prices` with an `EURUSD=X`
series and asserts a past-dated conversion produces the seeded rate rather than today's.
This is the change that makes the rest of this document verifiable, and it replaces the
assumption in the reporting design's file 11 that P1 would "update expectations that assume
FX conversion at the latest rate" — no such expectations exist.

### P4 — distinguishing no-data from provider-unavailable

`get_price` (`:267`) changes from a bare `Decimal` to a typed result distinguishing `NoData`
from `ProviderUnavailable`. `get_historical_prices` keeps its existing `Result` signature
and stops swallowing at `:429`, `:442`, `:449`, `:467` and `:502`.

Call sites that must move in the same change, or the tree does not compile:
`analytics.rs:73`, `stats_engine.rs:139` and `stats_engine.rs:180`.

**The trap in this change.** The only hermetic pricing test that exists works *because* the
network-first path falls back to the database: `tests/analytics_test.rs:169-171` seeds
prices with the comment *"so the engine uses them instead of a live Yahoo fetch"*. P4 is
precisely the change that turns that fallback into an error. So the read order must be
stated and tested: **database first for a dated lookup, provider only on a miss**, with the
fallback rule written down rather than inherited from an accident. Note also that seeding
every date the way `analytics_test.rs:171-183` does leaves no calendar gaps, so the
non-trading-day case needs a deliberately gapped seed.

`get_price` also hardcodes `let today = Utc::now().date_naive()` (`:268`), so it cannot
answer for a past date at all. The dated lookup must be built on `get_historical_prices`,
which already builds `chart/{symbol}?period1=…&period2=…&interval=1d` (`:406-424`), already
writes the series into `historical_prices` (`:488-497`) and already merges database and
provider rows (`:505-516`).

## Target Files

| File | Change |
| :-- | :-- |
| `backend_rust/src/lib.rs` | P2: dedupe with `max(rowid)` tie-break, unique index, drop the redundant plain index, real upsert. Idempotent `init_db` DDL, not a migration. |
| `backend_rust/src/services/currency_service.rs` | P1: `EURUSD=X`, range request, per-pair series cache with a stated window, bound and key versioning, persist to `historical_prices`. P4: `get_price` result type, stop swallowing at the five sites, DB-first read order. |
| `backend_rust/src/api_routes/analytics.rs` | Compile fix for the `get_price` call at `:73`; undated behaviour unchanged. |
| `backend_rust/src/engines/stats_engine.rs` | Compile fixes at `:139` and `:180`; keeps its current `.unwrap_or(Decimal::ONE)` permissiveness unless Open Questions decides otherwise. |
| `backend_rust/tests/fx_pricing_test.rs` (new) | Goal 5: seeded `EURUSD=X` series, past-dated conversion asserted as an exact decimal, no network. |
| `backend_rust/tests/decimal_migration_test.rs` | P2 migration test in the existing shape: legacy schema with duplicate rows, assert dedupe and index. |
| `backend_rust/tests/sell_pnl_test.rs` | Any expectation that moves with P1, in the same commit, delta explained in the message. |
| `backend_rust/tests/analytics_test.rs` | Gapped price seed for the non-trading-day case; assertions updated for the DB-first read order. |
| `frontend/src/components/SellPnlModal.tsx` | Footnote corrected from "at the latest available rate". No test pins that sentence — `SellPnlModal.test.tsx:59` only asserts `toHaveTextContent('converted to EUR')`. |
| `docs/superpowers/plans/2026-11-09-pricing-layer-truthfulness.md` (new) | Implementation plan, following the repo's existing convention. |

## Risks

| Risk | Mitigation |
| :-- | :-- |
| Every multi-currency figure moves at once | Four separately reviewed changes; each names its delta in the commit message; P1 ships alone with a release note |
| A bad dedupe bricks boot | Migration test asserting dedupe + index on a legacy schema, run before release |
| P4 turns the only hermetic pricing test into 503s | DB-first read order stated and tested; the FX test lands before P4 |
| Cache grows unbounded once past dates are reachable | Bound plus key versioning in P1, same change |
| `SQLITE_BUSY` on the write path — no WAL, no `busy_timeout` | Out of scope here; named so the reporting design does not assume writes are safe |

## Open Questions

1.  **The bounded lookback length and the cache window size.** Both are rules, not
    parameters: a delisted fund priced three years late versus rejected is a different tax
    answer. Each needs a value and a test that asserts it.
2.  **Does the undated path keep its permissiveness?** `stats_engine.rs:158` does
    `.unwrap_or(Decimal::ONE)` and `:180` relies on `get_price` never failing. Keeping that
    behaviour preserves the screen exactly; changing it makes the performance endpoint
    honest but moves its numbers.
3.  **Which duplicate survives the dedupe.** `max(rowid)` is the only deterministic choice
    available, but it is a guess about Yahoo's revisions. An alternative is to keep the
    *first* write, which is what the app originally fetched.
4.  **Hand-rolled test schemas.** `currency_service.rs:722`, `:745` and
        `decimal_migration_test.rs:21`, `:86` each `CREATE TABLE historical_prices`
        themselves, bypassing `init_db`. Either they gain the index too or assertions written
        against them prove nothing.

## Verification

*   `cargo test` from `backend_rust/`, including the new `fx_pricing_test.rs` and the P2
    migration test.
*   `bash scripts/check_access_gate.sh` — unchanged by this document, run to prove it.
*   `cargo clippy --all-targets` and `cargo build --release` locally; note CI runs the build
    and the tests but **not** clippy.
*   No network in any test. The suite's hermetic rule is the point of Goal 5.

--------------------------------------------------------------------------------

*Split out during the EGM review of the tax report design*
