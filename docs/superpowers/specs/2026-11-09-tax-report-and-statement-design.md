# Portfolio Tax Report and Portfolio Statement

*   **Author:** javierarrieta, with EGM brainstorm and draft support
*   **Status:** Proposed
*   **Date:** 2026-11-09
*   **Predecessor:** `2026-11-09-tax-report-and-statement-concepts.md`

## Overview

Two reports are added so the owner can produce capital-gains figures for a period they
choose, rather than only for the current moment.

The **tax report** is a CSV the owner loads into a spreadsheet and re-crunches. It lists
every sell dated inside a chosen `[start, end]` window at lot granularity, with the buy
lots each sell consumed, plus every lot still open at `end` with its unrealized gain or
loss at that date.

The **portfolio statement** is a page the owner reads or hands to someone. It shows
realized P&L cumulative from inception to a chosen date and unrealized P&L as of that
date, for one portfolio, optionally narrowed to a subset of its assets. The browser
prints it to PDF; the server never produces a PDF.

They are not two features. They are one dated-calculation change plus two renderers, and
one renderer needs no backend change at all until it is asked about the past.

## The Problem

Capital gains tax in the owner's country is settled quarterly. Every quarter the owner
must answer: what did I sell in this window, at what cost basis, with what gain — and
what did I still hold at the window's end, and what was it worth.

Today the app cannot answer either question for a past date. `GET
/api/portfolios/<id>/tax-summary` computes realized P&L over *all* transactions and
prices everything at the latest available close. There is no date parameter anywhere. The
numbers exist on screen for "now" and cannot be asked about "31 December".

Getting numbers out is worse. The only export in the app covers **one sell at a time**
(`frontend/src/utils/sellPnlCsv.ts`, reached from `SellPnlModal.tsx`). Producing a
quarterly report means opening the breakdown for every sell in the quarter and
assembling them by hand.

Three defects found during the brainstorm make the situation worse than it looks, and
they are outside this feature's scope but block it:

1.  **Foreign-currency assets cannot be priced at all.** `CurrencyService::get_rate`
    builds the Yahoo symbol as `EUR=USD`; the live endpoint returns "No data found" for
    that form and data for `EURUSD=X`. Any asset whose currency differs from the
    portfolio's base currency fails the request.
2.  **`get_rate` ignores its date argument.** It keys its cache on the requested date and
    then returns the latest close in the series, so the stored value looks correctly dated
    and is not. `tax_engine.rs:403` already calls it per transaction with past dates, so
    existing multi-currency figures are already converted at today's rate.
3.  **`historical_prices` has no unique constraint.** `INSERT OR REPLACE` therefore always
    inserts, so every fetch appends duplicate rows and "the close on date X" becomes an
    arbitrary row among N.

A dated report built on top of those three would produce numbers that are wrong in a way
nobody would notice, which is the specific outcome this design exists to prevent.

## Goals

*   **Goal 1 — windowed realized P&L.** For any `[start, end]`, the sum of per-lot P&L
    over the sold-lot rows of an asset equals that asset's `realized_pnl` from
    `tax-summary` under the same strategy and threshold, compared as exact decimals, and
    the sum over assets equals `total_realized_pnl`. This holds for FIFO, LIFO and HYBRID.
*   **Goal 2 — unrealized as of a date.** For any `end`, the sum of per-lot unrealized
    gain over the open-lot rows equals that asset's `unrealized_pnl` computed at the same
    as-of date and price source.
*   **Goal 3 — the quarterly workflow is a few clicks.** Quarter presets fill `start` and
    `end`; the default is the current year. No calendar component, no manual date typing
    for the four common cases.
*   **Goal 4 — reproducibility.** Every emitted file records the strategy, the threshold,
    the portfolio base currency, the requested date range, and the date each price was
    actually taken from. Same inputs, same numbers, regardless of when the report is
    regenerated.
*   **Goal 5 — no silent wrong numbers.** A price or FX rate that cannot be obtained fails
    the request and enumerates every missing symbol and every missing currency pair in one
    response. Where a date falls on a non-trading day, the file states the date actually
    used. No column is silently substituted, and the file states which columns are legal
    to sum.
*   **Goal 6 — a printable statement.** The printed page contains no navigation, no
    buttons and no dead controls, and its two headline figures are the payload totals
    rendered through the shared formatter, not arithmetic invented by the print view.
*   **Goal 7 — existing figures move only where declared.** The only change to numbers on
    existing endpoints is the dated-FX fix, which lands as its own reviewed change with a
    note naming which figures move.

## Non-Goals

