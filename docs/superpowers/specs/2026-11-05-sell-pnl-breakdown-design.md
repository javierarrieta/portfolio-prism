# Sell Transaction P&L Breakdown Design

**Date**: 2026-11-05
**Status**: Proposed
**Implementation plan**: `docs/superpowers/plans/2026-11-05-sell-pnl-breakdown.md`
**Review record**: `docs/superpowers/reviews/2026-11-05-sell-pnl-breakdown.md` — decisions taken, proposals rejected, and what was measured rather than assumed

**Normative for:** schemas and their JSON keys, the route contract, the reconciliation invariant,
and the UI contract (what each state must say). The plan is normative for task sequencing, file
lists and the test inventory. Where the two disagree, this document wins for behaviour and the plan
wins for order of work; the plan's *Deviations* section lists the sanctioned exceptions.

## Overview

Clicking a SELL transaction in the Transaction Ledger opens a detail view showing the realized
P&L for that single sale and the specific buy lots it consumed, with P&L per lot.

**Problem.** The Holdings & Tax lots tab reports one realized-P&L number per asset. A user who
wants to know why a particular sale produced its number — which purchases it consumed, what they
cost, how much of the sale went unmatched — cannot find out anywhere in the app, and the engine
discards exactly that information while it computes. Without it, a wrong figure and an
unusual-but-correct figure look identical on screen.

The lot attribution is **read-only and derived from the currently selected accounting strategy**
(FIFO / LIFO / HYBRID). The user cannot override which lots a sale consumed; the view explains
what the engine already did, and its numbers reconcile with the realized P&L already displayed
per asset in the Holdings tab. That reconciliation rests on two things stated below: one shared
replay (see *Single replay*) and one read-only ordering guarantee (see *New endpoint*).

## Requirements

- Clicking a SELL row in the Transaction Ledger shows realized P&L for that sale alone.
- The view lists the buy lots consumed, in consumption order, with per-lot cost basis, proceeds
  and P&L.
- Per-sell realized P&L across all sells of an asset **must sum exactly** to that asset's
  `realized_pnl` in `/tax-summary`. This is the central invariant. It is exact because nothing is
  quantised (see *New schemas* → numeric precision), not because a tolerance is applied.
- Read-only: no lot-override, no new persisted state, no migration.
- Matching reflects the strategy and `threshold_days` currently selected in the Holdings tab.
- Cases where the engine currently discards information silently (unmatched quantity, sells with
  no prior buys, split-adjusted prices, FX conversion) must be surfaced in the UI, not hidden.
- No new live price fetch: realized P&L does not depend on the current quote.

## Why the matching cannot be computed per-sell in isolation

Lot eligibility is order-dependent state, not a property of one row:

- Stock splits rescale **every** existing lot's quantity and unit cost
  (`tax_engine.rs`, the `SPLIT` branch), so a lot's cost basis at the time of a sale depends on
  every split before it.
- HYBRID buckets lots by age **relative to each individual sell's date**, so the consumption
  order depends on the sell being examined.
- Sells consume lots that earlier sells have already partially drained.

Therefore "show me the lots for sell #42" requires replaying that asset's full transaction
history with the same sort order, the same split pass and the same strategy. Replaying the whole
history and selecting group #42 is identical to replaying only the prefix — a sell's matches
depend on nothing after it — and it is the *same call* `calculate_lots` makes. That identity is
the design's safety property, and it is why the engine is refactored rather than duplicated.

## Backend Changes

### Engine: `backend_rust/src/engines/tax_engine.rs`

**Lot identity.** `InternalLot` gains `source_tx_id: i32`, populated from `tx.id` in the BUY
branch. Without it two buys on the same date at the same price are indistinguishable and the
view cannot say which purchase a cost basis came from.

**Single replay.** The body of the `for tx in sorted_txs` loop moves into a private `replay()`
returning `(buy_lots, Vec<SellBreakdown>)`. Two public entry points sit on top of it:

- `calculate_lots(...)` — unchanged signature, unchanged `AssetTaxSummary`. Calls `replay()`,
  builds open lots from the returned `buy_lots` exactly as today, and **derives**
  `AssetTaxSummary.realized_pnl` as `Σ breakdowns[i].realized_pnl`. The engine-wide accumulator
  is removed, not kept alongside the breakdowns.
