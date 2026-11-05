# Sell Transaction P&L Breakdown Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Clicking a SELL transaction shows its realized P&L and the buy lots it consumed, with P&L per lot.

**Architecture:** The tax engine gains lot identity and a single private `replay()` that records a `SellBreakdown` per sell. Two public entry points sit on that replay — the existing `calculate_lots` (unchanged signature and output) and a new `sell_breakdown` — so per-sell numbers reconcile with the per-asset realized total by construction rather than by duplicated logic. A new read-only endpoint exposes one sell's breakdown; the frontend extracts the transaction ledger into a component, makes SELL rows clickable, and opens a modal that fetches and renders the breakdown.

**Tech Stack:** Rust (Rocket 0.5, SQLx/SQLite, rust_decimal, utoipa 4.2), React 19 + TypeScript + Vite, Vitest + Testing Library + MSW.

**Spec:** `docs/superpowers/specs/2026-11-05-sell-pnl-breakdown-design.md`
**Review record:** `docs/superpowers/reviews/2026-11-05-sell-pnl-breakdown.md` — read it before reopening a rejected proposal or reintroducing a decimal string assertion.

**Normative for:** task sequencing, file lists and the test inventory. The spec is normative for
schemas and JSON keys, the route contract, the reconciliation invariant and the UI contract. Where
they disagree the spec wins for behaviour and this plan wins for order of work; the *Deviations*
list below is the only place a divergence is allowed to live.

## Global Constraints

- All money is `rust_decimal::Decimal` in the backend and serializes as a **JSON string** via `#[serde(with = "decimal_json")]`. Never `f64` for money in production code — and that covers **derived** money too: no `f64` intermediate anywhere in the new code, including the `roi` division (`realized_pnl / cost_basis` stays `Decimal` division) and any percentage or ratio the modal computes. The only `number` in the system is the display-formatted value produced at the fetch boundary. **Test seeders are exempt:** `tests/common/mod.rs::seed_transaction` already takes `qty`/`price`/`fee` as `f64`, and the new `seed_transaction_at` follows the existing harness rather than inventing a parallel one.
- The frontend receives decimal strings and converts them to `number` **only** at the fetch boundary in `frontend/src/utils/decimal.ts`. Components never parse decimal strings themselves, and never mutate the normalized response — no in-place sort. The response is already in consumption order; the table renders `matches` as received.
- All API routes are under `/api`.
- `AssetTaxSummary` and `TaxLot` must not change shape. The observable requirement is narrower and is the one to test: **the JSON value of every field in a `/tax-summary` entry is unchanged**. Whether the serializer emits `"0"` or `"0.0"` for a value is a by-product of `Decimal` scale, not a contract; the existing route test (`analytics_test.rs`) is the arbiter and must pass unmodified.
- The new endpoint performs **no** call to `CurrencyService::get_price`. Realized P&L needs no current quote. It **does** call `get_rate` when `asset_currency != currency`, because `get_rate` is cache-aside and fetches on a miss — that is expected and is not a violation.
- FX conversion happens **inside the engine replay**, at the same point `calculate_lots` converts today, so the per-sell numbers and the asset total are converted identically. The handler must not convert anything after the fact.
- The engine tests present on `origin/main` (14 at the time of writing) pass **unmodified** at every commit. Do not edit them to make a refactor pass.
- No new database table, no migration, no persisted match state. Read-only.
- `term` (SHORT/LONG) is populated **only** when `strategy == "HYBRID"`. Under FIFO/LIFO it is `null`.
- Branch per the repo convention: work on `feat/sell-pnl-breakdown` in a worktree, never in the primary checkout.

**Names, so the next reader does not wire the wrong field.** `SellBreakdown` is the engine type,
`SellPnlResponse` is the wire type, `SellPnlBreakdown` is the frontend mirror of the wire type. On
the wire the sell's id is `transaction_id`; `sell_transaction_id` exists only on the engine type,
and `buy_transaction_id` is the matched buy's id. `matched_qty` appears at two levels and means
different things: on `LotMatch` it is the quantity taken from that lot, on the response it is the
total matched across all lots. The UI surface has two names and both are load-bearing: the nav item
is **"Transaction Ledger"** (`App.tsx:335`) and the card heading inside it is **"Transaction
History"** (`App.tsx:423`). Task 9's wiring test clicks the nav item; Task 7's component test
asserts on the heading. Do not "unify" one to match the other.

## Review Focus

The spec implies these inputs; no single task's happy-path test covers them, and each is a way a real user gets a wrong number or a confusing screen. Each line has a test pinned to the task that owns the code.

1. **A SELL larger than every available buy** — the unmatched units must contribute neither cost basis nor P&L, and the UI must state how many units went unmatched. Expected: realized P&L counts only matched units; a banner names the unmatched count. (Task 3, Task 8)
2. **A SELL dated before every BUY, and any sell whose cost basis is zero** — `roi` must be `null` and render as `—`, never `0%`, `NaN%` or `Infinity%`; the modal must explain, not show an empty table. (Task 4, Task 8)
3. **A SPLIT between the buy and the sell** — `buy_price` and `unit_cost` in a match are split-adjusted, total cost basis is unchanged by the split, and the footnote appears. Expected: per-lot P&L still sums to the asset-level realized total. (Task 3, Task 8)
4. **Strategy switching** — FIFO/LIFO/HYBRID attribute different lots, but per-sell totals must still reconcile with the asset-level realized P&L under all three. Expected: attribution changes, totals do not. (Task 3, Task 4)
5. **A non-SELL transaction id, or a BUY/SPLIT row** — the endpoint answers 400 and the ledger offers no P&L affordance. Expected: no clickable control on BUY or SPLIT rows. (Task 4, Task 7)

## File Structure

| File | Responsibility |
|---|---|
| `backend_rust/src/db_types.rs` | modify — add `decimal_opt_json` for `Option<Decimal>` fields |
| `backend_rust/src/schemas.rs` | modify — `LotMatch`, `SellBreakdown`, `SellPnlResponse` |
| `backend_rust/src/engines/tax_engine.rs` | modify — lot identity, `sorted_transactions`, `replay`, `sell_breakdown` |
| `backend_rust/src/api_routes/analytics.rs` | modify — `get_transaction_sell_pnl` |
| `backend_rust/src/lib.rs` | modify — mount the new route |
| `backend_rust/src/openapi.rs` | modify — register path + schemas |
| `backend_rust/tests/common/mod.rs` | modify — `seed_transaction_at` (date-aware seeder) |
| `backend_rust/tests/sell_pnl_test.rs` | create — endpoint tests |
| `docs/openapi/openapi.json` | regenerate |
| `frontend/src/types.ts` | modify — `LotMatch`, `SellPnlBreakdown` |
| `frontend/src/types/api.d.ts` | regenerate — generated from the OpenAPI spec in Task 5; committed artifact, must not drift |
| `frontend/src/utils/decimal.ts` | modify — `normalizeSellPnl` |
| `frontend/src/utils/decimal.test.ts` | create — normalizer tests |
| `frontend/src/components/TransactionLedger.tsx` | create — extracted ledger + SELL affordance |
| `frontend/src/components/SellPnlModal.tsx` | create — the breakdown modal |
| `frontend/src/App.tsx` | modify — use `TransactionLedger`, own modal state |
| `frontend/src/test/mocks/handlers.ts` | modify — MSW handler for the new route |
| `frontend/src/test/components/TransactionLedger.test.tsx` | create |
| `frontend/src/test/components/SellPnlModal.test.tsx` | create |
| `frontend/src/test/components/AppLedger.test.tsx` | create — wiring test: SELL row click opens the modal |
| `frontend/src/utils/format.ts` | no change — `term_for` already exists and is the only source for SHORT/LONG labels |

**Deviation from the spec, deliberate.** Each of these is intentional; do not "fix" them back to the spec's first draft.

1. **`TransactionLedger` props.** The spec lists `currency`, `strategy` and `thresholdDays`. The extracted code shows the ledger only ever uses per-asset currency (`asset?.currency || 'USD'`), and the modal is rendered by `App`, not by the ledger — so the ledger's real props are `transactions`, `assets`, `onDeleteTransaction`, `onSelectSell`.
2. **`replay()` returns `(Vec<InternalLot>, Vec<SellBreakdown>)`, not a 3-tuple with `realized_pnl`.** `calculate_lots` derives the asset total as `Σ breakdowns[i].realized_pnl`. One computation, one place; see the spec's *Single replay*.
3. **The response is a flat `SellPnlResponse`.** The engine returns a `SellBreakdown`; the handler flattens it together with the context fields into one object. There is no nested `breakdown` key on the wire.
4. **`SellBreakdown` is not registered in OpenAPI.** Only `SellPnlResponse` and `LotMatch` are published; no response body is of the engine type.
5. **`strategy` is normalized at the handler boundary** (upper-case, unrecognized → `FIFO`) and the response reports the value that was applied, so the modal can never name a method it did not use.
6. **Modal copy is final as written in Task 8**, including the unmatched banner and the empty-matches sentence. The spec describes their intent; Task 8 states the exact strings and the tests assert them.

---

## Task 1: Optional-decimal serialization and the three new schemas

