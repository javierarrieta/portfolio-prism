# Portfolio Tax Report and Portfolio Statement

**Date**: 2026-11-09
**Status**: Proposed — RFC for feedback, not yet a formal design
**Origin**: EGM brainstorm, six-lens Goldfish review of a concepts brief
**Prior record relied on**: `../reviews/2026-11-05-sell-pnl-breakdown.md` (Decimal scale, split-sync ordering, hermetic seeding)

**Normative for:** the scope boundary between the two reports, the three prerequisite
defects that must land first, the pricing-date rule, and the reconciliation invariants a
report must satisfy against the existing tax-summary figures. A formal design document
(`/egm-draft`) is still required for the route contract, the response schema and the task
sequencing.

## Overview

The owner needs portfolio figures for capital gains tax, which in their country is
settled quarterly. The app can show these numbers only for the current moment. There is
no way to ask "what did this portfolio look like on 31 December", and the only way to
get numbers out is an export that covers one sell at a time.

Two reports are proposed. The review established that they are not two features: they
are one dated-calculation change plus two renderers, and one renderer is free until it
is asked about the past.

### Terms, used precisely

* **prefix replay** — reconstructing lot state by running the matching loop over the
  transaction list truncated at a date. The codebase already uses "replay" for an auth
  callback (`backend_rust/src/auth/routes.rs:257`), so documents say *prefix replay*.
* **`end` and `as_of` are the same quantity.** Report 1 calls it `end`, Report 2 calls it
  `as_of`. A past statement is as supported as a past CSV.
* **latent and unrealized are one measure at two levels.** Per-lot fields are
  `latent_gain_loss` / `latent_roi`; per-asset totals are `unrealized_pnl` /
  `unrealized_roi`. Both reports use the same measure.
* **cumulative** means from inception, i.e. from the first transaction in the portfolio.

### Report 1 — tax report (CSV working file)

A file the owner loads into a spreadsheet and re-crunches. Nobody files it or signs it.
Rows are at lot granularity, separated by the existing `row_type` column:

* `SOLD_LOT` — one row per buy lot consumed by each sell dated inside `[start, end]`,
  with the sell's context repeated on every row, as the existing single-sell export
  does. Realized P&L lives here, in the per-lot `lot_pnl` column.
* `OPEN_LOT` — one row per lot still holding quantity at `end`, priced at the last close
  at or before `end`. Unrealized P&L lives here.
* `UNMATCHED` — kept from the existing export: sold quantity no buy lot covers, money
  cells blank so `SUM(lot_pnl)` still equals realized P&L.

### Report 2 — portfolio statement (printed by the browser)

A page the owner reads or hands to someone, showing realized P&L **cumulative from
inception to `as_of`** and unrealized P&L **as of `as_of`**, for one portfolio, with an
optional subset of its assets. `as_of` defaults to today. No server-side PDF: the
frontend renders a normal page with a print stylesheet and the owner uses the browser's
print-to-PDF.

The backend returns JSON only. The frontend produces both the CSV and the print view.

## Corrections

The pre-review seed draft made three claims about the code that are false. They are
listed here because a fresh `/egm-draft` session would otherwise re-derive them.

| Seed claim | What the code does | Evidence |
| :-- | :-- | :-- |
| `get_rate` is broken for past dates but no current caller passes one, so this feature is the first to expose it | `replay` calls `get_rate(asset_currency, base_currency, tx.date)` per transaction, so past dates are already in use and existing multi-currency figures are already converted at the latest rate | `backend_rust/src/engines/tax_engine.rs:403` |
| `tax_lots` carries `original_qty`, so lot-granularity rows are available today | `original_qty` and `remaining_qty` are both set to `lot.qty`, which has already been decremented by any partial sell, so the originally-bought quantity is unrecoverable from the payload. `TaxLot` also carries no buy transaction id and `AssetTaxSummary` no `asset_id` | `tax_engine.rs:585-586`, `schemas.rs:88-121` |
| The shared change is one new engine function | It is three changes (below), and prefix replay itself needs no engine change at all | `tax_engine.rs:390, 549-551, 569, 614-618` |

