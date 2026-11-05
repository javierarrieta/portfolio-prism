# Sell Transaction P&L Breakdown Design

**Date**: 2026-11-05
**Status**: Proposed

## Overview

Clicking a SELL transaction in the Transaction Ledger opens a detail view showing the realized
P&L for that single sale and the specific buy lots it consumed, with P&L per lot.

The lot attribution is **read-only and derived from the currently selected accounting strategy**
(FIFO / LIFO / HYBRID). The user cannot override which lots a sale consumed; the view explains
what the engine already did, and its numbers are guaranteed to reconcile with the realized P&L
already displayed per asset in the Holdings tab.

## Requirements

- Clicking a SELL row in the Transaction Ledger shows realized P&L for that sale alone.
- The view lists the buy lots consumed, in consumption order, with per-lot cost basis, proceeds
  and P&L.
- Per-sell realized P&L across all sells of an asset **must sum exactly** to that asset's
  `realized_pnl` in `/tax-summary`. This is the central invariant.
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
returning `(buy_lots, realized_pnl, Vec<SellBreakdown>)`. Two public entry points sit on top of
it:

- `calculate_lots(...)` — unchanged signature, unchanged `AssetTaxSummary`. Calls `replay()`,
  builds open lots from the returned `buy_lots` exactly as today, discards the breakdowns.
- `sell_breakdown(sell_tx_id, ...)` — calls `replay()` with the same arguments, returns the one
  `SellBreakdown` whose `sell_transaction_id` matches.

One loop, one sort order, one split pass, one FX pass. The two entry points cannot disagree
because they are the same code; the reconciliation invariant is structural rather than something
kept true by discipline.

**The `continue` on no eligible lots must go.** Today a sell with no eligible lots is skipped
silently. It must instead emit a `SellBreakdown` with empty `matches` and
`unmatched_qty == sold_qty`. Likewise the leftover `qty_to_sell` after the consumption loop
becomes `unmatched_qty` instead of vanishing. Both are cases the engine hides today and the view
exists to expose.

### New schemas: `backend_rust/src/schemas.rs`

Every `Decimal` field uses `#[serde(with = "decimal_json")]`, as all existing money fields do.

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
```

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

- `roi` is `Option<Decimal>`: `cost_basis` is zero whenever nothing matched, and dividing there is
  exactly the silent nonsense this feature exists to prevent.
- `price` and `fee` are the raw as-entered values in `asset_currency`. Every other money field is
  in `currency` (the portfolio base currency).
- `fx_applied` is `asset_currency != currency`.

**This endpoint never calls `get_price`.** Realized P&L needs no current quote, so unlike
`/tax-summary` it performs no price fetch. FX rates for a non-base-currency asset come from the
existing in-memory rate cache (`currency_service.rs`, `get_rate`).

### Error handling

| Status | Condition |
|---|---|
| 404 | Transaction id does not exist |
| 400 | Transaction type is not `SELL` (defensive; the UI only makes SELL rows clickable) |
| 500 | Engine or FX failure, matching the existing `map_err(\|_\| Status::InternalServerError)` house style |

## Frontend Changes

### New component: `TransactionLedger` (`frontend/src/components/TransactionLedger.tsx`)

Extracted from the inline ledger block in `frontend/src/App.tsx` (the "Transaction History" card
under `activeTab === 'ledger'`). `App.tsx` is already 546 lines and this change adds row-click,
modal state and a fetch to that block; extracting it is the cleanup the work needs, not a
drive-by refactor.

Props: `transactions`, `assets`, `currency`, `strategy`, `thresholdDays`,
`onDeleteTransaction`, `onSelectSell(txId)`.

### SELL row affordance

SELL rows get an explicit affordance, not a bare `onClick` on `<tr>`: a small receipt icon button
in the Action column. A clickable row nobody can see is undiscoverable, and `<tr onClick>` with no
focusable element is unreachable by keyboard. The button provides tab order and Enter. The row
also carries `onClick` for mouse users, but the button is what makes it operable.

BUY and SPLIT rows are unchanged and show no affordance.

### New component: `SellPnlModal` (`frontend/src/components/SellPnlModal.tsx`)

Fetches `/api/transactions/<id>/pnl` on mount. `Loader2` while loading (existing pattern in
`App.tsx`), error state with retry, then header → summary → matches table → footnotes.

```
Sell P&L — AAPL · 01 Dec 2024 · FIFO
Sold 80 @ 130.00 USD · Fee 0.00 · Proceeds 10,400.00 USD
Realized P&L  +2,000.00 USD  (+23.9%)