*   **No server-side PDF, and no PDF dependency.** Browser print-to-PDF only. Every
    realistic Rust PDF option adds a large dependency tree or a second runtime to a 256Mi
    container, and font embedding and page-break layout are permanent maintenance.
*   **No tax computation.** The reports emit realized and unrealized figures and the
    short-term/long-term bucket the engine already produces for HYBRID. They do not apply
    tax rates, thresholds or allowances, and they do not know the owner's jurisdiction.
*   **No dividends or income.** The app does not record them.
*   **No cross-portfolio report.** One portfolio per report, as a scope limit rather than
    a correctness proof — two portfolios can share a base currency, in which case a
    combined total would be legitimate, and that is still out of scope.
*   **No pricing-provider failover or caching layer.** No batching frameworks, no
    concurrent fan-out, no new timeout knobs. The outbound work set is shrunk instead.
*   **No re-anchoring of HYBRID `threshold_days` to the as-of date.** It is measured from
    each sell, as the engine already does, so an already-filed quarter cannot reclassify
    itself when regenerated.
*   **No term classification on open-lot rows.** The engine has no such field and one is
    not invented here.

## Technical Plan (Architecture)

The design has four layers, and the seams between them are the point: the engine computes
lot state at a date, the pricing layer resolves a value for a date, the route assembles a
typed payload whose totals are owned by the server, and the frontend renders that payload
two ways without doing arithmetic of its own.

```text
                    frontend/src/components/ReportControls.tsx
                    (date range, strategy, quarter presets)
                         |
            +------------+-------------+
            |                          |
   CSV builder (taxReportCsv.ts)   Statement print view
   strings verbatim + BOM          shared formatter, display rounding
   + downloadCsv                   no summation, no controls
            |                          |
            +------------+-------------+
                         |  reads one payload, derives nothing
     GET /api/portfolios/<id>/tax-summary
              ?strategy&threshold_days[&from&to]
                    |  Access::Viewer, dated or not
        route: filter transactions to date <= end
                    |
        TaxLotEngine::replay_at(end)  -> (open_lots, sell_breakdowns)
                    |                          |
        pricing layer: close on/before end   realized uses recorded
        + FX on/before end, per pair range   tx prices, no quote
                    |
        Yahoo chart (period1/period2) + historical_prices
```

### Prerequisites, landed first and separately

Three defects block this design and each is its own reviewed change.

**P1 — dated FX.** `CurrencyService::get_rate` must request a date range and select the
rate for the requested date, using Yahoo's FX symbology (`EURUSD=X`, not `EUR=USD`). The
current implementation ignores its date argument and returns the latest close while
caching it under the requested date's key. Because `tax_engine.rs:403` already calls it
per transaction with past dates, fixing it changes existing realized and unrealized
figures for every foreign-currency asset, including figures asserted by
`test_sell_pnl_sums_to_tax_summary_realized_pnl` and `backend_rust/tests/sell_pnl_test.rs`.
That is the intended outcome and the change must name it. It also makes the sentence in
`SellPnlModal.tsx:283` ("at the latest available rate") false.

**P2 — `historical_prices` uniqueness.** Add `UNIQUE(symbol, date)`, dedupe existing
rows, and make the write a real upsert. Today the table has no primary key and only a
plain index, so `INSERT OR REPLACE` always inserts and reads return an arbitrary duplicate.

**P3 — no split writes on a read path.** `sync_splits_for_asset` must not run inside a
report request. Its dedupe is a `SELECT COUNT(*)` then `INSERT` with no transaction and no
unique constraint, so concurrent requests can both insert the same split and double share
quantity, and a split dated after `end` that gets inserted retroactively changes in-window
cost basis. Moving it off the read path is a prerequisite; a report that detects an
unrecorded split inside the window names it rather than fixing it.

### The engine change: one public dated replay

`calculate_lots` is not parameterised. Three separate changes are required, and doing the
obvious thing — adding an `as_of` argument — produces three bugs.

1.  A new public entry point, `replay_at(transactions, end, ...)`, returning **both** the
    open lots and the per-sell breakdowns from a single pass. Today `replay` is private
    and `calculate_lots` discards the breakdowns after summing them, so per-sell rows are
    only reachable through `sell_breakdown`, which replays the entire asset history per
    sell and re-enters the FX lookup per transaction.
2.  The valuation date becomes an explicit parameter. Today the FX rate for market value
    is taken at `latest_date`, derived from the last transaction in the list, so
    truncating the list silently moves the FX date to the last transaction before `end`
    — the nearby-day substitution the pricing rule forbids.
3.  The `date <= end` filter runs **before** sorting and matching, because the split branch
    rescales every lot in scope.

