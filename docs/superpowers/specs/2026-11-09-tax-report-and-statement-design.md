# Portfolio Tax Report and Portfolio Statement

*   **Author:** javierarrieta, with EGM brainstorm and draft support
*   **Status:** Proposed
*   **Date:** 2026-11-09
*   **Predecessor:** `2026-11-09-tax-report-and-statement-concepts.md`
*   **Blocked on:** `2026-11-09-pricing-layer-truthfulness.md` — dated FX,
    `historical_prices` uniqueness and the pricing result types. This document does not
    edit `currency_service.rs` or `lib.rs`.

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
one renderer needs no backend change until it is given a past date.

## The Problem

Capital gains tax in the owner's country is settled quarterly. Every quarter the owner
must answer: what did I sell in this window, at what cost basis, with what gain — and
what did I still hold at the window's end, and what was it worth.

Today the app cannot answer either question for a past date. `GET
/api/portfolios/<id>/tax-summary` computes realized P&L over *all* transactions and
prices everything at the latest available close. There is no date parameter anywhere. The
numbers exist on screen for "now" and cannot be asked about "31 December".

The only export in the app covers **one sell at a time**
(`frontend/src/utils/sellPnlCsv.ts`, reached from `SellPnlModal.tsx`). Producing a
quarterly report means opening the breakdown for every sell in the quarter and
assembling them by hand.

Three defects block this design. Each is a separate reviewed change that lands before any
report code, and each is enumerated under Prerequisites:

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

A dated report built on top of those three changes figures without changing any visible
signal.

## Goals

*   **Goal 1 — windowed realized P&L.** For any `[start, end]`, the sum of per-lot P&L
    over the sold-lot rows of an asset equals that asset's `realized_in_window`, compared
    as exact decimals at the stored scale, and the sum over assets equals the portfolio
    rollup. This holds for FIFO, LIFO and HYBRID. Separately: when `start` is the
    portfolio's first transaction date and `end` is the `tax-summary` pricing date, that
    same sum equals the asset's `realized_pnl` from `tax-summary` under the same strategy
    and threshold. `tax-summary` has no window, so that second equality is only claimed
    for inception-to-date; it is not claimed for a mid-year window.
*   **Goal 2 — unrealized as of a date.** For any `end`, the sum of per-lot unrealized
    gain over the open-lot rows equals that asset's `unrealized_at_end`, computed at the
    same date and the same close (`price_date_used`).
*   **Goal 3 — the quarterly workflow is a few clicks.** Quarter and current-year presets
    fill `start` and `end`; the default is the current year to date, not December 31, so
    the default request is never rejected as a future date. The two date inputs remain for
    every case a preset does not cover.
*   **Goal 4 — reproducibility.** Given the same inputs *and the same recorded pricing
    inputs*, the same numbers come out on regeneration. The CSV records the strategy, the
    threshold, the portfolio base currency, the requested date range, the
    `price_date_used` and `fx_rate_date_used` per asset, and a `price_source` flag saying
    where each value came from; the statement prints the same run context in its header
    block. The pricing-layer fixes are what make this hold; a split recorded after the fact
    legitimately
    changes a regenerated file, and the run context is where that is visible.
*   **Goal 5 — no silent wrong numbers.** A price or FX rate that cannot be obtained fails
    the request and enumerates the missing symbols and currency pairs in one response, up
    to a stated cap, with a count of what was omitted. Where a date falls on a non-trading
    day, the file states the date actually used. No column is silently substituted, and the
    file states which columns are legal to sum.
*   **Goal 6 — a printable statement.** The printed page contains no navigation, no
    buttons, no inputs and no interactive controls, and its two headline figures —
    cumulative realized P&L and unrealized P&L — are the payload totals rendered through
    `frontend/src/utils/formatters.ts`, not arithmetic invented by the print view.
*   **Goal 7 — existing figures move only where declared.** Every change to numbers on
    existing endpoints is named and ships with its own note. Within this document the set is
    P3 (splits no longer written on a report path) and the `TaxLot` fix. The dated-FX fix,
    the `historical_prices` uniqueness fix and the pricing result types move figures too,
    but they belong to `2026-11-09-pricing-layer-truthfulness.md` and are declared there.

## Non-Goals

*   **No server-side PDF, and no PDF dependency.** Browser print-to-PDF only; see
    Alternative A for the cost.
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
*   **No term classification on open-lot rows.** The engine has no such field, and this
    design does not add one.

## Risks and Non-Functional Requirements

