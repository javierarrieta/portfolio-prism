# Stage 6 — Comprehension Test

*   **Target:** `docs/superpowers/specs/2026-11-09-tax-report-and-statement-design.md`
*   **Ranges:** FeatureRange 1–414, DetailedRange 415–689
*   **Reviewer:** `egm-comprehension`, fresh context, no rubric applied
*   **Date:** 2026-11-09

## What this proposes

Two owner-facing reports built on one new capability: a dated replay of the tax-lot engine.
The tax report is a CSV listing every sell inside a `[start, end]` window at lot granularity
plus every lot still open at `end` with its unrealized gain/loss; the portfolio statement is
a printable page showing realized P&L cumulative from inception and unrealized P&L as of a
chosen date, for one portfolio, optionally narrowed to an asset subset. Both are driven by
one new backend shape: `GET /api/portfolios/<id>/tax-summary` gains optional `from`/`to`,
and returns a typed payload (run context, server-computed totals, and
`SOLD_LOT`/`OPEN_LOT`/`UNMATCHED` rows) in which every money value is a decimal string, so
that neither renderer does arithmetic. Three pre-existing pricing defects (dated FX,
`historical_prices` uniqueness, split writes on a read path) are prerequisites that land
first and separately, and work is cut into six slices ordered by network exposure.

## How the existing system works, per this document

`GET /api/portfolios/<id>/tax-summary` computes realized P&L over *all* transactions and
prices everything at the latest available close, with no date parameter anywhere; it requires
`Access::Viewer`, declares `body = serde_json::Value`, and silently falls back to FIFO for an
unrecognized strategy. `CurrencyService::get_rate` keys its cache on the requested date but
returns the latest close, and builds the wrong Yahoo FX symbol, so past-dated multi-currency
figures are already wrong; `get_price` returns `Decimal::ZERO` on any failure and
`get_historical_prices` swallows errors into an `Ok`, so "no data" and "could not ask" are
indistinguishable. `historical_prices` has no primary key or unique constraint, so
`INSERT OR REPLACE` always appends and "the close on date X" is an arbitrary row;
`sync_splits_for_asset` runs inside read paths with a non-transactional
`SELECT COUNT(*)`-then-`INSERT`. The only export today is one sell at a time via
`frontend/src/utils/sellPnlCsv.ts` reached from `SellPnlModal.tsx`, whose money values pass
through `Number()` in `normalizeTaxSummary`/`normalizeSellPnl`; there is no `@media print`
CSS anywhere; and `tests/route_coverage_test.rs` hard-asserts the size of the `/api` surface
(15 / 13).

Every section was paraphrasable; no section was unreadable.

## Findings

### Blocker 1 — P3 has no target-file entry and never says where the split call goes instead

> "**P3 — no split writes on a read path.** `sync_splits_for_asset` must not run inside a
> report request. … Moving it off the read path is a prerequisite; a report that detects an
> unrecorded split inside the window names it rather than fixing it."

P3 is one of three changes that must land before slice 1, yet none of the 25 target-file
entries mentions `sync_splits_for_asset`. File 1 covers P1/P4/work-set/cache, file 2 covers
P2, file 6 covers the route's dated path and never mentions splits. The document never says
where the call goes instead — background job, mutation endpoint, explicit refresh action, or
deleted. A fresh engineer cannot execute slice 0.

Two unstated consequences: `tax-summary` calls it per asset (`analytics.rs:65`), so removing
it from the undated path changes existing cost-basis numbers for portfolios relying on
auto-detected splits, colliding with Goal 7; and if it is removed only on the dated path, the
same route computes two different cost bases depending on whether `from`/`to` are present,
which the document neither forbids nor states.

### Blocker 2 — Single-parameter date cases are undefined, and one is the statement's primary case

> "`from` and `to` are `YYYY-MM-DD`, both inclusive … Absent parameters mean today's
> behaviour."

The Overview requires realized P&L "cumulative from inception to a chosen date", which is
`to` present with `from` absent — but "Absent parameters mean today's behaviour" can be read
as "`from` absent ⇒ today's behaviour". Nothing says whether `to` alone is legal, whether it
defaults `from` to inception, or whether it is a 422. Symmetrically `from` alone is
undefined, and the future-date rule covers only `to`: *"a `to` in the future, and an
unparseable date are all HTTP 422"* says nothing about a `from` in the future. The route
contract is what files 6, 7, 8, 9, 10, 13, 14, 17 and 18 all depend on.