The seed's fail rule read: *"Missing price or missing FX rate fails the whole request,
naming the symbols. No silent fallback to a nearby trading day."* Decision 8 replaces it.

## Analysis

### The shared change is three changes, not one parameter

`calculate_lots` takes `transactions: &[Transaction]` and has no date parameter, and the
engine documents that a sell's matches depend only on transactions at or before it
(`tax_engine.rs:614-618`). Filtering the list to `date <= end` produces the as-of state
with no engine change. `current_price` is already a parameter (`:536-546`), so an as-of
close can be injected the same way.

An implementer who adds `as_of: Option<Date>` to `calculate_lots` ships three bugs:

1. `replay`, which produces the per-sell matches, is **private** (`:390`), and
   `calculate_lots` **discards** the breakdowns after summing them (`:569`). A dated
   report needs open lots and per-sell breakdowns from one replay, so the change is a
   new public entry point returning both.
2. The FX date for market value comes from the transaction list:
   `latest_date = sorted_txs.last().map(|tx| tx.date).unwrap_or_else(Utc::now)`
   (`:549-551`). Truncating the list silently moves the FX date to the last transaction
   before `end` — the nearby-day substitution Decision 8 forbids. The valuation date
   must become an explicit parameter.
3. The `date <= end` filter must run **before** sorting and matching, because the split
   branch (`:517-531`) rescales every lot in scope, so a split dated after `end`
   retroactively changes in-window cost basis.

The only public per-sell entry today is `sell_breakdown` (`:619`), which replays the
whole asset history per sell and re-enters the FX lookup per transaction. Building a
quarter of sold-lot rows that way is O(sells x transactions) with an FX call per
transaction.

Realized P&L needs no market quote — it is computed from recorded transaction prices.
Only the unrealized half needs a close. That seam drives the staging below.

### One payload, two renderers, and the totalling trap

`frontend/src/utils/sellPnlCsv.ts` documents that repeating sell-level context on every
lot row means `SUM(realized_pnl)`, `SUM(proceeds)`, `SUM(cost_basis)` and
`SUM(sold_qty)` are each multiplied by the row count; only `lot_pnl` totals. A tax file
built to that shape **will** be summed by the owner, so the file must state which
columns are legal to sum. Adding an `OPEN_LOT` row kind also means most sell-level
columns are blank on it — "same shape as the existing export" understates the change.

The two reports' realized figures are deliberately different: the CSV is a window, the
statement is cumulative from inception. One payload must carry both, or the owner prints
one number and reads another on the same day. The rule that makes that impossible:
when `start` = inception and `end` = `as_of`, the two must agree, and that equality is a
test.

The payload must also be typed. `tax-summary` declares `body = serde_json::Value`, so
the generated OpenAPI has no schema and the frontend coerces absent fields to zero.

### "Fail the whole request" is not implementable as written

* `get_price` returns `Decimal::ZERO` on every failure path
  (`currency_service.rs:267-300`) with no `Result`, so a delisted symbol silently
  produces market value 0 and a large negative unrealized figure — the class of
  plausible-wrong number the rule exists to prevent.
* `get_historical_prices` logs every failure to stderr and returns `Ok(...)` from the DB
  fallback, so a caller cannot tell "provider down" from "no data for this symbol".
* Routes erase errors with `.map_err(|_| Status::InternalServerError)`
  (`api_routes/analytics.rs:83, 216`). No existing route carries a message; the only
  precedent for a machine-readable error body is the auth `Unavailable` responder
  (`auth/routes.rs:36-52`).
* `normalizeTaxSummary` (`frontend/src/utils/decimal.ts:59-84`) coerces any missing
  field to `0`.

Two corrections to the rule itself: a missing FX rate is a currency pair, not a symbol,
so "naming the symbols" is the wrong noun for that half; and failing on the first missing
asset produces whack-a-mole, so failures must be collected and reported together.