| Risk | What it is | Mitigating section |
| :-- | :-- | :-- |
| Existing figures move | The pricing-layer document's four changes each move numbers on endpoints the owner already trusts | Goal 7 and the Dependency section: those changes ship alone, with their own notes |
| Outbound cost and elapsed time | `replay` converts per transaction (`tax_engine.rs:403`); the process throttle is roughly five requests per second (`currency_service.rs:14`); `k8s/ingress.yaml` sets no `proxy-read-timeout`, so the ingress-nginx 60 s default applies, against roughly 33 s worst case per unreachable host (10 s reqwest timeout, `MAX_RETRIES = 3`, 1 s/2 s backoff) | Pricing resolution work-set shrink; Cost guard. The guard bounds call count, not wall clock: an elapsed deadline is tracked under Open Questions |
| Access posture | A parameter-dependent access rule on an existing route is invisible to the grep-based gate | Access level: one rule for both paths, proven by `isolation_test.rs` |
| Print fidelity | No `@media print` CSS exists and CI has no browser job, so nothing can prove the page prints | The Playwright smoke is the only honest check; it is not currently a CI gate |
| Single-replica assumptions | The guard and SQLite write behaviour both assume one pod; a rolling update with default maxSurge runs two for a window | Cost guard; state the dependency in the deployment manifest review |

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
   strings verbatim, no BOM       formatters.ts, display rounding
   (downloadCsv prepends BOM)     no summation, no controls
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

**Terminology.** The query parameters are `from` and `to`. Prose writes the window as
`[start, end]`. `replay_at(transactions, end, valuation_date, …)` takes `end` for
transaction filtering and `valuation_date` for FX; when the caller passes no valuation
date, `valuation_date = end`. There is no `as_of` parameter. Per-asset close dates are
`price_date_used` everywhere, including on rows; per-pair FX dates are
`fx_rate_date_used`. The open-lot gain fields are `unrealized_gain_loss` and
`unrealized_roi`, and they are the per-lot components of
`AssetTotals.unrealized_at_end`. The sold-lot P&L column is `lot_pnl`, which is
`LotMatch.pnl` renamed for the CSV.

### Dependency: the pricing layer

Four defects block this design. Three of them — dated FX, `historical_prices` uniqueness,
and honest pricing result types — are not part of this feature. They are a live bug fix
with their own blast radius and their own release note, and they are specified in
**`2026-11-09-pricing-layer-truthfulness.md`**, which this design is **blocked on** rather
than a subset of.

What this design takes from that document:

*   `get_rate` that actually honours its date argument, with `EURUSD=X` symbology and a
    per-pair series cache. Without it, a dated multi-currency report issues roughly one
    outbound call per transaction — about 200 for the flagship portfolio — against a
    process-wide throttle of roughly five requests per second
    (`currency_service.rs:14`).
*   `UNIQUE` `(symbol, date)` on `historical_prices`, so "the close on date X" is one row
    rather than an arbitrary one among N. Without it no as-of price is reproducible and
    Goal 4 is unreachable.
*   Pricing result types distinguishing `NoData` from `ProviderUnavailable`. The error
    contract below is meaningless without them.
*   A seedable FX store. Rates currently live only in a process `HashMap`
    (`currency_service.rs:125`), so no dated multi-currency test can be hermetic until they
    are persisted.

The fourth defect stays here. **P3 — no split writes on a report path.**
`sync_splits_for_asset` must not run inside a report request. Its dedupe is a
`SELECT COUNT(*)` then `INSERT` with no transaction and no unique constraint
(`currency_service.rs:46`, `:60`), so concurrent requests can both insert the same split row
and double the adjusted share count, and a split dated after `end` that gets inserted
retroactively changes in-window cost basis. The rule this design follows is the one the
single-sell P&L route already follows: the dated path does not sync splits, and the run
context states that splits are as recorded. Where the split call should go instead is an
open question with no destination in this deployment — `main.rs:63` is a plain `.launch()`
with no Rocket tasks and `k8s/` has no CronJob or Job — and is tracked under Open Questions
rather than solved here. This design does not detect unrecorded splits; it reports the split
rows that exist.

### The engine change: one public dated replay

`calculate_lots` is not parameterised. Three separate changes are required, and doing the
obvious thing — adding a single date argument — introduces three bugs.

1.  A new public entry point, `replay_at(transactions, end, ...)`, returning **both** the
    open lots and the per-sell breakdowns from a single pass. Today `replay` is private
    and `calculate_lots` discards the breakdowns after summing them, so per-sell rows are
    only reachable through `sell_breakdown`, which replays the entire asset history per
    sell and re-enters the FX lookup per transaction.
2.  The valuation date becomes an explicit parameter. Today the FX rate for market value
    is taken at `latest_date`, derived from the last transaction in the list
    (`tax_engine.rs:549-551`), so truncating the list silently moves the FX date to the
    last transaction before `end`. That is a different date from the one the caller
    requested, and it is not the on-or-before lookup the pricing rule defines.
3.  The `date <= end` filter runs **before** sorting and matching, because the split branch
    rescales every lot in scope.

Prefix replay itself needs no engine change: the engine already documents that a sell's
matches depend only on transactions at or before it (`tax_engine.rs`, matching loop). Both report shapes are then filters
over one result — cumulative realized is the sum of all breakdowns, window realized is
the sum of breakdowns whose sell date falls in `[start, end]`. There is one engine
function and two selections.

### Pricing resolution