Prefix replay itself needs no engine change: the engine already documents that a sell's
matches depend only on transactions at or before it. Both report shapes are then filters
over one result — cumulative realized is the sum of all breakdowns, window realized is
the sum of breakdowns whose sell date falls in `[start, end]`. There is one engine
function and two selections.

### Pricing resolution

Realized P&L needs no market quote; it is computed from recorded transaction prices.
Only the unrealized half needs a close, and it needs exactly one close per asset: the
last close **on or before** the requested date, within a bounded lookback. A bounded
lookback is what stops a symbol delisted years ago from pricing off a stale close it is
technically entitled to.

The same on-or-before rule applies to FX, which has no quote on weekends or market
holidays. FX is fetched **once per currency pair as a range** and looked up locally, not
once per transaction date, which is what keeps a 20-asset multi-currency portfolio from
issuing hundreds of requests through a process-wide throttle that caps the whole
process near five requests per second.

Failures are collected, not raised on the first one: the request resolves every asset,
then returns one error enumerating every missing symbol and every missing currency pair.
A missing FX rate is a pair, not a symbol, and the response says which it is.

### The payload

The backend owns every number. The response is a typed `ToSchema` struct — not
`serde_json::Value`, which is what `tax-summary` declares today and which leaves the
generated OpenAPI schemaless — with every money field serialized as a decimal string.

It carries three groups:

*   **Run context:** portfolio id and name, base currency, strategy, `threshold_days`,
    the requested `[start, end]`, and the pricing date actually used per asset.
*   **Totals:** per-asset realized-in-window, realized-cumulative, unrealized-at-`end`,
    market value and cost basis, plus portfolio-level totals of each. Computed in Rust
    from the same replay that produced the rows.
*   **Rows:** `SOLD_LOT`, `OPEN_LOT` and `UNMATCHED`, each carrying only lot-level values.

Renderers are forbidden from summing. The CSV writes the server-computed totals verbatim
and states in its header which columns are legal to sum. This is the decision that keeps
Goal 1 satisfiable: a total derived in TypeScript is a double and cannot reproduce exact
decimal equality against `tax-summary`.

### The renderers

The CSV builder follows the existing single-sell export's conventions — UTF-8 BOM, the
formula-lead guard on free-text cells, exponent-free numeric output — with one
deliberate difference: report money fields travel as strings end to end and never pass
through `normalizeTaxSummary` or `normalizeSellPnl`, which coerce every field through
`Number()` and would reduce the file to double precision.

The statement is a React view with a print stylesheet. There is no `@media print` rule
anywhere in the frontend today, so this is net-new. It renders the payload's totals
through the shared formatter, and its printed subtree contains no buttons or inputs.

### Sequencing

The cut line is the realized/unrealized seam, not the two reports. Everything left of it
touches no network.

| Slice | Ships | Network? | Risk |
| :-- | :-- | :-- | :-- |
| 0 | P1, P2, P3, each separately reviewed | no | low |
| 1 | Statement at `as_of = today`, pure frontend over the existing tax-summary payload | no | zero backend |
| 2 | `replay_at` in the engine, unit tests only | no | contained |
| 3 | Tax report CSV, sold-lot rows only — the quarterly realized figure | no | moderate |
| 4 | Open-lot rows, the pricing rule, and the failure contract | **yes** | **the risky piece** |
| 5 | Statement switches to the report payload and accepts a past `as_of` | yes | low |

### Decided during drafting

Four items were open when the Technical Plan was first written. They are settled here, and
the predecessor RFC is the source for the first one.

**Route.** Reports extend `GET /api/portfolios/<id>/tax-summary` with two optional query
parameters rather than adding a route. Absent `from` and `to`, the endpoint behaves exactly
as it does today. This is the RFC's decision; an earlier revision of this draft drew a
separate `/tax-report` route, which contradicted the rejection recorded as Alternative B.

**Access level.** Every call to `tax-summary` requires `Access::Viewer`, dated or not. There
is no parameter-dependent access rule. An earlier revision of this draft required `Editor`
for dated calls, and that was dropped on review for two reasons. It does not bound cost —
an `Editor` can issue the same expensive request as often as a `Viewer` can — so it
restricted who gets to be slow rather than how slow the app gets. And it locked the
read-only collaborator, which is the accountant or tax adviser case that motivates sharing
a portfolio at all, out of the past-dated statement they exist to receive. Cost is bounded
by the guard below instead, where the cost actually is.