### Precision is already lost in the browser

The backend serializes `Decimal` as strings, but `normalizeTaxSummary` and
`normalizeSellPnl` run every field through `Number()`. A CSV built from a normalized
payload is double-precision, which is the failure the prior record rejected rounding to
avoid — see *Decimal scale* in
[`docs/superpowers/reviews/2026-11-05-sell-pnl-breakdown.md`](../reviews/2026-11-05-sell-pnl-breakdown.md).
Report money must stay a string end to end and bypass `normalize*`, or the claim is
dropped.

### Outbound cost is per transaction date, not per asset

`get_rate`'s cache key is `(from, to, date)` and `replay` calls it per transaction, so
the FX work set is distinct (pair, transaction-date) tuples: a 20-asset multi-currency
portfolio with 10 transactions each is up to 200 outbound calls in one request. All
outbound HTTP passes one process-wide mutex with a 200 ms floor
(`currency_service.rs:15`), capping the process near 5 requests per second. The pod is
`replicas: 1` at 500m CPU / 256Mi (`k8s/app-deployment.yaml`) and no
`proxy-read-timeout` annotation exists in `k8s/`, so the ingress cuts off at its 60 s
default while the pod keeps working.

The fix is to shrink the work set, not to add concurrency: realized needs no quote, the
unrealized half needs exactly one close per asset, and FX needs one range request per
currency pair looked up locally. `join_all` would not help — the shared throttle
serializes it anyway.

## Prerequisites

Each is a live defect outside this feature's scope and each lands as its own reviewed
change.

### Dated FX

Three defects, all verified:

1. **Wrong symbology, so it cannot succeed.** `currency_service.rs:235` builds the
   Yahoo symbol as `format!("{}={}", from, to)`. Checked against the live endpoint:
   `EUR=USD` returns `"No data found, symbol may be delisted"`; `EURUSD=X` returns data.
   Any asset whose currency differs from the portfolio base currency therefore fails the
   request at `tax_engine.rs:403` via `analytics.rs:86`. The prior record already lists
   the unexercised FX path as an open item.
2. **The date argument is ignored.** The date goes in the cache key (`:221-222`) and the
   last close in the series is returned (`.rev().find_map`), so the stored value looks
   correctly dated and is not.
3. **It cannot be fixed by choosing a different element.** The URL at `:236` carries no
   `period1`/`period2`, so Yahoo returns its default short range and a past date is not
   in the response. The fix is a range fetch.

Fixing this **changes numbers on existing endpoints**, including figures pinned by
`test_sell_pnl_sums_to_tax_summary_realized_pnl` and `backend_rust/tests/sell_pnl_test.rs`,
and makes the sentence in `SellPnlModal.tsx:283` ("at the latest available rate") false.
It needs its own release note saying which figures move.

### `historical_prices` has no unique constraint

`lib.rs:170-178` creates the table with no primary key and a plain `CREATE INDEX`, not
`CREATE UNIQUE INDEX`. `get_historical_prices` writes with `INSERT OR REPLACE`
(`currency_service.rs:486-500`), which only replaces on a constraint violation — with
none, it always inserts. Every call appends roughly 250 more rows per symbol, and the
read path has no `DISTINCT`, no `ORDER BY` and no tie-break, so "the close on date X" is
an arbitrary row among N. `get_price` already reads that table with `fetch_optional`.
Needs `UNIQUE(symbol, date)`, a one-off dedupe, and a real upsert.

### A report endpoint must not write

`sync_splits_for_asset` inserts `SPLIT` transactions during a read, throttled once per
symbol per 30 days, so the first call after the window returns different numbers from
the second. Its dedupe is a `SELECT COUNT(*)` then `INSERT` with no transaction and no
unique constraint, so two concurrent requests can both insert the same split and double
share quantity. SQLite has no WAL or `busy_timeout` configured (`main.rs:41`), so
concurrent writers surface `SQLITE_BUSY`.