Lots consumed (FIFO order)
Buy date     Buy px    Qty   Cost basis   Proceeds    P&L     Held
2024-01-01   100.00     60     6,000.00    7,800.00  +1,800   335d
2024-06-01   120.00     20     2,400.00    2,600.00    +200   183d
```

Modal styling follows the existing overlay pattern (`position: fixed` overlay + `.glass-card`,
`zIndex: 100`) used by the asset and transaction modals in `PortfolioDetail.tsx`.

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
| `unmatched_qty > 0` | Amber banner: *"20 of 100 units had no buy lot on or before this date, so no cost basis was subtracted for them — realized P&L is overstated."* The single most valuable line in the feature. |
| `matches` empty | *"No buy lots existed on or before this date. Realized P&L is 0.00 because nothing was matched."* Not "no data". |
| any `SPLIT` transaction dated ≤ the sell date | Footnote: *"Buy prices and unit costs are split-adjusted, not the prices as entered."* Detected client-side from the transaction list already in memory. |
| `fx_applied` | Footnote: *"Cost basis and proceeds converted to {currency} at each transaction's date rate."* |
| always | Header names the strategy (and threshold when HYBRID), plus: *"Matching follows the selected method — LIFO would attribute different lots."* |
| `roi` null | ROI renders as `—`, never `0%` and never `Infinity%`. |

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
  `Σ matched_qty + Σ unmatched_qty == Σ sold_qty`, asserted over every existing fixture.
- The 14 pre-existing engine tests pass **with zero edits**. That is the proof the refactor
  preserved behaviour rather than re-derived it.

### Schema (`schemas.rs`)

Roundtrip serialization tests for `LotMatch` and `SellBreakdown`, matching the existing pattern.

### Route (`backend_rust/tests/`, existing `build_rocket` / `setup_db` / `seed_*` harness)

- 404 for an unknown transaction id.
- 400 for a BUY transaction id.
- 200 with the documented shape and exact decimal strings. This endpoint needs no live quote —
  unlike `/tax-summary`, whose tests must tolerate "price-derived fields are dynamic" — so these
  assertions are exact rather than structural.

### Frontend (Vitest + MSW)

- `TransactionLedger`: affordance renders on SELL and not on BUY or SPLIT; click calls
  `onSelectSell` with the right id.
- `SellPnlModal`: renders matches and totals; renders the unmatched banner; renders the empty-match
  explanation; handles the error path with retry.
- MSW handler added in `frontend/src/test/mocks/handlers.ts` for the new route.

## Success Criteria

1. Clicking a SELL row shows that sale's realized P&L and the lots it consumed, with per-lot P&L.
2. Summing realized P&L over all sells of an asset equals the realized P&L shown for that asset in
   the Holdings tab, under all three strategies.
3. Sells with unmatched quantity or with no prior buys say so explicitly instead of showing 0.00 or
   an empty table.
4. Switching strategy in the Holdings tab changes the attribution shown, and the modal names the
   strategy it used.
5. The endpoint performs no live price fetch.
6. The 14 pre-existing engine tests are unmodified and green.

## Rollout

- Branch `feat/sell-pnl-breakdown` in a fresh worktree off `origin/main`; never the primary
  checkout.
- Regenerate `docs/openapi/openapi.json` per `AGENTS.md` — one route and two schemas are added.
- Commit; do not push without explicit operator confirmation.