**Cost guard.** Two bounds, both on the dated path only, so the existing screen is
untouched. One report may be in flight per user at a time; a second concurrent request is
rejected with HTTP 429 and `retryable: true`. And each report carries an outbound-request
budget; exceeding it fails with the 503 contract rather than continuing to grind through
the process-wide throttle. The budget is the same cap the pricing section needs to keep a
20-asset multi-currency portfolio from issuing hundreds of calls, so it is not extra
machinery for its own sake. The in-flight slot must be released on every exit path,
including a panic, or one slow report permanently blocks that user.

**Error contract.** Two failure classes, one typed body, produced by a custom Rocket
`Responder` in the style of the auth `Unavailable` responder: HTTP 422 for "no data exists
at or before this date for this symbol or currency pair", HTTP 503 for "provider
unreachable or throttled", with a `retryable` flag that agrees with the status. Guard
rejections use HTTP 429 with `retryable: true`. No partial
file is produced. The missing-item list is capped, with a count of what was omitted.

This choice carries a fourth prerequisite. The pricing layer cannot currently distinguish
"no data" from "could not ask": `get_price` returns `Decimal::ZERO` on every failure and
`get_historical_prices` logs to stderr and returns `Ok(...)` from the database fallback.
Both must return a real result type, and that change also affects the existing screen.

**Date parameter semantics.** `from` and `to` are `YYYY-MM-DD`, both inclusive, and the day
boundary is evaluated in UTC — the same zone transaction timestamps are stored in, so no
conversion is involved. A sell at any time on `to` is inside the window. Absent parameters
mean today's behaviour. `from > to`, a `to` in the future, and an unparseable date are all
HTTP 422. `strategy` is validated strictly: absent means FIFO, present-but-unrecognized is a
422 naming the accepted values, rather than today's silent fallback to FIFO. The single-sell
P&L route already normalizes strictly, so the app currently runs two rules on one axis.
`threshold_days` must be non-negative; a negative HYBRID threshold classifies every holding
as long-term and is not a meaningful input. A typo'd strategy in a tax file is a defect, not
a default.

### Reconciliation with the predecessor

The draft was compared against the RFC after drafting. One contradiction and six additions
were found; all are resolved.

The RFC's non-goals excluded *"concurrency work, batching frameworks, cache layers or
timeout knobs beyond the work-set shrink."* The cost guard is beyond that shrink. The owner
kept the guard and amended the RFC's non-goal, narrowing it to batching frameworks,
concurrent fan-out and new timeout knobs, with the reason recorded in the RFC itself.

Six decisions the RFC did not raise were accepted: the P4 result types for the pricing
layer; the `TaxLot` fix; strict strategy and threshold validation; bounding and versioning
the FX cache; HTTP 429 in the error contract; and the `SOLD_LOT` row-kind name. Two of them
change existing endpoint behaviour and were decided individually rather than folded in.
`TaxLot` is corrected in place rather than worked around, because the Holdings table already
renders "Remaining Qty" and "Original Qty" as two columns fed from one value, so one of
those columns is meaningless today and the fix makes it correct. Strategy validation is
strict on both paths rather than only on the dated one, because one endpoint with two input
contracts is the same shape as the parameter-dependent access rule this draft already
rejected once.

## Alternatives Considered

### Alternative A: server-generated PDF for the statement

*   **Pros**: a byte-reproducible artifact; fetchable by script or `curl`; identical output
    regardless of the reader's browser and fonts; archivable.
*   **Cons**: no PDF engine exists in the project. The realistic Rust options are `typst`
    (pure Rust, large dependency tree, needs a font file), `genpdf` or `print` (system
    libraries in the image), or HTML through headless Chromium (a second runtime in a
    container that currently has none). Font embedding for non-ASCII symbols and currency
    names, page breaks, and a layout that cannot be unit-tested the way the CSV builder is.
*   **Reason for rejection**: the statement is read or emailed by the owner, not filed or
    signed. Browser print-to-PDF reuses the existing formatters so the numbers look like
    the rest of the app, and costs no dependency. The cost of the alternative is permanent
    maintenance in a 256Mi single-replica container for a property nobody asked for.

### Alternative B: a new `/tax-report` endpoint

*   **Pros**: a clean, purpose-shaped contract; no risk of changing behaviour on the
    existing screen; free to evolve without touching `tax-summary`.
*   **Cons**: duplicates the per-asset loop the existing route already runs — access check,
    asset list, transactions, pricing, engine call, totals. A second code path can silently
    disagree with the screen the owner is reconciling against. Also breaks
    `tests/route_coverage_test.rs`, which hard-asserts the exact size of the `/api` surface.
*   **Reason for rejection**: the previous feature invested deliberately in the opposite
    property — server-derived totals, a reconciliation test that holds under all three
    strategies, and `split_applied` moved server-side precisely so a stale browser copy is
    not the source of truth. A parallel endpoint reintroduces exactly the drift that work
    removed, for the sake of two optional query parameters.

