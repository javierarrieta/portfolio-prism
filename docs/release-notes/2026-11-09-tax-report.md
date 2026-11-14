# Dated tax report: a CSV and a page for a window you choose

> **The test counts in this note are as-of 2026-11-09 and are not maintained.** The backend
> suite, the access-gate handler count and the frontend suite have all grown since; the
> live numbers are whatever `cargo test`, `bash scripts/check_access_gate.sh` and
> `npm test` print today, and `docs/auth-testing.md` §0 carries the current gate set. The
> figures below are a dated transcript — leave them, do not "fix" them.

Branch `feat/report-export`.

## What a user will notice

**A sixth tab, Tax Report.** Two date inputs, four quarter buttons and *This year to date*.
The window defaults to the current year to date, not January 1 to December 31, because a
`to` in the future is a hard 422 and a default that always errors is worse than no default.
A quarter still in progress is clamped to today when you click it; a quarter that has not
started is disabled and says so in its tooltip.

**The same endpoint answers a window.** `GET /api/portfolios/<id>/tax-summary?from=&to=` now
returns a dated report: every sell inside the window at lot granularity (one row per buy lot
that sell consumed, plus an `UNMATCHED` row when a sell could not be matched), every lot still
open at the window's end with its unrealized gain measured **at that date**, per-asset totals
and a portfolio rollup. **No route was added** — the operation count is still 15, and the
OpenAPI document gains `from`, `to` and `format` as query parameters plus the typed
`ReportResponse` body. `?format=csv` answers the same report as `text/csv`, named by the
server. Absent `from` and `to`, the endpoint behaves exactly as it did.

**Either date alone is legal.** `to` alone means "from the portfolio's first transaction
through that date"; `from` alone means "from that date through today". The window is inclusive
on both ends and the boundary is UTC, so a sell at any time on `to` is inside it. `?format=json`
is accepted as an explicit no-op for the default; an unknown format name is a 422 naming the two
accepted values. An
unparseable date, a future date, and `from` after `to` are each a 422 naming the parameter.

**The file is named by the server.** `tax-report-<id>-<from>-<to>.csv`, and the name carries the
window **actually applied**, not the parameters as typed. With `from` absent that is the portfolio's
first transaction on or before `to`, so a `to`-only request for a portfolio whose first trade was
2024-01-01 is named `...-2024-01-01-<to>.csv`. The literal `inception` appears only when the
portfolio has no transaction at or before `to` at all — the rare case, not the common one. Row 1 is the header row; there is no comment row above it, and
the caveats ride as columns. The file opens with a UTF-8 BOM, like every other CSV this app
produces, because Excel decides the encoding from it.

**Every figure on the screen came from the server.** The tab renders the payload; it adds,
averages and subtracts nothing. The five headline figures are `totals.*` from the response, and
the per-asset totals are printed as columns in the CSV so a spreadsheet can be checked against
them without recomputing anything.

**Each row says where its price came from.** `price_source` is `database` or `provider`,
`price_date_used` is the date of the close that actually priced the asset, and
`fx_rate_date_used` is the date the currency pair's rate came from. One close per asset,
resolved once, repeated on that asset's rows.

## The one figure that moves

`TaxLot.original_qty` was the already-decremented quantity — the same number as
`remaining_qty` — so the Holdings table printed **Remaining Qty in both columns** and the
quantity actually bought was not recoverable from the response at all. It is now the quantity
the lot was opened with, split-adjusted alongside the remaining quantity. A lot that bought 60
and sold 20 now prints Original Qty 60 / Remaining Qty 40 where it printed 40 / 40.
`TaxLot` also gains `buy_transaction_id`, so a lot can be joined back to the ledger row a
person is reading. **One other screen can move, and only for a portfolio stored with a lowercase
currency**: the single-sell P&L breakdown's FX footnote now hides where it used to show, because
`fx_applied` was made case-insensitive to match the engine and the pricing layer. That footnote was
wrong before for such a portfolio; it is right now. Asset and portfolio currency are stored
verbatim, so the case is reachable, though nothing in the UI creates it.

## Strategy validation is strict on the dated path only