Realized P&L needs no market quote: it is computed from recorded transaction prices. It
is **not** quote-free for a foreign-currency asset, however — `replay` converts each
transaction at its own date's rate (`tax_engine.rs:403`), so realized conversion is the
largest outbound consumer in this design, roughly one `get_rate` call per transaction on a
cold cache. The work-set shrink below has to reach that call site or it shrinks nothing
that matters.

Only the unrealized half needs a close, and it needs exactly one close per asset: the last
close **on or before** the requested date, within a bounded lookback. A bounded lookback is
what stops a symbol delisted years ago from pricing off a stale close it is technically
entitled to.

The same on-or-before rule applies to FX, which has no quote on weekends or market
holidays. FX is fetched **once per currency pair as a range** and looked up locally, not
once per transaction date. That requires the cache to stop being keyed by date: it is
currently `HashMap<(String, String, NaiveDate), Decimal>` (`currency_service.rs:125`) and
the URL carries no range (`:236`), so "one range per pair" is a change of cache value type
to a dated series, not a key version. Without that change the per-transaction path keeps
issuing one call per distinct transaction date — about 200 for the flagship 20-asset
multi-currency portfolio — against a process-wide throttle of roughly five requests per
second (`currency_service.rs:14`).

Failures are collected, not raised on the first one: the route resolves every asset first,
then returns one error naming the missing symbols and currency pairs, up to the cap in the
error contract. A missing FX rate is a pair, not a symbol, and the response says which it
is.

### The payload

The backend owns every number. The response is a typed `ToSchema` struct — not
`serde_json::Value`, which is what `tax-summary` declares today and which leaves the
generated schema empty (`analytics.rs:16`, producing `"schema": {}` in the committed
spec) — with every money field serialized as a decimal string: `decimal_json` for
required money (`db_types.rs:22`) and `decimal_opt_json` for nullable money
(`db_types.rs:73`, the helper `schemas.rs:199-216` already uses for `roi`).

It carries three groups:

*   **Run context:** portfolio id and name, base currency, strategy, `threshold_days`,
    the requested `[start, end]`, the `price_date_used` and `fx_rate_date_used` per asset,
    a `price_source` flag per asset, and a statement that splits are as recorded.
*   **Totals:** per-asset `realized_in_window`, `realized_cumulative`,
    `unrealized_at_end`, `market_value` and `cost_basis`, plus a portfolio rollup carrying
    exactly those five fields summed across assets. Computed in Rust from the same replay
    that produced the rows. `ReportContext` is the single authority for `strategy` and
    `threshold_days`; the response does not repeat them at top level.
*   **Rows:** `SOLD_LOT` and `OPEN_LOT` carry lot-level values only; `UNMATCHED` carries a
    sell id and an unmatched quantity — the quantity no buy lot covers — with all money
    cells null.

Renderers must not sum row values. The CSV writes the server-computed totals verbatim and
states which columns are legal to sum. A total derived in TypeScript is a double and
cannot reproduce exact decimal equality against `tax-summary`.

### The renderers

The CSV builder follows the existing single-sell export's conventions — the formula-lead
guard on free-text cells and exponent-free numeric output — and emits **no BOM**, because
`downloadCsv` prepends it (`frontend/src/utils/download.ts:25`) and a second one corrupts
the header row. Both guards are module-private in `sellPnlCsv.ts` (`FORMULA_LEAD` at `:55`,
`text` at `:56`, `num` at `:62`) and `num` takes a `number`, so they are extracted into a
shared module and re-derived for strings rather than "reused" as written. Report money
fields travel as strings end to end and must not pass through `normalizeTaxSummary` or
`normalizeSellPnl`, which coerce every field through `Number()` and would reduce the file
to double precision.

The statement is a React view with a print stylesheet. There is no `@media print` rule
anywhere in the frontend (verified: no `@media print` in `frontend/src/`), so this
stylesheet does not exist yet. It renders the payload's totals through
`frontend/src/utils/formatters.ts`, and its printed subtree contains no buttons or inputs;
the asset-subset control sits outside the printed subtree.

### Sequencing

The cut line is the realized/unrealized seam, not the two reports. Everything left of it
touches no network.

| Slice | Ships | Reaches network in tests? | Blast radius on existing figures | New-code risk |
| :-- | :-- | :-- | :-- | :-- |
| Out | The pricing-layer changes: dated FX, `historical_prices` uniqueness, pricing result types | no | figures move for every foreign-currency asset | moderate |
| 1 | Statement at today's date, pure frontend over the existing tax-summary payload | no | none | low |
| 2 | `replay_at` in the engine, unit tests only | no | none | contained |
| 3 | Tax report CSV, sold-lot rows only — the quarterly realized figure | no | none | moderate |
| 4 | Open-lot rows, the pricing rule, and the failure contract | **yes** | none new here — the pricing-layer document's result types stop failed lookups returning zero | **the risky piece** |
| 5 | Statement switches to the report payload and accepts a past `to` | yes | none | low |

### Decided during drafting

Four items were open when the Technical Plan was first written. They are settled here, and
the predecessor RFC is the source for the first one.