### Alternative C: rows-only payload, totals derived by the renderers

*   **Pros**: smallest payload; the backend cannot be wrong about a total it never sends;
    no possibility of totals and rows disagreeing.
*   **Cons**: every total would be summed in TypeScript, where the numbers are doubles, so
    exact decimal equality against `tax-summary` is unreachable. The summable-columns rule
    would have to be re-implemented in both renderers.
*   **Reason for rejection**: it fails Goal 1 by construction and creates the two-renderer
    drift the design exists to avoid. Chosen design puts every number on the server side.

### Alternative D: payload carries totals plus machine-readable column metadata

*   **Pros**: the "do not sum these columns" rule that `sellPnlCsv.ts` currently documents
    in a comment becomes machine-checked; a third renderer inherits the rule for free.
*   **Cons**: the metadata is a second contract that must be kept in sync with the rows, and
    a wrong metadata entry is worse than none because it is authoritative.
*   **Reason for rejection**: deferred rather than rejected on merit. There is one renderer
    family today and the rule can be enforced by test; the metadata earns its place if a
    third consumer appears.

### Alternative E: two engine functions, one for window-realized and one for cumulative-realized

*   **Pros**: each report gets a function that returns exactly what it needs; no filter
    logic in the route; the two figures cannot be confused for each other.
*   **Cons**: both functions would run the same matching loop over the same transactions
    and differ only in which breakdowns they sum. Two entry points means two places to keep
    correct when the matching rules change, and a divergence between them is a tax figure
    that disagrees with itself.
*   **Reason for rejection**: a sell's matches depend only on transactions at or before it,
    so one prefix replay yields every breakdown, and cumulative realized is the sum of all
    of them while window realized is the sum of those whose sell date falls in the window.
    Both figures are filters over one vector, so a second function buys duplication and no
    clarity. The design states the reconciliation rule instead: when `start` is inception
    and `end` is the statement date, the two must agree, and that equality is a test.

### Alternative F: re-implement lot matching in TypeScript for the statement

*   **Pros**: the statement could compute a past as-of date with no backend change at all,
    since the browser already holds the raw transaction ledger.
*   **Cons**: a second implementation of FIFO, LIFO and HYBRID in a second language, with
    split adjustment and FX conversion also duplicated.
*   **Reason for rejection**: it breaks the engine's single-source-of-truth invariant,
    which the Rust code states outright — realized P&L is derived from the returned
    breakdowns by the caller and never accumulated on a separate path. Two implementations
    of tax lot matching that can disagree is the worst available outcome for a tax feature.

### Alternative G: an `asset_ids` subset parameter on the tax report

*   **Pros**: gives the owner an escape hatch when one unpriceable symbol blocks the whole
    report, which the fail-collect rule otherwise makes permanent.
*   **Cons**: a tax report is definitionally portfolio-wide, so narrowing it changes the tax
    answer while still presenting as complete. It also needs `require_asset_in` rather than
    a portfolio-level `require`, or it is an IDOR that the grep-based access gate will not
    catch.
*   **Reason for rejection**: the subset solves the statement's problem and not the tax
    report's. A subset is kept for the statement only, as a client-side filter over data
    already fetched. The pricing failure it was meant to soften is handled by Goal 5
    instead.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Sequenced by the slices in the Technical Plan. Slice numbers are given per file; a file
appearing in more than one slice is touched more than once.

### Target Files to Create/Modify

#### 1. `backend_rust/src/services/currency_service.rs` (Modify) — slices 0, 4

*   **Type / Scope**: pricing and FX service, outbound HTTP.
*   **Proposed Changes**:
    *   **P1, dated FX.** Rebuild `get_rate` to request a date range with `period1` and
        `period2` and select the rate on or before the requested date within a bounded
        lookback. Correct the symbol form from `EUR=USD` to `EURUSD=X`. Today the URL
        carries no range, so a past date is not in the response and no selector change can
        fix it. This moves existing realized and unrealized figures for every
        foreign-currency asset and must be landed and released on its own.
    *   **P4, honest failure types.** Change `get_price` and `get_historical_prices` to
        return a result distinguishing `NoData` from `ProviderUnavailable` instead of
        `Decimal::ZERO` and a swallowed `Ok`. The error contract depends on this
        distinction, and the existing screen inherits the honesty as a side effect.
    *   **Work-set shrink.** Add a per-asset single-close lookup (narrow window around the
        requested date) and a per-currency-pair range fetch with local date lookup, so a
        dated report issues one close per asset and one FX range per pair rather than one
        call per transaction date.
    *   **Cache hygiene.** The in-memory FX cache is keyed by date and never evicted; once
        past dates become reachable it grows without bound in a 256Mi pod. Bound it, and
        version the key so entries written by the current buggy behaviour cannot be read as
        correct after the fix.