`?strategy=average_cost&to=2024-06-30` is a 422 naming the accepted values
(`FIFO`, `LIFO`, `HYBRID`). Without a window the same input still falls back to FIFO, and **the response now echoes `FIFO`
rather than the string the client sent**. The value is normalized once and used for both the engine
call and the response, so the response can no longer name a matching method the engine did not use.

**This is the one exception to "absent `from` and `to`, the endpoint behaves exactly as it did",
and it moves figures, not just labels.** Before, `?strategy=hybrid` (lowercase) fell through the
engine's catch-all arm, **computed FIFO**, and echoed `hybrid`. Now it computes HYBRID. So a direct
API caller sending a case-varying token gets different numbers from this release. No shipped screen
is affected: the UI only ever sends the three uppercase tokens.

## The columns that may be summed

In the CSV, **`lot_pnl` filtered to `row_type=SOLD_LOT` is the only column whose sum is a
figure.** It equals that asset's `asset_realized_in_window`, which the file carries on the same
rows, so the reconciliation is a comparison and not a recomputation. Every other sell-level
column — `proceeds`, `cost_basis`, `realized_pnl`, `sold_qty`, `matched_qty`,
`sell_price_in_asset_currency` and `sell_fee_in_asset_currency` — is that **sell's** whole figure
repeated on each of that sell's lot rows, and summing them counts a four-lot sell four times. Every `asset_*` column is already a total. `realized_pnl` is the trap: it is the
whole sell's P&L, and it is the column a spreadsheet reaches for first.

This disclosure lives here rather than in the file, because anything above the header row breaks
the importer it is meant to warn: `detectColumns` (`frontend/src/utils/csvParser.ts:25`)
normalises every cell of row 1 against `FIELD_ALIASES`, so a leading `#` row maps every column
to `null` and an import silently yields nothing.

## Splits