**Route.** Reports extend `GET /api/portfolios/<id>/tax-summary` with two optional query
parameters rather than adding a route. Absent `from` and `to`, the endpoint behaves exactly
as it does today. This is the RFC's decision; an earlier revision of this draft drew a
separate `/tax-report` route, which contradicted the rejection recorded as Alternative B.

**Access level.** Every call to `tax-summary` requires `Access::Viewer`, dated or not. There
is no parameter-dependent access rule, and the route already requires `Viewer`
(`analytics.rs:30`), so this is a statement of intent rather than a behaviour change. An
earlier revision of this draft required `Editor` for dated calls, and that was dropped on
review: it does not bound cost — an `Editor` can issue the same expensive request as often
as a `Viewer` can — so it would only have restricted who gets to be slow. The grep-based
access gate cannot see a parameter-dependent rule on an existing route, so
`tests/isolation_test.rs` covers the dated variant.

**Cost guard.** Two bounds, both on the dated path only, so the existing screen is
untouched. One report may be in flight per authenticated subject (the `sub` claim) at a
time; a second concurrent request is rejected with HTTP 429 and `retryable: true`. And each
report carries an outbound-request budget; exceeding it fails with the 503 contract rather
than continuing to consume the process-wide throttle. The budget is the outbound cap the
pricing section needs anyway (see Pricing resolution). The in-flight slot must be released
on every exit path, including a panic, or one slow report permanently blocks that user. The
guard is per-process state and therefore depends on `k8s/app-deployment.yaml` keeping
`replicas: 1`; raising it gives each subject one slot per pod.

**Error contract.** Two failure classes, one typed body, produced by a custom Rocket
`Responder` in the style of the auth `Unavailable` responder
(`backend_rust/src/auth/routes.rs:33-52`): HTTP 422 for "no data exists at or before this
date for this symbol or currency pair", HTTP 503 for "provider unreachable or throttled".
`retryable` is `false` for 422 and `true` for 503. Guard rejections use HTTP 429 with
`retryable: true`. Validation failures — `from > to`, a future date, an unparseable date, an
unrecognized strategy, a negative `threshold_days` — also return 422, with
`ErrorBody.error` naming the offending parameter and `missing_prices` and `missing_rates`
empty; no-data failures return 422 with those lists populated. On any error the route
returns only the error body, and the frontend does not call `downloadCsv` when the request
fails, so no partial file reaches the owner. The missing-item list is capped, with a count
of what was omitted.

This choice depends on the pricing-layer document's P4. The pricing layer cannot currently
distinguish "no data" from "could not ask": `get_price` returns `Decimal::ZERO` on every
failure (`currency_service.rs:267`), and `get_historical_prices` already returns a
`Result` (`:406`) but swallows the failure at five sites (`:429`, `:442`, `:449`, `:467`,
`:502`) and hands back the database fallback as `Ok`. That document changes them; this one
consumes the distinction. It is named here because the error contract is meaningless
without it, and because the change reaches the existing screen: `get_price` is called from
`analytics.rs:73` and `stats_engine.rs:180`, and `get_historical_prices` from
`stats_engine.rs:139`.