#### 2. `backend_rust/src/lib.rs` (Modify) — slice 0

*   **Type / Scope**: schema creation and migrations.
*   **Proposed Changes**: **P2.** Add `UNIQUE(symbol, date)` to `historical_prices`, with a
    one-off dedupe of existing rows before the constraint is created. Without it, `INSERT OR
    REPLACE` always inserts and reads return an arbitrary duplicate, which makes any
    as-of price non-reproducible. Also register the report guard from file 3 in managed
    state alongside the existing `CurrencyService`.

#### 3. `backend_rust/src/report_guard.rs` (Create) — slice 4

*   **Type / Scope**: per-user concurrency slot and per-request outbound budget.
*   **Proposed Changes**: a small stateful type held in Rocket managed state. Acquire returns
    a guard that releases the user's slot on drop, so a panic or an early return cannot leak
    the slot; the budget is a counter decremented by the pricing layer before each outbound
    call, and exhausting it produces the `ProviderUnavailable` variant. Keep it in its own
    module rather than in `currency_service.rs` so the two concerns stay testable separately.

#### 4. `backend_rust/src/engines/tax_engine.rs` (Modify) — slices 2, 4

*   **Type / Scope**: the tax lot engine.
*   **Proposed Changes**:
    *   Add a public `replay_at(transactions, end, valuation_date, ...)` returning both open
    lots and per-sell breakdowns from one pass. `replay` is private today and
        `calculate_lots` discards breakdowns after summing them.
    *   Make the valuation date an explicit parameter. Today the FX rate for market value is
        taken at `latest_date`, derived from the last transaction in the list, so truncating
        the list silently moves the FX date.
    *   Filter to `date <= end` before sorting and matching, because the split branch
        rescales every lot in scope.
    *   Fix `TaxLot`: `original_qty` and `remaining_qty` are both set to the already-
        decremented `lot.qty`, so the originally-bought quantity is unrecoverable, and the
        struct carries no buy transaction id. Both are needed for a sold-lot and open-lot row
        to be traceable to a transaction.
    *   Unit tests: prefix replay equals full replay for realized P&L under FIFO, LIFO and
        HYBRID; a sell after `end` is excluded; a **split after `end`** does not move
        in-window cost basis; HYBRID matches are unchanged, pinning that `threshold_days`
        stays measured from each sell.

#### 5. `backend_rust/src/schemas.rs` (Modify) — slice 4

*   **Type / Scope**: request and response types.
*   **Proposed Changes**: typed report payload, every money field serialized as a decimal
    string via the existing `decimal_json` helper. Shape:

```text
ReportContext   portfolio_id, portfolio_name, base_currency, strategy, threshold_days,
                requested_from, requested_to
AssetTotals     asset_id, symbol, realized_in_window, realized_cumulative,
                unrealized_at_end, market_value, cost_basis,
                price_date_used, fx_rate_date_used, fx_applied, split_applied
ReportRow       row_type: SOLD_LOT | OPEN_LOT | UNMATCHED
                SOLD_LOT: sell + lot fields, mirroring LotMatch
                OPEN_LOT: buy_date, buy_transaction_id, original_qty, remaining_qty,
                          unit_cost, price_date, market_value, latent_gain_loss, latent_roi
                UNMATCHED: sell id, unmatched_qty, money cells null
ReportResponse  context, asset_totals: [AssetTotals], totals: portfolio rollup,
                rows: [ReportRow], strategy, threshold_days
ErrorBody       error, missing_prices: [symbol], missing_rates: [{from,to}],
                retryable, omitted_count
```

    `row_type` is an exhaustive enum both renderers must switch on. Nulls are nulls, not
    zeros, so a consumer can tell "not applicable" from "zero".

#### 6. `backend_rust/src/api_routes/analytics.rs` (Modify) — slices 3, 4

*   **Type / Scope**: the `tax-summary` handler.
*   **Proposed Changes**: add optional `from` and `to`. Access stays `Access::Viewer` for
    every call, dated or not. When the parameters are present: acquire the per-user report
    slot, returning 429 if one is already in flight; validate the dates and the strategy
    strictly; filter transactions to `date <= to`; call `replay_at`; build
    `AssetTotals` and rows server-side so no renderer sums anything; resolve prices with the
    collected-failure rule and return the typed error body with 422 or 503. Replace the
    `.map_err(|_| Status::InternalServerError)` erasure on this handler.