- `sell_breakdown(sell_tx_id, ...)` — calls `replay()` with the same arguments, returns the one
  `SellBreakdown` whose `sell_transaction_id` matches.

One loop, one sort order, one split pass, one FX pass. Deriving the total from the breakdowns is
what makes the invariant structural: realized P&L is computed in exactly one place, so the two
entry points have nothing to disagree about. Returning `realized_pnl` from `replay()` and
letting `calculate_lots` keep using it while the modal reads the breakdowns would leave two
independent paths and would make the claim below false — this is the one part of the refactor
that must not be simplified back. The two entry points cannot disagree because they are the same
code doing the same arithmetic once.

**The `continue` on no eligible lots must go.** Today a sell with no eligible lots is skipped
silently. It must instead emit a `SellBreakdown` with empty `matches` and
`unmatched_qty == sold_qty`. Likewise the leftover `qty_to_sell` after the consumption loop
becomes `unmatched_qty` instead of vanishing. Both are cases the engine hides today and the view
exists to expose.

### New schemas: `backend_rust/src/schemas.rs`

Every `Decimal` field uses `#[serde(with = "decimal_json")]`, as all existing money fields do.

**Numeric precision: no field is quantised.** Values stay full-precision `Decimal` end to end —
no `round_dp`, no `normalize()`, no fixed scale — and totals are defined as the sum of their
parts: a sell's `proceeds`, `cost_basis` and `realized_pnl` are the sums of its `matches`, and
the asset-level `realized_pnl` in `/tax-summary` is the sum of the per-sell breakdowns. Under
that rule the central invariant is exact by definition and needs no tolerance. Quantising per lot
would make `Σ rounded lots ≠ unrounded total` and turn the invariant into a tolerance argument,
so it is ruled out here, not left to implementation.

`decimal_json` serializes `Decimal::to_string()`, which preserves the scale the arithmetic
produced. The number of trailing zeros is therefore a by-product of the computation and **not
part of the contract**: `2000`, `2000.0` and `2000.000` are all valid renderings of the same
value, and an empty sum serializes as `"0"` (as `/tax-summary` already does). Consumers parse
decimals to numbers — the frontend does it in `normalizeSellPnl` — so nothing may assert on the
string form. Tests compare parsed `Decimal`s.

Measured against the pinned toolchain, for the FIFO example further down this document: the engine
emits `proceeds: "10400.00"`, `cost_basis: "8400.00"`, `realized_pnl: "2000.00"` and an `roi` of
`0.2380952380952380952380952381` — twenty-eight fractional digits, which is what `formatPercent`
renders as `+23.8%`. Not the `"10400.0"` the example below prints, and not the `"2000.000"` two
reviewers predicted; those were derivations, this is a compiled run. The sum of the two per-lot
P&L values compared equal to the sell total, which is the claim the no-quantisation rule above is
made to protect.

```rust
pub struct LotMatch {
    pub buy_transaction_id: i32,        // the BUY row in the ledger
    pub buy_date: DateTime<Utc>,
    pub buy_price: Decimal,             // split-adjusted, base currency
    pub unit_cost: Decimal,             // split-adjusted cost/share, incl. amortised buy fee
    pub matched_qty: Decimal,           // shares taken from this lot by this sell
    pub lot_qty_before_match: Decimal,  // distinguishes "drained the lot" from "partially used"
    pub cost_basis: Decimal,
    pub proceeds: Decimal,              // net of this sell's fee
    pub pnl: Decimal,
    pub holding_days: i64,
    pub term: Option<String>,           // "SHORT" | "LONG", populated for HYBRID only
}

pub struct SellBreakdown {
    pub sell_transaction_id: i32,
    pub sell_date: DateTime<Utc>,
    pub sold_qty: Decimal,
    pub matched_qty: Decimal,
    pub unmatched_qty: Decimal,         // units sold with no cost basis behind them
    pub proceeds: Decimal,
    pub cost_basis: Decimal,
    pub realized_pnl: Decimal,
    pub matches: Vec<LotMatch>,         // consumption order = FIFO / LIFO / HYBRID order
}

pub struct SellPnlResponse {
    // context, resolved server-side from the transaction's own asset and portfolio
    pub transaction_id: i32,            // the sell; the JSON key is `transaction_id`
    pub asset_id: i32,
    pub symbol: String,
    pub asset_type: String,
    pub strategy: String,               // the strategy actually applied (see New endpoint)
    pub threshold_days: i64,
    pub currency: String,               // portfolio base currency
    pub asset_currency: String,
    pub fx_applied: bool,
    // the breakdown, flattened onto the same object
    pub sell_date: DateTime<Utc>,
    pub sold_qty: Decimal,
    pub price: Decimal,                 // as entered, in asset_currency
    pub fee: Decimal,                   // as entered, in asset_currency
    pub proceeds: Decimal,
    pub cost_basis: Decimal,
    pub realized_pnl: Decimal,
    pub roi: Option<Decimal>,           // #[serde(with = "rust_decimal::serde::option_str")]
    pub matched_qty: Decimal,
    pub unmatched_qty: Decimal,
    pub matches: Vec<LotMatch>,
}
```