**Date parameter semantics.** `from` and `to` are `YYYY-MM-DD`, both inclusive, evaluated in
UTC. `Transaction.date` is a `DateTime<Utc>` (`models.rs:35`), not a date, so "a sell at any
time on `to` is inside the window" requires an explicit conversion — `date.date_naive() <=
to` — and `replay_at`'s `end` parameter is a `NaiveDate`. Absent parameters reproduce
current behaviour: all transactions, priced at the latest available close. `from > to`, a
`from` or `to` in the future, and an unparseable date are all HTTP 422. Whether `to` alone
or `from` alone is legal is an open question, not a settled rule; see Open Questions.
`strategy` is validated strictly on the dated path: absent means FIFO,
present-but-unrecognized is a 422 naming the accepted values, rather than the silent
fallback to FIFO that both `tax-summary` (`analytics.rs:217-221`) and the single-sell P&L
route do today. Extending strict validation to the undated path reverses a standing
rejection recorded in
`docs/superpowers/reviews/2026-11-05-sell-pnl-breakdown.md` and is tracked under Open
Questions. `threshold_days` must be non-negative; a negative HYBRID threshold classifies
every holding as long-term and is not a meaningful input.

### Reconciliation with the predecessor

The draft was compared against the RFC after drafting. It contained one contradiction and
six additions; each is resolved below.

The RFC's non-goals excluded *"concurrency work, batching frameworks, cache layers or
timeout knobs beyond the work-set shrink."* The cost guard is beyond that shrink. The owner
kept the guard and amended the RFC's non-goal, narrowing it to batching frameworks,
concurrent fan-out and new timeout knobs, with the reason recorded in the RFC itself
(`2026-11-09-tax-report-and-statement-concepts.md`, Non-Goals).

The owner accepted six decisions the RFC did not raise: the pricing result types (now the
pricing-layer document's responsibility); the `TaxLot` fix; strict strategy and threshold
validation; bounding and versioning the FX cache (also the pricing-layer document's);
HTTP 429 in the error contract; and the `SOLD_LOT` row-kind name. Two of them
change existing endpoint behaviour — the result types and strict strategy validation — and were decided
individually rather than folded in. `TaxLot` is corrected in place rather than worked
around, because the Holdings table renders "Remaining Qty" and "Original Qty" as two
columns fed from one value (`PortfolioDetail.tsx:322-323`, `:333-334`), so Original Qty is
meaningless today and the fix makes it correct. Strategy validation is strict on the dated
path, which has no back-compatibility obligation; extending it to the undated path is
tracked under Open Questions.

## Open Questions

None open from the predecessor RFC. Its five items are discharged here: totals ownership by
The payload and Alternative C; the error contract shape by Error contract; the access level
by Access level; date parameter semantics by Date parameter semantics; and the trading
calendar question by Pricing resolution's bounded lookback.

Open from this draft, each needing a decision before the slice that depends on it:

1.  **Single-parameter dates.** Is `to` alone legal, meaning cumulative from the
    portfolio's first transaction, and is `from` alone legal with an open end? Or is it
    both-or-neither with a 422 otherwise? The statement's cumulative figure and the CSV's
    `realized_cumulative` column both depend on this.
2.  **Numeric constants.** The bounded lookback length, the per-report outbound budget, the
    missing-item cap, the FX cache bound, and the print page count are named nowhere. Each
    needs a value and a place it is asserted by a test.
3.  **P3's destination.** With no Rocket tasks and no CronJob, where does the split sync
    go — a mutation endpoint, a background job, or nowhere, with the dated path simply not
    syncing as the single-sell route already does not?
4.  **Strict validation on the undated path.** Adopting it reverses the standing rejection of
    an input-validation matrix in
    `docs/superpowers/reviews/2026-11-05-sell-pnl-breakdown.md` and contradicts "absent
    parameters reproduce current behaviour". Keep it dated-path-only, or cite the record and
    amend Goal 7.
5.  **Elapsed deadline versus the guard.** The guard bounds call count. Two unreachable
    symbols exceed the ingress cutoff, the browser receives an HTML gateway error instead of
    the typed 503, and the slot stays held so the next click returns 429. Does an elapsed
    deadline replace the guard, supplement it, or is the gateway case accepted?
6.  **CSV summable-column disclosure.** `sellPnlCsv.ts:13-16` deliberately puts caveats in
    columns because a leading `#` row breaks naive parsers, including this app's own
    `detectColumns`. Which convention does the report CSV use?

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
*   **Reason for deferral**: deferred rather than rejected on merit. There is one renderer
    family today and the rule can be enforced by test; add the metadata when a third
    consumer exists. See Future Improvements.

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

### Future Improvements

*   Column metadata on the payload (Alternative D), once a third consumer exists.
*   A `price_source` breakdown richer than a flag, if a second pricing provider ever exists.
*   The split-sync destination that P3 leaves open.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Sequenced by the slices in the Technical Plan. Slice numbers are given per file; a file
appearing in more than one slice is touched more than once.

### Target Files to Create/Modify

### Moved to the pricing-layer document

Files 1 and 2 of the original list — `backend_rust/src/services/currency_service.rs`
and `backend_rust/src/lib.rs` — carried the dated-FX rebuild, the
`historical_prices` uniqueness fix, the cache work and the result-type change. They are
specified in **`2026-11-09-pricing-layer-truthfulness.md`**, which this design depends
on. Nothing in this document edits those two files.

#### 3. `backend_rust/src/report_guard.rs` (Create) — slice 4
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
    *   Fix `TaxLot` — **assigned to slice 3, not slice 2**, because `TaxLot` is defined in
        `schemas.rs:89-102`, is registered in `openapi.rs:35`, and appears in the committed
        `docs/openapi/openapi.json`, which CI diffs byte-for-byte. Landing it in slice 2
        fails CI and ships an undeclared change to a live endpoint's response shape.
        `original_qty` and `remaining_qty` are both set to the already-decremented `lot.qty`
        (`tax_engine.rs:587-588`), so the originally-bought quantity is unrecoverable, and
        the struct carries no buy transaction id. The buy id already exists upstream as
        `InternalLot.source_tx_id` (`tax_engine.rs:366`) and `LotMatch.buy_transaction_id`
        (`schemas.rs:134`), so this is a field addition, not new plumbing. Existing
        assertions that pin the buggy equality and must move with it:
        `tests/analytics_test.rs:51-52` and
        `frontend/src/test/components/PortfolioDetail.test.tsx:184-185`.
    *   Unit tests: prefix replay equals full replay for realized P&L under FIFO, LIFO and
        HYBRID; a sell after `end` is excluded; a **split after `end`** does not move
        in-window cost basis; HYBRID matches are unchanged, pinning that `threshold_days`
        stays measured from each sell.

#### 5. `backend_rust/src/schemas.rs` (Modify) — slice 4