#### 7. `backend_rust/src/openapi.rs` (Modify) — slice 4

*   **Type / Scope**: OpenAPI registration.
*   **Proposed Changes**: document the new query parameters and replace the
    `body = serde_json::Value` declaration for this route with the typed response, so the
    generated schema is no longer empty and the frontend can generate real types.

#### 8. `backend_rust/tests/tax_report_test.rs` (Create) — slices 3, 4

*   **Type / Scope**: integration tests, hermetic.
*   **Proposed Changes**: invariants I1, I2 and I4 from the predecessor RFC, compared as
    exact decimals. Seed `historical_prices` for every date and pre-seed `split_sync` for
    every symbol, following the pattern the prior review record established after the
    sell-pnl suite went from 1.33 s to 5.67 s by reaching the network. Cover: window equals
    the sum of per-sell breakdowns; cumulative equals all-time `tax-summary`; a non-trading
    `to` resolves to the prior trading day and reports the date used; a missing price
    returns 422 naming every missing symbol; an unreachable provider returns 503 with
    `retryable: true`; `from > to`, a future `to`, and an unknown strategy each return 422.

#### 9. `backend_rust/tests/isolation_test.rs` (Modify) — slice 4

*   **Type / Scope**: the real cross-user authorization matrix.
*   **Proposed Changes**: add the dated variant of `tax-summary` to the path list, and add
    cases proving a `Viewer` collaborator can request a dated report and a non-collaborator
    cannot. Add guard cases: a second concurrent report for the same user returns 429 with
    `retryable: true`, and the slot is free again after the first request completes or
    panics. The grep-based gate cannot see an access-level choice on an existing route.

#### 10. `backend_rust/tests/route_coverage_test.rs` and `backend_rust/tests/auth_test.rs` (Modify) — slice 4

*   **Type / Scope**: route surface and unauthenticated sweep.
*   **Proposed Changes**: no route is added, so the three hard counts stay at 15 and 13.
    `auth_test.rs` special-cases the tax routes to append strategy parameters; extend that
    to the dated variant so the sweep still proves what its comment claims.

#### 11. `backend_rust/tests/sell_pnl_test.rs` (Modify) — slice 0

*   **Type / Scope**: existing reconciliation tests.
*   **Proposed Changes**: expectations that assume FX conversion at the latest rate are
    updated as part of P1, in the same commit as the fix, with the delta explained in the
    commit message.

#### 12. `docs/openapi/openapi.json` (Regenerate) — slice 4

*   **Type / Scope**: committed API contract, diffed by CI.
*   **Proposed Changes**: regenerate with the command in `AGENTS.md`.

#### 13. `frontend/src/types/api.d.ts` (Regenerate) — slice 4

*   **Type / Scope**: generated API types.
*   **Proposed Changes**: regenerate via `npm run generate-types` so the renderers type
    against the real schema instead of a hand-maintained approximation.

#### 14. `frontend/src/utils/taxReportCsv.ts` (Create) — slice 3

*   **Type / Scope**: CSV builder, pure, no DOM.
*   **Proposed Changes**: one row per `ReportRow`, sell context repeated on sold-lot rows,
    a header comment naming which columns are legal to sum, the run context (strategy,
    threshold, requested range, pricing dates used) as columns, and money written verbatim
    from the payload strings. Reuses the formula-lead guard and the exponent-free formatting
    rule from `sellPnlCsv.ts`. It must not call `normalizeTaxSummary` or `normalizeSellPnl`,
    which would reduce the file to double precision.

#### 15. `frontend/src/utils/taxReportCsv.test.ts` (Create) — slice 3

*   **Type / Scope**: unit tests.
*   **Proposed Changes**: mirror `sellPnlCsv.test.ts`: the `lot_pnl` column sums to the
    payload's realized total, no BOM in the builder output, formula characters escaped,
    exponent notation never emitted, nulls rendered as empty cells, and every row kind
    present.

#### 16. `frontend/src/utils/decimal.ts` (Modify) — slice 3

*   **Type / Scope**: numeric normalization.
*   **Proposed Changes**: add a string-preserving path for the report payload, or leave this
    file untouched and have the report builder bypass it. Either way the invariant is that
    report money never passes through `Number()`.

#### 17. `frontend/src/App.tsx` (Modify) — slices 1, 4, 5

*   **Type / Scope**: data fetching and top-level state.
*   **Proposed Changes**: hold the report payload separately from `taxSummary` so the
    normalized copy is never the source for an export; fetch the dated variant only when a
    range is chosen; surface the typed error body to the user naming missing symbols and
    pairs.