`Option<Decimal>` uses `rust_decimal::serde::option_str`, which the pinned `rust_decimal` already
provides under its `serde` feature (`Some` → string, `None` → null). Do not hand-write a visitor
for it.

**Which types are published.** `SellPnlResponse` and `LotMatch` go into the OpenAPI components.
`SellBreakdown` is engine-internal and is **not** registered — no response body is of that type,
and publishing it would put an unconsumed type in the committed contract and in the generated
`api.d.ts`.

**Naming, stated once.** `SellBreakdown` is the engine type, `SellPnlResponse` is the wire type,
and the frontend mirrors the wire type as `SellPnlBreakdown`. The JSON keys on `SellPnlResponse`
are the contract; `sell_transaction_id` exists only on the engine type.

`AssetTaxSummary` does **not** change, so `/tax-summary`, its OpenAPI entry and the existing
frontend types are untouched.

**`term` is HYBRID-only on purpose.** Short/long bucketing exists only in the HYBRID branch and
is defined by `hybrid_threshold_days`, a HYBRID tuning knob. Reporting "LONG TERM" under FIFO
would present a strategy parameter as a tax fact. `holding_days` is returned for every strategy
— it is a plain date difference the user can judge unaided.

### New endpoint

`GET /api/transactions/<id>/pnl?strategy=FIFO&threshold_days=30`