The dated path does **not** sync splits (issue #50); the undated path keeps the sync it has
always performed. Split-adjusted history is as recorded when the report was generated, and both
the run context and the CSV's `splits_note` column say so. The consequence is stated plainly:
**a split recorded after you file a quarter legitimately changes that quarter's cost basis when
you regenerate it.** `split_applied` on each asset says whether a split dated on or before the
window end was applied to the lots in scope; a split dated after the window end changed nothing
inside it.

## The first request after a deploy may 503 once

A cold multi-currency portfolio can exceed the 20 s deadline on its first report and succeed on
retry, because that attempt warmed the per-pair series cache. That is the design working, not a
bug — without this sentence it reads as one.

**What the 20 s bound actually bounds.** The deadline is checked *between* the route's own
outbound calls, so it refuses to start new work past 20 s and fails closed with the typed 503.
It cannot preempt a call already in flight, and one more call can still be made after the last
check: when a stored currency-pair window is fresh but short, `replay_at`'s per-transaction rate
lookup still goes outbound for the dates it does not cover, after the route's final check. So
**a request can run past 20 s.** There is no route-local fix — the fetch decision belongs to the
pricing layer, which records no coverage per symbol. What the deadline guarantees is the failure
mode, not the duration: you get the typed 503 with `retryable: true`, not the ingress' HTML 504.

## A missing rate is not reliably a 422

The error contract is 422 for "no price or rate exists at or before this date" and 503 for "a
provider could not be reached or the deadline passed". `retryable` is `false` on the 422 and
`true` on the 503, and the report **fails closed** — no partial payload is ever returned,
because a partial tax report is the outcome this feature exists to prevent.

**Do not read a 503 as a broken server when the pair simply does not exist.** A rate that
genuinely does not exist is indistinguishable from a provider outage until the provider has been
asked: every "no data" answer this system can produce comes out of parsing a provider response,
and the database path either answers or falls through to a fetch. So for a delisted or
never-listed pair the route must make the call before it can know — and if the provider answers
with an error status, which is how an unknown symbol usually comes back, the answer is a
**retryable 503 on every attempt, not a 422**. Retrying will not eventually turn it into a 422.
The 422 with `missing_rates` is what you get when the provider answers successfully and reports
no data for that pair. If a holding's currency pair has no series, that is the finding; the
server is not flaky.

The two lists are capped — **20 symbols and 10 rate pairs** — and `missing_prices_total` /
`missing_rates_total` are counted **before** truncation, so a short list is distinguishable from
a complete one. There is no `truncated` flag: truncation is `total > listed`, and the screen
prints the count of what was omitted on the same line as the names.

There is no 429 in this contract and no per-user report slot. An earlier draft of the design
carried both; they were removed on review (decision D2) and replaced by the elapsed deadline.

## Which artifact is authoritative

**The CSV is the artifact to reconcile against.** The screen quantises: `TotalFigure` renders
through `formatCurrency`, which does `Number(value)` and then `Intl` at the currency's fraction
digits. The engine never rounds — `tax_engine.rs` contains no `.round(` or `.set_scale(` — so a
figure with more fractional digits than the currency prints as a rounded amount on screen and
verbatim in the file. One number, two renderings. The screen and the file do **not** always show
the same text, and the note would be wrong to promise it.

Two downloads over unchanged data are **byte-identical**, and there is deliberately no
generated-at column to break that. The JSON payload is the documented public surface; the CSV is
the same payload in another envelope, produced by a projection that re-derives no number.

The Date column renders the payload's own RFC3339 token — the same string the CSV cell carries —
so it is a timestamp, not a date-only format. And the dated and undated shapes differ in one
place worth knowing: an open lot with a zero cost basis reports a **null** `unrealized_roi` on
the dated report and a **zero** `latent_roi` on the undated summary.

## Limitations

- **The quarter presets only generate the current year.** `quarterPresets` builds its four
  quarters from `today`'s UTC year, so in April 2027 — filing season for tax year 2026 — every
  button is a 2027 window and Q4 2026 must be typed by hand. This bites at exactly the moment the
  feature is for.
- **The Download CSV link is absent while the report is in an error state.** The route's error
  responses carry no `Content-Disposition`, so an anchor click on a failing window would
  navigate the browser away from the app to a raw JSON body. The link appears when there is a
  report on screen.
- **The OpenAPI document describes only the dated shape for status 200.** The same operation
  serves the legacy summary object when no window is given, and that shape is disclosed in the
  response's prose description but not in its schema — `body = ReportResponse` is wrong for the
  undated half of the requests. A client generated from the document can parse a report and cannot
  parse a summary. Fixing it properly needs a `oneOf` over both shapes, or splitting the operation;
  neither was worth doing to a legacy response this release deliberately leaves alone.
- **A 500 is reachable and is not in the document.** It was reachable on the old route too, and the
  statuses are unchanged, but five of the sites now carry the typed error body and some of those
  bodies contain raw sqlx text. Nothing in the UI reads a 500 body.
- **A report is one portfolio.** There is no cross-portfolio report and no asset subset.
- **No PDF, no tax computation, no tax rates.** This answers "what did I sell, at what basis,
  with what gain, and what did I still hold". What to do with that is a tax question, and the app
  does not answer it.
- **The undated response is unchanged in shape.** The typed report did not replace the legacy
  summary object; the printable statement and the holdings breakdown read fields the report does
  not carry.

## What is verified, and what is not

**Covered by `cargo test` (313 tests, green as of this note), from `backend_rust/`:**

- `tests/tax_report_test.rs` (19) — the dated report summing its own rows and agreeing with the
  undated summary, the window selecting the sells but not the lots behind them, a window ending on
  a non-trading day reporting the close that actually priced it, the FX branch converting to the
  base currency and reporting the rate date it used, a same-currency asset never claiming a rate
  date, the invalid windows each being a 422 that names the parameter, `from` alone running
  through today, the undated shape the statement reads, the undated strategy being normalized and
  echoed, the 422 body's shape, the caps and the pair-not-symbol rule, the 422/503 and `retryable`
  mapping, and the deadline failing closed with no partial payload (exercised at `Duration::ZERO`,
  not at 20 s).
- The `replay_at` unit tests in `src/engines/tax_engine.rs` — `valuation_date` actually moving the
  conversion rate (two different seeded rates), a split dated after the window end not moving an
  earlier window while one inside it still rescales, the inclusive end boundary, the empty window
  costing no outbound call, and `replay_at` agreeing with `calculate_lots` when both look at today.
- `tests/report_csv_test.rs` (12) — the header, its order, and that no comment row precedes it;
  `lot_pnl` summed over `SOLD_LOT` rows equalling `asset_realized_in_window`; two dispatches over
  unchanged data being byte-identical; a date cell reading exactly the text the JSON field reads,
  including a sub-second timestamp; each RFC 4180 trigger forcing quoting on its own; the
  server's own filename, including both filename branches — a `to`-only request naming the first
  transaction in scope, and `inception` only when there is no transaction in scope; `format=csv`
  without a window being a 422 naming `format`, and `format=json` being the accepted no-op it now
  is; a window with no rows being a header and nothing else.
- `tests/analytics_test.rs` — that a lot reports the quantity that was bought and the buy it came
  from. `tests/isolation_test.rs` — the dated variant of the access rule, which the grep-based
  gate cannot see on an existing route. `tests/route_coverage_test.rs` — that no route was added.
- `bash scripts/check_access_gate.sh` (15 handlers inspected, 15 cleared), `cargo clippy
  --all-targets`, `cargo build --release`, and `cargo run --bin backend_rust -- --print-openapi`
  diffing empty against `docs/openapi/openapi.json`.

**Covered by `npm test` (22 files, 203 tests), from `frontend/`:** the default window being
January 1 through today rather than December 31 (a December default is a future date and the route
answers 422), the date being derived from UTC parts rather than the local calendar and both date
inputs being capped at the server's today — that is the bug that made the tab fail on open for
every user east of UTC — a quarter that has not begun being disabled and a quarter in progress
clamping to today, the download link carrying exactly the window on screen and disappearing when
the report does, the payload totals printing instead of a sum of the rows (the shared fixture has
`lot_pnl` summing to 1800.00 against `realized_in_window` of 1234.56, so a renderer that adds
fails), the run context naming the applied window and `inception`, the two error shapes including
a capped list being reported as capped rather than as the whole list, the portfolio-switch guard
that stops portfolio B's numbers rendering under portfolio A's name, the table being removed when
a same-selection refetch fails, and a panel rather than a blank page when the body is not the
typed error at all. `npm run lint` is at zero errors and `npm run typecheck` is clean.

**Not verified, and recorded as unwritable rather than assumed.** There is no provider seam in
the test suite — the pricing service takes no arguments and points at a hardcoded Yahoo URL, and
there is no HTTP mock — so three things have no hermetic test: a genuinely missing price, a
genuinely missing rate, and a real provider outage. What is pinned instead is the 422 body's
shape, the caps, the pair-not-symbol rule, and the `PricingError`-to-status mapping; the route's
*decision* to take the missing-rate branch is unproven. The reason is mechanical and is written
out at the bottom of `backend_rust/tests/tax_report_test.rs`, and it is the same fact stated
under "A missing rate is not reliably a 422" above: without asking the provider, a rate that does
not exist is indistinguishable from an outage.

**Also not verified by any gate:** that a real browser carries the session cookie into the
download. That is a manual step, and it is the one thing no CI job here can do. The walkthrough
is `docs/auth-testing.md` §5, *Downloading the tax report*. Treat it as a required step before
relying on the feature.

## Not changed here

- No new route, no new table, no new column, no DDL. The access level is the same
  `Access::Viewer` call every data route goes through.
- The undated `tax-summary` response keeps its shape and its keys. The printable statement and
  `PortfolioDetail` keep reading `assets[].tax_lots`, `current_shares` and
  `total_portfolio_value`.
- The undated path still syncs splits inside the read (#50) and still turns a pricing failure into
  a zero with a 200 response. A dated report is where a pricing failure becomes an error instead.
- No PDF, no server-side PDF dependency, no tax computation, no cross-portfolio report, no asset
  subset, no router and no deep link to a window.
- The printable statement's tab, print stylesheet and walkthrough are untouched. The report tab
  has no `[data-print="statement"]` root, so it prints like the other four tabs, exactly as
  before.