*   **Type / Scope**: request and response types, plus the `TaxLot` fix moved from file 4.
*   **Proposed Changes**: typed report payload, every money field serialized as a decimal
    string — `decimal_json` (`db_types.rs:22`) for required money and `decimal_opt_json`
    (`db_types.rs:73`) for nullable money, since `UNMATCHED` carries nulls and "nulls are
    nulls" is a stated rule. Shape:

```text
ReportContext   portfolio_id, portfolio_name, base_currency, strategy, threshold_days,
                requested_from, requested_to
AssetTotals     asset_id, symbol, realized_in_window, realized_cumulative,
                unrealized_at_end, market_value, cost_basis,
                price_date_used, fx_rate_date_used, price_source, fx_applied, split_applied
ReportRow       row_type: SOLD_LOT | OPEN_LOT | UNMATCHED
                SOLD_LOT: sell + lot fields, mirroring LotMatch; LotMatch.pnl is
                          exposed as lot_pnl
                OPEN_LOT: buy_date, buy_transaction_id, original_qty, remaining_qty,
                          unit_cost, price_date_used, market_value,
                          unrealized_gain_loss, unrealized_roi
                UNMATCHED: sell id, unmatched_qty, money cells null
ReportResponse  context, asset_totals: [AssetTotals],
                totals: {realized_in_window, realized_cumulative, unrealized_at_end,
                         market_value, cost_basis},
                rows: [ReportRow]
ErrorBody       error, missing_prices: [symbol], missing_rates: [{from,to}],
                retryable, omitted_count
```

    `row_type` is an exhaustive enum both renderers must switch on. Nulls are nulls, not
    zeros, so a consumer can tell "not applicable" from "zero". `price_date_used` on a row is
    the asset-level `price_date_used` repeated — one close per asset, not one per lot.

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
*   **Proposed Changes**: register the new report types in `components(schemas(...))` in
    `openapi.rs:26-40` — utoipa derives the query parameters from the Rocket route
    attribute (`analytics.rs:21`), so adding `&<from>&<to>` documents them automatically;
    what does not happen automatically is schema registration, and an unregistered type
    produces a `$ref` to nothing. Also replace the `body = serde_json::Value` declaration
    for this route with the typed response, so the generated schema is no longer empty.

#### 8. `backend_rust/tests/tax_report_test.rs` (Create) — slices 3, 4

*   **Type / Scope**: integration tests, hermetic.
*   **Proposed Changes**: the three reconciliation equalities, inlined rather than referred
    to by number: (a) the sum of sold-lot `lot_pnl` for an asset equals that asset's
    `realized_in_window`; (b) with `from` at the first transaction date, that sum equals
    `tax-summary`'s `realized_pnl` for the asset; (c) the sum of open-lot
    `unrealized_gain_loss` equals `unrealized_at_end`. All compared as exact decimals under
    FIFO, LIFO and HYBRID.

    Seeding follows the pattern the prior review record
    (`docs/superpowers/reviews/2026-11-05-sell-pnl-breakdown.md`) established after the
    sell-pnl suite went from 1.33 s to 5.67 s by reaching the network: seed
    `historical_prices` and pre-seed `split_sync`. Two gaps must be closed before this file
    can deliver what it claims. **There is no FX table** — `get_rate` takes no pool and reads
    only the process cache (`currency_service.rs:125`) — so a dated multi-currency test
    cannot be hermetic unless the pricing-layer document persists rates into
    `historical_prices` under
    `EURUSD=X`; and **there is no provider seam** — `CurrencyService::new()` takes no
    configuration and the Yahoo URL is hardcoded at `:236`, `:303`, `:421` and `:574` — so
    "an unreachable provider returns 503" cannot be written today. Seed every date the way
    `analytics_test.rs:171-183` does and there are no calendar gaps, so the non-trading-day
    case needs a deliberately gapped seed.

    Cases: window equals the sum of per-sell breakdowns; cumulative equals all-time
    `tax-summary`; a non-trading `to` resolves to the prior trading day and reports the date
    used; a missing price returns 422 naming every missing symbol; an unreachable provider
    returns 503 with `retryable: true`; and `from > to`, a future `from` or `to`, an
    unparseable date, an unknown strategy and a negative `threshold_days` each return 422.

#### 9. `backend_rust/tests/isolation_test.rs` (Modify) — slice 4

*   **Type / Scope**: the real cross-user authorization matrix.
*   **Proposed Changes**: add the dated variant of `tax-summary` to the path list, and add a
    case proving a non-owner cannot request a dated report. Note that `access.rs:37-57`
    grants `Access::Owner` to the owner and nothing to anyone else — there is no membership
    table — so a "`Viewer` collaborator can request a dated report" case cannot be written
    until sharing exists. Add guard cases only if guard tests are writable: a second
    concurrent report returning 429 needs two overlapping dispatches with a stall seam, and
    a panicking handler under Rocket's local `Client` unwinds the test thread rather than
    producing a response. The grep-based gate cannot see an access-level choice on an
    existing route.