`get_historical_prices` also writes the fetched range during a read, so the rule for
reports is **"no `SPLIT` inserts"**, not "no writes".

## Goals vs. Non-Goals

* **In-Scope Goals:** prefix replay and a close at or before `end`; a public dated replay
  returning open lots and per-sell breakdowns; one JSON endpoint per portfolio taking
  `from`, `to`, `strategy`, `threshold_days`; a frontend CSV with `SOLD_LOT` /
  `OPEN_LOT` / `UNMATCHED` rows and a stated list of summable columns; a printable
  statement page; strategy and `threshold_days` recorded in every emitted file; explicit
  date semantics (inclusive day boundaries, one named time zone, future dates rejected);
  the three prerequisites above.
* **Out-of-Scope Non-Goals:** server-side PDF and any PDF dependency; computing tax owed
  or applying tax rates; dividends and income, which the app does not track;
  cross-portfolio reports; concurrency work, batching frameworks, cache layers or
  timeout knobs beyond the work-set shrink; a calendar component — a quarter is two date
  inputs and four preset buttons.

## Alternatives Weighed

* **Server-generated PDF.** Rejected for browser print. No PDF engine exists in the
  project; every realistic Rust option adds a large dependency tree or a second runtime
  to a 256Mi container, and font embedding and page-break layout are permanent
  maintenance the CSV builder does not have.
* **A new `/tax-report` endpoint.** Rejected in favour of extending
  `GET /api/portfolios/<id>/tax-summary` with two optional query parameters. The existing
  route already runs the whole per-asset loop, and a parallel endpoint is a second code
  path that can silently disagree with the screen — the opposite of the reconciliation
  work the previous feature invested in.
* **An `asset_ids` subset parameter on the tax report.** Rejected. A tax report is
  definitionally portfolio-wide, so subsetting it changes the tax answer; it also needs
  `require_asset_in` rather than a portfolio-level `require`, or it is an IDOR that the
  grep-based gate will not catch. A subset is kept for the statement only, as a
  client-side filter.
* **Two engine functions, one for window-realized and one for cumulative-realized.**
  Rejected: both are filters over the same vector of sell breakdowns.
* **Re-implementing lot matching in TypeScript for the statement.** Rejected: it would
  break the engine's single-source-of-truth invariant.

## Key Decisions

**Settled with the owner**

1. Owner-facing working file, not a filing document.
2. Date range required, defaulting to the current year, quarter presets available.
3. The statement's realized figure is cumulative from inception to `as_of`.
4. Backend returns JSON; the frontend transforms it.
5. One portfolio per report — stated as a scope limit, not a correctness proof, since two
   portfolios can share a base currency and then a combined total is legitimate.
6. Browser print-to-PDF, no server PDF.
7. Full precision on the CSV, carried as strings end to end; the print view may round for
   display.

**Changed by review**

8. **Settled.** The seed's ban on any nearby-day fallback fails the motivating case:
   31 December 2023 and 30 June 2024 were Sundays, 31 March 2024 was Good Friday. The
   rule is now: use the last close **on or before** the requested date — the closest
   preceding trading day — and write the date actually used in its own column on every
   row, plus a `price_source` flag. Fail only when no close exists at or before the
   requested date, or when the provider is unreachable. A *silent* substitution stays
   banned: the disclosure is what makes a plausible-but-wrong figure impossible to hide.
   The same rule applies to FX, which has no quote on weekends or market holidays.
9. **Fail-collect, not fail-fast.** Resolve every asset, then return one error enumerating
   every missing price and every missing currency pair.
10. **Realized-only requests must not be blocked by a missing quote**, since realized P&L
    needs no market price.
11. **HYBRID `threshold_days` is measured from each sell.** Already the code's behaviour
    (`tax_engine.rs:452`) and the only defensible tax semantics — a relative-to-today
    rule would let an already-filed quarter reclassify itself. Recorded as a non-goal:
    do not re-anchor the threshold to the as-of date. Open-lot rows carry no term field;
    do not invent one.