#### 18. `frontend/src/components/ReportControls.tsx` (Create) — slice 3

*   **Type / Scope**: date range selection.
*   **Proposed Changes**: two date inputs plus four quarter presets and a current-year
    preset. No calendar component. Controls stay enabled for every access level, since the
    route requires only `Viewer`; when the guard rejects with 429 the UI says a report is
    already running and offers a retry.

#### 19. `frontend/src/components/PortfolioStatement.tsx` (Create) — slices 1, 5

*   **Type / Scope**: printable statement view.
*   **Proposed Changes**: renders portfolio totals and per-asset figures through the shared
    formatter, with an asset subset filter applied client-side. Slice 1 renders the existing
    `taxSummary` payload at today's date; slice 5 switches it to the dated payload. It
    derives no totals of its own and contains no buttons or inputs inside the printed
    subtree.

#### 20. `frontend/src/styles/print.css` (Create) and its import (Modify) — slice 1

*   **Type / Scope**: print stylesheet. There is no `@media print` rule in the project
    today.
*   **Proposed Changes**: hide app chrome, set page margins and break rules, and force
    background-safe number rendering. Imported from the app entry so it ships in the main
    bundle.

#### 21. `frontend/src/test/components/PortfolioStatement.test.tsx` (Create) and `frontend/src/test/mocks/handlers.ts` (Modify) — slice 1

*   **Type / Scope**: component tests and API mocks.
*   **Proposed Changes**: assert the statement root carries the hook the print stylesheet
    targets, that the printed subtree contains no `button` or `input`, and that the two
    headline figures equal the payload totals rendered through the shared formatter. Add
    the report route to the mock server, respecting its existing ordering comment.

#### 22. `frontend/scripts/check_print_styles.sh` (Create) and `.github/workflows/` (Modify) — slice 1

*   **Type / Scope**: CI gate. CI runs `cargo test` and frontend checks and sees no CSS.
*   **Proposed Changes**: a grep-style script in the spirit of
    `backend_rust/scripts/check_access_gate.sh` asserting the print stylesheet exists and
    hides the chrome selectors, wired into the existing frontend job.

#### 23. `frontend/e2e/statement.smoke.mjs` (Create) and `frontend/e2e/run.sh` (Modify) — slice 1

*   **Type / Scope**: browser smoke, following the existing sell-pnl e2e pattern.
*   **Proposed Changes**: load the statement, emulate print media, assert chrome is
    `display: none`, that there is no horizontal overflow, and that `page.pdf()` returns a
    non-empty buffer within a stated page count. This is the only honest check that printing
    works; there is no server artifact to assert on.

#### 24. `frontend/src/components/SellPnlModal.tsx` (Modify) — slice 0

*   **Type / Scope**: existing single-sell export UI.
*   **Proposed Changes**: its footnote says figures are converted "at the latest available
    rate". That sentence becomes false when P1 lands and is corrected in the same change.

#### 25. `docs/superpowers/plans/2026-11-09-tax-report.md` (Create) — all slices

*   **Type / Scope**: implementation plan, following the repo's existing convention.
*   **Proposed Changes**: the slice table expanded into ordered tasks with the file list
    above, plus a *Deviations* section for any sanctioned exception, matching the shape of
    the prior feature's plan.

### Verification Plan

*   **Compilation/Build**: `cargo build --release` and `cargo build` from `backend_rust/`;
    `npm run build` from `frontend/`.
*   **Unit Tests**: `cargo test` from `backend_rust/`, which now includes
    `tax_report_test.rs` and the prefix-replay engine tests; `npm test` from `frontend/`.
*   **Static Analysis & Linting**: `cargo clippy --all-targets`;
    `bash scripts/check_access_gate.sh`; `npm run lint`; `npm run typecheck`.
*   **Access gate**: `tests/isolation_test.rs` must cover the dated variant, since the
    grep-based script cannot see an access-level choice on an existing route.
*   **Reconciliation**: invariants I1 through I5, asserted as exact decimals with no
    tolerance, under FIFO, LIFO and HYBRID.
*   **Print**: `bash scripts/check_print_styles.sh` and `npm run e2e:statement`.
*   **Manual, cannot be automated**: a real OIDC login and a real browser print preview,
    both listed in `docs/auth-testing.md` as the two things CI cannot check.
*   **Provider smoke**: `bash scripts/auth_smoke.sh` is unrelated; pricing changes are
    verified hermetically against seeded `historical_prices` rows, never against the live
    endpoint, so the suite has no network dependency.

--------------------------------------------------------------------------------

*Drafted with EGM*