#### 10. `backend_rust/tests/route_coverage_test.rs` and `backend_rust/tests/auth_test.rs` (Modify) — slice 4

*   **Type / Scope**: route surface and unauthenticated sweep.
*   **Proposed Changes**: no route is added, so the three hard counts stay at 15 and 13
    (`route_coverage_test.rs:140`, `:262`, `:275-278`; `to_document_path` at `:86-92`
    truncates at `?`, so added query parameters leave them intact). `auth_test.rs:46`
    special-cases the tax routes to append strategy parameters; there is one tax-summary
    route, so covering the dated variant means a second parameterised pass over the same
    path, not a new entry. The comment at `auth_test.rs:43-45` claiming the tax routes
    require a strategy is already false — `analytics.rs:59` defaults to FIFO — and must be
    corrected in the same change.

#### 11. Moved to the pricing-layer document

The existing reconciliation tests move with the dated-FX fix, which lives in
2026-11-09-pricing-layer-truthfulness.md. Nothing in this document needs that file: no
expectation there assumes FX conversion at the latest rate.

#### 12. `docs/openapi/openapi.json` (Regenerate) — slice 4

*   **Type / Scope**: committed API contract, diffed by CI.
*   **Proposed Changes**: regenerate with the command in `AGENTS.md`.

#### 13. `frontend/src/types/api.d.ts` (Regenerate) — slice 4

*   **Type / Scope**: generated API types.
*   **Proposed Changes**: regenerate via `npm run generate-types`. Note that nothing
    currently imports `api.d.ts` — a repo-wide grep finds only the package.json script line
    — while every renderer uses the hand-written types in `frontend/src/types.ts`
    (`TaxLot` :42, `AssetTaxSummary` :51, `TaxSummary` :89, `LotMatch` :97). Regenerating
    alone changes nothing for the renderers; either `types.ts` gains the report types or the
    renderers are repointed at the generated ones, and that choice needs naming here.

#### 14. `frontend/src/utils/taxReportCsv.ts` (Create) — slice 3

*   **Type / Scope**: CSV builder, pure, no DOM.
*   **Proposed Changes**: one row per `ReportRow`, sell context repeated on sold-lot rows,
    the run context (strategy, threshold, requested range, pricing dates used) as columns,
    and money written verbatim from the payload strings. **The column list, order and the
    summable-column disclosure are not specified anywhere in this document and must be
    written down here** — the CSV is the deliverable and the owner archives it. Note the
    convention conflict to resolve: `sellPnlCsv.ts:13-16` deliberately carries caveats as
    columns rather than a leading `#` row, because a comment row breaks naive parsers
    including this app's own `detectColumns`. The formula-lead guard and the exponent-free
    rule live in a shared module extracted from `sellPnlCsv.ts` (`:55`, `:56`, `:62`),
    re-derived for strings. Emit no BOM; `downloadCsv` prepends it. Report money must not
    call `normalizeTaxSummary` or `normalizeSellPnl`.

#### 15. `frontend/src/utils/taxReportCsv.test.ts` (Create) — slice 3

*   **Type / Scope**: unit tests.
*   **Proposed Changes**: mirror `sellPnlCsv.test.ts`: the `lot_pnl` column sums to
    `AssetTotals.realized_in_window` for that asset, no BOM in the builder output, formula
    characters escaped, exponent notation never emitted, nulls rendered as empty cells, and
    every row kind present. The float-summed assertion pattern in
    `sellPnlCsv.test.ts:184-194` uses `toBeCloseTo(…, 9)` because that payload went through
    `Number()`; with strings preserved this test must compare decimal strings or state which
    decimal helper it uses.

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
    preset — five presets, not four. The default is the current year **to date**, because a
    default of January 1 to December 31 is a future `to` for most of the year and the route
    rejects it. No calendar widget is added; the native `<input type="date">` picker remains
    available. Controls stay enabled for every access level, since the route requires only
    `Viewer`; when the guard rejects with 429 the UI says a report is already running and
    offers a retry.

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
    headline figures equal the payload totals rendered through
    `frontend/src/utils/formatters.ts`. **Name the selectors here rather than at
    implementation time**, or a CI grep can pass while hiding nothing: statement hook
    `data-print="statement"`; chrome selectors `.nav-menu`, `.nav-item` and
    `header.app-header` (`App.tsx:385-396`). `handlers.ts` already registers
    `/api/portfolios/:id/tax-summary` and MSW matches on path, so the change is branching
    that handler on `from`/`to`, not adding a route; respect the existing ordering comment at
    `handlers.ts:130-131`.

#### 22. `frontend/scripts/check_print_styles.sh` (Create) and `.github/workflows/` (Modify) — slice 1

*   **Type / Scope**: CI gate. CI runs `cargo test` and frontend checks and sees no CSS.
*   **Proposed Changes**: a grep-style script in the spirit of
    `backend_rust/scripts/check_access_gate.sh` asserting the print stylesheet exists and
    hides the chrome selectors, wired into the existing frontend job.

#### 23. `frontend/e2e/statement.smoke.mjs` (Create) and `frontend/e2e/run.sh` (Modify) — slice 1