## Staging

The cut line is the realized/unrealized seam, not the two reports. Everything left of it
is network-free and hermetically testable, and is most of the stated tax need.

| Slice | Ships | Network? | Risk |
| :-- | :-- | :-- | :-- |
| 0 | The three prerequisites, each separately reviewed, plus the backend FX test the prior record lists as an open item | no | low |
| 1 | Statement at `as_of = today`, pure frontend over the payload the browser already holds | no | zero backend |
| 2 | Engine: public dated replay returning `(open_lots, breakdowns)`, unit tests only | no | contained |
| 3 | Tax report CSV, **sold-lot rows only** — the quarterly realized figure | no | moderate |
| 4 | Open-lot rows: close at-or-before `end`, FX at `end`, and the pricing-failure contract | **yes** | **the risky piece** |
| 5 | Statement switches to the report payload and accepts a past `as_of` | yes | low |

Route and payload changes also require, or CI fails: `PROTECTED_ROUTES` plus the three
counts in `tests/route_coverage_test.rs:140, 262, 276`; the tax-route special case in
`tests/auth_test.rs:41-49`; the path list in `tests/isolation_test.rs:190-197`, which is
the real isolation gate because the gate script's Rule B cannot see
`WHERE portfolio_id = ?`; `src/openapi.rs` and the `src/lib.rs` mounts; and OpenAPI
regeneration per `AGENTS.md`.

## Test Invariants

* **I1** — for any `end` at or after every sell in the window, the sum of `lot_pnl` over
  an asset's sold-lot rows equals that asset's `realized_pnl` from `tax-summary`, and the
  sum over assets equals `total_realized_pnl`. Exact `Decimal` equality, never
  serialized-string comparison. Holds because realized P&L is converted at the
  transaction-date rate and never touches a current price.
* **I2** — the window sum equals the sum of `/api/transactions/<id>/pnl` over the sells in
  the window: the existing `test_sell_pnl_sums_to_tax_summary_realized_pnl` lifted from
  two sells to a set.
* **I3** — open-lot `latent_gain_loss` sums to the asset's `unrealized_pnl` at the same
  as-of date, asserted against a **seeded** `historical_prices` row, never a live call.
* **I4** — matched plus unmatched quantity equals total quantity sold in the window.
* **I5** — no column may be summed across both `row_type` values; state it in the file
  header the way `sellPnlCsv.ts` does.
* Engine unit tests must cover a sell after `end` and a **split after `end`**: the existing
  reconciliation test is realized-only and stays green while open lots are wrong.
* New backend tests use the hermetic seeding pattern (seed `historical_prices` for every
  date, pre-seed `split_sync`), per the prior record's measured cost of not doing so.
* The print view has no server artifact, so "done" is DOM structure plus print CSS plus
  one browser smoke: chrome hidden, no `button` or `input` in the printed subtree,
  headline figures equal to the payload totals, and one playwright pass under
  `emulateMedia` for print. There is no checksum and no byte-for-byte reproducibility
  claim.

## Still Open

* Exact response schema, including whether the payload is row-level only or also carries
  precomputed totals. The review's position: pick one owner for every number, because
  aggregates beside raw rows is how two renderers drift.
* Error contract shape and status code for a collected pricing failure.
* Which access level may call a report — `Viewer` is assumed, unstated, and the gate
  script will not catch the choice.
* Date parameter semantics: inclusive boundaries, format, time zone, `start > end`,
  future `end`, and `start` before the first transaction.
* Whether "closest preceding trading day" needs a trading calendar, or whether "the
  newest row at or before the date, within a stated maximum lookback" is enough. The
  second needs no new dependency; the first is only correct if the lookback is bounded,
  or a delisted symbol silently prices off a close from years ago. A maximum lookback
  of a few days, failing beyond it, is the cheaper rule and is what this brief assumes.