Added in `backend_rust/src/api_routes/analytics.rs` (an analytical read needing
`CurrencyService`; Rocket's `/transactions/<id>` and `/transactions/<id>/pnl` do not collide).
The portfolio is not in the path: base currency is resolved from the transaction's asset →
`assets.portfolio_id` → `portfolios.base_currency`, so the answer can never disagree with the
portfolio that owns the trade.

Defaults `strategy=FIFO`, `threshold_days=30`, identical to `/tax-summary` — a different default
would make the modal contradict the Holdings tab.

**Which transactions the replay sees.** `calculate_lots` is keyed by `symbol` + `asset_type` across
a portfolio; this route is keyed by a `transaction_id`. It resolves the transaction's own
`asset_id` and replays **that asset's** transactions only (`SELECT * FROM transactions WHERE
asset_id = ?`), never every asset that happens to share a symbol. Two assets with the same symbol
in different portfolios therefore never mix, and the numbers a user sees in the modal come from the
same row set the Holdings tab used for that asset.

**The response reports the strategy it applied, not the string it received.** The engine's
dispatch ends in a `_ => FIFO` fallback, so an unrecognized `?strategy=` value would otherwise
return 200 echoing `"xyz"` while the attribution used FIFO — and the modal's claim to name the
method it used would be false. Normalize once at the boundary: upper-case the value, map anything
unrecognized to `FIFO`, and use that single resolved value for both the dispatch and the
response. No new 400s and no range policy on `threshold_days`; the frontend owns those query
strings and the fallback is documented behaviour.

```json
{ "transaction_id": 42, "asset_id": 7, "symbol": "AAPL", "asset_type": "STOCK",
  "strategy": "FIFO", "threshold_days": 30,
  "currency": "USD", "asset_currency": "USD", "fx_applied": false,
  "sell_date": "2024-12-01T00:00:00Z", "sold_qty": "80.0", "price": "130.0", "fee": "0.0",
  "proceeds": "10400.0", "cost_basis": "8400.0", "realized_pnl": "2000.0", "roi": "0.238",
  "matched_qty": "80.0", "unmatched_qty": "0.0",
  "matches": [
    { "buy_transaction_id": 1, "buy_date": "2024-01-01T00:00:00Z", "buy_price": "100.0",
      "unit_cost": "100.0", "matched_qty": "60.0", "lot_qty_before_match": "60.0",
      "cost_basis": "6000.0", "proceeds": "7800.0", "pnl": "1800.0",
      "holding_days": 335, "term": null },
    { "buy_transaction_id": 2, "buy_date": "2024-06-01T00:00:00Z", "buy_price": "120.0",
      "unit_cost": "120.0", "matched_qty": "20.0", "lot_qty_before_match": "40.0",
      "cost_basis": "2400.0", "proceeds": "2600.0", "pnl": "200.0",
      "holding_days": 183, "term": null } ] }
```

Field notes:

- `proceeds`, `cost_basis` and `realized_pnl` cover **matched units only**. Unmatched units
  contribute neither proceeds nor cost basis, so `proceeds ≠ sold_qty × price − fee` whenever
  `unmatched_qty > 0`. The modal must not present those three numbers as though they multiply
  out; see *Edge-case display*.
- `roi` is `Option<Decimal>`: `cost_basis` is zero whenever nothing matched, and dividing there
  would produce a meaningless ratio. It renders as `—`, never `0%` and never `Infinity%`.
- `price` and `fee` are the raw as-entered values in `asset_currency`. Every other money field is
  in `currency` (the portfolio base currency). Because one payload carries two currencies, **every
  rendered amount carries an explicit currency suffix** — including the per-lot table, whose
  `buy_price` is split-adjusted base currency sitting next to an as-entered `price`. A reader must
  never have to infer which currency a column is in.
- `fx_applied` is `asset_currency != currency`. The example above is a same-currency asset, so it
  shows `false`; for a foreign-currency asset `price` and `fee` stay in `asset_currency` while
  every other money field is converted and `fx_applied` is `true`.

**This endpoint never calls `get_price`.** Realized P&L needs no current quote, so unlike
`/tax-summary` it performs no price fetch. FX rates for a non-base-currency asset come from the
existing in-memory rate cache (`currency_service.rs`, `get_rate`).

**It is also strictly read-only, and that has one consequence worth stating.** `/tax-summary`
calls `currency_service.sync_splits_for_asset` before it computes lots (`analytics.rs`), and
that call inserts newly detected `SPLIT` transactions into the database — throttled to once per
symbol per 30 days, with its errors discarded. This endpoint deliberately does not: it reads
`SELECT * FROM transactions WHERE asset_id = ?` and writes nothing. The two entry points share
one replay loop, but they are guaranteed to see the same transaction set only because the UI
always fetches `/tax-summary` before a SELL row can be clicked: `fetchPortfolioData` awaits
tax-summary (`App.tsx:83`) before it fetches the ledger (`App.tsx:89`), the ledger tab is gated
on a selected portfolio, and every strategy change refetches. That is a property of the current
UI, not of this design.

The ordering is pinned by a route test (see *Testing → Route*) rather than by construction. A
future caller that reaches this endpoint without a preceding `/tax-summary` for the same asset
must either run the sync itself or accept that a split detected in between is missing from its
replay. Adding the sync to this handler was considered and rejected: it would make a GET a
writer, contradict the read-only requirement above, and put a Yahoo fetch (10 s timeout, three
retries) on the one interaction where the user expects instant feedback.

### Error handling

| Status | Condition |
|---|---|
| 404 | Transaction id does not exist |
| 404 | Transaction exists but its asset row is missing (orphaned) |
| 404 | Asset exists but its portfolio row is missing (orphaned) |
| 400 | Transaction type is not `SELL` (defensive; the UI only makes SELL rows clickable) |
| 500 | Engine or FX failure, matching the existing `map_err(\|_\| Status::InternalServerError)` house style |

An unrecognized `strategy` is **not** an error: it resolves to FIFO (see above).

**Trust model.** This is a single-user local application with no authentication anywhere in the
API; a bare transaction id in the path is consistent with the existing
`DELETE /api/transactions/<id>`. Id enumeration is not treated as a threat here, and this
endpoint adds no scoping that the rest of the API does not already have.

## Frontend Changes

### New component: `TransactionLedger` (`frontend/src/components/TransactionLedger.tsx`)

Extracted from the inline ledger block in `frontend/src/App.tsx` (the "Transaction History" card
under `activeTab === 'ledger'`). `App.tsx` is already 546 lines and this change adds row-click,
modal state and a fetch to that block. The extraction ships as its own `refactor:` commit,
separate from the behaviour change.

Props: `transactions`, `assets`, `onDeleteTransaction`, `onSelectSell(txId)`. The ledger renders
currency per row from that row's asset (`asset?.currency || 'USD'`) and the modal is rendered by
`App`, not by the ledger, so `currency`, `strategy` and `thresholdDays` are deliberately **not**
props — the ledger would only forward them.

### SELL row affordance

SELL rows get an explicit affordance, not a bare `onClick` on `<tr>`: a small receipt icon button
in the Action column. A clickable row nobody can see is undiscoverable, and `<tr onClick>` with no
focusable element is unreachable by keyboard. The button provides tab order and Enter. The row
also carries `onClick` for mouse users, but the button is what makes it operable.

BUY and SPLIT rows are unchanged and show no affordance.

### New component: `SellPnlModal` (`frontend/src/components/SellPnlModal.tsx`)

Fetches `/api/transactions/<id>/pnl` on mount. `Loader2` while loading (existing pattern in
`App.tsx`), error state with retry, then header → summary → matches table → footnotes.

```text
Sell P&L — AAPL · 01 Dec 2024 · Method: FIFO
Sold 80.0 AAPL @ 130.00 USD (as entered) · Fee 0.00 USD
Proceeds 10,400.00 USD · Cost basis 8,400.00 USD
Realized P&L  +2,000.00 USD   (+23.8%)

Lots consumed (FIFO order) — all amounts in USD, the portfolio base currency
Buy date     Buy px     Qty   Cost basis    Proceeds      P&L   Held
2024-01-01   100.00      60     6,000.00     7,800.00  +1,800.00  335d
2024-06-01   120.00      20     2,400.00     2,600.00    +200.00  183d
```

Every amount carries its currency, because the payload mixes two of them. `2000 / 8400 = 0.238095…`
renders as `+23.8%` through `formatPercent`; the example and the mock must agree, since both are
copied into tests.

Modal styling follows the existing overlay pattern (`position: fixed` overlay + `.glass-card`,
`zIndex: 100`) used by the asset and transaction modals in `PortfolioDetail.tsx`. Keyboard support
matches that existing pattern: the trigger is a real `<button>` with an `aria-label`, and the
button's handler calls `stopPropagation` so the button and the row `onClick` do not both fire.
A focus trap is not introduced here — neither existing modal has one — and is left as a follow-up
for all three together.

### State and types

- `App.tsx`: `const [sellPnlTxId, setSellPnlTxId] = useState<number | null>(null)`. The modal
  receives the same `strategy` / `thresholdDays` state already passed to `PortfolioDetail`.
- `frontend/src/types.ts`: add `LotMatch` and `SellPnlBreakdown` mirroring the backend schemas,
  with decimals as `number` per the documented contract.
- `frontend/src/utils/decimal.ts`: add `normalizeSellPnl` so decimal strings convert to numbers at
  the fetch boundary, like `normalizeTaxSummary` and `normalizeTransactionList`.

## Edge-case display

Each row below is something the engine does silently today.

| Condition | Display |
|---|---|
| `unmatched_qty > 0` | Amber banner: *"{unmatched_qty} of {sold_qty} units had no buy lot on or before this date. Neither their cost basis nor their proceeds are included in this figure."* The engine drops unmatched units from **both** sides, so the figure is incomplete rather than biased in a stated direction — do not write "overstated" or "understated". A test asserts this wording, not only the counts. |
| `matches` empty | *"No buy lots were available to match on or before this date, so nothing was matched and realized P&L is 0.00."* Not "no data". **Available**, not *existed*: lots may have existed and earlier sells already drained them, which the payload cannot distinguish. |
| any `SPLIT` on the same asset dated ≤ the sell date | Footnote: *"Buy prices and unit costs are split-adjusted, not the prices as entered."* Detected client-side from the transaction list already in memory, **filtered to the sell's own `asset_id`**. A split that never touched a consumed lot still triggers the footnote; accepted, rather than adding a server-computed flag for a display nicety. |
| `fx_applied` | Footnote: *"Cost basis and proceeds were converted to {currency} at the latest available {asset_currency}→{currency} rate."* Not "at each transaction's date rate": `get_rate` uses the requested date only as a cache key and returns the last non-null close in the series it fetched, so every transaction in a replay converts at effectively the same rate. The other wording would misdescribe the very conversion this footnote discloses. |
| always | Header names the strategy actually applied (and the threshold when HYBRID), plus: *"Attribution follows the {strategy} method. A different method would consume different lots."* Do not name one specific alternative: under HYBRID, "LIFO would attribute different lots" drops the threshold and implies FIFO is the only other option. |
| `roi` null | ROI renders as `—`, never `0%` and never `Infinity%`. |

## Alternatives considered

- **Duplicate a per-sell matcher in a new module.** Rejected: two implementations of lot
  consumption that must agree forever is the failure this feature exists to avoid.
- **Recompute the breakdown in the browser** from data the Holdings tab already fetches. Rejected:
  the browser would need the engine's sort order, split rescaling, fee amortisation, FX conversion
  and HYBRID bucketing — the same second implementation, and one that cannot be reconciled against
  the first.
- **Persist a `sell_lot_matches` table.** Rejected: a migration plus invalidation on every
  transaction create/delete, asset delete and split sync, for data that is purely derived.
- **Extend `/tax-summary` with `include_breakdown=true`.** Rejected: it couples the modal to a
  portfolio-wide call that fetches a live quote per asset and mutates split rows — the two things
  this endpoint is defined by not doing — and it recomputes every asset to answer about one sell.
- **A batch endpoint returning every sell's breakdown.** Rejected for now, and named because it is
  the strongest contender: one call would make Success Criterion 2 machine-checkable and would be
  exactly what a per-row realized-P&L column needs. It is also more work than the feature itself,
  and the per-row column it would enable is out of scope below. Revisit if the ledger ever needs
  realized P&L per row.
- **Caching breakdowns.** Rejected: no measured bottleneck; see *Risks*.

## Risks and accepted trade-offs

- **FX provider dependency.** `get_rate` is cache-aside: on a miss it issues an outbound Yahoo
  Finance request (10 s timeout, three retries with backoff) and returns the last non-null close
  regardless of the date requested. A foreign-currency asset with many distinct transaction dates
  on a cold cache therefore makes the modal wait on several sequential external calls, and a
  failure surfaces as a 500. Accepted: the same dependency and the same failure mode already exist
  on every path through the engine, and the alternative — a degraded "these figures are in a
  currency you did not ask for" state on a tax screen — is worse than a loud failure.
- **Split-sync ordering dependency.** See *New endpoint*. The invariant depends on `/tax-summary`
  having run first for the asset. Accepted, documented, and pinned by the cross-endpoint route test
  rather than left implicit.
- **Full-history replay per click.** The handler replays the asset's entire transaction history:
  one `SELECT`, one sort, an O(sells × lots) consumption loop. Assumed scale is a personal
  portfolio — hundreds of transactions per asset, not hundreds of thousands — so no caching, no
  memoization and no latency target is defined here. If an asset ever grows large enough for this
  to matter, the first fix is a rate-lookup memo inside `replay`, not a cache layer.
- **No authentication.** Single-user local application; see *Error handling → Trust model*.

## Out of scope

- Specific-lot identification (user chooses which lots a sale consumed). Requires a persisted
  `sell_lot_matches` table, a migration, and invalidation on every transaction create/delete,
  asset delete, and the automatic split sync (`sync_splits_for_asset` mutates history behind the
  engine's back). Rejected: the requirement is read-only.
- A realized-P&L column on every SELL row in the ledger. Needs every sell's matches on every page
  load and duplicates the per-asset realized total already shown in the Holdings tab.
- Unrealized P&L in the modal. Realized-only by design; it is what keeps the endpoint free of any
  price fetch.

## Testing

### Engine (`tax_engine.rs` test module, existing style)

- One sell split across two lots under FIFO; LIFO reverses the consumption order.
- HYBRID consumes short-term lots first.
- Unmatched tail quantity.
- Sell dated before any buy → empty `matches`, `unmatched_qty == sold_qty`.
- Split-adjusted `unit_cost` and `buy_price` in a match.
- `holding_days` correctness.
- **Invariant**: `Σ sell_breakdowns[i].realized_pnl == realized_pnl` and
  `Σ matched_qty + Σ unmatched_qty == Σ sold_qty`, asserted over every existing fixture. State
  honestly what each half proves: because `calculate_lots` derives its total from the breakdowns,
  the first equality is a determinism check and cannot otherwise fail; the second is the
  substantive accounting assertion. The invariant as the user experiences it is cross-endpoint
  and is covered under *Route*, not here.
- The 14 pre-existing engine tests pass **with zero edits**. That is the proof the refactor
  preserved behaviour rather than re-derived it.

### Schema (`schemas.rs`)

Roundtrip serialization tests for `LotMatch` and `SellBreakdown`, matching the existing pattern.

### Route (`backend_rust/tests/`, existing `build_rocket` / `setup_db` / `seed_*` harness)

- 404 for an unknown transaction id.
- 400 for a BUY transaction id.
- 200 with the documented shape. This endpoint needs no live quote — unlike `/tax-summary`, whose
  tests must tolerate "price-derived fields are dynamic" — so the values are asserted exactly.
  Compare parsed `Decimal`s, never serialized strings: scale is a by-product of the arithmetic
  (see *New schemas*) and `Decimal: PartialEq` normalizes it.
- **Cross-endpoint reconciliation.** Seed one asset with two sells, call
  `GET /api/portfolios/<id>/tax-summary?strategy=X`, then `GET /api/transactions/<id>/pnl` for
  each sell, and assert the per-sell `realized_pnl` values sum to that asset's `realized_pnl` in
  the tax-summary response — under all three strategies. This is the only test that covers the
  central invariant as the user sees it: two separate HTTP calls agreeing on transaction set,
  base-currency resolution and strategy defaults. The engine-level test cannot cover it, because
  within one engine call the equality holds by construction.

### Frontend (Vitest + MSW)

- `TransactionLedger`: affordance renders on SELL and not on BUY or SPLIT; click calls
  `onSelectSell` with the right id.
- `SellPnlModal`: renders matches and totals; renders the unmatched banner; renders the empty-match
  explanation; handles the error path with retry.
- MSW handler added in `frontend/src/test/mocks/handlers.ts` for the new route.

## Success Criteria

1. Clicking a SELL row shows that sale's realized P&L and the lots it consumed, with per-lot P&L.
2. Summing realized P&L over all sells of an asset equals the realized P&L shown for that asset in
   the Holdings tab, under all three strategies. Covered by the cross-endpoint route test, not only
   by the engine test.
3. Sells with unmatched quantity, or with nothing available to match, say so in words instead of
   leaving the reader to interpret an empty table or a bare zero. (The empty-match sentence does
   print `0.00` — that is the explanation being given, not a substitute for one.)
4. The modal requests and displays the strategy and threshold currently selected in the Holdings
   tab, and the attribution shown follows that strategy. The response's `strategy` is the value
   that was applied, so an unrecognized query value renders as `FIFO`. This is deliberately not
   phrased as "switching strategy changes the numbers": a single-lot sell, or two strategies that
   happen to consume the same lots, produce identical attribution, and the criterion would then
   fail on correct behaviour.
5. The endpoint performs no current-quote fetch: no `get_price` call anywhere in the handler. FX
   rate lookups still occur when `asset_currency != currency`, because `get_rate` is cache-aside
   and fetches on a miss. Verified by
   `grep -n "get_price" backend_rust/src/api_routes/analytics.rs` showing no hit inside
   `get_transaction_sell_pnl`.
6. The engine tests present on `origin/main` (14 at the time of writing) are unmodified and green.

## Rollout

- Branch `feat/sell-pnl-breakdown` in a fresh worktree off `origin/main`; never the primary
  checkout.
- Regenerate `docs/openapi/openapi.json` per `AGENTS.md` — one route and two published schemas
  (`SellPnlResponse`, `LotMatch`) are added; `SellBreakdown` stays engine-internal.
- Commit; do not push without explicit operator confirmation.