### Major 3 — The default range conflicts with the future-date rule

> "Quarter presets fill `start` and `end`; the default is the current year." and "a `to` in
> the future … HTTP 422."

If the default is Jan 1 – Dec 31 of the current year, the default request has a future `to`
and is rejected. The document never says which year the quarter presets belong to (current
year's Q1–Q4 or trailing four quarters), nor what a preset with a future `to` should do.
Goal 3 also counts "the four common cases" while file 18 specifies *"four quarter presets and
a current-year preset"* — five.

### Major 4 — No CSV column names, order, or header text, and a test references an undefined column

> "one row per `ReportRow`, sell context repeated on sold-lot rows, a header comment naming
> which columns are legal to sum, the run context … as columns, and money written verbatim
> from the payload strings."

The CSV is the deliverable and is archived by the owner, yet no column names, order, or
header-comment text appear anywhere. File 15 pins *"the `lot_pnl` column sums to the
payload's realized total"* — a column this document never defines; the payload calls the
concept `realized_in_window` / `realized_cumulative`, and `SOLD_LOT` only says "mirroring
LotMatch" (whose field is `pnl`). Goal 5's summable-columns rule also cannot be written
without the list.

### Major 5 — Three quantities the plan depends on are named but never valued

> "the last close **on or before** the requested date, within a bounded lookback."

The bounded lookback (days, and whether FX uses the same bound), the per-report outbound
budget, and the FX cache bound are all named and never given a value. The document asserts
the budget is derivable — *"the same cap the pricing section needs"* — but the pricing
section gives no cap either. Each is a number an implementer must invent, and the lookback
directly changes reported numbers: a delisted fund priced three years late versus rejected.

### Major 6 — The strict-validation precedent is false, its target file is missing, and an existing test comment is already wrong

> "The single-sell P&L route already normalizes strictly, so the app currently runs two rules
> on one axis."

`analytics.rs:217-221` ends in `_ => "FIFO".to_string()` — the same silent fallback the
document condemns. The premise for the strict-validation decision is wrong, and the decision
has no target-file entry: file 6 describes only the `tax-summary` handler, so whether
`/transactions/<id>/pnl` changes behaviour is unstated, and that would be an existing-endpoint
change Goal 7 limits to the FX fix. Related: file 10 says to extend the `auth_test.rs` sweep,
but the comment at `auth_test.rs:43-45` ("The two tax routes require a strategy; without one
the handler answers 422") is already false today, so the sweep cannot be extended to prove
it. **Verified: `analytics.rs:217-221` and `auth_test.rs:43-45` both as described.**

### Major 7 — The verification plan names a command the file list cannot produce

> "`bash scripts/check_print_styles.sh` and `npm run e2e:statement`."

`frontend/package.json` is not among the 25 entries and today defines only
`"e2e:sell-pnl": "bash e2e/run.sh"`; `e2e/run.sh` hardcodes `node e2e/sell-pnl.smoke.mjs`.
Nothing adds an `e2e:statement` script. The print line also mixes working-directory
conventions: the created path is `frontend/scripts/check_print_styles.sh` but it is invoked
as `scripts/check_print_styles.sh`, while the adjacent backend line is only correct from
`backend_rust/`.

### Major 8 — Four files must agree on selectors no file names

> (file 21) "assert the statement root carries the hook the print stylesheet targets";
> (file 22) "asserting the print stylesheet exists and hides the chrome selectors".

The statement-root class/attribute and the app-chrome selectors are never enumerated, and
`PortfolioDetail.tsx` / `App.tsx` class names are not quoted. A CI grep whose pattern is
invented at implementation time can pass while hiding nothing — the failure mode the plan's
own `check_access_gate.sh` precedent exists to prevent.

### Major 9 — P4's effect on the undated path is stated as a side effect but never specified

> "Both must return a real result type, and that change also affects the existing screen."

The document never says what the undated path does after P4: whether an unobtainable price
now returns 422/503, whether fail-collect applies there, or which status it uses — the error
contract is written for the report path only. File 17 surfaces the typed error body only for
the report fetch. Existing tests relying on the zero fallback are not listed; file 11 covers
`sell_pnl_test.rs` only. This is a user-visible change on a screen the plan claims is
untouched.

### Major 10 — `totals: portfolio rollup` is the one field group never enumerated

> "ReportResponse  context, asset_totals: [AssetTotals], totals: portfolio rollup, rows:
> [ReportRow], strategy, threshold_days"

The prose says "plus portfolio-level totals of each", implying five fields without naming
them, and the existing endpoint's key names (`total_realized_pnl`, `total_unrealized_pnl`,
`total_portfolio_value`) differ from the row names, so the frontend cannot type or render the
headline figures from this document. Also unspecified: types for `price_date_used`,
`fx_rate_date_used`, `fx_applied`, `split_applied`, and whether `fx_rate_date_used` is null
when `fx_applied` is false — the "nulls are nulls" rule is stated only for `UNMATCHED`.
`ReportResponse` repeats `strategy`/`threshold_days` already in `ReportContext` with no
statement of which is authoritative.

### Major 11 — The `TaxLot` fix changes a live response and pins an existing test, neither of which is listed

> "Fix `TaxLot`: `original_qty` and `remaining_qty` are both set to the already-decremented
> `lot.qty` …"

Unlike P1, which names the tests that move, no entry names the existing assertions.
`backend_rust/tests/analytics_test.rs:51-52` asserts `original_qty == "100.0"` and
`remaining_qty == "100.0"` — it currently pins the buggy equality. **Verified.**
Additionally `TaxLot` is defined in `schemas.rs:90`, while the fix is listed under file 4
(`tax_engine.rs`), and file 5 describes its `schemas.rs` changes as the report payload only,
so the file that must gain `buy_transaction_id` is not identified.

### Nit 12 — `UNMATCHED` is never defined in this document

> "UNMATCHED: sell id, unmatched_qty, money cells null".

The definition lives only in the predecessor ("sold quantity no buy lot covers"). A reader
who does not open it cannot know when to emit one — fully unmatched sells only, or a partial
remainder per sell — nor which cells "money cells" enumerates.

### Nit 13 — Unnamed pointers: the prior review record and "the shared formatter"

> "following the pattern the prior review record established after the sell-pnl suite went
> from 1.33 s to 5.67 s by reaching the network."

The record is never named: `docs/superpowers/reviews/2026-11-05-sell-pnl-breakdown.md`.
Same class: *"the shared formatter"* (file 19, Goal 6) is unnamed and is easy to confuse with
the opposite convention the CSV section mandates — `sellPnlCsv.ts:7` builds *"from the
normalized numbers, never from `utils/formatters`"*. Naming `frontend/src/utils/formatters.ts`
settles both renderers.

### Nit 14 — Two of the three CSV conventions said to be "reused" cannot be reused as written

> "The CSV builder follows the existing single-sell export's conventions — UTF-8 BOM, the
> formula-lead guard on free-text cells, exponent-free numeric output".

The BOM is owned by `downloadCsv` (`utils/download.ts:25`), which is why file 15 pins "no BOM
in the builder output"; the sentence invites a builder-level BOM the test then forbids. And
the exponent-free rule is `const num = (v: number): string` — it takes a `number`, while the
report builder's central rule is that money never passes through `Number()`, so it must be
re-derived for strings rather than reused.

### Nit 15 — The manual-verification cross-reference points at something not in that file

> "a real OIDC login and a real browser print preview, both listed in `docs/auth-testing.md`
> as the two things CI cannot check."

`docs/auth-testing.md` lists a real login and whether the provider ends its own session on
logout. The browser print preview is new to this document and is not in that file.

### Nit 16 — Three under-specifications in the print slice

The import entry point is unnamed (`main.tsx` vs `App.tsx`, which file 17 also modifies); the
asset-subset filter is by definition a control yet "the printed subtree contains no buttons or
inputs" never says the filter sits outside the printed subtree; and "within a stated page
count" never states the count.

### Nit 17 — The same quantity appears under two names with no mapping

Rows carry `latent_gain_loss` / `latent_roi` while totals carry `realized`/`unrealized_at_end`;
rows carry `price_date` while totals carry `price_date_used`. Since Goal 2 asserts row-sum
equals total, stating that `price_date` is the asset-level `price_date_used` repeated (one
close per asset) removes a real question about whether per-lot prices are possible.