**Files:**
- Modify: `backend_rust/src/db_types.rs` (append module + tests inside the existing `mod tests`)
- Modify: `backend_rust/src/schemas.rs` (append structs after `AssetTaxSummary`; tests inside existing `mod tests`)
- Test: same files, `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `crate::db_types::decimal_opt_json::{serialize, deserialize}` for `Option<Decimal>`
  - `schemas::LotMatch { buy_transaction_id: i32, buy_date: DateTime<Utc>, buy_price: Decimal, unit_cost: Decimal, matched_qty: Decimal, lot_qty_before_match: Decimal, cost_basis: Decimal, proceeds: Decimal, pnl: Decimal, holding_days: i64, term: Option<String> }`
  - `schemas::SellBreakdown { sell_transaction_id: i32, sell_date: DateTime<Utc>, sold_qty: Decimal, matched_qty: Decimal, unmatched_qty: Decimal, proceeds: Decimal, cost_basis: Decimal, realized_pnl: Decimal, matches: Vec<LotMatch> }`
  - `schemas::SellPnlResponse { transaction_id: i32, asset_id: i32, symbol: String, asset_type: String, strategy: String, threshold_days: i64, currency: String, asset_currency: String, fx_applied: bool, split_applied: bool, sell_date: DateTime<Utc>, sold_qty: Decimal, price: Decimal, fee: Decimal, proceeds: Decimal, cost_basis: Decimal, realized_pnl: Decimal, roi: Option<Decimal>, matched_qty: Decimal, unmatched_qty: Decimal, matches: Vec<LotMatch> }`

- [x] **Step 1: Write the failing tests for `decimal_opt_json`**

Append inside `mod tests` in `backend_rust/src/db_types.rs`:

```rust
    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct WrapOpt { #[serde(with = "decimal_opt_json")] d: Option<Decimal> }

    #[test]
    fn optional_decimal_serializes_some_as_string() {
        let w = WrapOpt { d: Some(Decimal::from_str("9.99").unwrap()) };
        assert_eq!(serde_json::to_string(&w).unwrap(), r#"{"d":"9.99"}"#);
    }

    #[test]
    fn optional_decimal_serializes_none_as_null() {
        let w = WrapOpt { d: None };
        assert_eq!(serde_json::to_string(&w).unwrap(), r#"{"d":null}"#);
    }

    #[test]
    fn optional_decimal_deserializes_null_number_and_string() {
        assert_eq!(serde_json::from_str::<WrapOpt>(r#"{"d":null}"#).unwrap().d, None);
        assert_eq!(
            serde_json::from_str::<WrapOpt>(r#"{"d":"9.99"}"#).unwrap().d,
            Some(Decimal::from_str("9.99").unwrap())
        );
        assert_eq!(
            serde_json::from_str::<WrapOpt>(r#"{"d":100}"#).unwrap().d,
            Some(Decimal::from_str("100").unwrap())
        );
    }

    #[test]
    fn optional_decimal_rejects_garbage_string() {
        assert!(serde_json::from_str::<WrapOpt>(r#"{"d":"abc"}"#).is_err());
    }
```

- [x] **Step 2: Run them to verify they fail**

Run: `cd backend_rust && cargo test optional_decimal 2>&1 | tail -20`
Expected: compile error — `cannot find module or macro decimal_opt_json`. The filter `optional_decimal` matches the four new test names; `decimal_opt` matches none of them, which would make this step look like a passing run rather than a missing module. The tests live in `mod tests` with `use super::*;`, so the module must be a sibling of `decimal_json` in the file root.

- [x] **Step 3: Implement `decimal_opt_json`**

`decimal_opt_json` is `decimal_json` lifted to `Option<Decimal>`. Write it beside that module, in the file root (not inside `mod tests`), under the same string rules:

- `serialize(&Option<Decimal>, S)`: `None` → `serialize_none()`; `Some(d)` → `serialize_str(&d.to_string())`. Exact string, no `normalize()`, no fixed scale — the same rule `decimal_json` already applies.
- `deserialize<D>(D) -> Result<Option<Decimal>, D::Error>`: `deserialize_option` with a visitor accepting `null`, a JSON number, and a string parseable by `Decimal::from_str`, rejecting anything else. Reuse `decimal_json`'s existing visitor for the `Some` arm rather than writing a second number/string parser.

> **Possible improvement, deliberately not taken in this plan.** `rust_decimal` ships an equivalent: `rust_decimal::serde::str_option`. (The name the review cited, `option_str`, does not exist — checked against the locked `rust_decimal 1.42.1`.) Two caveats, both verified against the vendored source rather than assumed: it is gated behind `#[cfg(feature = "serde-with-str")]`, and `Cargo.toml:19` enables only `features = ["serde"]`; and the project already hand-writes `decimal_json` for the non-optional case, so a hand-written `decimal_opt_json` is house style, not reinvention.
>
> To take it later: enable `features = ["serde", "serde-with-str"]`, swap the `with` attribute on `roi`, delete this module and its four `optional_decimal_*` tests, and confirm those four are the only tests that disappear. `serde-with-str` only adds the `with` modules — unlike `serde-str` it does not change the blanket `Decimal` impl — so existing serialization is unaffected.

- [x] **Step 4: Run them to verify they pass**

Run: `cd backend_rust && cargo test db_types 2>&1 | tail -20`
Expected: PASS — **12 tests**: the 8 pre-existing `db_types` tests plus the 4 new ones. Use the `db_types` filter, not `decimal`: the pre-existing tests are named `serialize_is_exact_string`, `integer_input_is_exact` and similar, so a `decimal` filter silently runs only 6 tests and reports green while never touching the code the new module sits next to.

- [x] **Step 5: Write the failing schema roundtrip tests**

Append inside `mod tests` in `backend_rust/src/schemas.rs`:

```rust
    #[test]
    fn test_lot_match_roundtrip() {
        let m = LotMatch {
            buy_transaction_id: 11,
            buy_date: sample_datetime(),
            buy_price: Decimal::from_str("100.0").unwrap(),
            unit_cost: Decimal::from_str("100.5").unwrap(),
            matched_qty: Decimal::from_str("60.0").unwrap(),
            lot_qty_before_match: Decimal::from_str("100.0").unwrap(),
            cost_basis: Decimal::from_str("6030.0").unwrap(),
            proceeds: Decimal::from_str("7800.0").unwrap(),
            pnl: Decimal::from_str("1770.0").unwrap(),
            holding_days: 335,
            term: Some("SHORT".to_string()),
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: LotMatch = serde_json::from_str(&json).unwrap();
        assert_eq!(back.buy_transaction_id, 11);
        assert_eq!(back.unit_cost, Decimal::from_str("100.5").unwrap());
        assert_eq!(back.holding_days, 335);
        assert_eq!(back.term, Some("SHORT".to_string()));
    }

    #[test]
    fn test_sell_breakdown_roundtrip_and_decimals_are_strings() {
        let b = SellBreakdown {
            sell_transaction_id: 42,
            sell_date: sample_datetime(),
            sold_qty: Decimal::from_str("80.0").unwrap(),
            matched_qty: Decimal::from_str("80.0").unwrap(),
            unmatched_qty: Decimal::ZERO,
            proceeds: Decimal::from_str("10400.0").unwrap(),
            cost_basis: Decimal::from_str("8400.0").unwrap(),
            realized_pnl: Decimal::from_str("2000.0").unwrap(),
            matches: vec![],
        };
        let json = serde_json::to_string(&b).unwrap();
        assert!(json.contains(r#""realized_pnl":"2000.0""#), "decimals must serialize as strings: {json}");
        let back: SellBreakdown = serde_json::from_str(&json).unwrap();
        assert_eq!(back.realized_pnl, Decimal::from_str("2000.0").unwrap());
        assert!(back.matches.is_empty());
    }

    #[test]
    fn test_sell_pnl_response_roi_null_when_no_cost_basis() {
        let r = SellPnlResponse {
            transaction_id: 42, asset_id: 7, symbol: "AAPL".to_string(),
            asset_type: "STOCK".to_string(), strategy: "FIFO".to_string(), threshold_days: 30,
            currency: "USD".to_string(), asset_currency: "USD".to_string(),
            fx_applied: false, split_applied: false,
            sell_date: sample_datetime(),
            sold_qty: Decimal::from_str("50.0").unwrap(),
            price: Decimal::from_str("120.0").unwrap(),
            fee: Decimal::ZERO,
            proceeds: Decimal::ZERO, cost_basis: Decimal::ZERO, realized_pnl: Decimal::ZERO,
            roi: None,
            matched_qty: Decimal::ZERO, unmatched_qty: Decimal::from_str("50.0").unwrap(),
            matches: vec![],
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains(r#""roi":null"#), "roi must be null, not 0: {json}");
        let back: SellPnlResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.roi, None);
        assert_eq!(back.unmatched_qty, Decimal::from_str("50.0").unwrap());
    }
```

- [x] **Step 6: Run them to verify they fail**

Run: `cd backend_rust && cargo test sell_pnl_response 2>&1 | tail -20`
Expected: compile error — `cannot find struct LotMatch` / `SellBreakdown` / `SellPnlResponse`.

- [x] **Step 7: Implement the schemas**

Add three `#[derive(Debug, Serialize, Deserialize, ToSchema)]` structs after `AssetTaxSummary`, with exactly the fields listed in **Interfaces** above and the field comments from the spec's *New schemas*. Only four things are worth writing down:

- every `Decimal` field carries `#[serde(with = "decimal_json")]`; `roi` alone carries `#[serde(with = "decimal_opt_json")]`.
- `term: Option<String>` on `LotMatch` — `"SHORT"` / `"LONG"`, populated for HYBRID only, `null` otherwise.
- `SellBreakdown` is the engine's internal type and is **not** registered in OpenAPI; `SellPnlResponse` and `LotMatch` are.
- `SellPnlResponse` flattens the breakdown next to its context fields. There is no nested `breakdown` key on the wire, and the sell's id is `transaction_id` there, not `sell_transaction_id`.

Also extend the `use` line at the top of `schemas.rs` so `decimal_opt_json` is in scope:

```rust
use crate::db_types::{decimal_json, decimal_opt_json};
```

No utoipa workaround is needed for `Option<Decimal>`. Verified by compiling a `ToSchema` struct carrying `#[serde(with = …)] roi: Option<Decimal>` against the pinned utoipa 4.2.3 — it derives cleanly, and the generated schema is a nullable string. Do not add `#[schema(value_type = …)]` speculatively; if a future utoipa bump does complain, the error names the field and the fix goes directly above `pub roi` without changing the runtime type.

- [x] **Step 8: Run the whole backend test suite to verify nothing regressed**

Run: `cd backend_rust && cargo test 2>&1 | tail -30`
Expected: PASS. The 14 `tax_engine` tests and the pre-existing `schemas` tests are untouched.

- [x] **Step 9: Commit**

```bash
git add backend_rust/src/db_types.rs backend_rust/src/schemas.rs
git commit -m "feat: add sell P&L breakdown schemas with optional-decimal json"
```

---

## Task 2: Extract `replay()` and give lots an identity — behaviour unchanged

**Files:**
- Modify: `backend_rust/src/engines/tax_engine.rs`

**Interfaces:**
- Consumes: `crate::models::Transaction`, `crate::schemas::{TaxLot, AssetTaxSummary}`, `CurrencyService`.
- Produces:
  - `struct InternalLot { source_tx_id: i32, date: DateTime<Utc>, price: Decimal, qty: Decimal, unit_cost: Decimal }` (private)
  - `fn TaxLotEngine::sorted_transactions(&[Transaction]) -> Vec<Transaction>` (private)
  - `async fn TaxLotEngine::replay(sorted_txs: &[Transaction], currency_service: &CurrencyService, asset_currency: &str, base_currency: &str, strategy: &str, hybrid_threshold_days: i64) -> Result<(Vec<InternalLot>, Vec<SellBreakdown>)>` (private)
  - `TaxLotEngine::calculate_lots` — **signature and output identical to before**

This is a pure refactor. There is no new test here: the acceptance proof is that the 14 existing engine tests pass with **zero edits to the test module**. If one of them fails, the refactor changed behaviour — fix the refactor, not the test.

- [x] **Step 1: Capture the green baseline**

Run: `cd backend_rust && cargo test tax_engine 2>&1 | tail -10`
Expected: `test result: ok. 14 passed`. If this is not green before you start, stop and report it — the plan assumes it is.

- [x] **Step 2: Add `source_tx_id` to `InternalLot` and share the sort**

Replace the `InternalLot` struct with:

```rust
struct InternalLot {
    source_tx_id: i32,
    date: DateTime<Utc>,
    price: Decimal,
    qty: Decimal,
    unit_cost: Decimal,
}
```

Add this associated function as the first item in `impl TaxLotEngine`:

```rust
    /// Canonical transaction order. Splits must apply to existing lots before
    /// any same-day buy/sell, so they sort ahead of everything else on a tie.
    fn sorted_transactions(transactions: &[Transaction]) -> Vec<Transaction> {
        let split_priority = |tx: &Transaction| {
            if tx.r#type.to_uppercase() == "SPLIT" { 0 } else { 1 }
        };
        let mut sorted_txs = transactions.to_vec();
        sorted_txs.sort_by(|a, b| {
            a.date.cmp(&b.date).then_with(|| split_priority(a).cmp(&split_priority(b)))
        });
        sorted_txs
    }
```

- [x] **Step 3: Move the loop body into `replay()`**

Add this associated function after `sorted_transactions`. **Move the body of the existing `for tx in sorted_txs { ... }` loop in `calculate_lots` into it — do not retype it.** The sort order, the split rescaling, the fee amortisation, the FX conversion and the HYBRID bucketing are already correct there; the entire point of the refactor is that they come across unchanged.

```rust
    /// Single source of truth for lot matching. Both public entry points call this,
    /// which is what makes per-sell realized P&L reconcile with the asset-level
    /// total: `realized_pnl` is *derived* from the returned breakdowns by the
    /// caller, never accumulated on a separate path.
    async fn replay(
        sorted_txs: &[Transaction],
        currency_service: &CurrencyService,
        asset_currency: &str,
        base_currency: &str,
        strategy: &str,
        hybrid_threshold_days: i64,
    ) -> Result<(Vec<InternalLot>, Vec<SellBreakdown>)>
```

Exactly **three** changes to the moved loop, and nothing else:

1. **Delete the early `continue` for an empty eligible set** (`tax_engine.rs:286-288` today). A sell with nothing to match against must still produce a breakdown, all of it unmatched, so the UI can say so.
2. **Delete the engine-wide `realized_pnl` accumulator.** Accumulate `sell_cost_basis` and `sell_proceeds` per sell instead; the asset total is derived from the breakdowns in Step 4. Keeping the accumulator *and* the breakdowns leaves two independent paths and makes the doc comment above false — this is the one thing that must not be simplified back.
3. **Push a `SellBreakdown` for every sell**, after the consumption loop, with `matched_qty = tx_qty - qty_to_sell`, `unmatched_qty = qty_to_sell` (the leftover that today vanishes), `realized_pnl = sell_proceeds - sell_cost_basis`, and `matches: vec![]`. Step 6 fills in the per-lot detail; the totals are already correct at this point, which is why the 14 existing tests stay green before any per-lot data exists.

`InternalLot` gains `source_tx_id: i32`, set from `tx.id` in the BUY branch. Without it two buys on the same date at the same price are indistinguishable and the view cannot say which purchase a cost basis came from.

- [x] **Step 4: Rewrite `calculate_lots` to call `replay`**

Keep the `calculate_lots` signature byte-identical — nine parameters, `symbol` and `asset_type` first — and replace everything from it through the end of its `for tx in sorted_txs { ... }` loop with four steps:

1. `let sorted_txs = Self::sorted_transactions(transactions);`
2. Keep the existing `current_price_base` block exactly as it is (current price × rate at the last transaction's date, `Utc::now` fallback). It is the one place a current quote is legitimately involved, and it belongs to `calculate_lots`, not to `replay`.
3. `let (buy_lots, breakdowns) = Self::replay(&sorted_txs, currency_service, asset_currency, base_currency, strategy, hybrid_threshold_days).await?;`
4. Derive the total instead of accumulating it: `let realized_pnl: Decimal = breakdowns.iter().map(|b| b.realized_pnl).sum();` — the old `let mut realized_pnl = Decimal::ZERO;` and the `realized_pnl +=` inside the loop are gone.

Leave the rest of `calculate_lots` (the `let mut open_lots = Vec::new();` block through `Ok(AssetTaxSummary { ... })`) exactly as it is. The only change it needs is how `realized_pnl` is obtained — the old `let mut realized_pnl = Decimal::ZERO;` accumulator and the `realized_pnl +=` inside the loop are gone, replaced by the derived sum shown above. Extend the `use` at the top of the file:

```rust
use crate::schemas::{AssetTaxSummary, LotMatch, SellBreakdown, TaxLot};
```

`LotMatch` is unused until Task 3. That is a warning at most: CI (`.github/workflows/ci.yml:21`) runs `cargo test` and nothing else, and this repository has no clippy step. Do not add `#[allow(unused_imports)]` for a gate that does not exist — Task 3 consumes the import two commits later.

- [x] **Step 5: Run the existing engine tests — they must pass unmodified**

Run: `cd backend_rust && cargo test tax_engine 2>&1 | tail -15`
Expected: `test result: ok. 14 passed; 0 failed`. They are green here because Step 3 already produces correct per-sell totals; Step 6 only adds the per-lot detail those 14 tests do not look at. If any fail now, the refactor changed behaviour — fix the refactor, not the test. **Do not edit the test module.**

- [x] **Step 6: Add `term_for`, then record each match**

First, at file scope in `tax_engine.rs`, after the `impl TaxLotEngine` block:

```rust
/// Short/long-term bucketing exists only for HYBRID, where `threshold_days` is
/// a real strategy parameter. Reporting a term under FIFO or LIFO would present
/// a HYBRID tuning knob as a tax fact, so it stays null there.
fn term_for(strategy: &str, holding_days: i64, threshold_days: i64) -> Option<String> {
    if strategy.to_uppercase() != "HYBRID" {
        return None;
    }
    Some(if holding_days <= threshold_days { "SHORT".to_string() } else { "LONG".to_string() })
}
```

The return type is `Option<String>`, not `Option<&'static str>`, because `LotMatch.term` is `Option<String>` — it is a serialized schema field. A `&'static str` would not compile against the struct.

Then, in `replay`, inside the SELL branch, delete the two accumulator lines added in Step 3 (`let mut sell_cost_basis = Decimal::ZERO;` and `let mut sell_proceeds = Decimal::ZERO;`) and put the match vector in their place, immediately before the consumption loop:

```rust
                let mut matches: Vec<LotMatch> = Vec::new();
```

Replace the body of the consumption loop after `let proceeds = matched_qty * sell_unit_proceeds_base;` — delete the two `sell_cost_basis += cost_basis;` / `sell_proceeds += proceeds;` lines and put this in their place:

```rust
                    matches.push(LotMatch {
                        buy_transaction_id: lot.source_tx_id,
                        buy_date: lot.date,
                        buy_price: lot.price,
                        unit_cost: lot.unit_cost,
                        matched_qty,
                        lot_qty_before_match: lot.qty,
                        cost_basis,
                        proceeds,
                        pnl: proceeds - cost_basis,
                        holding_days: tx.date.signed_duration_since(lot.date).num_days(),
                        term: term_for(strategy, tx.date.signed_duration_since(lot.date).num_days(), hybrid_threshold_days),
                    });

                    lot.qty -= matched_qty;
                    qty_to_sell -= matched_qty;
```

Note the ordering requirement: `lot_qty_before_match: lot.qty` must be read **before** `lot.qty -= matched_qty`, and `lot` is still mutably borrowed at that point, which is legal because the field read happens in the same expression as the earlier value. If the borrow checker objects, hoist the reads into locals before the mutation:

```rust
                    let (lot_qty_before_match, lot_price, lot_unit_cost, lot_date, lot_source) =
                        (lot.qty, lot.price, lot.unit_cost, lot.date, lot.source_tx_id);
```

and use those locals in the `LotMatch` literal.

Then replace the `breakdowns.push(...)` from Step 3 so its totals are derived from `matches` rather than from hand-carried accumulators:

```rust
                let matched_qty: Decimal = matches.iter().map(|m| m.matched_qty).sum();
                let cost_basis: Decimal = matches.iter().map(|m| m.cost_basis).sum();
                let proceeds: Decimal = matches.iter().map(|m| m.proceeds).sum();

                breakdowns.push(SellBreakdown {
                    sell_transaction_id: tx.id,
                    sell_date: tx.date,
                    sold_qty: tx_qty,
                    matched_qty,
                    unmatched_qty: tx_qty - matched_qty,
                    proceeds,
                    cost_basis,
                    realized_pnl: proceeds - cost_basis,
                    matches,
                });
```

- [x] **Step 7: Confirm the placeholder is gone**

`grep -n "matches: vec!\[\]" backend_rust/src/engines/tax_engine.rs` must return nothing. If it matches, Step 6's per-lot recording is not wired up yet, and the modal would render a correct total with an empty lot table — the one failure mode this feature exists to prevent.

- [x] **Step 8: Run the engine tests**

Run: `cd backend_rust && cargo test tax_engine 2>&1 | tail -15`
Expected: `test result: ok. 14 passed; 0 failed` — same 14 tests, same assertions, none edited.

- [x] **Step 9: Run the whole backend suite**

Run: `cd backend_rust && cargo test 2>&1 | tail -20`
Expected: PASS, including `analytics_test`, `transactions_test`, `portfolios_test`, `decimal_migration_test`, `cors_test`.

- [x] **Step 10: Commit**

```bash
git add backend_rust/src/engines/tax_engine.rs
git commit -m "refactor: single lot-matching replay with per-sell breakdown recorded"
```

---

## Task 3: `sell_breakdown` entry point and engine-level breakdown tests

**Files:**
- Modify: `backend_rust/src/engines/tax_engine.rs` (public entry point + tests in the existing `mod tests`)
- Test: same file

**Interfaces:**
- Consumes: `TaxLotEngine::replay`, `schemas::{LotMatch, SellBreakdown}`, `term_for`.
- Produces: `async fn TaxLotEngine::sell_breakdown(sell_transaction_id: i32, transactions: &[Transaction], asset_currency: &str, base_currency: &str, currency_service: &CurrencyService, strategy: &str, hybrid_threshold_days: i64) -> Result<Option<SellBreakdown>>`

Note the signature has **no `current_price`** — realized P&L does not depend on a quote. That is what keeps the endpoint free of price fetches.

- [x] **Step 1: Write the failing tests**

Append inside `mod tests` in `tax_engine.rs`. The existing `make_tx` helper already takes an id, so reuse it.

```rust
    async fn run_breakdown(txs: &[Transaction], sell_id: i32, strategy: &str, threshold: i64) -> Option<SellBreakdown> {
        let svc = CurrencyService::new();
        TaxLotEngine::sell_breakdown(
            sell_id, txs, "USD", "USD", &svc, strategy, threshold,
        ).await.unwrap()
    }

    fn two_buys_one_sell() -> Vec<Transaction> {
        vec![
            make_tx(1, "BUY", "60.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "BUY", "40.0", "120.0", "0.0", "2024-06-01T00:00:00Z"),
            make_tx(3, "SELL", "80.0", "130.0", "0.0", "2024-12-01T00:00:00Z"),
        ]
    }

    #[tokio::test]
    async fn test_sell_breakdown_fifo_consumes_oldest_first() {
        let txs = two_buys_one_sell();
        let b = run_breakdown(&txs, 3, "FIFO", 30).await.expect("breakdown for sell 3");
        assert_eq!(b.sell_transaction_id, 3);
        assert_eq!(b.sold_qty, Decimal::from_str("80.0").unwrap());
        assert_eq!(b.matches.len(), 2);
        assert_eq!(b.matches[0].buy_transaction_id, 1);
        assert_eq!(b.matches[0].matched_qty, Decimal::from_str("60.0").unwrap());
        assert_eq!(b.matches[0].lot_qty_before_match, Decimal::from_str("60.0").unwrap());
        assert_eq!(b.matches[0].pnl, Decimal::from_str("1800.0").unwrap());
        assert_eq!(b.matches[0].holding_days, 335);
        assert_eq!(b.matches[1].buy_transaction_id, 2);
        assert_eq!(b.matches[1].matched_qty, Decimal::from_str("20.0").unwrap());
        assert_eq!(b.matches[1].lot_qty_before_match, Decimal::from_str("40.0").unwrap());
        assert_eq!(b.matches[1].pnl, Decimal::from_str("200.0").unwrap());
        assert_eq!(b.matches[1].holding_days, 183);
        assert_eq!(b.realized_pnl, Decimal::from_str("2000.0").unwrap());
        assert_eq!(b.unmatched_qty, Decimal::ZERO);
    }

    #[tokio::test]
    async fn test_sell_breakdown_lifo_consumes_newest_first() {
        let txs = two_buys_one_sell();
        let b = run_breakdown(&txs, 3, "LIFO", 30).await.unwrap();
        assert_eq!(b.matches[0].buy_transaction_id, 2);
        assert_eq!(b.matches[0].matched_qty, Decimal::from_str("40.0").unwrap());
        assert_eq!(b.matches[1].buy_transaction_id, 1);
        assert_eq!(b.realized_pnl, Decimal::from_str("1600.0").unwrap());
    }

    #[tokio::test]
    async fn test_sell_breakdown_term_is_null_except_hybrid() {
        let txs = vec![
            make_tx(1, "BUY", "100.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "BUY", "100.0", "110.0", "0.0", "2024-11-15T00:00:00Z"),
            make_tx(3, "SELL", "100.0", "120.0", "0.0", "2024-12-01T00:00:00Z"),
        ];
        let fifo = run_breakdown(&txs, 3, "FIFO", 30).await.unwrap();
        assert_eq!(fifo.matches[0].term, None, "FIFO must not claim a tax term");
        assert_eq!(fifo.matches[0].holding_days, 335);

        let hybrid = run_breakdown(&txs, 3, "HYBRID", 30).await.unwrap();
        assert_eq!(hybrid.matches[0].buy_transaction_id, 2, "HYBRID takes the 16-day lot first");
        assert_eq!(hybrid.matches[0].term, Some("SHORT".to_string()));
        assert_eq!(hybrid.matches[0].holding_days, 16);
    }

    #[tokio::test]
    async fn test_sell_breakdown_hybrid_labels_long_term_bucket() {
        let txs = vec![
            make_tx(1, "BUY", "100.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "SELL", "100.0", "120.0", "0.0", "2024-12-01T00:00:00Z"),
        ];
        let b = run_breakdown(&txs, 2, "HYBRID", 30).await.unwrap();
        assert_eq!(b.matches[0].term, Some("LONG".to_string()));
    }

    #[tokio::test]
    async fn test_sell_breakdown_reports_unmatched_quantity() {
        let txs = vec![
            make_tx(1, "BUY", "50.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "SELL", "100.0", "120.0", "0.0", "2024-12-01T00:00:00Z"),
        ];
        let b = run_breakdown(&txs, 2, "FIFO", 30).await.unwrap();
        assert_eq!(b.matched_qty, Decimal::from_str("50.0").unwrap());
        assert_eq!(b.unmatched_qty, Decimal::from_str("50.0").unwrap());
        assert_eq!(b.matches.len(), 1);
        // Only the matched half contributes: 50 * (120 - 100), not 100 * 20.
        assert_eq!(b.realized_pnl, Decimal::from_str("1000.0").unwrap());
    }

    #[tokio::test]
    async fn test_sell_breakdown_before_any_buy_is_all_unmatched() {
        let txs = vec![
            make_tx(1, "SELL", "50.0", "120.0", "0.0", "2024-12-01T00:00:00Z"),
            make_tx(2, "BUY", "50.0", "100.0", "0.0", "2025-01-01T00:00:00Z"),
        ];
        let b = run_breakdown(&txs, 1, "FIFO", 30).await.unwrap();
        assert!(b.matches.is_empty(), "a sell cannot consume a lot bought after it");
        assert_eq!(b.unmatched_qty, Decimal::from_str("50.0").unwrap());
        assert_eq!(b.matched_qty, Decimal::ZERO);
        assert_eq!(b.cost_basis, Decimal::ZERO);
        assert_eq!(b.realized_pnl, Decimal::ZERO);
    }

    #[tokio::test]
    async fn test_sell_breakdown_reports_split_adjusted_cost() {
        let txs = vec![
            make_tx(1, "BUY", "100.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "SPLIT", "2.0", "1.0", "0.0", "2024-06-01T00:00:00Z"),
            make_tx(3, "SELL", "100.0", "60.0", "0.0", "2024-12-01T00:00:00Z"),
        ];
        let b = run_breakdown(&txs, 3, "FIFO", 30).await.unwrap();
        assert_eq!(b.matches.len(), 1);
        assert_eq!(b.matches[0].buy_price, Decimal::from_str("50.0").unwrap());
        assert_eq!(b.matches[0].unit_cost, Decimal::from_str("50.0").unwrap());
        assert_eq!(b.matches[0].lot_qty_before_match, Decimal::from_str("200.0").unwrap());
        assert_eq!(b.matches[0].cost_basis, Decimal::from_str("5000.0").unwrap());
        assert_eq!(b.realized_pnl, Decimal::from_str("1000.0").unwrap());
    }

    #[tokio::test]
    async fn test_sell_breakdown_unknown_id_returns_none() {
        let txs = two_buys_one_sell();
        assert!(run_breakdown(&txs, 999, "FIFO", 30).await.is_none());
    }

    #[tokio::test]
    async fn test_sell_breakdowns_sum_to_asset_realized_pnl() {
        // The reconciliation invariant, checked under every strategy, over a
        // fixture with two sells, a partial lot, an unmatched tail and a split.
        let txs = vec![
            make_tx(1, "BUY", "60.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "BUY", "40.0", "120.0", "0.0", "2024-06-01T00:00:00Z"),
            make_tx(3, "SPLIT", "2.0", "1.0", "0.0", "2024-08-01T00:00:00Z"),
            make_tx(4, "SELL", "80.0", "130.0", "5.0", "2024-12-01T00:00:00Z"),
            make_tx(5, "SELL", "300.0", "70.0", "0.0", "2025-01-01T00:00:00Z"),
        ];
        for strategy in ["FIFO", "LIFO", "HYBRID"] {
            let svc = CurrencyService::new();
            let summary = TaxLotEngine::calculate_lots(
                "TEST", "STOCK", &txs, Decimal::from_str("110.0").unwrap(),
                "USD", "USD", &svc, strategy, 30,
            ).await.unwrap();

            let sorted = TaxLotEngine::sorted_transactions(&txs);
            let (_, breakdowns) = TaxLotEngine::replay(
                &sorted, &svc, "USD", "USD", strategy, 30,
            ).await.unwrap();

            let summed: Decimal = breakdowns.iter().map(|b| b.realized_pnl).sum();
            assert_eq!(summed, summary.realized_pnl, "per-sell totals must equal the asset total under {strategy}");

            let sold: Decimal = breakdowns.iter().map(|b| b.sold_qty).sum();
            let accounted: Decimal = breakdowns.iter().map(|b| b.matched_qty + b.unmatched_qty).sum();
            assert_eq!(sold, accounted, "every sold unit is either matched or declared unmatched under {strategy}");
        }
    }
```

- [x] **Step 2: Run them to verify they fail**

Run: `cd backend_rust && cargo test sell_breakdown 2>&1 | tail -20`
Expected: compile error — `no function `sell_breakdown` found for struct TaxLotEngine`.

- [x] **Step 3: Implement `sell_breakdown`**

Add as the second public associated function in `impl TaxLotEngine`, right after `calculate_lots`:

```rust
    /// Realized-P&L breakdown for one sell. Replays the whole asset history
    /// rather than a prefix: a sell's matches depend only on transactions at or
    /// before it, so this is identical to a prefix replay and is the same call
    /// `calculate_lots` makes. Takes no current price — realized P&L needs none.
    pub async fn sell_breakdown(
        sell_transaction_id: i32,
        transactions: &[Transaction],
        asset_currency: &str,
        base_currency: &str,
        currency_service: &CurrencyService,
        strategy: &str,
        hybrid_threshold_days: i64,
    ) -> Result<Option<SellBreakdown>> {
        let sorted_txs = Self::sorted_transactions(transactions);
        let (_, breakdowns) = Self::replay(
            &sorted_txs,
            currency_service,
            asset_currency,
            base_currency,
            strategy,
            hybrid_threshold_days,
        )
        .await?;
        Ok(breakdowns.into_iter().find(|b| b.sell_transaction_id == sell_transaction_id))
    }
```

- [x] **Step 4: Run the new tests**

Run: `cd backend_rust && cargo test sell_breakdown 2>&1 | tail -20`
Expected: PASS — 9 new tests.

- [x] **Step 5: Run the whole engine module to prove nothing drifted**

Run: `cd backend_rust && cargo test tax_engine 2>&1 | tail -10`
Expected: `test result: ok. 23 passed` (14 original + 9 new), 0 failed.

- [x] **Step 6: Commit**

```bash
git add backend_rust/src/engines/tax_engine.rs
git commit -m "feat: per-sell lot breakdown entry point with engine tests"
```

---

## Task 4: The `/api/transactions/<id>/pnl` endpoint

**Files:**
- Modify: `backend_rust/src/api_routes/analytics.rs` (append handler)
- Modify: `backend_rust/src/lib.rs` (mount in the `/api` group)
- Modify: `backend_rust/tests/common/mod.rs` (date-aware seeder)
- Create: `backend_rust/tests/sell_pnl_test.rs`

**Interfaces:**
- Consumes: `TaxLotEngine::sell_breakdown` (Task 3), `schemas::SellPnlResponse` (Task 1), models `Transaction`/`Asset`/`Portfolio`.
- Produces: `api_routes::analytics::get_transaction_sell_pnl`, mounted at `GET /api/transactions/<id>/pnl?strategy=&threshold_days=`, returning `Json<SellPnlResponse>`.

- [x] **Step 1: Add the date-aware test seeder**

Append to `backend_rust/tests/common/mod.rs` (the `#[allow(dead_code)]` attribute is required — this file is compiled into every test binary):

```rust
#[allow(dead_code)]
pub async fn seed_transaction_at(
    pool: &SqlitePool,
    asset_id: i32,
    tx_type: &str,
    qty: f64,
    price: f64,
    fee: f64,
    date: &str,
) -> i32 {
    let date = DateTime::parse_from_rfc3339(date).unwrap().with_timezone(&Utc);
    let res = sqlx::query_as::<_, (i32,)>(
        "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) VALUES (?, ?, ?, ?, ?, ?) RETURNING id"
    )
    .bind(asset_id)
    .bind(tx_type)
    .bind(qty)
    .bind(price)
    .bind(fee)
    .bind(date)
    .fetch_one(pool)
    .await
    .unwrap();
    res.0
}
```

- [x] **Step 2: Write the failing endpoint tests**

Create `backend_rust/tests/sell_pnl_test.rs`:

```rust
mod common;

use common::{build_rocket, setup_db, seed_portfolio, seed_asset, seed_transaction_at};
use rocket::http::Status;
use rocket::local::asynchronous::Client;
use rocket::{Build, Rocket};
use rust_decimal::Decimal;

/// Money fields are full-precision Decimals whose serialized scale is a by-product of the
/// arithmetic (spec: New schemas → numeric precision). Compare values, never strings —
/// `"2000"`, `"2000.0"` and `"2000.000"` are the same number and any of them may appear.
fn dec(body: &serde_json::Value, key: &str) -> Decimal {
    body[key]
        .as_str()
        .unwrap_or_default()
        .parse::<Decimal>()
        .unwrap_or_else(|_| panic!("{key} is not a decimal string: {}", body[key]))
}

fn d(s: &str) -> Decimal {
    s.parse::<Decimal>().unwrap()
}

// FIFO fixture: two buys, one sell of 80 units split across both lots.
// Returns the unlaunched Rocket; each test wraps it in its own tracked Client.
async fn fifo_fixture() -> (Rocket<Build>, i32, i32, i32) {
    let pool = setup_db().await;
    let port_id = seed_portfolio(&pool, "Sell PnL", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "AAPL", "Apple").await;
    let buy1 = seed_transaction_at(&pool, asset_id, "BUY", 60.0, 100.0, 0.0, "2024-01-01T00:00:00Z").await;
    let buy2 = seed_transaction_at(&pool, asset_id, "BUY", 40.0, 120.0, 0.0, "2024-06-01T00:00:00Z").await;
    let sell = seed_transaction_at(&pool, asset_id, "SELL", 80.0, 130.0, 0.0, "2024-12-01T00:00:00Z").await;
    let _ = buy2;
    (build_rocket(pool), sell, buy1, asset_id)
}

#[tokio::test]
async fn test_sell_pnl_unknown_transaction_is_404() {
    let (rocket, _, _, _) = fifo_fixture().await;
    let client = Client::tracked(rocket).await.unwrap();
    let resp = client.get("/api/transactions/999999/pnl?strategy=FIFO&threshold_days=30").dispatch().await;
    assert_eq!(resp.status(), Status::NotFound);
}

#[tokio::test]
async fn test_sell_pnl_rejects_non_sell_transaction() {
    let (rocket, _, buy1, _) = fifo_fixture().await;
    let client = Client::tracked(rocket).await.unwrap();
    let resp = client.get(format!("/api/transactions/{}/pnl?strategy=FIFO", buy1)).dispatch().await;
    assert_eq!(resp.status(), Status::BadRequest);
}

#[tokio::test]
async fn test_sell_pnl_fifo_breakdown_shape() {
    let (rocket, sell, buy1, _) = fifo_fixture().await;
    let client = Client::tracked(rocket).await.unwrap();
    let resp = client.get(format!("/api/transactions/{}/pnl?strategy=FIFO&threshold_days=30", sell)).dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();

    assert_eq!(body["transaction_id"], sell);
    assert_eq!(body["symbol"], "AAPL");
    assert_eq!(body["strategy"], "FIFO");
    assert_eq!(body["threshold_days"], 30);
    assert_eq!(body["currency"], "USD");
    assert_eq!(body["asset_currency"], "USD");
    assert_eq!(body["fx_applied"], false);
    // This fixture has no SPLIT row, so the server-side flag must be false. The
    // split footnote is a response field now, not something the browser guesses.
    assert_eq!(body["split_applied"], false);
    assert_eq!(dec(&body, "sold_qty"), d("80"));
    assert_eq!(dec(&body, "price"), d("130"));
    assert_eq!(dec(&body, "realized_pnl"), d("2000"));
    assert_eq!(dec(&body, "matched_qty"), d("80"));
    assert_eq!(dec(&body, "unmatched_qty"), d("0"));
    assert_eq!(body["matches"].as_array().unwrap().len(), 2);
    assert_eq!(body["matches"][0]["buy_transaction_id"], buy1);
    assert_eq!(dec(&body["matches"][0], "matched_qty"), d("60"));
    assert_eq!(body["matches"][0]["holding_days"], 335);
    assert!(body["matches"][0]["term"].is_null(), "term is HYBRID-only");
    assert_eq!(dec(&body["matches"][1], "matched_qty"), d("20"));
    // Decimal division carries full precision, so pin the leading digits rather
    // than an exact 28-digit string.
    assert!(body["roi"].as_str().unwrap().starts_with("0.238"), "unexpected roi: {}", body["roi"]);
}

#[tokio::test]
async fn test_sell_pnl_lifo_attributes_different_lots() {
    let (rocket, sell, _, _) = fifo_fixture().await;
    let client = Client::tracked(rocket).await.unwrap();
    let resp = client.get(format!("/api/transactions/{}/pnl?strategy=LIFO&threshold_days=30", sell)).dispatch().await;
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
    assert_eq!(body["strategy"], "LIFO");
    assert_eq!(dec(&body, "realized_pnl"), d("1600"));
}

#[tokio::test]
async fn test_sell_pnl_defaults_to_fifo_when_params_omitted() {
    let (rocket, sell, _, _) = fifo_fixture().await;
    let client = Client::tracked(rocket).await.unwrap();
    let resp = client.get(format!("/api/transactions/{}/pnl", sell)).dispatch().await;
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
    assert_eq!(body["strategy"], "FIFO");
    assert_eq!(body["threshold_days"], 30);
    assert_eq!(dec(&body, "realized_pnl"), d("2000"));
}

#[tokio::test]
async fn test_sell_pnl_unmatched_quantity_is_declared_not_dropped() {
    let pool = setup_db().await;
    let port_id = seed_portfolio(&pool, "Unmatched", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "TSLA", "Tesla").await;
    seed_transaction_at(&pool, asset_id, "BUY", 50.0, 100.0, 0.0, "2024-01-01T00:00:00Z").await;
    let sell = seed_transaction_at(&pool, asset_id, "SELL", 100.0, 120.0, 0.0, "2024-12-01T00:00:00Z").await;
    let client = Client::tracked(build_rocket(pool)).await.unwrap();

    let resp = client.get(format!("/api/transactions/{}/pnl?strategy=FIFO", sell)).dispatch().await;
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
    assert_eq!(dec(&body, "sold_qty"), d("100"));
    assert_eq!(dec(&body, "matched_qty"), d("50"));
    assert_eq!(dec(&body, "unmatched_qty"), d("50"));
    assert_eq!(dec(&body, "realized_pnl"), d("1000"));
}

#[tokio::test]
async fn test_sell_pnl_roi_is_null_when_cost_basis_is_zero() {
    let pool = setup_db().await;
    let port_id = seed_portfolio(&pool, "No Buys", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "NOBUYS", "No Buys").await;
    let sell = seed_transaction_at(&pool, asset_id, "SELL", 50.0, 120.0, 0.0, "2024-12-01T00:00:00Z").await;
    let client = Client::tracked(build_rocket(pool)).await.unwrap();

    let resp = client.get(format!("/api/transactions/{}/pnl?strategy=FIFO", sell)).dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
    assert!(body["roi"].is_null(), "roi must be null, never a division by zero");
    assert!(body["matches"].as_array().unwrap().is_empty());
    assert_eq!(dec(&body, "unmatched_qty"), d("50"));
    // An empty `matches` sums to `Decimal::ZERO` at scale 0, which serializes as "0" —
    // exactly what `analytics_test.rs` already asserts for `/tax-summary`. Comparing as
    // Decimals means the assertion holds whichever scale the arithmetic produces.
    assert_eq!(dec(&body, "realized_pnl"), d("0"));
}

#[tokio::test]
async fn test_sell_pnl_sums_to_tax_summary_realized_pnl() {
    // The central invariant as the user experiences it: two separate HTTP calls that must
    // agree on transaction set, base-currency resolution and strategy defaults. The
    // engine-level test cannot catch a divergence here, because within one engine call the
    // equality holds by construction.
    let pool = setup_db().await;
    let port_id = seed_portfolio(&pool, "Reconcile", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "RECON", "Reconcile").await;
    seed_transaction_at(&pool, asset_id, "BUY", 60.0, 100.0, 0.0, "2024-01-01T00:00:00Z").await;
    seed_transaction_at(&pool, asset_id, "BUY", 40.0, 120.0, 0.0, "2024-06-01T00:00:00Z").await;
    let sell1 = seed_transaction_at(&pool, asset_id, "SELL", 50.0, 130.0, 0.0, "2024-08-01T00:00:00Z").await;
    let sell2 = seed_transaction_at(&pool, asset_id, "SELL", 40.0, 150.0, 0.0, "2024-12-01T00:00:00Z").await;
    let client = Client::tracked(build_rocket(pool)).await.unwrap();

    for strategy in ["FIFO", "LIFO", "HYBRID"] {
        let tax_resp = client
            .get(format!("/api/portfolios/{}/tax-summary?strategy={}&threshold_days=30", port_id, strategy))
            .dispatch()
            .await;
        assert_eq!(tax_resp.status(), Status::Ok);
        let tax: serde_json::Value = serde_json::from_str(&tax_resp.into_string().await.unwrap()).unwrap();
        let asset = tax["assets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["symbol"] == "RECON")
            .expect("asset present in tax summary");
        // realized_pnl is not price-derived, so unlike market_value it is stable here.
        let asset_realized = asset["realized_pnl"]
            .as_str()
            .unwrap()
            .parse::<Decimal>()
            .unwrap();

        let mut sum = Decimal::ZERO;
        for sell in [sell1, sell2] {
            let resp = client
                .get(format!("/api/transactions/{}/pnl?strategy={}&threshold_days=30", sell, strategy))
                .dispatch()
                .await;
            assert_eq!(resp.status(), Status::Ok, "{} sell {}", strategy, sell);
            let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
            sum += dec(&body, "realized_pnl");
        }
        assert_eq!(
            sum, asset_realized,
            "{strategy}: per-sell realized P&L must sum to the asset total the Holdings tab shows"
        );
    }
}
```

- [x] **Step 3: Run them to verify they fail**

Run: `cd backend_rust && cargo test --test sell_pnl_test 2>&1 | tail -20`
Expected: FAIL — 7 of the 8 tests fail with 404 Not Found. `test_sell_pnl_unknown_transaction_is_404` passes vacuously here because an unmounted route also 404s; it only becomes meaningful once the route exists.

- [x] **Step 4: Implement the handler**

Append to `backend_rust/src/api_routes/analytics.rs`. Route and signature:

```rust
#[utoipa::path(
    get,
    path = "/api/transactions/<id>/pnl",
    params(
        ("id" = i32, Path, description = "Sell transaction id"),
        ("strategy" = Option<String>, Query, description = "FIFO, LIFO or HYBRID (default FIFO)"),
        ("threshold_days" = Option<i64>, Query, description = "HYBRID short-term threshold (default 30)")
    ),
    responses(
        (status = 200, description = "Realized P&L breakdown for the sell", body = SellPnlResponse),
        (status = 400, description = "Transaction is not a SELL"),
        (status = 404, description = "Transaction not found")
    )
)]
#[get("/transactions/<id>/pnl?<strategy>&<threshold_days>")]
pub async fn get_transaction_sell_pnl(
    id: i32,
    pool: &State<SqlitePool>,
    currency_service: &State<CurrencyService>,
    strategy: Option<String>,
    threshold_days: Option<i64>,
) -> Result<Json<SellPnlResponse>, Status>
```

The body is seven steps, each following the pattern already in `get_portfolio_tax_summary` in the same file — copy its error-handling idiom (`map_err(|_| Status::InternalServerError)`) rather than inventing one:

1. `SELECT * FROM transactions WHERE id = ?` via `fetch_optional`; `None` → `Status::NotFound`.
2. `if tx.r#type.to_uppercase() != "SELL" { return Err(Status::BadRequest); }`
3. Load the asset by `tx.asset_id`, then its portfolio by `asset.portfolio_id`; either missing → `NotFound`. The base currency comes from the portfolio that owns the asset, never from a path parameter, so the answer cannot disagree with the portfolio on screen.
4. Normalize the strategy **once**: `"LIFO"` and `"HYBRID"` pass through upper-cased, anything else — including absent — becomes `"FIFO"`. Use that single value for both the engine call and the response, so the modal can never name a method it did not use. `threshold_days.unwrap_or(30)`. `fx_applied = asset.currency != portfolio.base_currency`.
5. `SELECT * FROM transactions WHERE asset_id = ?` — the same query and the same scope `get_portfolio_tax_summary` uses — then `TaxLotEngine::sell_breakdown(&tx.id, &transactions, &asset.currency, &portfolio.base_currency, currency_service, &strategy_val, threshold_val)`. `None` → `NotFound`. **No `get_price` call anywhere in this function**; `get_rate` inside the engine is expected and fine.
6. `split_applied = transactions.iter().any(|t| t.r#type.to_uppercase() == "SPLIT" && t.date <= tx.date)`. This is free — the list is already loaded — and it is scoped to exactly the asset the replay consumed by construction. The modal's split footnote reads this field instead of scanning whatever transaction list the browser happens to be holding, which can be one fetch stale.
7. `roi = if breakdown.cost_basis > Decimal::ZERO { Some(breakdown.realized_pnl / breakdown.cost_basis) } else { None }` — `Decimal` division, never `f64`.
8. Build `SellPnlResponse` from the breakdown plus context. `price` and `fee` are `str_to_decimal(&tx.price)` / `str_to_decimal(&tx.fee)` — as entered, in asset currency. Everything else comes straight from the breakdown.

Extend the `use` block at the top of `analytics.rs`:

```rust
use crate::schemas::SellPnlResponse;
```

- [x] **Step 5: Mount the route**

In `backend_rust/src/lib.rs`, add to the existing `.mount("/api", routes![...])` list. There are **two** mounts in `build_rocket` — `/api/portfolios` and `/api` — and this route belongs in `/api`, because its `#[get]` path already begins with `/transactions`. Putting it in the `/api/portfolios` group yields `/api/portfolios/transactions/<id>/pnl` and every request 404s while the code compiles cleanly.

```rust
            api_routes::analytics::get_transaction_sell_pnl,
```

- [x] **Step 6: Run the endpoint tests**

Run: `cd backend_rust && cargo test --test sell_pnl_test 2>&1 | tail -20`
Expected: PASS — 8 tests.

- [x] **Step 7: Run the whole backend suite**

Run: `cd backend_rust && cargo test 2>&1 | tail -20`
Expected: PASS. `cors_test` and `analytics_test` still green — `/tax-summary` is untouched.

- [x] **Step 8: Verify the no-current-quote claim**

The plan asserts three times that this endpoint never fetches a current price, and nothing else checks it:

```bash
grep -n "get_price" backend_rust/src/api_routes/analytics.rs
```

Expected: no hit between `pub async fn get_transaction_sell_pnl` and the end of that function. Hits inside `get_tax_summary` are expected and unrelated. `get_rate` calls inside the new handler are **not** a violation — FX conversion is cache-aside and legitimately fetches on a miss.

**Known coverage gap, recorded rather than closed:** every backend test seeds `currency='USD'` assets into `'USD'` portfolios, so `asset_currency == base_currency`, every rate lookup short-circuits to `ONE`, and `get_rate` is never exercised. The one path in this endpoint that can reach the network and return 500 has no test. Closing it needs a `seed_asset_with_currency` helper plus a stubbed rate source; until then, do not claim the FX path is verified.

- [x] **Step 9: Commit**

```bash
git add backend_rust/src/api_routes/analytics.rs backend_rust/src/lib.rs backend_rust/tests/common/mod.rs backend_rust/tests/sell_pnl_test.rs
git commit -m "feat: expose per-sell realized P&L at GET /api/transactions/<id>/pnl"
```

---

## Task 5: OpenAPI registration and regenerated spec

**Files:**
- Modify: `backend_rust/src/openapi.rs`
- Regenerate: `docs/openapi/openapi.json`, `frontend/src/types/api.d.ts`

**Interfaces:**
- Consumes: `api_routes::analytics::get_transaction_sell_pnl`, `schemas::{LotMatch, SellBreakdown, SellPnlResponse}`.
- Produces: an `openapi.json` containing `/api/transactions/<id>/pnl` and the three new component schemas.

- [x] **Step 1: Register the path and schemas**

In `backend_rust/src/openapi.rs`, add to the `paths(...)` list after `crate::api_routes::analytics::get_portfolio_performance,`:

```rust
        crate::api_routes::analytics::get_transaction_sell_pnl,
```

and to the `components(schemas(...))` list after `schemas::AssetTaxSummary,`:

```rust
            schemas::LotMatch,
            schemas::SellPnlResponse,
```

`SellBreakdown` is the engine's internal type; no response body is of it, so it is **not**
registered. Registering it would put an unconsumed type in the committed contract and in the
generated `api.d.ts`.

- [x] **Step 2: Verify it compiles and the spec serves the new route**

Run: `cd backend_rust && cargo check 2>&1 | tail -10`
Expected: no errors. `Option<Decimal>` derives `ToSchema` without help (verified against utoipa 4.2.3 in Task 1), so there is no contingency to apply here.

- [x] **Step 3: Regenerate the committed spec**

The `AGENTS.md` recipe (`cargo run --release &` then `sleep 12`) races a fixed sleep against a
release build that may take minutes, and if the server is not up the `curl` returns empty and the
redirect **truncates the committed spec to zero bytes**. Poll instead of sleeping, and never
overwrite the file unless the response is a JSON object:

```bash
cd backend_rust
touch portfolio.db
cargo run --release > /tmp/sell-pnl-server.log 2>&1 &
SERVER_PID=$!
for i in $(seq 1 60); do
  OUT=$(curl -sf http://127.0.0.1:8000/api-docs/openapi.json || true)
  if printf '%s' "$OUT" | python3 -c 'import json,sys; d=json.load(sys.stdin); assert d.get("paths")' 2>/dev/null; then
    printf '%s' "$OUT" > ../docs/openapi/openapi.json
    echo "wrote $(wc -c < ../docs/openapi/openapi.json) bytes"
    break
  fi
  sleep 2
done
kill $SERVER_PID
```

Expected: a `wrote N bytes` line where N is larger than the current file. If nothing is written,
the server never came up — read `/tmp/sell-pnl-server.log` and fix that; do not commit an empty or
truncated `openapi.json`.

- [x] **Step 4: Verify the new route and schemas landed**

From the **repository root** (not `backend_rust/`):

```bash
python3 -c "import json;d=json.load(open('docs/openapi/openapi.json'));print('/api/transactions/<id>/pnl' in d['paths']);print(sorted(s for s in d['components']['schemas'] if 'Sell' in s or s=='LotMatch'))"
```

Expected: `True` and `['LotMatch', 'SellPnlResponse']` — and `git diff --stat docs/openapi/openapi.json` showing additions only, with no existing path or schema removed.

- [x] **Step 5: Regenerate the generated TypeScript types**

From the **repository root** — Step 3 already left you in `backend_rust/`, and `cd ../frontend` from there would land outside the repo:

```bash
cd frontend && npm install && npm run generate-types
```

Nothing imports `src/types/api.d.ts` today, but it is a committed generated artifact and must not drift from the spec it was generated from.

- [x] **Step 6: Commit**

```bash
git add docs/openapi/openapi.json frontend/src/types/api.d.ts backend_rust/src/openapi.rs
git commit -m "chore: register sell P&L route in OpenAPI and regenerate spec"
```

---

## Task 6: Frontend types and the decimal normalizer

**Files:**
- Modify: `frontend/src/types.ts`
- Modify: `frontend/src/utils/decimal.ts`
- Create: `frontend/src/utils/decimal.test.ts`

**Interfaces:**
- Consumes: the endpoint JSON from Task 4.
- Produces:
  - `types.ts`: `LotMatch`, `SellPnlBreakdown`
  - `decimal.ts`: `normalizeSellPnl(t: unknown): SellPnlBreakdown`

- [x] **Step 1: Add the types**

Append to `frontend/src/types.ts`:

```ts
export interface LotMatch {
  buy_transaction_id: number;
  buy_date: string;
  buy_price: number;
  unit_cost: number;
  matched_qty: number;
  lot_qty_before_match: number;
  cost_basis: number;
  proceeds: number;
  pnl: number;
  holding_days: number;
  term: 'SHORT' | 'LONG' | null;
}

export interface SellPnlBreakdown {
  transaction_id: number;
  asset_id: number;
  symbol: string;
  asset_type: string;
  strategy: string;
  threshold_days: number;
  currency: string;
  asset_currency: string;
  fx_applied: boolean;
  /** A SPLIT on this asset dated on or before the sell: buy prices are split-adjusted. */
  split_applied: boolean;
  sell_date: string;
  sold_qty: number;
  price: number;
  fee: number;
  proceeds: number;
  cost_basis: number;
  realized_pnl: number;
  /** null when cost basis is zero; render as an em dash, never 0%. */
  roi: number | null;
  matched_qty: number;
  unmatched_qty: number;
  matches: LotMatch[];
}
```

- [x] **Step 2: Write the failing normalizer tests**

Create `frontend/src/utils/decimal.test.ts` (sits beside the source, matching `csvParser.test.ts`):

```ts
import { describe, it, expect } from 'vitest'
import { normalizeSellPnl } from './decimal'

const raw = {
  transaction_id: 42, asset_id: 7, symbol: 'AAPL', asset_type: 'STOCK',
  strategy: 'FIFO', threshold_days: 30,
  currency: 'USD', asset_currency: 'USD', fx_applied: false, split_applied: false,
  sell_date: '2024-12-01T00:00:00Z',
  sold_qty: '80.0', price: '130.0', fee: '0.0',
  proceeds: '10400.0', cost_basis: '8400.0', realized_pnl: '2000.0', roi: '0.238',
  matched_qty: '80.0', unmatched_qty: '0.0',
  matches: [
    { buy_transaction_id: 1, buy_date: '2024-01-01T00:00:00Z', buy_price: '100.0',
      unit_cost: '100.0', matched_qty: '60.0', lot_qty_before_match: '60.0',
      cost_basis: '6000.0', proceeds: '7800.0', pnl: '1800.0',
      holding_days: 335, term: null },
  ],
}

describe('normalizeSellPnl', () => {
  it('converts decimal strings to numbers', () => {
    const d = normalizeSellPnl(raw)
    expect(d.sold_qty).toBe(80)
    expect(d.realized_pnl).toBe(2000)
    expect(d.matches[0].matched_qty).toBe(60)
    expect(d.matches[0].pnl).toBe(1800)
    expect(d.matches[0].holding_days).toBe(335)
  })

  it('keeps roi null when there is no cost basis', () => {
    const d = normalizeSellPnl({ ...raw, roi: null, cost_basis: '0.0' })
    expect(d.roi).toBeNull()
  })

  it('reads a present roi as a number', () => {
    expect(normalizeSellPnl(raw).roi).toBeCloseTo(0.238)
  })

  it('survives a malformed payload without throwing', () => {
    const d = normalizeSellPnl(null)
    expect(d.matches).toEqual([])
    expect(d.realized_pnl).toBe(0)
    expect(d.roi).toBeNull()
  })

  it('passes term through', () => {
    const d = normalizeSellPnl({
      ...raw,
      matches: [{ ...raw.matches[0], term: 'SHORT' }],
    })
    expect(d.matches[0].term).toBe('SHORT')
  })
})
```

- [x] **Step 3: Run them to verify they fail**

Run: `cd frontend && npx vitest run src/utils/decimal.test.ts 2>&1 | tail -20`
Expected: FAIL — `normalizeSellPnl is not exported` / is not a function.

- [x] **Step 4: Implement the normalizer**

In `frontend/src/utils/decimal.ts`, extend the type import at the top to include `SellPnlBreakdown` and `LotMatch`:

```ts
import type {
  Asset, HistoryItem, LotMatch, PerformanceMetrics, PortfolioPerformance,
  SellPnlBreakdown, TaxSummary, Transaction,
} from '../types';
```

Append `normalizeSellPnl(t: unknown): SellPnlBreakdown` to the end of `decimal.ts`, built from the helpers already in that file (`asRecord`, `str`, `num`, `toNumber`, `DecimalLike`, `UnknownRecord`). Rules, not code:

- two small local helpers: `optNum` (`null`/`undefined` → `null`, else `toNumber`) and `termOf` (only the literals `'SHORT'` and `'LONG'` survive; anything else → `null`).
- every decimal-ish field goes through `num`; `roi` through `optNum`; `transaction_id`, `asset_id`, `threshold_days` and `holding_days` are plain numbers with `?? 0` fallbacks.
- `currency` and `asset_currency` fall back to `'USD'`; `fx_applied` and `split_applied` are `=== true`, never truthy-coerced.
- `matches` is `Array.isArray(r.matches) ? … : []`, each element passed through `asRecord`. A malformed payload must produce a zeroed breakdown with `matches: []` and never throw — that is what the "survives a malformed payload" test pins.
- do not sort `matches`. The payload is already in consumption order and sorting here would hide the engine's ordering from the tests that assert it.

- [x] **Step 5: Run the tests**

Run: `cd frontend && npx vitest run src/utils/decimal.test.ts 2>&1 | tail -20`
Expected: PASS — 5 tests.

- [x] **Step 6: Commit**

```bash
git add frontend/src/types.ts frontend/src/utils/decimal.ts frontend/src/utils/decimal.test.ts
git commit -m "feat: frontend types and normalizer for sell P&L breakdown"
```

---

## Task 7: Extract `TransactionLedger` and add the SELL affordance

**Files:**
- Create: `frontend/src/components/TransactionLedger.tsx`
- Modify: `frontend/src/App.tsx` (replace the inline ledger block at lines 421-475, drop now-unused imports)
- Create: `frontend/src/test/components/TransactionLedger.test.tsx`

(`frontend/src/test/mocks/handlers.ts` is **not** touched here — the MSW handler for the new route arrives in Task 8 Step 1 and is committed with that task.)

**Interfaces:**
- Consumes: `Transaction`, `Asset` from `../types`, `formatCurrency`.
- Produces: `TransactionLedger` default export with props `{ transactions: Transaction[]; assets: Asset[]; onDeleteTransaction: (txId: number) => Promise<void>; onSelectSell: (txId: number) => void }`.

- [ ] **Step 1: Write the failing component tests**

Create `frontend/src/test/components/TransactionLedger.test.tsx`:

```tsx
import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import TransactionLedger from '../../components/TransactionLedger'
import type { Asset, Transaction } from '../../types'

const assets: Asset[] = [
  { id: 1, portfolio_id: 1, symbol: 'AAPL', name: 'Apple Inc.', asset_type: 'STOCK', currency: 'USD', isin: null, transactions: [] },
]

const txs: Transaction[] = [
  { id: 10, asset_id: 1, type: 'SELL', quantity: 80, price: 130, fee: 0, date: '2024-12-01T00:00:00Z' },
  { id: 11, asset_id: 1, type: 'BUY', quantity: 60, price: 100, fee: 0, date: '2024-01-01T00:00:00Z' },
  { id: 12, asset_id: 1, type: 'SPLIT', quantity: 2, price: 1, fee: 0, date: '2024-06-01T00:00:00Z' },
]

const noop = async () => {}

const renderLedger = (onSelectSell = vi.fn()) => {
  render(
    <TransactionLedger
      transactions={txs}
      assets={assets}
      onDeleteTransaction={noop}
      onSelectSell={onSelectSell}
    />,
  )
  return onSelectSell
}

describe('TransactionLedger', () => {
  it('renders every transaction', () => {
    renderLedger()
    expect(screen.getAllByRole('row')).toHaveLength(4)
  })

  it('offers a realized P&L control on a SELL row and fires with its id', () => {
    const onSelectSell = renderLedger()
    const btn = screen.getByRole('button', { name: /realized P&L for AAPL/i })
    fireEvent.click(btn)
    expect(onSelectSell).toHaveBeenCalledWith(10)
  })

  it('offers no P&L control on BUY or SPLIT rows', () => {
    renderLedger()
    expect(screen.getAllByRole('button', { name: /realized P&L/i })).toHaveLength(1)
  })

  it('clicking a SELL row body also opens the breakdown', () => {
    const onSelectSell = renderLedger()
    fireEvent.click(screen.getByText('SELL'))
    expect(onSelectSell).toHaveBeenCalledWith(10)
  })

  it('clicking a BUY row body does nothing', () => {
    const onSelectSell = renderLedger()
    fireEvent.click(screen.getByText('BUY'))
    expect(onSelectSell).not.toHaveBeenCalled()
  })

  it('still renders the delete control for every row', () => {
    renderLedger()
    expect(screen.getAllByTitle('Delete transaction')).toHaveLength(3)
  })
})
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cd frontend && npx vitest run src/test/components/TransactionLedger.test.tsx 2>&1 | tail -20`
Expected: FAIL — cannot resolve `../../components/TransactionLedger`.

- [ ] **Step 3: Create the component**
Create `frontend/src/components/TransactionLedger.tsx`. **Copy the table markup out of `App.tsx`** — the block rendered under `{activeTab === 'ledger' && ( … )}`, the one whose heading is `Transaction History`. Locate it by that heading, not by a line number; `App.tsx:421-475` is where it sits today and any edit above it moves that range. Props are in **Interfaces** above; `formatCurrency` comes from `../utils/formatters`, `Trash2`/`TrendingUp` from `lucide-react`.

Four changes to the copied markup, all of them behaviour, none of them accidental:

1. the delete button gains `title="Delete transaction"` — the new test locates it by that title;
2. the delete handler becomes `(e) => { e.stopPropagation(); void onDeleteTransaction(tx.id); }`, so deleting from a SELL row does not also open the modal;
3. a SELL row gets `onClick={() => onSelectSell(tx.id)}` and `cursor: pointer`, and the row gains a `TrendingUp` button with `aria-label="View realized P&L"` whose own handler calls `stopPropagation()` before `onSelectSell(tx.id)`;
4. BUY and SPLIT rows get no click handler and no button — the absence of the affordance is what tells the user this row has no realized P&L to show.

Everything else must come across unchanged: column order, the `2 : 1` and `—` cells, the `--text-secondary` colours, the empty-state paragraph. `App.test.tsx` asserts on those. Do not describe this step as a verbatim move when reviewing it — diff it against the original block.

- [ ] **Step 4: Run the tests**

Run: `cd frontend && npx vitest run src/test/components/TransactionLedger.test.tsx 2>&1 | tail -20`
Expected: PASS — 6 tests.

- [ ] **Step 5: Replace the inline block in `App.tsx`**

Replace the whole `{activeTab === 'ledger' && ( ... )}` block (`frontend/src/App.tsx:421-475`) with:

```tsx
            {activeTab === 'ledger' && (
              <TransactionLedger
                transactions={transactions}
                assets={currentPortfolio.assets || []}
                onDeleteTransaction={handleDeleteTransaction}
                onSelectSell={() => undefined}
              />
            )}
```

`onSelectSell` is a placeholder on purpose. Task 9 owns the modal state and replaces it with
`setSellPnlTxId`. Do **not** declare `sellPnlTxId` in this task: an unused `useState` binding trips
`@typescript-eslint/no-unused-vars`, and Step 6 runs the lint gate.

Add the import alongside the other component imports:

```tsx
import TransactionLedger from './components/TransactionLedger';
```

Remove the imports the extraction orphans, or `npm run lint` fails: `Trash2` from the `lucide-react` import list, and the whole `import { formatCurrency } from './utils/formatters';` line. `History` stays — it is still used by the nav item.

- [ ] **Step 6: Run the full frontend suite and lint**

Run: `cd frontend && npm run lint && npx vitest run 2>&1 | tail -25`
Expected: lint clean; all tests pass, including the pre-existing `App.test.tsx`.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/components/TransactionLedger.tsx frontend/src/App.tsx frontend/src/test/components/TransactionLedger.test.tsx
git commit -m "refactor: extract TransactionLedger with a clickable sell P&L affordance"
```

---

## Task 8: The `SellPnlModal`

**Files:**
- Create: `frontend/src/components/SellPnlModal.tsx`
- Modify: `frontend/src/test/mocks/handlers.ts`
- Modify: `frontend/src/test/setup.ts` (pin the test timezone)
- Create: `frontend/src/test/components/SellPnlModal.test.tsx`

**Interfaces:**
- Consumes: `GET /api/transactions/<id>/pnl`, `normalizeSellPnl` (Task 6), `formatCurrency`, `formatPercent`.
- Produces: `SellPnlModal` default export with props `{ transactionId: number; strategy: string; thresholdDays: number; onClose: () => void }`.

- [ ] **Step 1: Pin the test timezone, then add the MSW handler**

The modal renders dates with `toLocaleDateString`/`toLocaleString`, which are host-timezone dependent, and `vite.config.js` sets no `TZ`. `2024-01-01T00:00:00Z` renders as `2023-12-31` on any machine west of UTC, and two of the three new frontend test files would fail there for a reason that looks like a component bug. Add to `frontend/src/test/setup.ts`, above the existing imports' effects:

```ts
process.env.TZ = 'UTC'
```

The modal therefore renders dates in UTC, and the tests below assert UTC dates.

In `frontend/src/test/mocks/handlers.ts`, add before the catch-all `http.all('*', ...)` entry:

```ts
  http.get('/api/transactions/:id/pnl', ({ params }) => {
    const id = Number(params.id)
    return HttpResponse.json({
      transaction_id: id, asset_id: 1, symbol: 'AAPL', asset_type: 'STOCK',
      strategy: 'FIFO', threshold_days: 30,
      currency: 'USD', asset_currency: 'USD', fx_applied: false, split_applied: false,
      sell_date: '2024-12-01T00:00:00Z',
      sold_qty: '80.0', price: '130.0', fee: '0.0',
      proceeds: '10400.0', cost_basis: '8400.0', realized_pnl: '2000.0', roi: '0.238',
      matched_qty: '80.0', unmatched_qty: '0.0',
      matches: [
        { buy_transaction_id: 1, buy_date: '2024-01-01T00:00:00Z', buy_price: '100.0',
          unit_cost: '100.0', matched_qty: '60.0', lot_qty_before_match: '60.0',
          cost_basis: '6000.0', proceeds: '7800.0', pnl: '1800.0', holding_days: 335, term: null },
        { buy_transaction_id: 2, buy_date: '2024-06-01T00:00:00Z', buy_price: '120.0',
          unit_cost: '120.0', matched_qty: '20.0', lot_qty_before_match: '40.0',
          cost_basis: '2400.0', proceeds: '2600.0', pnl: '200.0', holding_days: 183, term: null },
      ],
    })
  }),
  http.get('/api/transactions/999/pnl', () => new HttpResponse(null, { status: 404 })),
```

- [ ] **Step 2: Write the failing modal tests**

Create `frontend/src/test/components/SellPnlModal.test.tsx`:

```tsx
import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import SellPnlModal from '../../components/SellPnlModal'
import { server } from '../mocks/server'
import { http, HttpResponse } from 'msw'

const base = {
  transaction_id: 42, asset_id: 1, symbol: 'AAPL', asset_type: 'STOCK',
  strategy: 'FIFO', threshold_days: 30,
  currency: 'USD', asset_currency: 'USD', fx_applied: false, split_applied: false,
  sell_date: '2024-12-01T00:00:00Z',
  sold_qty: '80.0', price: '130.0', fee: '0.0',
  proceeds: '10400.0', cost_basis: '8400.0', realized_pnl: '2000.0', roi: '0.238',
  matched_qty: '80.0', unmatched_qty: '0.0', matches: [],
}

const renderModal = (props = {}) => render(
  <SellPnlModal
    transactionId={42}
    strategy="FIFO"
    thresholdDays={30}
    onClose={vi.fn()}
    {...props}
  />,
)

describe('SellPnlModal', () => {
  it('shows the sell summary and every consumed lot', async () => {
    renderModal()
    // The symbol lives inside the heading text "Sell P&L — AAPL", so match it
    // as a substring; getByText with a bare string compares whole text content.
    await waitFor(() => expect(screen.getByText(/AAPL/)).toBeInTheDocument())
    expect(screen.getByText(/2,000/)).toBeInTheDocument()
    expect(screen.getByText('2024-01-01')).toBeInTheDocument()
    expect(screen.getByText('2024-06-01')).toBeInTheDocument()
    expect(screen.getByTestId('strategy-note')).toHaveTextContent('realized · FIFO')
  })

  it('footnotes the FX conversion for a foreign-currency asset', async () => {
    server.use(http.get('/api/transactions/42/pnl', () => HttpResponse.json({
      ...base, currency: 'EUR', asset_currency: 'USD', fx_applied: true,
    })))
    renderModal()
    await waitFor(() => expect(screen.getByTestId('fx-note')).toBeInTheDocument())
    expect(screen.getByTestId('fx-note')).toHaveTextContent('converted to EUR')
  })

  it('names the unmatched quantity instead of hiding it', async () => {
    server.use(http.get('/api/transactions/42/pnl', () => HttpResponse.json({
      ...base, sold_qty: '100.0', matched_qty: '50.0', unmatched_qty: '50.0', realized_pnl: '1000.0',
    })))
    renderModal()
    await waitFor(() => expect(screen.getByTestId('unmatched-banner')).toBeInTheDocument())
    expect(screen.getByTestId('unmatched-banner')).toHaveTextContent('50 of 100')
    // The wording is the finding, not the counts: the engine drops unmatched units from BOTH
    // sides, so the figure is incomplete. "overstated"/"understated" would be an unsupported claim.
    expect(screen.getByTestId('unmatched-banner')).toHaveTextContent(
      'Neither their cost basis nor their proceeds are included',
    )
    expect(screen.getByTestId('unmatched-banner')).not.toHaveTextContent('overstated')
  })

  it('explains a sell with nothing to match', async () => {
    server.use(http.get('/api/transactions/42/pnl', () => HttpResponse.json({
      ...base, sold_qty: '50.0', matched_qty: '0.0', unmatched_qty: '50.0',
      cost_basis: '0.0', proceeds: '0.0', realized_pnl: '0.0', roi: null,
    })))
    renderModal()
    await waitFor(() => expect(screen.getByTestId('no-lots-note')).toBeInTheDocument())
    expect(screen.getByTestId('no-lots-note')).toHaveTextContent('No buy lots were available to match')
  })

  it('renders roi as an em dash when cost basis is zero', async () => {
    server.use(http.get('/api/transactions/42/pnl', () => HttpResponse.json({
      ...base, cost_basis: '0.0', realized_pnl: '0.0', roi: null,
    })))
    renderModal()
    await waitFor(() => expect(screen.getByTestId('roi')).toBeInTheDocument())
    expect(screen.getByTestId('roi')).toHaveTextContent('—')
    expect(screen.getByTestId('roi')).not.toHaveTextContent('NaN')
    expect(screen.getByTestId('roi')).not.toHaveTextContent('Infinity')
  })

  it('footnotes split adjustment only when the response says a split applies', async () => {
    // Driven by the payload, not by a prop the caller computes: the server knows
    // whether a split touched this asset because it read the same rows the
    // numbers came from. `base` carries split_applied: false.
    renderModal()
    await waitFor(() => expect(screen.queryByTestId('split-note')).not.toBeInTheDocument())

    server.use(http.get('/api/transactions/42/pnl', () => HttpResponse.json({
      ...base, split_applied: true,
    })))
    renderModal()
    await waitFor(() => expect(screen.getByTestId('split-note')).toBeInTheDocument())
  })

  it('shows an error and retries', async () => {
    server.use(http.get('/api/transactions/42/pnl', () => new HttpResponse(null, { status: 500 })))
    renderModal()
    await waitFor(() => expect(screen.getByTestId('modal-error')).toBeInTheDocument())
    server.use(http.get('/api/transactions/42/pnl', () => HttpResponse.json(base)))
    fireEvent.click(screen.getByRole('button', { name: /retry/i }))
    await waitFor(() => expect(screen.getByText(/AAPL/)).toBeInTheDocument())
  })

  it('calls onClose when dismissed', async () => {
    const onClose = vi.fn()
    renderModal({ onClose })
    await waitFor(() => expect(screen.getByText(/AAPL/)).toBeInTheDocument())
    fireEvent.click(screen.getByRole('button', { name: /close/i }))
    expect(onClose).toHaveBeenCalled()
  })
})
```

- [ ] **Step 3: Run them to verify they fail**

Run: `cd frontend && npx vitest run src/test/components/SellPnlModal.test.tsx 2>&1 | tail -20`
Expected: FAIL — cannot resolve `../../components/SellPnlModal`.

- [ ] **Step 4: Create the modal**

Create `frontend/src/components/SellPnlModal.tsx`. Build it from the parts below. **The quoted strings are load-bearing** — the tests assert them verbatim, and several of them exist because a reviewer found the alternative was wrong, not because they read nicely. Do not paraphrase them.

**Imports:** `useEffect`/`useState` from react; `AlertTriangle`, `Info`, `Loader2`, `X` from `lucide-react`; `formatCurrency`, `formatPercent` from `../utils/formatters`; `normalizeSellPnl` from `../utils/decimal`; `SellPnlBreakdown` from `../types`. Local helper `pnlColor = (v: number) => (v >= 0 ? 'var(--color-success)' : 'var(--color-danger)')`.

**Props:** `{ transactionId: number; strategy: string; thresholdDays: number; onClose: () => void }`. Nothing about the split footnote is the caller's business — it reads `data.split_applied` from the payload.

**Fetch.** `useState` for `data` / `loading` / `error`, plus a `retryKey` counter. One `useEffect` keyed on `[transactionId, strategy, thresholdDays, retryKey]` fetching `` `/api/transactions/${transactionId}/pnl?strategy=${strategy}&threshold_days=${thresholdDays}` `` and piping the JSON through `normalizeSellPnl`. Keying on strategy and threshold is what makes the modal refetch when the Holdings tab changes strategy — that is the whole point, not an accident to prune.

**Overlay.** The existing `position: fixed` + `.glass-card` + `zIndex: 100` pattern from `PortfolioDetail.tsx`; backdrop click and the `X` button both call `onClose()`. `Loader2` + "Loading realized P&L..." while in flight. On failure, `data-testid="modal-error"` with a retry button that bumps `retryKey`.

**Header.** Symbol · sell date · `{sold_qty} sold @ {formatCurrency(price, asset_currency)}` · ` · fee {…}` only when the fee is non-zero · ` · proceeds {formatCurrency(proceeds, currency)}`. When `asset_currency !== currency`, insert `data-testid="as-entered-currency"` reading `price and fee as entered in {asset_currency}` — the payload carries two currencies and the reader must never guess which column is which.

**Realized line.** `formatCurrency(realized_pnl, currency)` in `pnlColor`; `data-testid="roi"` rendering `—` when `roi === null` else `formatPercent(roi)`; `data-testid="strategy-note"` reading `realized · {strategy}` plus ` (threshold {threshold_days}d)` only when the strategy is HYBRID.

**`data-testid="unmatched-banner"`** when `unmatched_qty > 0` (amber, `AlertTriangle`):

> `{unmatched_qty} of {sold_qty} units had no buy lot on or before this date. Neither their cost basis nor their proceeds are included in this figure.`

Quantities via `toLocaleString(undefined, { maximumFractionDigits: 6 })`. The sentence deliberately makes no claim about the figure being too high or too low — the engine drops unmatched units from both sides, so it is simply incomplete.

**`data-testid="no-lots-note"`** when `matches.length === 0`:

> `No buy lots were available to match on or before this date. Realized P&L is {formatCurrency(realized_pnl, currency)} because nothing was matched.`

"Available", not "existed": lots may have existed and earlier sells already drained them, which the payload cannot distinguish.

**Matches table**, rendered in the order received — do not sort. Columns: Buy date (`new Date(m.buy_date).toLocaleDateString('en-CA')`; UTC is pinned in `src/test/setup.ts`), Buy price (`formatCurrency(m.buy_price, data.currency)`), Qty from lot (`{m.matched_qty}` then a muted ` of {m.lot_qty_before_match}`), Cost basis, Proceeds, P&L (`formatCurrency(m.pnl, data.currency)` in `pnlColor`), Held (`{m.holding_days}d`), and a Term column only when HYBRID rendering `{m.term ?? '—'}`.

**Footnotes** (muted, `Info` icon, stacked):

- always: `Attribution follows the {strategy} method. A different method would consume different lots.`
- `data-testid="split-note"` when `data.split_applied`: `Buy prices and unit costs are split-adjusted, not the prices as entered.`
- `data-testid="fx-note"` when `fx_applied`: `Cost basis and proceeds were converted to {currency} at the latest available {asset_currency}→{currency} rate.` Not "at each transaction's date rate" — `get_rate` uses the requested date only as a cache key and returns the last non-null close, so every transaction in a replay converts at effectively the same rate.

- [ ] **Step 5: Run the tests**

Run: `cd frontend && npx vitest run src/test/components/SellPnlModal.test.tsx 2>&1 | tail -25`
Expected: PASS — 8 tests.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/components/SellPnlModal.tsx frontend/src/test/mocks/handlers.ts frontend/src/test/components/SellPnlModal.test.tsx
git commit -m "feat: sell P&L modal with per-lot breakdown and edge-case notes"
```

---

## Task 9: Wire the modal into `App` and prove the whole path

**Files:**
- Modify: `frontend/src/App.tsx`
- Create: `frontend/src/test/components/AppLedger.test.tsx`

**Interfaces:**
- Consumes: `TransactionLedger` (Task 7), `SellPnlModal` (Task 8), `transactions` state.
- Produces: the shipped flow — SELL row in the ledger opens the modal.

- [ ] **Step 1: Own the modal state**

In `frontend/src/App.tsx`, add to the UI state block (next to `showNewPortfolioModal`). Task 7 deliberately left this to Task 9, so it is not there yet:

```tsx
  const [sellPnlTxId, setSellPnlTxId] = useState<number | null>(null);
```

Add the import:

```tsx
import SellPnlModal from './components/SellPnlModal';
```

Clear the id whenever the modal's subject stops being the thing on screen — switching portfolio,
or deleting the transaction the modal is showing. Without this the modal keeps rendering a sell
belonging to the previous portfolio (with that portfolio's base currency resolved server-side),
or one that no longer exists, and the figures on screen describe nothing the user can see.

```tsx
  useEffect(() => { setSellPnlTxId(null); }, [selectedId]);
```

and in `handleDeleteTransaction` (`App.tsx`), after the successful `DELETE` and before the
`fetchPortfolioData` call:

```tsx
      setSellPnlTxId(null);
```

The modal needs no derived context: `split_applied` arrives in the payload, computed server-side from the rows the breakdown was built from. `App` only tracks which sell was clicked.

- [ ] **Step 2: Render the modal**

Add just before the closing `</main>` of the main content pane, inside the `currentPortfolio` branch so it only exists when a portfolio is active:

```tsx
        {sellPnlTxId !== null && (
          <SellPnlModal
            transactionId={sellPnlTxId}
            strategy={strategy}
            thresholdDays={thresholdDays}
            onClose={() => setSellPnlTxId(null)}
          />
        )}
```

- [ ] **Step 3: Write the integration test**

Create `frontend/src/test/components/AppLedger.test.tsx`:

```tsx
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import App from '../../App'
import { server } from '../mocks/server'
import { http, HttpResponse } from 'msw'

describe('App ledger → sell P&L', () => {
  beforeEach(() => {
    vi.stubGlobal('alert', vi.fn())
    vi.stubGlobal('confirm', vi.fn(() => true))
  })
  afterEach(() => vi.unstubAllGlobals())

  it('opens the breakdown when a SELL row is clicked', async () => {
    const user = userEvent.setup()
    server.use(http.get('/api/portfolios/1/transactions', () => HttpResponse.json([
      { id: 10, asset_id: 1, type: 'SELL', quantity: '80.0', price: '130.0', fee: '0.0', date: '2024-12-01T00:00:00Z' },
    ])))
    render(<App />)

    await waitFor(() => expect(screen.getByText('Transaction Ledger')).toBeInTheDocument())
    await user.click(screen.getByText('Transaction Ledger'))
    await user.click(await screen.findByRole('button', { name: /realized P&L for AAPL/i }))

    await waitFor(() => expect(screen.getByText(/Sell P&L/)).toBeInTheDocument())
    expect(await screen.findByText('2024-01-01')).toBeInTheDocument()
  })
})
```

- [ ] **Step 4: Run it**

Run: `cd frontend && npx vitest run src/test/components/AppLedger.test.tsx 2>&1 | tail -25`
Expected: PASS. If the ledger tab never appears, the MSW `/api/portfolios` fixture is not being served — check `server` setup in `src/test/mocks/server.ts` before changing the component.

- [ ] **Step 5: Full gate**

```bash
cd frontend && npm run lint && npx vitest run && cd ../backend_rust && cargo test
grep -n "get_price" src/api_routes/analytics.rs
```

Expected: lint clean, all frontend tests pass, all backend tests pass, and no `get_price` hit inside `get_transaction_sell_pnl`.

- [ ] **Step 6: Manual smoke (optional but recommended)**

Run `./start.sh` from the repo root, open the Transaction Ledger, click a SELL row, and confirm the per-lot P&L column sums to the realized P&L shown for that asset in the Holdings tab.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/App.tsx frontend/src/test/components/AppLedger.test.tsx
git commit -m "feat: open per-lot P&L breakdown from the transaction ledger"
```

---

## Done Criteria

1. Clicking a SELL row shows that sale's realized P&L and every lot it consumed with per-lot P&L.
2. Summing realized P&L over all sells of an asset equals the realized P&L shown for that asset in the Holdings tab, under FIFO, LIFO and HYBRID — proved by `test_sell_pnl_sums_to_tax_summary_realized_pnl`, which calls both endpoints, not only by the engine test.
3. Sells with unmatched quantity, or with nothing available to match, say so in words. The unmatched banner's exact wording is asserted, including that it does **not** claim the figure is overstated.
4. The modal requests and displays the strategy and threshold selected in the Holdings tab, and the response's `strategy` is the value that was applied (an unrecognized query value renders as `FIFO`).
5. No `get_price` call inside `get_transaction_sell_pnl`, checked with `grep -n "get_price" backend_rust/src/api_routes/analytics.rs`. FX `get_rate` lookups are expected when the asset currency differs from the base currency.
6. The engine tests present on `origin/main` (14 at the time of writing) are unmodified and green.
7. `cargo test`, `npm run lint` and `npx vitest run` are all green on the branch.
8. `docs/openapi/openapi.json` is non-empty, contains the new path, and its diff removes no existing path or schema; `frontend/src/types/api.d.ts` is regenerated from it.

**Known gap carried deliberately:** the FX conversion path has no backend test (every fixture is USD-into-USD, so `get_rate` is never reached). It is recorded in Task 4 Step 8 rather than silently untested. Do not describe the FX path as verified.