*   **Type / Scope**: browser smoke, following the existing sell-pnl e2e pattern.
*   **Proposed Changes**: load the statement, emulate print media, assert chrome is
    `display: none`, that there is no horizontal overflow, and that `page.pdf()` returns a
    non-empty buffer within a stated page count (**state the count**). This is the only
    honest check that printing works; there is no server artifact to assert on. Note the
    prerequisites: `frontend/package.json` is not in this list and must gain an
    `e2e:statement` script — `e2e/run.sh` hardcodes `node e2e/sell-pnl.smoke.mjs` — and
    `.github/workflows/ci.yml` has **no browser job at all**, so this smoke test is a local
    step requiring a chromium binary, not a gate.

#### 24. Moved to the pricing-layer document

The single-sell export UI carries a footnote reading "at the latest available rate".
That sentence becomes false when the dated-FX fix lands, and it is corrected there,
not here.

#### 25. `docs/superpowers/plans/2026-11-09-tax-report.md` (Create) — all slices

*   **Type / Scope**: implementation plan, following the repo's existing convention.
*   **Proposed Changes**: the slice table expanded into ordered tasks with the file list
    above, plus a *Deviations* section for any sanctioned exception, matching the shape of
    the prior feature's plan.

### Files This Plan Must Add and Currently Omits

Each of these is forced by a change already in the list. A file list that cannot name where
its own decisions land is mis-cut.

| File | Why it is forced |
| :-- | :-- |
| `backend_rust/tests/analytics_test.rs` | Pins `original_qty == remaining_qty` at `:51-52` |
| `frontend/src/components/PortfolioDetail.tsx` | Renders the two lot columns from one value (`:322-323`, `:333-334`) |
| `frontend/src/test/components/PortfolioDetail.test.tsx` | Pins the buggy equality at `:184-185` |
| `backend_rust/src/main.rs` | `build_rocket(...)` call site at `:63` if managed state changes the signature |
| `backend_rust/tests/common/mod.rs` | Three `build_rocket` sites at `:46`, `:56`, `:65` |
| `backend_rust/tests/cors_test.rs` | Five `CurrencyService::new()` sites if the constructor changes |
| `frontend/src/types.ts` | The hand-written types every renderer actually uses; `api.d.ts` has zero importers |
| `frontend/package.json` | Required for any `e2e:statement` script |
| `frontend/src/main.tsx` | The only entry-import site for a stylesheet (`main.tsx:3`) |
| `frontend/src/utils/csvCells.ts` (new) | Holds the formula-lead and exponent-free guards extracted from `sellPnlCsv.ts` |
| `docs/superpowers/reviews/2026-11-09-tax-report.md` | Decision and rejection record, following the convention in `docs/superpowers/reviews/2026-11-05-sell-pnl-breakdown.md` |
| `docs/superpowers/specs/2026-11-09-tax-report-and-statement-concepts.md` | The amended non-goal at its Non-Goals section must be edited if the guard changes |

### Verification Plan

*   **Compilation/Build**: `cargo build --release` and `cargo build` from `backend_rust/`;
    `npm run build` from `frontend/`.
*   **Unit Tests**: `cargo test` from `backend_rust/`, which now includes
    `tax_report_test.rs` and the prefix-replay engine tests; `npm test` from `frontend/`.
*   **Static Analysis & Linting**: `cargo clippy --all-targets` — **local only; CI does not
    run clippy**; `bash scripts/check_access_gate.sh`; `npm run lint`; `npm run typecheck`.
    In this workspace `frontend/node_modules` is absent and there is no chromium binary, so
    the frontend checks need a networked `npm ci` and the print smoke cannot run at all.
*   **Access gate**: `tests/isolation_test.rs` must cover the dated variant, since the
    grep-based script cannot see an access-level choice on an existing route. The gate's real
    constraints on a refactored `analytics.rs`: Rule B ends a handler at the first column-0
    `}` and requires a `require*` call before any `WHERE id` query; Rule C inspects the
    signature only up to the first `->`; a new file under `src/api_routes/` must import
    `crate::access` to satisfy Rule A.
*   **Reconciliation**: the three equalities inlined in file 8, asserted as exact decimals
    with no tolerance, under FIFO, LIFO and HYBRID.
*   **Print**: `bash frontend/scripts/check_print_styles.sh` (note the path — the backend
    gate is invoked from `backend_rust/`) and `npm run e2e:statement` from `frontend/`. Both
    are local steps: CI has no browser job and no CSS check today.
*   **Manual, cannot be automated**: a real OIDC login and a real browser print preview. The
    login is the item `docs/auth-testing.md` lists as unverifiable in CI; the print preview is
    new to this document and is not listed there.
*   **Provider smoke**: `bash scripts/auth_smoke.sh` is unrelated; pricing changes are
    verified hermetically against seeded `historical_prices` rows, never against the live
    endpoint, so the suite has no network dependency.

--------------------------------------------------------------------------------

*Drafted with EGM*
