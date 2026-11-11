# Dated Tax Report (CSV + On-Screen) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Answer "what did I sell in this window, at what cost basis, with what gain — and what did I still hold at the end of it" for a window the owner chooses, as a server-generated CSV and as a page, on the existing `GET /api/portfolios/<id>/tax-summary` route.

**Architecture:** One new public engine entry, `TaxLotEngine::replay_at`, filters `date <= to` before sorting and matching and returns open lots and per-sell breakdowns from a single pass. The route resolves one close per asset and covers each FX pair with a single range fetch *before* that replay, so every outbound call is in the route's own loop where a 20 s elapsed deadline can sit between calls. The route owns every total; the CSV is a projection of the payload the route already computed, so no number is ever re-derived in TypeScript.

**Tech Stack:** Rust (Rocket 0.5, SQLx/SQLite, utoipa 4.2, `rust_decimal::Decimal`), React 18 + Vite, TypeScript, Vitest + Testing Library + MSW.

**Spec:** `docs/superpowers/specs/2026-11-09-tax-report-and-statement-design.md`. Where this plan and the spec disagree, the spec wins unless the disagreement is recorded in *Deviations from the spec* at the bottom of this file — and then the reason is there. Review record: `docs/superpowers/reviews/2026-11-09-tax-report/consolidated_review_report.md` (§9 is the settled-values table this plan obeys).

## Global Constraints

- **Money is `rust_decimal::Decimal` end to end**, serialized with `decimal_json` (`db_types.rs:22`) for required money and `decimal_opt_json` (`db_types.rs:73`) for nullable money: JSON strings on the wire, exact values in Rust. Tests compare Decimals by value, never by serialized string, except where a test's whole point is the wire form.
- **The backend owns every total.** Every figure the screen or the file shows comes from the payload. No renderer — Rust or TypeScript — adds, averages or otherwise combines payload figures.
- **No new dependency.** Nothing is added to `backend_rust/Cargo.toml` or `frontend/package.json`.
- **No network in tests.** There is no `[dev-dependencies]` section and no HTTP mock: tests are hermetic *by construction*, which means every price and rate a test needs is seeded into `historical_prices` and every pricing call is answered by the database-first read path. A test that would need a provider seam is mis-designed and must be rewritten or explicitly recorded as unwritable (see Task 4, Step 8).
- **A stored price window answers only when it reaches the requested date.** `stored_window_is_fresh` (`currency_service.rs:65`) accepts a newest row within `PRICE_WINDOW_FRESH_DAYS = 4` of the requested end date. Every hermetic seed in this plan must reach within 4 days of the window's `to`, or the route will ask the provider and the test will reach the network.
- **Schema changes ship with a regenerated `docs/openapi/openapi.json`** in the same commit. CI diffs it byte-for-byte (`.github/workflows/ci.yml`, "Generate and Validate OpenAPI Spec"):
  ```bash
  cd backend_rust && cargo run --bin backend_rust -- --print-openapi > ../docs/openapi/openapi.json
  ```
  It needs no database, no server and no OIDC configuration.
- **Schema changes are idempotent DDL inside `init_db`** (`lib.rs:118`). There is no `migrations/` directory and no `sqlx::migrate!`. **This plan adds no table and no column**, so no DDL is required; if a task ever seems to need one, stop and report it.
- **One access rule: `Access::Viewer`, dated or not.** No route is added, so `tests/route_coverage_test.rs`'s hard counts (15 operations, `:140`, `:262`) must stay untouched, and `bash scripts/check_access_gate.sh` must keep passing. The grep-based gate cannot see an access-level choice on an existing route, so the dated variant is proven in `tests/isolation_test.rs` (Task 4).
- **Dates are UTC and inclusive.** `Transaction.date` is a `DateTime<Utc>` (`models.rs:35`), so "a sell at any time on `to` is inside the window" is `date.date_naive() <= to`. Either parameter alone is legal: `to` alone means cumulative from the portfolio's first transaction, `from` alone means through today. `from > to`, a future `from` or `to`, and an unparseable date are 422s.
- **The error contract is 422 / 503 with a typed body.** 422 for "no price or rate exists at or before this date" *and* for every validation failure (with `missing_prices`/`missing_rates` empty); 503 for "provider unreachable" and for the elapsed deadline. `retryable` is `false` for 422, `true` for 503. There is no 429 — the cost guard was deleted (decision D2). Lists are capped at 20 symbols and 10 rate pairs with `missing_prices_total` / `missing_rates_total` so truncation is never silent.
- **The elapsed deadline is `REPORT_DEADLINE = 20 s`, checked between outbound calls.** One asset can cost two calls (a close and a currency pair), so a check placed between assets can be followed by 66 s of work and the ingress' 60 s default would return an HTML 504 instead of the designed 503. It is a parameter, not a global, so a test can pass `Duration::ZERO`.
- **CSV convention: row 1 is always the header row, caveats ride as columns.** A leading `#` row is ruled out by evidence: `detectColumns` (`frontend/src/utils/csvParser.ts:25`) normalises every cell of row 1 against `FIELD_ALIASES`, so a `#` row maps every column to `null` and an import silently yields nothing.
- **Split sync is out of scope (issue #50).** The dated path does not call `sync_splits_for_asset`; the run context states that splits are as recorded. The undated path keeps its existing call, unchanged.
- **Backend gates**, all from `backend_rust/`: `cargo test`, `bash scripts/check_access_gate.sh`, `cargo clippy --all-targets`, `cargo build --release`.
- **Frontend gates**, all from `frontend/`: `npm run test`, `npm run lint`, `npm run typecheck`.
- **Branch:** `feat/report-export`. Commit after every task. Do not push.

## What is already true on this branch — verified against the code, not the spec

The spec was written against an unmerged pricing layer (`2026-11-09-pricing-layer-truthfulness.md`) and describes it in ways that no longer hold. The layer is merged (PR #51) and the statement half of the spec is already shipped here. Read this section before Task 1; it is what the tasks assume.

### The merged pricing layer, and what this plan relies on

1. **A freshness rule replaced "any row in the window".** `PRICE_WINDOW_FRESH_DAYS = 4` (`currency_service.rs:31`) is applied by `stored_window_is_fresh(newest, end_date)` (`:65`), and `get_historical_prices` (`:678`) is **database-first**: it answers from `historical_prices` only when the newest row in the window is within 4 days of `end_date`, and otherwise asks the provider for the tail and merges it over the stored rows (`merge_series`, `:75`). `stored_window_is_fresh`, `merge_series` and `select_rate` (`:52`) are pure functions.
   **The tax report relies on:** `get_historical_prices` as the one dated close lookup (see 3), and `stored_window_is_fresh` + `select_rate` as the *same* rule it uses to decide whether the database can answer (Task 4, `resolve_close`). Reusing the predicate is what stops the route's read order and the layer's from drifting. Every hermetic seed must reach within 4 days of `to`.
2. **`get_rate` takes the pool as a parameter** — `get_rate(&self, from_curr, to_curr, date: DateTime<Utc>, pool: &SqlitePool) -> Result<RateAt>` (`:396`) — and returns `RateAt { rate, date_used }`. Nothing is stored on the service.
   **The tax report relies on:** `date_used`, which is what `AssetTotals.fx_rate_date_used` reports, and the pool parameter, which is what lets `replay_at` convert a dated market value (Task 2).
3. **`fetch_yahoo_price` returns `Result<Decimal, PricingError>` and has exactly one caller** (`get_price`, `:545`). `get_price` also hardcodes `Utc::now()` (`:551`).
   **The tax report relies on:** the conclusion, not the function — **`get_price` cannot serve a dated report**, so the dated path uses `get_historical_prices` only. Do not reach for `get_price` in any task below.
4. **The outage fallback lives in the consumer, not the service.** `get_historical_prices` returns `Err(ProviderUnavailable)`/`Err(NoData)` and `StatsEngine::calculate_portfolio_performance` is the thing that falls back to `get_historical_prices_from_db` (`stats_engine.rs:139-160`), with a comment saying so.
   **The tax report relies on:** the *absence* of that fallback in the report path. The dated report fails closed (422/503, no partial payload) — that is Goal 5. `get_historical_prices_from_db` appears in this plan only as the route's own database-first read under the freshness predicate (Task 4), never as an outage fallback.
5. **The market-value FX leg converts at "now", not at the latest transaction date.** `calculate_lots` calls `get_rate(asset_currency, base_currency, Utc::now(), pool)` (`tax_engine.rs:578`) — there is no `latest_date` anywhere in the tree. The spec's "today the FX rate for market value is taken at `latest_date`" is **stale**, and so is its "truncating the list silently moves the FX date" motivation.
   **The tax report relies on:** the *requirement*, which survives the stale rationale unchanged — an undated endpoint may price at today, and a dated report must price at the date it was asked about. `replay_at`'s `valuation_date` parameter is what replaces that constant `Utc::now()`.

Two more things the pricing layer settled that this plan consumes: `PricingError::{NoData, ProviderUnavailable}` (`:258`) exists and is typed; `no_rate_error(...)` (`:277`) is a **plain `anyhow!` error with no type tag**, which is why Task 4's classification has to enumerate what `replay_at` can return rather than pattern-matching one enum.

### The statement half is shipped — do not re-plan it

`PortfolioStatement.tsx`, `src/print.css` (imported at `src/main.tsx:4`), the fifth tab in `App.tsx` and the `taxMeta` record all exist. The spec's Goal 6 is satisfied by that work. Two consequences this plan must respect:

- **`App.tsx` is edited by both features.** The tax report is a sixth tab, a sixth nav item, and a second fetch site. Read the current file first; do not restructure the statement branch (the fragment that keeps both children direct children of `.main-content` is load-bearing for `print.css`).
- **`print.css` hides the shell by *position*, not by class.** The Print button is hidden because it is a sibling of the statement root under `.main-content:has([data-print="statement"])`; there is no `.btn` rule. A download anchor on the report tab is therefore harmless — the report tab has no `[data-print="statement"]`, so nothing is hidden there and it prints like the other four tabs, exactly as today.

### Verdict on the three changes from the brief

| Change | Verified state | Effect on this plan |
| :-- | :-- | :-- |
| Pricing layer landed, differs from the spec | Confirmed, five drift points above | `resolve_close` reuses the layer's own predicate; `get_price` unusable; FX amounts (`FX_LOOKBACK_DAYS = 30`, `FX_SERIES_WINDOW_DAYS = 365`) reused, not re-invented |
| Statement half shipped | Confirmed on this branch | Goal 6 out of scope; `App.tsx`/`print.css` planned against what is there |
| Four values settled | Used verbatim (§9 of the review record) | Dates, caps, 20 s between calls, header-row CSV convention |

## Review Focus

The failure modes this design implies that no single task's own tests fully cover, most likely to bite first. Each gets a pinning test in the task that owns the code.

1. **`valuation_date` becomes a decorative parameter.** If `replay_at` ignores it and keeps a `Utc::now()`-shaped constant, a report dated 2024 converts a 2024 close at a 2026 rate and every number is plausible. Pinned by Task 2's test with two different seeded rates.
2. **A renderer sums the rows.** `AssetTotals.realized_in_window` is the number the owner reconciles against `tax-summary`; a screen that adds up `lot_pnl` shows a double and stops agreeing. Pinned by Task 6's deliberately inconsistent fixture (rows that do not sum to the payload's totals).
3. **A stale report renders under another portfolio's heading.** This is the statement's bug reborn: `setTaxSummary` runs once and is never cleared, and a page with B's numbers under A's title looks correct. Pinned by Task 6's portfolio-switch test.
4. **A split dated after `to` moves an already-filed window.** The split branch rescales every lot in scope, so the `date <= to` filter has to run before sorting and matching. Pinned by Task 2's split-after-`end` test.
5. **The undated response shape changes under the shipped statement.** The typed report payload cannot replace the legacy object — the statement and `PortfolioDetail` read `assets[].tax_lots`, `current_shares` and `total_portfolio_value`, which the typed payload does not carry. Pinned by Task 4's undated-shape test and by the statement's existing suite.

---

## Task 1: `TaxLot` reports the lot that was actually bought

Today `original_qty` and `remaining_qty` are both set to the already-decremented `lot.qty` (`tax_engine.rs:587-588`, `original_qty: lot.qty, remaining_qty: lot.qty`), so `PortfolioDetail.tsx:322-323,333-334` renders the same number in two columns and the originally-bought quantity is unrecoverable from the response. The buy id already exists upstream as `InternalLot.source_tx_id` (`tax_engine.rs:366`), so this is a field addition, not new plumbing. The spec assigns this fix to the report slice because `TaxLot` is registered in `openapi.rs:35` and appears in the committed `docs/openapi/openapi.json`, which CI diffs byte-for-byte.

This is the **only change in this plan that moves a figure on an existing screen**, and it is declared in Task 7's release note.

**Files:**
- Modify: `backend_rust/src/engines/tax_engine.rs` (`InternalLot`, the `replay` buy branch, the split branch, `calculate_lots`'s lot loop)
- Modify: `backend_rust/src/schemas.rs` (`TaxLot`)
- Regenerate: `docs/openapi/openapi.json`
- Modify: `frontend/src/types.ts` (`TaxLot`)
- Modify: `frontend/src/utils/decimal.ts:82-88` (`normalizeTaxSummary`'s `tax_lots` map)
- Modify: `frontend/src/test/components/PortfolioDetail.test.tsx` (the lot fixture at `:180-186`)
- Modify: `backend_rust/tests/common/mod.rs` (add `seed_price`, `seed_split_sync`)
- Test: `backend_rust/tests/analytics_test.rs` (new case)

**Interfaces:**
- Consumes: nothing.
- Produces: `TaxLot { buy_transaction_id: i32, original_qty }` where `original_qty` is the split-adjusted quantity originally bought. Task 2's `replay_at` returns the same two facts for open lots, from the same `InternalLot` fields.

- [ ] **Step 1: Add the seeding helpers the test needs**

Both are `#[allow(dead_code)]` like every other helper in that file, because each test binary compiles `common` separately and not all of them use every helper.

Append to `backend_rust/tests/common/mod.rs`:

```rust
/// One stored close. The pricing layer reads `historical_prices` before the provider,
/// so a seeded row is what makes a test hermetic — see `stored_window_is_fresh`
/// (`currency_service.rs:65`), which only answers when the newest row in the window is
/// within `PRICE_WINDOW_FRESH_DAYS` of the date asked for.
#[allow(dead_code)]
pub async fn seed_price(pool: &SqlitePool, symbol: &str, date: NaiveDate, close: &str) {
    sqlx::query(
        "INSERT INTO historical_prices (symbol, date, close_price) VALUES (?, ?, ?)
             ON CONFLICT(symbol, date) DO UPDATE SET close_price = excluded.close_price",
    )
    .bind(symbol)
    .bind(date)
    .bind(close)
    .execute(pool)
    .await
    .unwrap();
}

/// Pin split auto-detection for `symbol` so `sync_splits_for_asset` returns on its
/// 30-day cache check instead of fetching. Same row `analytics_test.rs:218` writes.
#[allow(dead_code)]
pub async fn seed_split_sync(pool: &SqlitePool, symbol: &str) {
    sqlx::query("INSERT OR REPLACE INTO split_sync (symbol, last_synced_at) VALUES (?, datetime('now'))")
        .bind(symbol)
        .execute(pool)
        .await
        .unwrap();
}
```

`NaiveDate` is already imported in that file (`use chrono::{DateTime, Utc};` — extend it to `use chrono::{DateTime, NaiveDate, Utc};`).

- [ ] **Step 2: Write the failing test**

Add to `backend_rust/tests/analytics_test.rs` (imports: add `seed_price`, `seed_split_sync` to the `common::` list):

```rust
#[tokio::test]
async fn a_lot_reports_the_quantity_that_was_bought_and_the_buy_it_came_from() {
    // Before this change `original_qty` and `remaining_qty` are both the already
    // decremented quantity, so the Holdings table prints the same number twice and the
    // originally-bought quantity is not recoverable from the response at all.
    //
    // Both seeds are load-bearing for hermeticity: `get_price` answers from today's row
    // instead of fetching, and `sync_splits_for_asset` returns on its cache check
    // instead of fetching. Neither call is mocked, so without them this test hits the
    // network and takes seconds.
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "Lot qty", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "LOTQTY", "Lot qty").await;
    let buy = seed_transaction_at(&pool, asset_id, "BUY", 60.0, 100.0, 0.0, "2024-01-01T00:00:00Z").await;
    seed_transaction_at(&pool, asset_id, "SELL", 20.0, 130.0, 0.0, "2024-12-01T00:00:00Z").await;
    seed_price(&pool, "LOTQTY", Utc::now().date_naive(), "110.0").await;
    seed_split_sync(&pool, "LOTQTY").await;

    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();
    let resp = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?strategy=FIFO&threshold_days=30"))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Ok);
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
    let lot = &body["assets"][0]["tax_lots"][0];

    assert_eq!(lot["buy_transaction_id"], buy, "the lot must name the buy it came from");
    assert_eq!(lot["original_qty"], "60.0", "60 were bought");
    assert_eq!(lot["remaining_qty"], "40.0", "20 were sold");
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cd backend_rust && cargo test --test analytics_test a_lot_reports_the_quantity_that_was_bought -- --nocapture`
Expected: FAIL — `lot["buy_transaction_id"]` is `null` and `lot["original_qty"]` is `"40.0"`.

- [ ] **Step 4: Track the original quantity on the internal lot**

In `backend_rust/src/engines/tax_engine.rs`, `struct InternalLot` gains two facts it already knows but discards:

```rust
struct InternalLot {
    source_tx_id: i32,
    date: DateTime<Utc>,
    price: Decimal,
    qty: Decimal,
    /// The quantity this lot was opened with, split-adjusted alongside `qty`, so a
    /// lot that has been partly sold still reports what was bought.
    original_qty: Decimal,
    unit_cost: Decimal,
}
```

In `replay`'s BUY branch, set `original_qty: tx_qty` in the pushed `InternalLot`. In the SPLIT branch, scale it with the rest:

```rust
let ratio = tx_qty / denominator;
for lot in buy_lots.iter_mut() {
    lot.qty *= ratio;
    lot.original_qty *= ratio;
    lot.unit_cost /= ratio;
    lot.price /= ratio;
}
```

In `calculate_lots`'s `for lot in buy_lots` loop, the pushed `TaxLot` becomes:

```rust
open_lots.push(TaxLot {
    buy_transaction_id: lot.source_tx_id,
    buy_date: lot.date,
    buy_price: lot.price,
    original_qty: lot.original_qty,
    remaining_qty: lot.qty,
    latent_gain_loss,
    latent_roi,
});
```

- [ ] **Step 5: Add the field to the schema**

In `backend_rust/src/schemas.rs`, `struct TaxLot` gains its first field:

```rust
/// The buy that opened this lot. Present so a lot can be joined back to the ledger
/// row a person is reading.
pub buy_transaction_id: i32,
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cd backend_rust && cargo test --test analytics_test`
Expected: PASS, including `test_tax_summary_with_assets_fifo`, whose lot fixture has no sell (`original_qty` and `remaining_qty` are both `"100.0"` there and stay that way — that assertion does not pin the bug and does not move).

- [ ] **Step 7: Regenerate the committed contract**

```bash
cd backend_rust && cargo run --bin backend_rust -- --print-openapi > ../docs/openapi/openapi.json
git diff --stat ../docs/openapi/openapi.json
```

Expected: a diff adding `buy_transaction_id` to `components.schemas.TaxLot.required` and `.properties` as `{"type": "integer", "format": "int32"}`. Commit the regenerated file in Step 11; a task that changes `TaxLot` without it fails CI.

- [ ] **Step 8: Carry the field through the frontend type and its normalizer**

In `frontend/src/types.ts`, `interface TaxLot` gains `buy_transaction_id: number;` (first field, matching the payload order).

In `frontend/src/utils/decimal.ts`, inside `normalizeTaxSummary`'s `tax_lots.map`, add to the returned object:

```ts
            buy_transaction_id: num(lr, 'buy_transaction_id'),
```

This line is forced, not optional: the normalizer rebuilds each lot, and `TaxLot` now requires the field, so `npm run typecheck` fails without it. Nothing renders it.

- [ ] **Step 9: Fix the test fixture and pin the two columns apart**

In `frontend/src/test/components/PortfolioDetail.test.tsx`, the lot fixture (`original_qty: 50`, `remaining_qty: 50`) gains `buy_transaction_id: 1` for the type, and the two quantities are pulled apart so the screen's two columns are pinned to two different sources:

```ts
            {
              buy_transaction_id: 1,
              buy_date: '2024-01-15T00:00:00Z',
              buy_price: 150,
              original_qty: 50,
              remaining_qty: 30,
              latent_gain_loss: 750,
              latent_roi: 0.1667,
            },
```

`PortfolioDetail.tsx` itself does **not** change: it already reads `lot.original_qty` and `lot.remaining_qty` into two cells (`:333-334`). Only the value the backend sends was wrong.

- [ ] **Step 10: Run every gate this task touches**

```bash
cd backend_rust && cargo test && bash scripts/check_access_gate.sh && cargo clippy --all-targets
cd ../frontend && npm run test && npm run lint && npm run typecheck
```

Expected: all green; `openapi.json` matches the regenerated file (`git status` clean of unintended edits).

- [ ] **Step 11: Commit**

```bash
git add backend_rust/src/engines/tax_engine.rs backend_rust/src/schemas.rs \
        backend_rust/tests/common/mod.rs backend_rust/tests/analytics_test.rs \
        docs/openapi/openapi.json frontend/src/types.ts frontend/src/utils/decimal.ts \
        frontend/src/test/components/PortfolioDetail.test.tsx
git commit -m "fix(tax): report a lot's original quantity and the buy it came from"
```

---

## Task 2: `replay_at` — the one public dated replay

Three separate changes are required and doing the obvious thing — adding a single date argument to `calculate_lots` — introduces three bugs, which the spec enumerates under *The engine change: one public dated replay*. This task adds one public entry point that returns **both** open lots and per-sell breakdowns from one pass, filters `date <= end` before sorting and matching, and takes the valuation date as an explicit parameter. Alternative E (two engine functions) is rejected: a sell's matches depend only on transactions at or before it, so one prefix replay yields every breakdown and the two report figures are selections over it.

`calculate_lots` and `sell_breakdown` are **not** refactored to call this: `calculate_lots` must keep including a future-dated transaction (it has no `end`), and changing it would move figures on a live endpoint. The duplication is deliberate and is held together by a test (Step 4).

**Files:**
- Modify: `backend_rust/src/engines/tax_engine.rs` (`replay_at` and its in-file tests)
- Modify: `backend_rust/src/schemas.rs` (`DatedLot`, `DatedReplay`, beside `SellBreakdown`)

**Interfaces:**
- Consumes: `InternalLot.source_tx_id`, `InternalLot.original_qty` and `InternalLot.unit_cost` from Task 1; `CurrencyService::get_rate(..., pool) -> Result<RateAt>`; the private `replay(...) -> Result<(Vec<InternalLot>, Vec<SellBreakdown>)>`.
- Produces, for Task 4:
  ```rust
  pub struct DatedReplay {
      pub open_lots: Vec<DatedLot>,
      pub sell_breakdowns: Vec<SellBreakdown>,
      pub current_price_base: Decimal,
      pub fx_rate_date_used: Option<NaiveDate>,
  }
  pub struct DatedLot {
      pub buy_transaction_id: i32,
      pub buy_date: DateTime<Utc>,
      pub buy_price: Decimal,      // asset currency, split-adjusted
      pub original_qty: Decimal,   // split-adjusted
      pub remaining_qty: Decimal,
      pub unit_cost: Decimal,      // base currency, fee-amortised
      pub market_value: Decimal,   // base currency
      pub unrealized_gain_loss: Decimal,
      pub unrealized_roi: Option<Decimal>,
  }
  pub async fn replay_at(
      transactions: &[Transaction], end: NaiveDate, valuation_date: NaiveDate,
      current_price: Decimal, asset_currency: &str, base_currency: &str,
      currency_service: &CurrencyService, strategy: &str,
      hybrid_threshold_days: i64, pool: &SqlitePool,
  ) -> Result<DatedReplay>
  ```

- [ ] **Step 1: Write the failing test — the `date <= end` filter**

In the `#[cfg(test)] mod tests` block of `backend_rust/src/engines/tax_engine.rs` (extend `use chrono::{...}` there with `NaiveDate`), add the helper and four tests. Start with the helper:

```rust
    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    /// A pool with the real schema, for the fixtures that seed FX rows. `empty_pool()`
    /// above is a bare `:memory:` connection with no tables, which is enough for the
    /// USD/USD fixtures and not enough for these.
    async fn priced_pool() -> SqlitePool {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        let cfg = crate::auth::StartupConfig::from_env_with(&|key| {
            (key == "AUTH_MODE").then(|| "disabled".to_string())
        })
        .expect("AUTH_MODE=disabled parses in a debug build");
        crate::init_db(&pool, &cfg).await.unwrap();
        pool
    }

    async fn seed_fx(pool: &SqlitePool, symbol: &str, rows: &[(&str, &str)]) {
        for (date, price) in rows {
            sqlx::query("INSERT INTO historical_prices (symbol, date, close_price) VALUES (?, ?, ?)")
                .bind(symbol)
                .bind(day(date))
                .bind(*price)
                .execute(pool)
                .await
                .unwrap();
        }
    }
```

```rust
    #[tokio::test]
    async fn test_replay_at_excludes_a_sell_after_the_end_date() {
        let txs = vec![
            make_tx(1, "BUY", "100.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "SELL", "100.0", "120.0", "0.0", "2024-12-01T00:00:00Z"),
        ];
        let svc = CurrencyService::new();
        let pool = empty_pool().await;
        let out = TaxLotEngine::replay_at(
            &txs, day("2024-06-30"), day("2024-06-30"),
            Decimal::from_str("110.0").unwrap(), "USD", "USD", &svc, "FIFO", 30, &pool,
        )
        .await
        .unwrap();

        assert!(out.sell_breakdowns.is_empty(), "a sell after `end` is outside the window");
        assert_eq!(out.open_lots.len(), 1);
        assert_eq!(out.open_lots[0].remaining_qty, Decimal::from_str("100.0").unwrap());
        assert_eq!(out.open_lots[0].buy_transaction_id, 1);
    }
```

- [ ] **Step 2: Run it and confirm it does not compile**

Run: `cd backend_rust && cargo test --lib engines::tax_engine::tests::test_replay_at_excludes_a_sell_after_the_end_date`
Expected: FAIL to compile — `no function or associated item named replay_at`.

- [ ] **Step 3: Add the two return types to `schemas.rs`**

Immediately after `struct SellBreakdown` in `backend_rust/src/schemas.rs` (engine outputs live here, which is where `TaxLot` and `SellBreakdown` already are; neither type is registered in `openapi.rs`, exactly like `SellBreakdown`, so this changes no committed schema):

```rust
/// One lot still open at `end`, valued at the requested date. The report's OPEN_LOT row
/// is this struct; `unrealized_roi` is `None` rather than zero when the lot has no cost
/// basis, so a consumer can tell "not applicable" from "0%".
#[derive(Debug, Clone)]
pub struct DatedLot {
    pub buy_transaction_id: i32,
    pub buy_date: DateTime<Utc>,
    /// Split-adjusted purchase price per unit, in the asset's own currency.
    pub buy_price: Decimal,
    /// The quantity originally bought, split-adjusted: a 100-share lot after a 2:1 split
    /// is 200 whether or not any of it has been sold.
    pub original_qty: Decimal,
    pub remaining_qty: Decimal,
    /// Fee-amortised cost per unit, in the portfolio's base currency.
    pub unit_cost: Decimal,
    pub market_value: Decimal,
    pub unrealized_gain_loss: Decimal,
    pub unrealized_roi: Option<Decimal>,
}

/// One dated replay: the open lots and every sell's lot attribution, from a single pass
/// over the transactions with `date <= end`.
#[derive(Debug, Clone)]
pub struct DatedReplay {
    pub open_lots: Vec<DatedLot>,
    pub sell_breakdowns: Vec<SellBreakdown>,
    /// The base-currency price the open lots were valued at. The report sums lot values
    /// and never multiplies this by a quantity of its own.
    pub current_price_base: Decimal,
    /// The date whose close priced the market value, or `None` when the asset and the
    /// base currency are the same and no conversion happened.
    pub fx_rate_date_used: Option<NaiveDate>,
}
```

- [ ] **Step 4: Write the rest of the tests before the implementation**

Add these three to the same in-file test module. They are the assertions the implementation has to satisfy, and each one fails differently if a piece of the rule is dropped.

```rust
    #[tokio::test]
    async fn test_replay_at_a_split_after_end_does_not_move_the_window() {
        // The split branch rescales every lot in scope, so the filter has to run before
        // sorting and matching rather than after. Without the filter this fixture
        // reports a 200-unit lot at a 50.0 unit cost for a window that closed a month
        // before the split happened.
        let txs = vec![
            make_tx(1, "BUY", "100.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "SPLIT", "2.0", "1.0", "0.0", "2024-08-01T00:00:00Z"),
        ];
        let svc = CurrencyService::new();
        let pool = empty_pool().await;
        let out = TaxLotEngine::replay_at(
            &txs, day("2024-06-30"), day("2024-06-30"),
            Decimal::from_str("100.0").unwrap(), "USD", "USD", &svc, "FIFO", 30, &pool,
        )
        .await
        .unwrap();

        assert_eq!(out.open_lots.len(), 1);
        assert_eq!(out.open_lots[0].remaining_qty, Decimal::from_str("100.0").unwrap());
        assert_eq!(out.open_lots[0].original_qty, Decimal::from_str("100.0").unwrap());
        assert_eq!(out.open_lots[0].unit_cost, Decimal::from_str("100.0").unwrap());
        assert!(out.sell_breakdowns.is_empty());
    }

    #[tokio::test]
    async fn test_replay_at_prices_the_market_value_at_the_valuation_date() {
        // Review Focus 1. The realized leg converts the buy at the buy's own date
        // (1.0000 here) and the market-value leg converts at `valuation_date` (1.2500).
        // A replay that used one rate for both legs reports a 10000.0 market value and
        // no gain at all -- plausible enough to survive a reader who is not looking for
        // a date.
        let pool = priced_pool().await;
        seed_fx(&pool, "EURUSD=X", &[
            ("2024-01-01", "1.0000"),
            ("2024-03-29", "1.2500"),
            ("2024-03-30", "1.2500"),
            ("2024-03-31", "1.2500"),
        ]).await;
        let txs = vec![make_tx(1, "BUY", "100.0", "100.0", "0.0", "2024-01-01T00:00:00Z")];
        let svc = CurrencyService::new();

        let out = TaxLotEngine::replay_at(
            &txs, day("2024-03-31"), day("2024-03-31"),
            Decimal::from_str("100.0").unwrap(), "EUR", "USD", &svc, "FIFO", 30, &pool,
        )
        .await
        .unwrap();

        assert_eq!(out.fx_rate_date_used, Some(day("2024-03-31")));
        assert_eq!(out.current_price_base, Decimal::from_str("125.0").unwrap());
        assert_eq!(out.open_lots[0].unit_cost, Decimal::from_str("100.0").unwrap(),
            "the realized leg converts the buy at the buy's own date");
        assert_eq!(out.open_lots[0].market_value, Decimal::from_str("12500.0").unwrap());
        assert_eq!(out.open_lots[0].unrealized_gain_loss, Decimal::from_str("2500.0").unwrap());
    }

    #[tokio::test]
    async fn test_replay_at_agrees_with_calculate_lots_when_both_look_at_today() {
        // Two copies of the same valuation loop is the cost of leaving `calculate_lots`
        // alone; this is what keeps them from drifting. USD/USD, so neither leg needs a
        // rate and `calculate_lots`'s `Utc::now()` is a no-op.
        let txs = vec![
            make_tx(1, "BUY", "60.0", "100.0", "0.0", "2024-01-01T00:00:00Z"),
            make_tx(2, "BUY", "40.0", "120.0", "0.0", "2024-06-01T00:00:00Z"),
            make_tx(3, "SELL", "80.0", "130.0", "0.0", "2024-12-01T00:00:00Z"),
        ];
        let today = ChronoUtc::now().date_naive();
        let price = Decimal::from_str("110.0").unwrap();

        for strategy in ["FIFO", "LIFO", "HYBRID"] {
            let svc = CurrencyService::new();
            let pool = empty_pool().await;
            let summary = TaxLotEngine::calculate_lots(
                "TEST", "STOCK", &txs, price, "USD", "USD", &svc, strategy, 30, &pool,
            )
            .await
            .unwrap();
            let out = TaxLotEngine::replay_at(
                &txs, today, today, price, "USD", "USD", &svc, strategy, 30, &pool,
            )
            .await
            .unwrap();

            assert_eq!(out.open_lots.len(), summary.tax_lots.len(), "{strategy}: one row per open lot");
            for (dated, taxed) in out.open_lots.iter().zip(summary.tax_lots.iter()) {
                assert_eq!(dated.buy_transaction_id, taxed.buy_transaction_id, "{strategy}");
                assert_eq!(dated.buy_price, taxed.buy_price, "{strategy}");
                assert_eq!(dated.original_qty, taxed.original_qty, "{strategy}");
                assert_eq!(dated.remaining_qty, taxed.remaining_qty, "{strategy}");
                assert_eq!(dated.unrealized_gain_loss, taxed.latent_gain_loss, "{strategy}");
                assert_eq!(
                    dated.unrealized_roi.unwrap_or(Decimal::ZERO),
                    taxed.latent_roi,
                    "{strategy}: the dated report calls a zero-cost lot's ROI null, the undated summary calls it zero",
                );
            }
            assert_eq!(
                out.sell_breakdowns.iter().map(|b| b.realized_pnl).sum::<Decimal>(),
                summary.realized_pnl,
                "{strategy}: every breakdown is present, so the selection is the only difference",
            );
        }
    }
```

- [ ] **Step 5: Implement `replay_at`**

In `backend_rust/src/engines/tax_engine.rs`, add the import (`use chrono::{DateTime, NaiveDate, Utc};`) and `use crate::schemas::{AssetTaxSummary, DatedLot, DatedReplay, LotMatch, SellBreakdown, TaxLot};`, then add the method to `impl TaxLotEngine`, immediately after `calculate_lots`:

```rust
    /// The engine's one dated entry point: the open lots and every sell's lot
    /// attribution at `end`, from a single pass.
    ///
    /// `end` filters transactions; `valuation_date` prices them for the market value. A
    /// caller that has no separate valuation date passes `end` for both — which is what
    /// the terminated report does. The two are separate parameters because the
    /// alternative (deriving the FX date from the transaction list, the way the old
    /// `latest_date` did) silently prices a past date with whatever rate the last
    /// transaction happened to carry.
    ///
    /// The `date <= end` filter runs *before* `sorted_transactions`, not after, because
    /// the SPLIT branch rescales every lot in scope: a split dated after `end` would
    /// move the cost basis of a window that closed before it happened.
    ///
    /// `too_many_arguments` is silenced for the same reason as `calculate_lots`: the
    /// tenth is the `SqlitePool` threaded through for `get_rate`.
    #[allow(clippy::too_many_arguments)]
    pub async fn replay_at(
        transactions: &[Transaction],
        end: NaiveDate,
        valuation_date: NaiveDate,
        current_price: Decimal,
        asset_currency: &str,
        base_currency: &str,
        currency_service: &CurrencyService,
        strategy: &str,
        hybrid_threshold_days: i64,
        pool: &SqlitePool,
    ) -> Result<DatedReplay> {
        let in_window: Vec<Transaction> = transactions
            .iter()
            .filter(|tx| tx.date.date_naive() <= end)
            .cloned()
            .collect();
        let sorted_txs = Self::sorted_transactions(&in_window);

        // An asset with nothing dated at or before `end` has no lots and needs no rate:
        // returning here is what keeps a position bought after the window from costing
        // an outbound FX call.
        if sorted_txs.is_empty() {
            return Ok(DatedReplay {
                open_lots: Vec::new(),
                sell_breakdowns: Vec::new(),
                current_price_base: Decimal::ZERO,
                fx_rate_date_used: None,
            });
        }

        let mut fx_rate_date_used = None;
        let current_price_base = if asset_currency != base_currency {
            // The market-value leg marks the position at the close the caller quoted for
            // `valuation_date`, so it is converted at that date's rate. The realized legs
            // in `replay` are each converted at their own transaction date, which is the
            // correct rule for a historical figure and why the two legs cannot share one
            // lookup.
            let at = currency_service
                .get_rate(
                    asset_currency,
                    base_currency,
                    valuation_date.and_hms_opt(0, 0, 0).unwrap().and_utc(),
                    pool,
                )
                .await?;
            fx_rate_date_used = Some(at.date_used);
            current_price * at.rate
        } else {
            current_price
        };

        let (buy_lots, sell_breakdowns) = Self::replay(
            &sorted_txs,
            pool,
            currency_service,
            asset_currency,
            base_currency,
            strategy,
            hybrid_threshold_days,
        )
        .await?;

        let mut open_lots = Vec::new();
        for lot in buy_lots {
            if lot.qty <= Decimal::ZERO {
                continue;
            }
            let lot_cost = lot.qty * lot.unit_cost;
            let lot_market_value = lot.qty * current_price_base;
            let unrealized_gain_loss = lot_market_value - lot_cost;
            let unrealized_roi = if lot_cost > Decimal::ZERO {
                Some(unrealized_gain_loss / lot_cost)
            } else {
                None
            };
            open_lots.push(DatedLot {
                buy_transaction_id: lot.source_tx_id,
                buy_date: lot.date,
                buy_price: lot.price,
                original_qty: lot.original_qty,
                remaining_qty: lot.qty,
                unit_cost: lot.unit_cost,
                market_value: lot_market_value,
                unrealized_gain_loss,
                unrealized_roi,
            });
        }

        Ok(DatedReplay { open_lots, sell_breakdowns, current_price_base, fx_rate_date_used })
    }
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cd backend_rust && cargo test --lib engines::tax_engine`
Expected: PASS — all four new tests and every pre-existing engine test.

- [ ] **Step 7: Run every gate this task touches**

```bash
cd backend_rust && cargo test && bash scripts/check_access_gate.sh && cargo clippy --all-targets
```

Expected: all green. No schema is registered in `openapi.rs` by this task, so `docs/openapi/openapi.json` is unchanged — confirm with `cargo run --bin backend_rust -- --print-openapi | diff - ../docs/openapi/openapi.json`.

- [ ] **Step 8: Commit**

```bash
git add backend_rust/src/engines/tax_engine.rs backend_rust/src/schemas.rs
git commit -m "feat(engine): add a dated replay returning open lots and sell breakdowns"
```

---

## Task 3: The report payload — types, registration, generated contract

The payload is a typed `ToSchema` struct, not the `serde_json::Value` that `tax-summary` declares today (`analytics.rs:16`, which produces `"schema": {}` in the committed spec). Every money field is a decimal string: `decimal_json` for required money, `decimal_opt_json` for nullable money, because `UNMATCHED` rows carry nulls and "nulls are nulls" rather than zeros.

This task lands the contract one commit ahead of the behavior: the route's documented 200 body becomes `ReportResponse` here and Task 4 makes the route produce it. Nothing asserts the served response schema beyond `route_coverage_test.rs`'s operation counts, and no route is added, so those counts stay at 15.

**Files:**
- Modify: `backend_rust/src/schemas.rs` (the report payload types, at the end of the file)
- Modify: `backend_rust/src/openapi.rs` (`components(schemas(...))` at `:26-40`, and the `tax-summary` `responses` block in `analytics.rs:10-19`)
- Regenerate: `docs/openapi/openapi.json`
- Regenerate: `frontend/src/types/api.d.ts` (`npm run generate-types`)
- Modify: `frontend/src/types.ts` (hand-written report types — `api.d.ts` has zero importers, so these are what the renderer in Task 6 uses)
- Test: `backend_rust/tests/tax_report_test.rs` (new file)

**Interfaces:**
- Consumes: `DatedLot`, `DatedReplay` (Task 2), `SellBreakdown`, `LotMatch`, `Asset`, `Transaction`.
- Produces:
  ```rust
  pub enum RowKind { SoldLot, OpenLot, Unmatched }        // "SOLD_LOT" | "OPEN_LOT" | "UNMATCHED"
  pub enum PriceSource { Database, Provider }             // "database" | "provider"
  pub struct ReportContext { portfolio_id, portfolio_name, base_currency, strategy,
                             threshold_days, requested_from: Option<NaiveDate>,
                             requested_to: NaiveDate, splits_note: String }
  pub struct AssetTotals { asset_id, symbol, asset_type, asset_currency,
                           realized_in_window, realized_cumulative, unrealized_at_end,
                           market_value, cost_basis, price_date_used, fx_rate_date_used,
                           price_source, fx_applied, split_applied }
  pub struct ReportTotals { realized_in_window, realized_cumulative, unrealized_at_end,
                            market_value, cost_basis }
  pub struct ReportRow { .. }                             // one shape, three kinds, nulls not omissions
  pub struct ReportResponse { context, asset_totals, totals, rows }
  pub struct ErrorBody { error, missing_prices, missing_rates, missing_prices_total,
                         missing_rates_total, retryable }
  impl ReportRow { pub fn sold_lot(..), pub fn open_lot(..), pub fn unmatched(..) }
  ```

- [ ] **Step 1: Write the failing test**

Create `backend_rust/tests/tax_report_test.rs` (it grows in Tasks 4 and 5):

```rust
//! The dated tax report: the payload's typing, the three reconciliation equalities and
//! the failure contract.
//!
//! Hermetic by construction, like `fx_pricing_test.rs`: every price and rate these tests
//! need is seeded into `historical_prices`, and the pricing layer reads the database
//! before the provider.

mod common;

use backend_rust::models::{Asset, Transaction};
use backend_rust::schemas::{LotMatch, PriceSource, ReportRow, RowKind, SellBreakdown};
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use std::str::FromStr;

fn day(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn ts(s: &str) -> DateTime<Utc> {
    day(s).and_hms_opt(0, 0, 0).unwrap().and_utc()
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn fixture_asset() -> Asset {
    Asset {
        id: 1,
        portfolio_id: 1,
        symbol: "AAPL".to_string(),
        name: "Apple".to_string(),
        asset_type: "STOCK".to_string(),
        sector: None,
        currency: "USD".to_string(),
        isin: None,
    }
}

fn fixture_sell(id: i32) -> Transaction {
    Transaction {
        id,
        asset_id: 1,
        r#type: "SELL".to_string(),
        quantity: "80.0".to_string(),
        price: "130.0".to_string(),
        fee: "0.0".to_string(),
        date: ts("2024-12-01"),
    }
}

/// A sell of 80 units against one 60-unit lot, so 20 units are unmatched: the fixture
/// that carries both shapes, and the same one the engine's own tests use.
fn fixture_breakdown(sell_id: i32) -> SellBreakdown {
    SellBreakdown {
        sell_transaction_id: sell_id,
        sell_date: ts("2024-12-01"),
        sold_qty: dec("80.0"),
        matched_qty: dec("60.0"),
        unmatched_qty: dec("20.0"),
        proceeds: dec("7800.0"),
        cost_basis: dec("6000.0"),
        realized_pnl: dec("1800.0"),
        matches: vec![LotMatch {
            buy_transaction_id: 1,
            buy_date: ts("2024-01-01"),
            buy_price: dec("100.0"),
            unit_cost: dec("100.0"),
            matched_qty: dec("60.0"),
            lot_qty_before_match: dec("60.0"),
            cost_basis: dec("6000.0"),
            proceeds: dec("7800.0"),
            pnl: dec("1800.0"),
            holding_days: 335,
            term: None,
        }],
    }
}

#[test]
fn an_unmatched_row_serialises_its_money_cells_as_null_not_zero() {
    // The rule the whole payload obeys: a consumer has to be able to tell "not
    // applicable" from "zero". `UNMATCHED` is where every money cell is inapplicable,
    // which makes it the row kind where a default would be easiest and least visible.
    let asset = fixture_asset();
    let sell = fixture_sell(7);
    let breakdown = fixture_breakdown(7);
    let row = ReportRow::unmatched(&asset, None, None, Some(PriceSource::Database), &sell, &breakdown);
    let v = serde_json::to_value(&row).unwrap();

    assert_eq!(v["row_type"], RowKind::Unmatched.as_str());
    assert_eq!(v["sell_transaction_id"], 7);
    assert_eq!(v["unmatched_qty"], "20.0");
    assert!(v["lot_pnl"].is_null(), "money that does not exist is null, not \"0\"");
    assert!(v["realized_pnl"].is_null());
    assert!(v["proceeds"].is_null());
    assert!(v["buy_transaction_id"].is_null());
    assert_eq!(v["price_source"], "database");
}

#[test]
fn a_sold_lot_row_carries_money_as_decimal_strings() {
    let asset = fixture_asset();
    let sell = fixture_sell(3);
    let breakdown = fixture_breakdown(3);
    let matched = &breakdown.matches[0];
    let row = ReportRow::sold_lot(
        &asset, Some(day("2024-06-30")), None, Some(PriceSource::Database),
        &sell, &breakdown, matched,
    );
    let v = serde_json::to_value(&row).unwrap();

    assert_eq!(v["row_type"], RowKind::SoldLot.as_str());
    assert_eq!(v["lot_pnl"], "1800.0", "LotMatch.pnl under the CSV's name for it");
    assert_eq!(v["realized_pnl"], "1800.0");
    assert_eq!(v["sell_price"], "130.0", "as entered, in the asset's currency");
    assert_eq!(v["unmatched_qty"], "20.0", "the sell's context repeats on every lot row");
    assert!(v["market_value"].is_null(), "an open-lot cell on a sold-lot row");
    assert_eq!(v["price_date_used"], "2024-06-30");
}
```


- [ ] **Step 2: Run it to verify it fails**

Run: `cd backend_rust && cargo test --test tax_report_test`
Expected: FAIL to compile — no `ReportRow` in `schemas`.

- [ ] **Step 3: Add the payload types**

Append to `backend_rust/src/schemas.rs` (imports: extend the `chrono` line to `use chrono::{DateTime, NaiveDate, Utc};`):

```rust
// --- Dated tax report -------------------------------------------------------------

/// How the dated report priced an asset's close. Recorded so a regenerated file can
/// state where its numbers came from (Goal 4), and so a database-answered report can be
/// told from a provider-answered one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PriceSource {
    Database,
    Provider,
}

impl PriceSource {
    /// The token the CSV column carries. One spelling per concept.
    pub fn as_str(self) -> &'static str {
        match self {
            PriceSource::Database => "database",
            PriceSource::Provider => "provider",
        }
    }
}

/// Which selection a row is. Exhaustive on purpose: both renderers switch on it, and the
/// CSV's "this column totals, that one does not" rule is expressed through it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RowKind {
    SoldLot,
    OpenLot,
    Unmatched,
}

impl RowKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RowKind::SoldLot => "SOLD_LOT",
            RowKind::OpenLot => "OPEN_LOT",
            RowKind::Unmatched => "UNMATCHED",
        }
    }
}

/// What a dated request was answered about. `ReportContext` is the single authority for
/// `strategy` and `threshold_days`: the response does not repeat them at top level.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReportContext {
    pub portfolio_id: i32,
    pub portfolio_name: String,
    pub base_currency: String,
    pub strategy: String,
    pub threshold_days: i64,
    /// The window actually applied, not the parameters as typed. `requested_to` is the
    /// call's `to`, or today when `to` was absent; `requested_from` is the call's `from`,
    /// or the portfolio's first transaction on or before `requested_to`, or `None` when
    /// the portfolio has no transaction at all. An open end is visible in the artifact
    /// rather than inferred by its reader.
    pub requested_from: Option<NaiveDate>,
    pub requested_to: NaiveDate,
    /// Splits are as recorded when this was generated; the dated path deliberately does
    /// not sync them (issue #50), so a split recorded after the fact changes past
    /// figures. A constant string in the payload rather than prose in each renderer,
    /// because the CSV carries it as a cell (the file's first row is the header row).
    pub splits_note: String,
}

/// One asset's dated totals. Every field is computed in Rust from the same replay that
/// produced the rows; no renderer sums anything.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AssetTotals {
    pub asset_id: i32,
    pub symbol: String,
    pub asset_type: String,
    pub asset_currency: String,
    #[serde(with = "decimal_json")]
    pub realized_in_window: Decimal,
    #[serde(with = "decimal_json")]
    pub realized_cumulative: Decimal,
    #[serde(with = "decimal_json")]
    pub unrealized_at_end: Decimal,
    #[serde(with = "decimal_json")]
    pub market_value: Decimal,
    #[serde(with = "decimal_json")]
    pub cost_basis: Decimal,
    /// The close that priced this asset, and the date it actually came from. Null when
    /// the asset has no transaction at or before the window end, so no close was needed.
    pub price_date_used: Option<NaiveDate>,
    pub fx_rate_date_used: Option<NaiveDate>,
    pub price_source: Option<PriceSource>,
    pub fx_applied: bool,
    /// A SPLIT dated on or before the window end was applied to the lots in scope.
    pub split_applied: bool,
}

/// The portfolio rollup: exactly the five per-asset figures, summed across assets.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReportTotals {
    #[serde(with = "decimal_json")]
    pub realized_in_window: Decimal,
    #[serde(with = "decimal_json")]
    pub realized_cumulative: Decimal,
    #[serde(with = "decimal_json")]
    pub unrealized_at_end: Decimal,
    #[serde(with = "decimal_json")]
    pub market_value: Decimal,
    #[serde(with = "decimal_json")]
    pub cost_basis: Decimal,
}

/// One report row. Three selections share one shape: the kind-specific cells are null on
/// the rows where they do not apply, never omitted and never zero.
///
/// The asset-level pricing facts repeat on every row of that asset — one close per asset,
/// not one per lot — so the CSV is a projection of this vector and needs no join.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReportRow {
    pub row_type: RowKind,
    pub asset_id: i32,
    pub symbol: String,
    pub asset_type: String,
    pub asset_currency: String,
    pub price_date_used: Option<NaiveDate>,
    pub fx_rate_date_used: Option<NaiveDate>,
    pub price_source: Option<PriceSource>,

    // Sell-level, mirrored from `SellBreakdown` and the sell transaction. Repeated on
    // every lot row of that sell so the file pivots, exactly as `sellPnlCsv.ts` does.
    pub sell_transaction_id: Option<i32>,
    pub sell_date: Option<DateTime<Utc>>,
    #[serde(with = "decimal_opt_json")]
    pub sold_qty: Option<Decimal>,
    /// As entered, in `asset_currency`.
    #[serde(with = "decimal_opt_json")]
    pub sell_price: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub sell_fee: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub proceeds: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub cost_basis: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub realized_pnl: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub matched_qty: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub unmatched_qty: Option<Decimal>,

    // Buy-lot cells: present on SOLD_LOT and OPEN_LOT.
    pub buy_transaction_id: Option<i32>,
    pub buy_date: Option<DateTime<Utc>>,
    /// Split-adjusted, in `asset_currency`.
    #[serde(with = "decimal_opt_json")]
    pub buy_price: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub unit_cost: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub lot_matched_qty: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub lot_qty_before_match: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub lot_cost_basis: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub lot_proceeds: Option<Decimal>,
    /// The column that totals. `LotMatch.pnl` under the name the CSV uses.
    #[serde(with = "decimal_opt_json")]
    pub lot_pnl: Option<Decimal>,
    pub holding_days: Option<i64>,
    /// "SHORT" | "LONG" for HYBRID only; null otherwise, and never claimed on an open lot.
    pub term: Option<String>,

    // Open-lot cells, on OPEN_LOT only.
    #[serde(with = "decimal_opt_json")]
    pub original_qty: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub remaining_qty: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub market_value: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub unrealized_gain_loss: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub unrealized_roi: Option<Decimal>,
}

impl ReportRow {
    /// The empty row every constructor starts from: every kind-specific cell null.
    fn of(kind: RowKind, asset_id: i32, symbol: &str, asset_type: &str, asset_currency: &str,
          price_source: Option<PriceSource>) -> Self {
        ReportRow {
            row_type: kind,
            asset_id,
            symbol: symbol.to_string(),
            asset_type: asset_type.to_string(),
            asset_currency: asset_currency.to_string(),
            price_date_used: None,
            fx_rate_date_used: None,
            price_source,
            sell_transaction_id: None, sell_date: None, sold_qty: None, sell_price: None,
            sell_fee: None, proceeds: None, cost_basis: None, realized_pnl: None,
            matched_qty: None, unmatched_qty: None,
            buy_transaction_id: None, buy_date: None, buy_price: None, unit_cost: None,
            lot_matched_qty: None, lot_qty_before_match: None, lot_cost_basis: None,
            lot_proceeds: None, lot_pnl: None, holding_days: None, term: None,
            original_qty: None, remaining_qty: None, market_value: None,
            unrealized_gain_loss: None, unrealized_roi: None,
        }
    }

    /// One buy lot a sell consumed, with the sell's context repeated.
    #[allow(clippy::too_many_arguments)]
    pub fn sold_lot(
        asset: &Asset,
        price_date_used: Option<NaiveDate>,
        fx_rate_date_used: Option<NaiveDate>,
        price_source: Option<PriceSource>,
        sell: &Transaction,
        breakdown: &SellBreakdown,
        matched: &LotMatch,
    ) -> Self {
        let mut row = ReportRow::of(RowKind::SoldLot, asset.id, &asset.symbol, &asset.asset_type, &asset.currency, price_source);
        row.price_date_used = price_date_used;
        row.fx_rate_date_used = fx_rate_date_used;
        row.sell_transaction_id = Some(sell.id);
        row.sell_date = Some(sell.date);
        row.sold_qty = Some(breakdown.sold_qty);
        row.sell_price = Some(crate::db_types::str_to_decimal(&sell.price));
        row.sell_fee = Some(crate::db_types::str_to_decimal(&sell.fee));
        row.proceeds = Some(breakdown.proceeds);
        row.cost_basis = Some(breakdown.cost_basis);
        row.realized_pnl = Some(breakdown.realized_pnl);
        row.matched_qty = Some(breakdown.matched_qty);
        row.unmatched_qty = Some(breakdown.unmatched_qty);
        row.buy_transaction_id = Some(matched.buy_transaction_id);
        row.buy_date = Some(matched.buy_date);
        row.buy_price = Some(matched.buy_price);
        row.unit_cost = Some(matched.unit_cost);
        row.lot_matched_qty = Some(matched.matched_qty);
        row.lot_qty_before_match = Some(matched.lot_qty_before_match);
        row.lot_cost_basis = Some(matched.cost_basis);
        row.lot_proceeds = Some(matched.proceeds);
        row.lot_pnl = Some(matched.pnl);
        row.holding_days = Some(matched.holding_days);
        row.term = matched.term.clone();
        row
    }

    /// A sell's unmatched tail: the quantity no buy lot covers. Money cells stay null;
    /// `unmatched_qty` is a quantity, not money.
    pub fn unmatched(
        asset: &Asset,
        price_date_used: Option<NaiveDate>,
        fx_rate_date_used: Option<NaiveDate>,
        price_source: Option<PriceSource>,
        sell: &Transaction,
        breakdown: &SellBreakdown,
    ) -> Self {
        let mut row = ReportRow::of(RowKind::Unmatched, asset.id, &asset.symbol, &asset.asset_type, &asset.currency, price_source);
        row.price_date_used = price_date_used;
        row.fx_rate_date_used = fx_rate_date_used;
        row.sell_transaction_id = Some(sell.id);
        row.sell_date = Some(sell.date);
        row.sold_qty = Some(breakdown.sold_qty);
        row.sell_price = Some(crate::db_types::str_to_decimal(&sell.price));
        row.sell_fee = Some(crate::db_types::str_to_decimal(&sell.fee));
        row.matched_qty = Some(breakdown.matched_qty);
        row.unmatched_qty = Some(breakdown.unmatched_qty);
        row
    }

    /// One lot still open at the window end, valued at the requested date.
    pub fn open_lot(
        asset: &Asset,
        price_date_used: Option<NaiveDate>,
        fx_rate_date_used: Option<NaiveDate>,
        price_source: Option<PriceSource>,
        lot: &DatedLot,
    ) -> Self {
        let mut row = ReportRow::of(RowKind::OpenLot, asset.id, &asset.symbol, &asset.asset_type, &asset.currency, price_source);
        row.price_date_used = price_date_used;
        row.fx_rate_date_used = fx_rate_date_used;
        row.buy_transaction_id = Some(lot.buy_transaction_id);
        row.buy_date = Some(lot.buy_date);
        row.buy_price = Some(lot.buy_price);
        row.unit_cost = Some(lot.unit_cost);
        row.original_qty = Some(lot.original_qty);
        row.remaining_qty = Some(lot.remaining_qty);
        row.market_value = Some(lot.market_value);
        row.unrealized_gain_loss = Some(lot.unrealized_gain_loss);
        row.unrealized_roi = lot.unrealized_roi;
        row
    }
}

/// The dated report: the run context, per-asset totals, the portfolio rollup and every row.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReportResponse {
    pub context: ReportContext,
    pub asset_totals: Vec<AssetTotals>,
    pub totals: ReportTotals,
    pub rows: Vec<ReportRow>,
}

/// One currency pair a rate could not be found for. A missing rate is a pair, not a
/// symbol, and the body says which it is.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MissingRate {
    pub from: String,
    pub to: String,
}

/// The one body every failure returns. Validation failures carry an empty pair of lists
/// and name the offending parameter in `error`; a pricing failure populates them.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ErrorBody {
    pub error: String,
    pub missing_prices: Vec<String>,
    pub missing_rates: Vec<MissingRate>,
    /// How many were found, before the lists were capped. Truncation is never silent.
    pub missing_prices_total: usize,
    pub missing_rates_total: usize,
    pub retryable: bool,
}
```

- [ ] **Step 4: Register the types and retype the response**

In `backend_rust/src/openapi.rs`, add to `components(schemas(...))` after `schemas::SellPnlResponse`:

```rust
        schemas::ReportContext,
        schemas::AssetTotals,
        schemas::ReportTotals,
        schemas::ReportRow,
        schemas::RowKind,
        schemas::PriceSource,
        schemas::ReportResponse,
        schemas::ErrorBody,
        schemas::MissingRate,
```

In `backend_rust/src/api_routes/analytics.rs`, replace the `tax-summary` `#[utoipa::path]` block (`:10-19`) with:

```rust
/// The tax summary, or — with `from` and/or `to` — the dated tax report.
///
/// Same route, same access level, two shapes. Without a window the response is the
/// summary object this endpoint has always returned, which carries `assets` with the
/// open lots the Holdings tab renders; it is not described here, and its shape is
/// deliberately unchanged (the printable statement and `PortfolioDetail` both read it).
/// With a window the response is `ReportResponse`.
///
/// `format=csv` returns the same report as `text/csv`, byte-identical between calls over
/// unchanged data. JSON stays the documented public surface.
#[utoipa::path(
    get,
    path = "/api/portfolios/<id>/tax-summary",
    params(
        ("from" = Option<String>, Query, description = "Window start, YYYY-MM-DD, inclusive, UTC. Absent means from the portfolio's first transaction."),
        ("to" = Option<String>, Query, description = "Window end, YYYY-MM-DD, inclusive, UTC. Absent means today."),
        ("format" = Option<String>, Query, description = "`csv` returns the report as text/csv. Any other value is a 422.")
    ),
    responses(
        (status = 200, description = "Dated tax report when a window is given; the legacy summary object otherwise", body = ReportResponse),
        (status = 404, description = "Portfolio not found", body = ErrorBody),
        (status = 422, description = "Invalid window, or a price or rate does not exist at or before the window end", body = ErrorBody),
        (status = 503, description = "A pricing provider is unreachable, or the report exceeded its elapsed deadline", body = ErrorBody)
    )
)]
```

- [ ] **Step 5: Regenerate both contracts**

```bash
cd backend_rust && cargo run --bin backend_rust -- --print-openapi > ../docs/openapi/openapi.json
cd ../frontend && npm run generate-types
```

Expected: `openapi.json` gains `ReportContext`, `AssetTotals`, `ReportTotals`, `ReportRow`, `RowKind`, `PriceSource`, `ReportResponse`, `ErrorBody`, `MissingRate` in `components.schemas`, gains `from`/`to`/`format` query parameters on the `tax-summary` operation, replaces its empty 200 schema with a `$ref` to `ReportResponse`, and gains 422/503 responses. `api.d.ts` follows. Neither file has a consumer that can break: nothing imports `api.d.ts`, and the hand-written types in `types.ts` are what the renderers use.

- [ ] **Step 6: Add the hand-written frontend types**

Append to `frontend/src/types.ts`:

```ts
/**
 * The dated tax report. Deliberately NOT normalized through `number` the way every other
 * payload in this file is (see the header): the report's figures are displayed from the
 * exact decimal strings the server produced, and the same strings are what the CSV
 * carries. Round-tripping them through a double here would make the screen and the file
 * two renderings of different numbers, which is the one thing this feature exists to
 * avoid. `formatCurrency` already accepts `string | number`.
 */
export type ReportRowKind = 'SOLD_LOT' | 'OPEN_LOT' | 'UNMATCHED';
export type ReportPriceSource = 'database' | 'provider';

export interface ReportContext {
  portfolio_id: number;
  portfolio_name: string;
  base_currency: string;
  strategy: string;
  threshold_days: number;
  /** The window actually applied; null when the portfolio has no transaction at all. */
  requested_from: string | null;
  requested_to: string;
  splits_note: string;
}

export interface ReportAssetTotals {
  asset_id: number;
  symbol: string;
  asset_type: string;
  asset_currency: string;
  realized_in_window: string;
  realized_cumulative: string;
  unrealized_at_end: string;
  market_value: string;
  cost_basis: string;
  price_date_used: string | null;
  fx_rate_date_used: string | null;
  price_source: ReportPriceSource | null;
  fx_applied: boolean;
  split_applied: boolean;
}

export interface ReportTotals {
  realized_in_window: string;
  realized_cumulative: string;
  unrealized_at_end: string;
  market_value: string;
  cost_basis: string;
}

export interface ReportRow {
  row_type: ReportRowKind;
  asset_id: number;
  symbol: string;
  asset_type: string;
  asset_currency: string;
  price_date_used: string | null;
  fx_rate_date_used: string | null;
  price_source: ReportPriceSource | null;
  sell_transaction_id: number | null;
  sell_date: string | null;
  sold_qty: string | null;
  sell_price: string | null;
  sell_fee: string | null;
  proceeds: string | null;
  cost_basis: string | null;
  realized_pnl: string | null;
  matched_qty: string | null;
  unmatched_qty: string | null;
  buy_transaction_id: number | null;
  buy_date: string | null;
  buy_price: string | null;
  unit_cost: string | null;
  lot_matched_qty: string | null;
  lot_qty_before_match: string | null;
  lot_cost_basis: string | null;
  lot_proceeds: string | null;
  /** The only column whose sum over SOLD_LOT rows is an asset's realized_in_window. */
  lot_pnl: string | null;
  holding_days: number | null;
  term: 'SHORT' | 'LONG' | null;
  original_qty: string | null;
  remaining_qty: string | null;
  market_value: string | null;
  unrealized_gain_loss: string | null;
  unrealized_roi: string | null;
}

export interface TaxReport {
  context: ReportContext;
  asset_totals: ReportAssetTotals[];
  totals: ReportTotals;
  rows: ReportRow[];
}

export interface ReportErrorBody {
  error: string;
  missing_prices: string[];
  missing_rates: { from: string; to: string }[];
  missing_prices_total: number;
  missing_rates_total: number;
  retryable: boolean;
}
```

The header comment at the top of `types.ts` gains one sentence so the exception is stated where the rule is:

```ts
 * The dated tax report (`TaxReport`) is the one exception and says so at its definition:
 * its money stays in the exact decimal strings the server sent.
```

- [ ] **Step 7: Run the tests and both gates**

```bash
cd backend_rust && cargo test --test tax_report_test && cargo test --test route_coverage_test
cd ../frontend && npm run test && npm run lint && npm run typecheck
```

Expected: PASS. `route_coverage_test` still sees 15 operations because no route was added; `to_document_path` (`:86-92`) truncates at `?`, so the new query parameters do not move the count.

- [ ] **Step 8: Commit**

```bash
git add backend_rust/src/schemas.rs backend_rust/src/openapi.rs backend_rust/src/api_routes/analytics.rs \
        backend_rust/tests/tax_report_test.rs docs/openapi/openapi.json \
        frontend/src/types.ts frontend/src/types/api.d.ts
git commit -m "feat(api): type the dated tax report payload and document it"
```

---

## Task 4: The dated `tax-summary` route

The route stops being undated: `from` and `to` are optional, either alone is legal, and with a window the response is the typed report from Task 3. Access stays `Access::Viewer` for every call, dated or not, and **no route is added** — so `tests/route_coverage_test.rs`'s hard counts stay at 15/15/13 and `to_document_path` (`:86-92`) keeps truncating at `?`.

Three rules this task is the home of, all settled and none of them re-opened here: the 422/503 contract with capped lists, the 20 s elapsed deadline checked between outbound calls, and the undated path keeping its legacy response shape (the shipped statement and `PortfolioDetail` read `assets[].tax_lots`, `current_shares` and `total_portfolio_value`, which the typed payload does not carry).

`?format=csv` is **Task 5**, deliberately: this task adds no `format` parameter and no unreachable branch.

**Files:**
- Modify: `backend_rust/src/api_routes/analytics.rs` (the handler, and the report machinery below it)
- Modify: `backend_rust/src/schemas.rs` (`AssetTotals::empty`, `ReportTotals::zero` — helpers only, no field change, so the committed contract does not move)
- Modify: `backend_rust/tests/common/mod.rs` (add `seed_daily_prices`)
- Modify: `backend_rust/tests/isolation_test.rs` (the dated variant)
- Modify: `backend_rust/tests/auth_test.rs` (the dated sweep pass and its stale comment)
- Test: `backend_rust/tests/tax_report_test.rs`

**Interfaces:**
- Consumes: `replay_at`/`DatedLot`/`DatedReplay` (Task 2), the payload types (Task 3), and from `currency_service.rs`: `get_historical_prices` (public), `get_historical_prices_from_db`, `stored_window_is_fresh`, `select_rate`, `fx_symbol`, `FX_LOOKBACK_DAYS`, `PricingError` — all `pub(crate)`, so the route can reuse the layer's own rule without editing that file.
- Produces, for Task 5 and 6:
  ```rust
  pub const REPORT_DEADLINE: Duration = Duration::from_secs(20);
  pub const SPLITS_NOTE: &str;
  pub struct ReportWindow { pub from: Option<NaiveDate>, pub to: NaiveDate }
  pub struct MissingItems { .. }           // pub: caps are testable without a provider
  pub struct ReportError { pub status: Status, pub body: ErrorBody }   // impl Responder
  pub enum TaxSummaryResponse { Legacy(Json<serde_json::Value>), Report(Json<ReportResponse>) }
  pub async fn build_tax_report(.., window: &ReportWindow, deadline: Duration)
      -> Result<ReportResponse, ReportError>
  ```

- [ ] **Step 1: Add the dense-price helper and write the route tests**

Append to `backend_rust/tests/common/mod.rs`:

```rust
/// A close for `symbol` on every calendar day in `[start, end]`. Dense on purpose: the
/// pricing layer answers from the database only when the newest stored row is within
/// `PRICE_WINDOW_FRESH_DAYS` of the date asked for, so a sparse seed reaches the
/// provider and the test is no longer hermetic. `analytics_test.rs`'s own fixture skips
/// weekends for the opposite reason — to exercise the gap — and that is why this one is
/// shared rather than copied.
#[allow(dead_code)]
pub async fn seed_daily_prices(pool: &SqlitePool, symbol: &str, start: &str, end: &str, close: &str) {
    let mut day = NaiveDate::parse_from_str(start, "%Y-%m-%d").unwrap();
    let last = NaiveDate::parse_from_str(end, "%Y-%m-%d").unwrap();
    while day <= last {
        seed_price(pool, symbol, day, close).await;
        day = day.succ_opt().expect("a date has a successor");
    }
}
```

Append to `backend_rust/tests/tax_report_test.rs`:

```rust
use backend_rust::api_routes::analytics::{build_tax_report, MissingItems, ReportError, ReportWindow};
use backend_rust::models::{Portfolio, Transaction};
use backend_rust::services::currency_service::{CurrencyService, PricingError};
use common::{build_rocket, disabled_config, seed_price, seed_split_sync, setup_db, DEFAULT_USER};
use rocket::http::Status;
use rocket::local::asynchronous::Client;
use sqlx::sqlite::SqlitePool;
use std::time::Duration;

/// One portfolio, two assets and four transactions, with a close seeded for each asset
/// up to `through`. The window end a test uses must be no later than `through`, or the
/// pricing layer stops answering from the database.
///
/// SOLDIT: buys 60 @ 100 (Jan) and 40 @ 120 (Mar), sells 80 @ 130 (May) and 10 @ 140
/// (late June) — so 10 units are unmatched, which is the row kind that carries nulls.
/// HELDIT: one buy of 50 @ 200 (Feb).
async fn fixture(pool: &SqlitePool, through: &str) -> (i32, i32, i32) {
    let port_id = common::seed_portfolio(pool, DEFAULT_USER, "Dated", "USD").await;
    let sold = common::seed_asset(pool, port_id, "SOLDIT", "Sold it").await;
    let held = common::seed_asset(pool, port_id, "HELDIT", "Held it").await;

    common::seed_transaction_at(pool, sold, "BUY", 60.0, 100.0, 0.0, "2024-01-01T00:00:00Z").await;
    common::seed_transaction_at(pool, sold, "BUY", 40.0, 120.0, 0.0, "2024-03-01T00:00:00Z").await;
    common::seed_transaction_at(pool, sold, "SELL", 80.0, 130.0, 0.0, "2024-05-01T00:00:00Z").await;
    common::seed_transaction_at(pool, sold, "SELL", 10.0, 140.0, 0.0, "2024-06-25T00:00:00Z").await;
    common::seed_transaction_at(pool, held, "BUY", 50.0, 200.0, 0.0, "2024-02-01T00:00:00Z").await;

    common::seed_daily_prices(pool, "SOLDIT", "2024-01-01", through, "110.0").await;
    common::seed_daily_prices(pool, "HELDIT", "2024-02-01", through, "220.0").await;
    // No split is seeded, so this only has to keep `sync_splits_for_asset` on its cache
    // path for the undated call.
    seed_split_sync(pool, "SOLDIT").await;
    seed_split_sync(pool, "HELDIT").await;

    (port_id, sold, held)
}

fn dec_at(v: &serde_json::Value, path: &str) -> Decimal {
    Decimal::from_str(v.pointer(path).and_then(|p| p.as_str()).unwrap_or_else(|| panic!("{path} missing"))).unwrap()
}

#[tokio::test]
async fn a_dated_report_sums_its_own_rows_and_agrees_with_the_undated_summary() {
    // Goal 1, all three equalities, under all three strategies:
    // (a) the sold-lot rows' `lot_pnl` sum to the asset's `realized_in_window`;
    // (b) with `from` at the portfolio's first transaction, that sum is also the asset's
    //     `realized_pnl` on the undated endpoint, which is fully driven here because the
    //     window is the whole history and both calls price from the same seeded rows;
    // (c) the open-lot rows' `unrealized_gain_loss` sums to `unrealized_at_end`.
    let pool = setup_db(&disabled_config()).await;
    let through = Utc::now().date_naive().to_string();
    let (port_id, _, _) = fixture(&pool, &through).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();

    for strategy in ["FIFO", "LIFO", "HYBRID"] {
        let dated = client
            .get(format!(
                "/api/portfolios/{port_id}/tax-summary?strategy={strategy}&threshold_days=30&from=2024-01-01&to={through}"
            ))
            .dispatch()
            .await;
        assert_eq!(dated.status(), Status::Ok, "{strategy}");
        let report: serde_json::Value = serde_json::from_str(&dated.into_string().await.unwrap()).unwrap();

        for totals in report["asset_totals"].as_array().unwrap() {
            let symbol = totals["symbol"].as_str().unwrap();
            let rows: Vec<&serde_json::Value> = report["rows"].as_array().unwrap().iter()
                .filter(|r| r["symbol"] == symbol)
                .collect();

            let sold: Decimal = rows.iter()
                .filter(|r| r["row_type"] == "SOLD_LOT")
                .map(|r| dec_at(r, "/lot_pnl"))
                .sum();
            assert_eq!(sold, dec_at(totals, "/realized_in_window"), "{strategy}/{symbol}: rows must sum to the server total");

            let open: Decimal = rows.iter()
                .filter(|r| r["row_type"] == "OPEN_LOT")
                .map(|r| dec_at(r, "/unrealized_gain_loss"))
                .sum();
            assert_eq!(open, dec_at(totals, "/unrealized_at_end"), "{strategy}/{symbol}");
        }

        // The rollup is the server's, and the rows must not be the only source of it.
        let rollup: Decimal = report["asset_totals"].as_array().unwrap().iter()
            .map(|t| dec_at(t, "/realized_in_window"))
            .sum();
        assert_eq!(rollup, dec_at(&report, "/totals/realized_in_window"), "{strategy}");

        let undated = client
            .get(format!("/api/portfolios/{port_id}/tax-summary?strategy={strategy}&threshold_days=30"))
            .dispatch()
            .await;
        let summary: serde_json::Value = serde_json::from_str(&undated.into_string().await.unwrap()).unwrap();
        for asset in summary["assets"].as_array().unwrap() {
            let symbol = asset["symbol"].as_str().unwrap();
            let dated_asset = report["asset_totals"].as_array().unwrap().iter()
                .find(|t| t["symbol"] == symbol)
                .unwrap();
            assert_eq!(
                dec_at(dated_asset, "/realized_cumulative"),
                dec_at(asset, "/realized_pnl"),
                "{strategy}/{symbol}: cumulative realized must equal what the Holdings tab shows",
            );
        }
    }
}

#[tokio::test]
async fn an_undated_call_still_returns_the_shape_the_statement_reads() {
    // Review Focus 5. The printable statement and `PortfolioDetail` read `assets[].tax_lots`,
    // `current_shares` and `total_portfolio_value`, none of which the typed report carries.
    // If this test fails because `context` appears and `assets` does not, the typed payload
    // has replaced the legacy response and both shipped screens are broken.
    let pool = setup_db(&disabled_config()).await;
    let through = Utc::now().date_naive().to_string();
    let (port_id, _, _) = fixture(&pool, &through).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();

    let resp = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?strategy=FIFO&threshold_days=30"))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Ok);
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();

    assert!(body["assets"].is_array());
    assert!(body["assets"][0]["tax_lots"].is_array());
    assert!(body["assets"][0]["current_shares"].is_string());
    assert!(body["total_portfolio_value"].is_string());
    assert_eq!(body["strategy"], "FIFO");
    assert!(body.get("rows").is_none(), "the legacy shape has no rows");
    assert!(body.get("context").is_none(), "and no run context");
}

#[tokio::test]
async fn the_undated_path_still_names_the_strategy_the_engine_used() {
    // The spec's file 6 fix: the handler defaults but does not normalise, so
    // `?strategy=average_cost` reports `average_cost` while the engine ran FIFO. The
    // printable statement's run-context line is what depends on this, and the numbers do
    // not move -- the engine already fell through to FIFO.
    let pool = setup_db(&disabled_config()).await;
    let through = Utc::now().date_naive().to_string();
    let (port_id, _, _) = fixture(&pool, &through).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();

    let resp = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?strategy=average_cost&threshold_days=30"))
        .dispatch()
        .await;
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
    assert_eq!(body["strategy"], "FIFO", "the response must not name a method the engine did not use");
}

#[tokio::test]
async fn a_window_end_on_a_non_trading_day_reports_the_close_it_used() {
    // 2024-06-30 is a Sunday and the fixture stops on the Friday before it (2 days, inside
    // `PRICE_WINDOW_FRESH_DAYS = 4`), so the database answers and the file states which
    // close priced the position. Silent substitution is the failure Goal 5 names.
    let pool = setup_db(&disabled_config()).await;
    let (port_id, _, _) = fixture(&pool, "2024-06-28").await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();

    let resp = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?strategy=FIFO&threshold_days=30&to=2024-06-30"))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Ok);
    let report: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();

    assert_eq!(report["context"]["requested_to"], "2024-06-30");
    assert_eq!(
        report["context"]["requested_from"], "2024-01-01",
        "`to` alone is cumulative from the portfolio's first transaction",
    );
    for totals in report["asset_totals"].as_array().unwrap() {
        assert_eq!(totals["price_date_used"], "2024-06-28", "{}", totals["symbol"]);
        assert_eq!(totals["price_source"], "database");
    }
    for row in report["rows"].as_array().unwrap() {
        assert_eq!(row["price_date_used"], "2024-06-28", "one close per asset, repeated on its rows");
    }
}

#[tokio::test]
async fn the_window_selects_the_sells_but_not_the_lots_behind_them() {
    // A sell dated before `from` is out of the window's rows and out of
    // `realized_in_window`, and still part of `realized_cumulative` — and the buy lots it
    // consumed still show up as open lots (or not) at `end`. Getting this wrong changes a
    // filed quarter.
    let pool = setup_db(&disabled_config()).await;
    let (port_id, _, _) = fixture(&pool, "2024-06-28").await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();

    let resp = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?strategy=FIFO&threshold_days=30&from=2024-06-01&to=2024-06-30"))
        .dispatch()
        .await;
    let report: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();

    assert!(report["rows"].as_array().unwrap().iter().all(|r| r["sell_date"].is_null() || r["sell_date"].as_str().unwrap() >= "2024-06-01"));
    let soldit = report["asset_totals"].as_array().unwrap().iter()
        .find(|t| t["symbol"] == "SOLDIT").unwrap();
    // Only the 10 June units are in the window, and 10 of them are unmatched: FIFO takes
    // the oldest lot first, and the May sell already consumed all 60 units of it.
    assert_eq!(dec_at(soldit, "/realized_in_window"), dec("0.0"), "an unmatched tail realizes nothing");
    assert_eq!(dec_at(soldit, "/realized_cumulative"), dec("1800.0") + dec("-0.0") + dec("0.0"));
    let unmatched = report["rows"].as_array().unwrap().iter()
        .filter(|r| r["row_type"] == "UNMATCHED")
        .collect::<Vec<_>>();
    assert_eq!(unmatched.len(), 1, "the June sell's unmatched tail is a row of its own");
    assert!(unmatched[0]["realized_pnl"].is_null());
    assert_eq!(unmatched[0]["unmatched_qty"], "10.0");
}
```

`dec` is the helper already defined in Step 1 of Task 3.

- [ ] **Step 2: Run them to verify they fail**

Run: `cd backend_rust && cargo test --test tax_report_test`
Expected: FAIL to compile — `build_tax_report` does not exist, and `from`/`to` are not accepted parameters. (The handler needs the query parameters before anything can be dispatched.)

- [ ] **Step 3: Write the validation and failure-contract tests**

Add to `backend_rust/tests/tax_report_test.rs`:

```rust
/// The validation failures, each of which names the parameter and carries two empty
/// lists. They are all 422 with `retryable: false`: the client typed something the
/// server cannot act on, and retrying changes nothing.
#[tokio::test]
async fn invalid_windows_are_422s_that_name_the_parameter() {
    let pool = setup_db(&disabled_config()).await;
    let (port_id, _, _) = fixture(&pool, Utc::now().date_naive().to_string().as_str()).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();

    let today = Utc::now().date_naive();
    let tomorrow = (today + chrono::Duration::days(1)).to_string();
    let cases: Vec<(&str, String, &str)> = vec![
        ("from after to", format!("from=2024-06-30&to=2024-06-01"), "from"),
        ("future to", format!("to={tomorrow}"), "to"),
        ("future from", format!("from={tomorrow}"), "from"),
        ("unparseable to", "to=31-06-2024".to_string(), "to"),
        ("unparseable from", "from=not-a-date".to_string(), "from"),
        ("unknown strategy", "to=2024-06-30&strategy=average_cost".to_string(), "strategy"),
        ("negative threshold", "to=2024-06-30&threshold_days=-1".to_string(), "threshold_days"),
    ];

    for (label, query, param) in cases {
        let resp = client
            .get(format!("/api/portfolios/{port_id}/tax-summary?{query}"))
            .dispatch()
            .await;
        assert_eq!(resp.status(), Status::UnprocessableEntity, "{label}");
        let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
        assert!(
            body["error"].as_str().unwrap().contains(param),
            "{label}: the body must name the offending parameter, got {}",
            body["error"],
        );
        assert_eq!(body["missing_prices"], serde_json::json!([]), "{label}");
        assert_eq!(body["missing_rates"], serde_json::json!([]), "{label}");
        assert_eq!(body["retryable"], false, "{label}");
    }
}

#[tokio::test]
async fn a_missing_portfolio_is_404_and_an_unknown_strategy_on_the_undated_path_is_not_an_error() {
    let pool = setup_db(&disabled_config()).await;
    let (port_id, _, _) = fixture(&pool, "2024-06-28").await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();

    let missing = client.get("/api/portfolios/9999/tax-summary?strategy=FIFO").dispatch().await;
    assert_eq!(missing.status(), Status::NotFound);

    // Dated-only strictness (decision D6): the same unknown value that is a 422 with a
    // window is a 200 without one.
    let undated = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?strategy=average_cost"))
        .dispatch()
        .await;
    assert_eq!(undated.status(), Status::Ok);
}

#[test]
fn the_missing_item_lists_are_capped_and_the_totals_say_so() {
    // The cap is testable without a provider because it is applied when the collected
    // failures render, not when they are found. See Step 9 for why the collection itself
    // has no hermetic test.
    let mut missing = MissingItems::new();
    for i in 0..21 {
        missing.add_price(&format!("SYM{i}"));
    }
    for i in 0..11 {
        missing.add_rate(&format!("C{i}"), "USD");
    }
    // Duplicates are not items: the list names symbols and pairs, once each.
    missing.add_price("SYM0");
    missing.add_rate("C0", "USD");

    let body = missing.into_error_body("no price or rate exists at or before the requested date");
    assert_eq!(body.missing_prices.len(), 20);
    assert_eq!(body.missing_prices_total, 21);
    assert_eq!(body.missing_rates.len(), 10);
    assert_eq!(body.missing_rates_total, 11);
    assert!(!body.retryable);
}

#[test]
fn a_missing_rate_is_named_as_a_pair_not_a_symbol() {
    let mut missing = MissingItems::new();
    missing.add_rate("EUR", "USD");
    let body = missing.into_error_body("no price or rate exists at or before the requested date");
    assert_eq!(body.missing_rates[0].from, "EUR");
    assert_eq!(body.missing_rates[0].to, "USD");
    assert!(body.missing_prices.is_empty());
}

#[test]
fn the_two_failure_classes_carry_the_right_status_and_retryable_flag() {
    // The contract, pinned on the constructors rather than through a provider.
    let no_data = ReportError::no_data(MissingItems::new());
    assert_eq!(no_data.status, Status::UnprocessableEntity);
    assert!(!no_data.body.retryable);

    let provider = ReportError::provider("timeout".to_string());
    assert_eq!(provider.status, Status::ServiceUnavailable);
    assert!(provider.body.retryable);

    let deadline = ReportError::deadline();
    assert_eq!(deadline.status, Status::ServiceUnavailable);
    assert!(deadline.body.retryable);

    // `PricingError`'s two variants mean different user actions, and the report's mapping
    // is the place that difference becomes a status code.
    assert_eq!(pricing_failure_status(&PricingError::NoData), Status::UnprocessableEntity);
    assert_eq!(
        pricing_failure_status(&PricingError::ProviderUnavailable("timeout".to_string())),
        Status::ServiceUnavailable,
    );
}

#[tokio::test]
async fn the_deadline_fails_closed_instead_of_returning_a_partial_report() {
    // The one hermetic route to a 503: the check runs before the first pricing step, so
    // `Duration::ZERO` trips it with no outbound call at all. A test that waited 20 s
    // would not be worth having.
    let pool = setup_db(&disabled_config()).await;
    let (port_id, _, _) = fixture(&pool, "2024-06-28").await;
    let portfolio = load_fixture_portfolio(&pool, port_id).await;
    let assets = load_fixture_assets(&pool, port_id).await;
    let svc = CurrencyService::new();

    let err = build_tax_report(
        &pool, &svc, &portfolio, &assets, "FIFO", 30,
        &ReportWindow { from: None, to: day("2024-06-30") },
        Duration::ZERO,
    )
    .await
    .expect_err("a zero deadline must not produce a report");

    assert_eq!(err.status, Status::ServiceUnavailable);
    assert!(err.body.retryable);
    assert!(err.body.missing_prices.is_empty(), "no partial payload, and no missing items either");
}
```

`load_fixture_portfolio` / `load_fixture_assets` are two three-line helpers in the test file that read the rows back through SQLx (`SELECT * FROM portfolios WHERE id = ?`, `SELECT * FROM assets WHERE portfolio_id = ?`), because `build_tax_report` takes the loaded rows rather than the ids.

- [ ] **Step 4: Run them to verify they fail**

Run: `cd backend_rust && cargo test --test tax_report_test`
Expected: FAIL to compile (`MissingItems`, `ReportError`, `build_tax_report`, `pricing_failure_status`).

- [ ] **Step 5: Implement the window, the caps, and the error type**

Append to `backend_rust/src/api_routes/analytics.rs`. Extend the imports at the top of that file:

```rust
use std::io::Cursor;
use std::time::{Duration, Instant};
use chrono::{NaiveDate, Utc};
use rocket::http::{ContentType, Status};
use rocket::request::Request;
use rocket::response::{Responder, Response};
use rocket::serde::json::Json;
use crate::schemas::{
    AssetTotals, ErrorBody, MissingRate, ReportContext, ReportResponse, ReportRow, ReportTotals,
};
use crate::services::currency_service::{
    fx_symbol, select_rate, stored_window_is_fresh, PricingError, FX_LOOKBACK_DAYS,
};
```

```rust
// ---------------------------------------------------------------------------
// The dated tax report
// ---------------------------------------------------------------------------

/// Wall-clock bound on one report.
///
/// Checked between outbound calls, not between assets: one asset can cost two calls (a
/// close and a currency pair), so a check placed between assets can be followed by 66 s
/// of work — past the ingress' 60 s default, which turns the designed 503 into an HTML
/// 504 with no typed body and no `retryable`. It cannot preempt a call already in flight;
/// it refuses the next one. A cold portfolio can legitimately exceed this once and
/// succeed on retry, because the first attempt warmed the per-pair series.
pub const REPORT_DEADLINE: Duration = Duration::from_secs(20);

/// The caps on the two missing-item lists (settled value: 20 symbols, 10 pairs). The list
/// is for a human to read; a portfolio with 200 unpriced symbols needs the count, not the
/// list, and `missing_*_total` is what keeps truncation from being silent.
const MISSING_PRICES_CAP: usize = 20;
const MISSING_RATES_CAP: usize = 10;

/// The run context's splits statement. A constant in the payload rather than prose in each
/// renderer, because the CSV carries it as a cell and the file's first row is the header
/// row.
pub const SPLITS_NOTE: &str =
    "Split-adjusted history is as recorded when this report was generated; a split recorded later changes past figures.";

/// The window a dated request asks about, after defaults are applied.
pub struct ReportWindow {
    /// `None` means inception — or "the portfolio has no transaction at all".
    pub from: Option<NaiveDate>,
    pub to: NaiveDate,
}

/// Pricing failures collected across every asset, so one response can name all of them
/// instead of failing on the first (Goal 5).
///
/// `pub` so `tests/tax_report_test.rs` can pin the caps: the collection itself cannot be
/// exercised hermetically (see Step 9), and the cap is the part of that contract that can.
#[derive(Default)]
pub struct MissingItems {
    prices: Vec<String>,
    rates: Vec<MissingRate>,
}

impl MissingItems {
    pub fn new() -> Self { Self::default() }

    /// Once per symbol, in the order first seen.
    pub fn add_price(&mut self, symbol: &str) {
        if !self.prices.iter().any(|s| s == symbol) {
            self.prices.push(symbol.to_string());
        }
    }

    /// Once per pair. A missing rate is a pair, not a symbol, and `missing_rates` says so.
    pub fn add_rate(&mut self, from: &str, to: &str) {
        if !self.rates.iter().any(|r| r.from == from && r.to == to) {
            self.rates.push(MissingRate { from: from.to_string(), to: to.to_string() });
        }
    }

    pub fn is_empty(&self) -> bool { self.prices.is_empty() && self.rates.is_empty() }

    /// Totals are counted before truncation, so a client can tell a short list from a
    /// complete one.
    pub fn into_error_body(self, error: &str) -> ErrorBody {
        let prices_total = self.prices.len();
        let rates_total = self.rates.len();
        ErrorBody {
            error: error.to_string(),
            missing_prices: self.prices.into_iter().take(MISSING_PRICES_CAP).collect(),
            missing_rates: self.rates.into_iter().take(MISSING_RATES_CAP).collect(),
            missing_prices_total: prices_total,
            missing_rates_total: rates_total,
            retryable: false,
        }
    }
}

/// The route's one error type, in the style of the auth `Unavailable` responder
/// (`auth/routes.rs:33-52`): a responder, so `Result<..>` needs no conversion layer, and
/// public with public fields so a test can assert a failure it produced directly.
pub struct ReportError {
    pub status: Status,
    pub body: ErrorBody,
}

impl ReportError {
    /// A parameter the client can fix. 422, never retryable.
    pub fn validation(param: &str, detail: &str) -> Self {
        Self::plain(Status::UnprocessableEntity, format!("{param}: {detail}"), false)
    }

    /// Prices or rates that do not exist at or before the requested date. 422: asking
    /// again changes nothing.
    pub fn no_data(missing: MissingItems) -> Self {
        Self {
            status: Status::UnprocessableEntity,
            body: missing.into_error_body("no price or rate exists at or before the requested date"),
        }
    }

    /// A provider we could not ask. 503: the same request may succeed later.
    pub fn provider(detail: String) -> Self {
        Self::plain(Status::ServiceUnavailable, format!("pricing provider unavailable: {detail}"), true)
    }

    /// The elapsed deadline. Same 503 as an outage, and fail-closed: no partial payload.
    pub fn deadline() -> Self {
        Self::plain(
            Status::ServiceUnavailable,
            format!("the report exceeded its {} second deadline", REPORT_DEADLINE.as_secs()),
            true,
        )
    }

    pub fn not_found() -> Self {
        Self::plain(Status::NotFound, "portfolio not found".to_string(), false)
    }

    pub fn internal(detail: String) -> Self {
        Self::plain(Status::InternalServerError, detail, false)
    }

    /// `require` answers `NotFound` for a portfolio the caller cannot see, and
    /// `InternalServerError` when the access check itself failed (`access.rs:78-93`).
    pub fn from_status(status: Status) -> Self {
        match status {
            Status::NotFound => Self::not_found(),
            other => Self::internal(format!("access check failed: {}", other.reason().unwrap_or("unknown"))),
        }
    }

    fn plain(status: Status, error: String, retryable: bool) -> Self {
        Self {
            status,
            body: ErrorBody {
                error,
                missing_prices: Vec::new(),
                missing_rates: Vec::new(),
                missing_prices_total: 0,
                missing_rates_total: 0,
                retryable,
            },
        }
    }
}

impl std::fmt::Debug for ReportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ReportError({}, {})", self.status, self.body.error)
    }
}

impl<'r> Responder<'r, 'static> for ReportError {
    fn respond_to(self, _: &'r Request<'_>) -> rocket::response::Result<'static> {
        // Infallible in practice; the fallback is a valid body rather than a second
        // failure mode.
        let body = serde_json::to_string(&self.body)
            .unwrap_or_else(|_| r#"{"error":"internal","retryable":false}"#.to_string());
        Response::build()
            .status(self.status)
            .header(ContentType::JSON)
            .sized_body(body.len(), Cursor::new(body))
            .ok()
    }
}

/// The status a pricing failure maps to. Pure so the mapping is testable without a
/// provider: `PricingError::NoData` is a fact about the world (422), and
/// `ProviderUnavailable` is a fact about this attempt (503).
pub fn pricing_failure_status(error: &PricingError) -> Status {
    match error {
        PricingError::NoData => Status::UnprocessableEntity,
        PricingError::ProviderUnavailable(_) => Status::ServiceUnavailable,
    }
}

/// The route's two shapes. The enum is hand-written rather than Rocket's `Either`: that
/// type is re-exported `#[doc(hidden)]` (`rocket-0.5.1/src/lib.rs:186`), and this match is
/// where Task 5 adds the CSV arm.
pub enum TaxSummaryResponse {
    Legacy(Json<serde_json::Value>),
    Report(Json<ReportResponse>),
}

impl<'r> Responder<'r, 'static> for TaxSummaryResponse {
    fn respond_to(self, request: &'r Request<'_>) -> rocket::response::Result<'static> {
        match self {
            TaxSummaryResponse::Legacy(json) => json.respond_to(request),
            TaxSummaryResponse::Report(json) => json.respond_to(request),
        }
    }
}

fn parse_window(
    from: Option<&str>,
    to: Option<&str>,
    today: NaiveDate,
) -> Result<Option<ReportWindow>, ReportError> {
    let (Some(from), Some(to)) = (from, to) else {
        let parsed_from = from.map(|raw| parse_date_param("from", raw)).transpose()?;
        let parsed_to = to.map(|raw| parse_date_param("to", raw)).transpose()?.unwrap_or(today);
        if let Some(from) = parsed_from {
            if from > parsed_to {
                return Err(ReportError::validation("from", "must not be after `to`"));
            }
            if from > today {
                return Err(ReportError::validation("from", "is in the future"));
            }
        }
        if parsed_to > today {
            return Err(ReportError::validation("to", "is in the future"));
        }
        return Ok(Some(ReportWindow { from: parsed_from, to: parsed_to }));
    };
    // Both are present: one window, validated the same way. `from` alone means through
    // today and `to` alone means from inception — both-or-neither was rejected because it
    // forces every caller to invent a start date, and an invented start silently changes
    // the number.
    let parsed_from = parse_date_param("from", from)?;
    let parsed_to = parse_date_param("to", to)?;
    if parsed_from > parsed_to {
        return Err(ReportError::validation("from", "must not be after `to`"));
    }
    if parsed_from > today {
        return Err(ReportError::validation("from", "is in the future"));
    }
    if parsed_to > today {
        return Err(ReportError::validation("to", "is in the future"));
    }
    Ok(Some(ReportWindow { from: Some(parsed_from), to: parsed_to }))
}

fn parse_date_param(param: &str, raw: &str) -> Result<NaiveDate, ReportError> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map_err(|_| ReportError::validation(param, "expected YYYY-MM-DD"))
}

/// Strict on the dated path: a present-but-unrecognized strategy is a 422 naming the
/// accepted values, rather than the silent fallback to FIFO that both this endpoint and
/// the single-sell route used to run. Dated-path only (decision D6).
fn strict_strategy(strategy: Option<&str>) -> Result<String, ReportError> {
    match strategy {
        None => Ok("FIFO".to_string()),
        Some(raw) => match raw.to_uppercase().as_str() {
            "FIFO" => Ok("FIFO".to_string()),
            "LIFO" => Ok("LIFO".to_string()),
            "HYBRID" => Ok("HYBRID".to_string()),
            other => Err(ReportError::validation(
                "strategy",
                &format!("`{other}` is not one of FIFO, LIFO, HYBRID"),
            )),
        },
    }
}

/// The undated path's rule, which is the sell-P&L route's (`analytics.rs:216-222`):
/// normalize once and use the normalized value for both the engine call and the response,
/// so the response can never name a matching method the engine did not use. It still
/// falls back to FIFO rather than rejecting: "absent parameters reproduce today's
/// behaviour" holds for an unrecognized one too.
fn normalize_strategy(strategy: Option<&str>) -> String {
    match strategy.unwrap_or_default().to_uppercase().as_str() {
        "LIFO" => "LIFO".to_string(),
        "HYBRID" => "HYBRID".to_string(),
        _ => "FIFO".to_string(),
    }
}
```

- [ ] **Step 6: Implement the close resolution and the report builder**

Still in `backend_rust/src/api_routes/analytics.rs`:

```rust
/// Why one asset's close could not be resolved.
enum CloseFailure {
    /// No close exists at or before the date. A 422 item, not a provider outage.
    NoData,
    /// We could not ask. A 503.
    Provider(String),
    /// The database itself failed. A 500.
    Database(String),
}

/// The close for `symbol` on or before `to`, with the date that actually priced it and
/// which source answered.
///
/// The read order is the pricing layer's, reused rather than re-invented:
/// `stored_window_is_fresh` is the *same* predicate `get_historical_prices` applies
/// (`currency_service.rs:713-717`), so the route's rule and the layer's cannot drift, and
/// the provider is asked only when the stored window stops short of the requested date.
/// The one thing this adds over calling `get_historical_prices` alone is the source flag
/// Goal 4 requires: the layer returns a series, not where it came from.
async fn resolve_close(
    currency_service: &CurrencyService,
    pool: &SqlitePool,
    symbol: &str,
    to: NaiveDate,
) -> Result<(Decimal, NaiveDate, PriceSource), CloseFailure> {
    let window_start = to - chrono::Duration::days(FX_LOOKBACK_DAYS);

    let stored = currency_service
        .get_historical_prices_from_db(symbol, window_start, to, pool)
        .await
        .map_err(|e| CloseFailure::Database(e.to_string()))?;
    let newest = stored.iter().map(|(date, _)| *date).max();
    if stored_window_is_fresh(newest, to) {
        if let Some((date_used, close)) = select_rate(&stored, to, FX_LOOKBACK_DAYS) {
            return Ok((close, date_used, PriceSource::Database));
        }
    }

    match currency_service.get_historical_prices(symbol, window_start, to, pool).await {
        Ok(series) => match select_rate(&series, to, FX_LOOKBACK_DAYS) {
            Some((date_used, close)) => Ok((close, date_used, PriceSource::Provider)),
            None => Err(CloseFailure::NoData),
        },
        Err(PricingError::NoData) => Err(CloseFailure::NoData),
        Err(PricingError::ProviderUnavailable(detail)) => Err(CloseFailure::Provider(detail)),
    }
}

/// Build the dated report.
///
/// `deadline` is a parameter rather than the constant alone so a test can pass
/// `Duration::ZERO` and prove the fail-closed path without waiting 20 s.
///
/// Everything that leaves the process happens in this function's own loop, in a fixed
/// order per asset — one pair fetch, then one close — with the deadline checked before
/// each. That is the point of resolving the pair here rather than letting `replay_at`
/// discover transaction dates one at a time: a range fetched now covers every date the
/// replay will ask about, so the engine's per-transaction `get_rate` calls read
/// `historical_prices` and the deadline can actually sit between the calls.
#[allow(clippy::too_many_arguments)]
pub async fn build_tax_report(
    pool: &SqlitePool,
    currency_service: &CurrencyService,
    portfolio: &Portfolio,
    assets: &[Asset],
    strategy: &str,
    threshold_days: i64,
    window: &ReportWindow,
    deadline: Duration,
) -> Result<ReportResponse, ReportError> {
    let start = Instant::now();
    let expired = || start.elapsed() >= deadline;

    // Every asset's transactions first: the effective window's start is the portfolio's
    // earliest transaction, which is not knowable one asset at a time.
    let mut loaded: Vec<(Asset, Vec<Transaction>)> = Vec::with_capacity(assets.len());
    for asset in assets {
        let txs = sqlx::query_as::<_, Transaction>("SELECT * FROM transactions WHERE asset_id = ?")
            .bind(asset.id)
            .fetch_all(pool)
            .await
            .map_err(|e| ReportError::internal(e.to_string()))?;
        loaded.push((asset.clone(), txs));
    }

    let first_in_scope = loaded
        .iter()
        .flat_map(|(_, txs)| txs.iter())
        .filter(|tx| tx.date.date_naive() <= window.to)
        .map(|tx| tx.date.date_naive())
        .min();
    let requested_from = window.from.or(first_in_scope);

    let mut missing = MissingItems::new();
    let mut asset_totals = Vec::new();
    let mut rows = Vec::new();
    let mut totals = ReportTotals::zero();

    for (asset, txs) in &loaded {
        let fx_applied = asset.currency != portfolio.base_currency;
        // A split dated after the window end changed nothing inside the window, and the
        // replay's filter is what makes that true of the figures as well as the flag.
        let split_applied = txs.iter().any(|tx| {
            tx.r#type.to_uppercase() == "SPLIT" && tx.date.date_naive() <= window.to
        });

        let in_scope = txs.iter().any(|tx| tx.date.date_naive() <= window.to);
        if !in_scope {
            // Nothing dated at or before the window end: nothing realized, nothing open,
            // and no close to obtain — so no outbound call and no failure either.
            asset_totals.push(AssetTotals::empty(asset, fx_applied, split_applied));
            continue;
        }

        if fx_applied {
            if expired() {
                return Err(ReportError::deadline());
            }
            let pair = fx_symbol(&asset.currency, &portfolio.base_currency);
            let earliest = txs.iter()
                .map(|tx| tx.date.date_naive())
                .min()
                .expect("the asset has at least one transaction");
            let warm_start = earliest - chrono::Duration::days(FX_LOOKBACK_DAYS);
            match currency_service.get_historical_prices(&pair, warm_start, window.to, pool).await {
                Ok(_) => {}
                Err(PricingError::NoData) => {
                    missing.add_rate(&asset.currency, &portfolio.base_currency);
                    continue;
                }
                Err(PricingError::ProviderUnavailable(detail)) => {
                    return Err(ReportError::provider(detail));
                }
            }
        }

        if expired() {
            return Err(ReportError::deadline());
        }
        let (close, price_date_used, price_source) =
            match resolve_close(currency_service, pool, &asset.symbol, window.to).await {
                Ok(found) => found,
                Err(CloseFailure::NoData) => {
                    missing.add_price(&asset.symbol);
                    continue;
                }
                Err(CloseFailure::Provider(detail)) => return Err(ReportError::provider(detail)),
                Err(CloseFailure::Database(detail)) => return Err(ReportError::internal(detail)),
            };

        let replay = match TaxLotEngine::replay_at(
            txs, window.to, window.to, close, &asset.currency, &portfolio.base_currency,
            currency_service, strategy, threshold_days, pool,
        )
        .await
        {
            Ok(replay) => replay,
            Err(e) => {
                // `replay_at` can fail three ways and they are enumerated rather than
                // inferred: the pricing layer's typed errors, a `sqlx` error, and
                // `no_rate_error` — a plain `anyhow!` from that layer with no type tag
                // (`currency_service.rs:277`), which is "this pair has no close near that
                // date". The last one is the only remaining case.
                if let Some(sq) = e.downcast_ref::<sqlx::Error>() {
                    return Err(ReportError::internal(sq.to_string()));
                }
                if let Some(PricingError::ProviderUnavailable(detail)) = e.downcast_ref::<PricingError>() {
                    return Err(ReportError::provider(detail.clone()));
                }
                missing.add_rate(&asset.currency, &portfolio.base_currency);
                continue;
            }
        };

        let in_window = |date: NaiveDate| {
            requested_from.map_or(true, |from| date >= from) && date <= window.to
        };
        let realized_in_window: Decimal = replay.sell_breakdowns.iter()
            .filter(|b| in_window(b.sell_date.date_naive()))
            .map(|b| b.realized_pnl)
            .sum();
        let realized_cumulative: Decimal = replay.sell_breakdowns.iter()
            .map(|b| b.realized_pnl)
            .sum();
        let market_value: Decimal = replay.open_lots.iter().map(|lot| lot.market_value).sum();
        let cost_basis: Decimal = replay.open_lots.iter()
            .map(|lot| lot.remaining_qty * lot.unit_cost)
            .sum();
        // Derived from the two sums rather than by adding the per-lot gains, so Goal 2's
        // equality is a comparison of two independently computed figures.
        let unrealized_at_end = market_value - cost_basis;

        for breakdown in &replay.sell_breakdowns {
            if !in_window(breakdown.sell_date.date_naive()) {
                continue;
            }
            let sell = txs.iter()
                .find(|tx| tx.id == breakdown.sell_transaction_id)
                .expect("every breakdown came from this transaction set");
            for matched in &breakdown.matches {
                rows.push(ReportRow::sold_lot(
                    asset, Some(price_date_used), replay.fx_rate_date_used,
                    Some(price_source), sell, breakdown, matched,
                ));
            }
            if breakdown.unmatched_qty > Decimal::ZERO {
                rows.push(ReportRow::unmatched(
                    asset, Some(price_date_used), replay.fx_rate_date_used,
                    Some(price_source), sell, breakdown,
                ));
            }
        }
        for lot in &replay.open_lots {
            rows.push(ReportRow::open_lot(
                asset, Some(price_date_used), replay.fx_rate_date_used,
                Some(price_source), lot,
            ));
        }

        totals.realized_in_window += realized_in_window;
        totals.realized_cumulative += realized_cumulative;
        totals.unrealized_at_end += unrealized_at_end;
        totals.market_value += market_value;
        totals.cost_basis += cost_basis;

        asset_totals.push(AssetTotals {
            asset_id: asset.id,
            symbol: asset.symbol.clone(),
            asset_type: asset.asset_type.clone(),
            asset_currency: asset.currency.clone(),
            realized_in_window,
            realized_cumulative,
            unrealized_at_end,
            market_value,
            cost_basis,
            price_date_used: Some(price_date_used),
            fx_rate_date_used: replay.fx_rate_date_used,
            price_source: Some(price_source),
            fx_applied,
            split_applied,
        });
    }

    if !missing.is_empty() {
        // Fail closed. A partial tax report is the outcome Goal 5 exists to prevent, so
        // "return what was collected" is not an option.
        return Err(ReportError::no_data(missing));
    }

    Ok(ReportResponse {
        context: ReportContext {
            portfolio_id: portfolio.id,
            portfolio_name: portfolio.name.clone(),
            base_currency: portfolio.base_currency.clone(),
            strategy: strategy.to_string(),
            threshold_days,
            requested_from,
            requested_to: window.to,
            splits_note: SPLITS_NOTE.to_string(),
        },
        asset_totals,
        totals,
        rows,
    })
}
```

Add the two helpers to `backend_rust/src/schemas.rs` (methods only — the generated contract does not move, and Step 10 re-runs the generator to prove it):

```rust
impl AssetTotals {
    /// An asset with nothing dated at or before the window end: no realized figure, no
    /// open lot, and no close to report.
    pub fn empty(asset: &Asset, fx_applied: bool, split_applied: bool) -> Self {
        AssetTotals {
            asset_id: asset.id,
            symbol: asset.symbol.clone(),
            asset_type: asset.asset_type.clone(),
            asset_currency: asset.currency.clone(),
            realized_in_window: Decimal::ZERO,
            realized_cumulative: Decimal::ZERO,
            unrealized_at_end: Decimal::ZERO,
            market_value: Decimal::ZERO,
            cost_basis: Decimal::ZERO,
            price_date_used: None,
            fx_rate_date_used: None,
            price_source: None,
            fx_applied,
            split_applied,
        }
    }
}

impl ReportTotals {
    pub fn zero() -> Self {
        ReportTotals {
            realized_in_window: Decimal::ZERO,
            realized_cumulative: Decimal::ZERO,
            unrealized_at_end: Decimal::ZERO,
            market_value: Decimal::ZERO,
            cost_basis: Decimal::ZERO,
        }
    }
}
```

- [ ] **Step 7: Rewire the handler**

Replace the body of `get_portfolio_tax_summary` in `backend_rust/src/api_routes/analytics.rs`. The signature gains two parameters and its return type changes; the `#[utoipa::path]` block is already the one Task 3 wrote.

```rust
#[get("/<id>/tax-summary?<strategy>&<threshold_days>&<from>&<to>")]
pub async fn get_portfolio_tax_summary(
    user: AuthedUser,
    id: i32,
    pool: &State<SqlitePool>,
    currency_service: &State<CurrencyService>,
    strategy: Option<String>,
    threshold_days: Option<i64>,
    from: Option<String>,
    to: Option<String>,
) -> Result<TaxSummaryResponse, ReportError> {
    // First statement in the body on purpose: `check_access_gate.sh`'s Rule B ends a
    // handler at the first column-0 `}` and requires a `require*` call before any
    // `WHERE id` query anywhere in it.
    require(pool.inner(), user.id, id, Access::Viewer)
        .await
        .map_err(ReportError::from_status)?;

    let pool = pool.inner();
    let today = Utc::now().date_naive();
    let window = parse_window(from.as_deref(), to.as_deref(), today)?;

    // Strict on the dated path, permissive on the undated one (decision D6).
    let strategy_val = match &window {
        Some(_) => strict_strategy(strategy.as_deref())?,
        None => normalize_strategy(strategy.as_deref()),
    };
    let threshold_val = threshold_days.unwrap_or(30);
    if window.is_some() && threshold_val < 0 {
        return Err(ReportError::validation("threshold_days", "must not be negative"));
    }

    let portfolio = load_portfolio(pool, id).await?;
    let assets = load_assets(pool, id).await?;

    match window {
        Some(window) => {
            let report = build_tax_report(
                pool, currency_service.inner(), &portfolio, &assets,
                &strategy_val, threshold_val, &window, REPORT_DEADLINE,
            )
            .await?;
            Ok(TaxSummaryResponse::Report(Json(report)))
        }
        None => {
            let summary = undated_summary(pool, currency_service.inner(), &portfolio, &assets, &strategy_val, threshold_val).await?;
            Ok(TaxSummaryResponse::Legacy(Json(summary)))
        }
    }
}

async fn load_portfolio(pool: &SqlitePool, id: i32) -> Result<Portfolio, ReportError> {
    sqlx::query_as::<_, Portfolio>("SELECT * FROM portfolios WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| ReportError::internal(e.to_string()))?
        .ok_or_else(ReportError::not_found)
}

async fn load_assets(pool: &SqlitePool, portfolio_id: i32) -> Result<Vec<Asset>, ReportError> {
    sqlx::query_as::<_, Asset>("SELECT * FROM assets WHERE portfolio_id = ?")
        .bind(portfolio_id)
        .fetch_all(pool)
        .await
        .map_err(|e| ReportError::internal(e.to_string()))
}

/// The undated response, moved verbatim out of the handler: same numbers, same shape,
/// same keys, including the empty-portfolio early return and the splits sync this
/// endpoint has always performed before reading. The dated path deliberately does not
/// sync splits (issue #50).
async fn undated_summary(
    pool: &SqlitePool,
    currency_service: &CurrencyService,
    portfolio: &Portfolio,
    assets: &[Asset],
    strategy: &str,
    threshold_days: i64,
) -> Result<serde_json::Value, ReportError> {
    if assets.is_empty() {
        return Ok(serde_json::json!({
            "assets": [],
            "total_portfolio_value": "0",
            "total_realized_pnl": "0",
            "total_unrealized_pnl": "0"
        }));
    }

    let mut asset_summaries = Vec::new();
    let mut total_value = Decimal::ZERO;
    let mut total_realized = Decimal::ZERO;
    let mut total_unrealized = Decimal::ZERO;

    for asset in assets {
        // Auto-detect and persist any unrecorded stock splits for this asset before
        // computing tax lots, so they feed into P&L and value.
        let _ = currency_service.sync_splits_for_asset(asset, pool).await;

        let transactions = sqlx::query_as::<_, Transaction>("SELECT * FROM transactions WHERE asset_id = ?")
            .bind(asset.id)
            .fetch_all(pool)
            .await
            .map_err(|e| ReportError::internal(e.to_string()))?;

        // A pricing failure is still exactly what it was on this path: a zero, and a 200
        // response. This endpoint is undated, so the type change must not move a number
        // here; a dated report is where an `Err` becomes an error instead.
        let current_price = match currency_service
            .get_price(&asset.symbol, asset.isin.as_deref(), pool)
            .await
        {
            Ok(price) => price,
            Err(e) => {
                eprintln!("WARN: no price for {}: {}", asset.symbol, e);
                Decimal::ZERO
            }
        };

        let summary = TaxLotEngine::calculate_lots(
            &asset.symbol, &asset.asset_type, &transactions, current_price,
            &asset.currency, &portfolio.base_currency, currency_service,
            strategy, threshold_days, pool,
        )
        .await
        .map_err(|e| ReportError::internal(e.to_string()))?;

        total_value += summary.market_value;
        total_realized += summary.realized_pnl;
        total_unrealized += summary.unrealized_pnl;
        asset_summaries.push(summary);
    }

    Ok(serde_json::json!({
        "assets": asset_summaries,
        "total_portfolio_value": total_value.to_string(),
        "total_realized_pnl": total_realized.to_string(),
        "total_unrealized_pnl": total_unrealized.to_string(),
        "strategy": strategy,
        "threshold_days": threshold_days,
        "currency": portfolio.base_currency,
    }))
}
```

- [ ] **Step 8: Add the dated variant to the access proofs**

In `backend_rust/tests/isolation_test.rs`, extend `b_cannot_read_a_s_tax_summary_performance_or_sell_pnl` with the dated variant, in the same test so the denial and the success are adjacent:

```rust
    // The dated variant, which the grep-based access gate cannot see: it is a
    // parameter-dependent path on an existing route, so this matrix is the real gate
    // (Access level, spec). `require` runs before the window is even parsed, so B's
    // dated request is a 404 and not a 422.
    let dated_b = format!("/api/portfolios/{}/tax-summary?strategy=FIFO&threshold_days=365&from=2024-01-01&to=2024-12-31", u.a_portfolio);
    assert_eq!(b.get(&dated_b).dispatch().await.status(), Status::NotFound);

    // A's own dated request gets past the access check and stops at the window: the
    // fixture has one transaction and no seeded close, so a window that could be answered
    // would reach the provider. `from > to` proves the request arrived instead.
    let invalid_a = format!("/api/portfolios/{}/tax-summary?from=2024-12-31&to=2024-01-01", u.a_portfolio);
    assert_eq!(a.get(&invalid_a).dispatch().await.status(), Status::UnprocessableEntity);
```

The 200 path of the dated variant is covered in `tax_report_test.rs` with a seeded close, which this file does not seed.

In `backend_rust/tests/auth_test.rs`, correct the stale comment and cover the dated variant. The `filter` closure currently appends strategy parameters to the two tax routes and its comment claims those routes *require* a strategy — `analytics.rs:59` defaults to FIFO, so that was never true:

```rust
            // The tax routes default `strategy` and `threshold_days`; the parameters are
            // here so the sweep exercises the same shape the UI sends, not because the
            // route requires them.
            if path.ends_with("/tax-summary") || path.ends_with("/pnl") {
                (r.method, format!("{path}?strategy=FIFO&threshold_days=365"))
            } else if path.starts_with("/api/assets/lookup") {
```

and add a second, dated pass over the same path (no new entry, because no route is added):

```rust
#[tokio::test]
async fn every_tax_route_denies_the_dated_variant_without_a_session() {
    let client = oidc_client().await;
    let dated = "/api/portfolios/1/tax-summary?strategy=FIFO&threshold_days=365&from=2024-01-01&to=2024-12-31";
    assert_eq!(client.get(dated).dispatch().await.status(), Status::Unauthorized);
    let csvish = "/api/portfolios/1/tax-summary?from=2024-01-01&to=2024-12-31";
    assert_eq!(client.get(csvish).dispatch().await.status(), Status::Unauthorized);
}
```

- [ ] **Step 9: Record what cannot be tested here, and why**

Two cases the spec asks for cannot be written as integration tests, and this plan does not pretend otherwise. Both are the same missing seam, and the compensating tests are in Step 3.

1. **"A missing price returns 422 naming every missing symbol."** A seeded database cannot produce `Err(NoData)`: `get_historical_prices` answers from `historical_prices` when the window's newest row is within `PRICE_WINDOW_FRESH_DAYS` of the requested date, and otherwise asks the provider (`currency_service.rs:713-717`). There is no third path, so "no data" is only ever the provider's answer — and the provider is a hardcoded Yahoo URL (`CurrencyService::new()` takes no arguments, and the URL is built at `:756`), so a test that wants it reaches the network. What is pinned instead: the *shape* of the 422 (`missing_prices`, `missing_rates`, both totals, `retryable: false`) through the validation failures, which share the body; the caps and the pair-not-symbol rule through `MissingItems`; and `pricing_failure_status` for the mapping itself.
2. **"An unreachable provider returns 503 with `retryable: true`."** Same seam, and the deadline test is the hermetic path to that body: `the_deadline_fails_closed_instead_of_returning_a_partial_report` asserts the status, `retryable`, and that no partial payload comes back.

If a future change adds a provider seam, these two cases become writable and should be added; until then, a test asserting them would either hit the network or pass for the wrong reason.

- [ ] **Step 10: Run every gate**

```bash
cd backend_rust && cargo test && bash scripts/check_access_gate.sh && cargo clippy --all-targets
cargo run --bin backend_rust -- --print-openapi | diff - ../docs/openapi/openapi.json
```

Expected: all green, and the generator's output identical to the committed file — this task added methods to `schemas.rs`, not fields, so nothing in the contract moves. `check_access_gate.sh` must still report the same handler count it reported before: the handler still takes `AuthedUser` and calls `require` first.

- [ ] **Step 11: Commit**

```bash
git add backend_rust/src/api_routes/analytics.rs backend_rust/src/schemas.rs \
        backend_rust/tests/common/mod.rs backend_rust/tests/tax_report_test.rs \
        backend_rust/tests/isolation_test.rs backend_rust/tests/auth_test.rs
git commit -m "feat(api): serve a dated tax report from tax-summary"
```

---

## Task 5: `?format=csv` — the server writes the file

The CSV is produced by the server, not assembled in the browser: the same route answers `?format=csv` and serializes the payload it already computed, so the file is emitted where the exact decimals live. That is what makes Goal 4 testable at all — `curl --cookie` (or the Rocket local client) reproduces the bytes — and it is why every argument for server-owned numbers is an argument for emitting the file there.

`report_csv.rs` is **deliberately not under `src/api_routes/`**: the access gate's Rule A requires every file in that directory to import `crate::access` and call a `require*` function, and a serializer has no business calling one.

**Files:**
- Create: `backend_rust/src/report_csv.rs`
- Modify: `backend_rust/src/lib.rs` (one line: `pub mod report_csv;`)
- Modify: `backend_rust/src/api_routes/analytics.rs` (`format` parameter, the `Csv` variant, the filename)
- Test: `backend_rust/tests/report_csv_test.rs`

**Interfaces:**
- Consumes: `ReportResponse`, `ReportRow`, `RowKind`, `PriceSource` (Task 3); `ReportError::validation` (Task 4).
- Produces: `pub fn render(report: &ReportResponse) -> String` and `pub const CSV_COLUMNS: [&str; 49]`.

- [ ] **Step 1: Write the failing tests**

Create `backend_rust/tests/report_csv_test.rs`:

```rust
//! The CSV the owner archives: its columns, its totals and its bytes.

mod common;

use common::{build_rocket, disabled_config, seed_split_sync, setup_db, DEFAULT_USER};
use rocket::http::{ContentType, Status};
use rocket::local::asynchronous::Client;
use rust_decimal::Decimal;
use sqlx::sqlite::SqlitePool;
use std::str::FromStr;

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

/// The CSV as `(header, rows)`, split the plain way: no fixture in this file contains a
/// comma inside a cell except the one that deliberately does.
fn parse(body: &str) -> (Vec<&str>, Vec<Vec<&str>>) {
    let lines: Vec<&str> = body.trim_end_matches("\r\n").split("\r\n").collect();
    let header: Vec<&str> = lines[0].split(',').collect();
    let rows = lines[1..].iter().map(|l| l.split(',').collect()).collect();
    (header, rows)
}

fn cell<'a>(header: &[&str], row: &'a [&'a str], column: &str) -> &'a str {
    let i = header.iter().position(|c| *c == column).unwrap_or_else(|| panic!("no {column} column"));
    row[i]
}

/// One portfolio whose name and symbol both start with a formula character, one sell that
/// leaves an unmatched tail, and one lot still open: every row kind and both cell hazards
/// in one request.
async fn csv_fixture(pool: &SqlitePool) -> i32 {
    let port_id = common::seed_portfolio(pool, DEFAULT_USER, "=cmd|'/c calc'!A1", "USD").await;
    let asset_id = common::seed_asset(pool, port_id, "-SYM", "Minus symbol").await;
    common::seed_transaction_at(pool, asset_id, "BUY", 60.0, 100.0, 0.0, "2024-01-01T00:00:00Z").await;
    common::seed_transaction_at(pool, asset_id, "SELL", 80.0, 130.0, 0.0, "2024-05-01T00:00:00Z").await;
    common::seed_daily_prices(pool, "-SYM", "2024-01-01", "2024-06-28", "110.0").await;
    seed_split_sync(pool, "-SYM").await;
    port_id
}

async fn csv_body(port_id: i32, query: &str) -> (Client, String) {
    let pool = setup_db(&disabled_config()).await;
    let _ = csv_fixture(&pool).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();
    let resp = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?{query}&format=csv"))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Ok);
    (client, resp.into_string().await.unwrap())
}

#[tokio::test]
async fn the_file_is_csv_with_the_servers_own_filename() {
    let (_, body) = csv_body(1, "strategy=FIFO&threshold_days=30&from=2024-01-01&to=2024-06-30").await;
    // The body assertion is here so the request is real; the headers are asserted on a
    // second dispatch, because `into_string` consumes the response.
    assert!(body.starts_with("row_type,"), "row 1 is the header row: {body:.80}");
}

#[tokio::test]
async fn the_headers_are_the_columns_order_and_no_comment_row_precedes_them() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = csv_fixture(&pool).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();
    let resp = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?from=2024-01-01&to=2024-06-30&format=csv"))
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::Ok);
    assert_eq!(resp.content_type(), Some(ContentType::new("text", "csv")));
    let disposition = resp.headers().get_one("Content-Disposition").unwrap_or("").to_string();
    assert_eq!(disposition, format!("attachment; filename=\"tax-report-{port_id}-2024-01-01-2024-06-30.csv\""));

    let body = resp.into_string().await.unwrap();
    let (header, _) = parse(&body);
    assert_eq!(header, backend_rust::report_csv::CSV_COLUMNS.to_vec());
    assert!(!body.starts_with('#'), "a leading `#` row maps every column to null on import");
}

#[tokio::test]
async fn the_lot_pnl_column_sums_to_the_assets_realized_in_window() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = csv_fixture(&pool).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();

    let json: serde_json::Value = serde_json::from_str(
        &client
            .get(format!("/api/portfolios/{port_id}/tax-summary?strategy=FIFO&from=2024-01-01&to=2024-06-30"))
            .dispatch()
            .await
            .into_string()
            .await
            .unwrap(),
    )
    .unwrap();
    let body = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?strategy=FIFO&from=2024-01-01&to=2024-06-30&format=csv"))
        .dispatch()
        .await
        .into_string()
        .await
        .unwrap();

    let (header, rows) = parse(&body);
    let sold: Decimal = rows.iter()
        .filter(|r| cell(&header, r, "row_type") == "SOLD_LOT")
        .map(|r| dec(cell(&header, r, "lot_pnl")))
        .sum();
    // The JSON totals are the authority; the file has to agree with them, in Decimals and
    // not in serialized strings.
    assert_eq!(sold, dec(json["asset_totals"][0]["realized_in_window"].as_str().unwrap()));
    assert_eq!(sold, dec("1800.0"));

    // And the columns that must NOT be summed are the ones the release note names.
    let repeated: Decimal = rows.iter()
        .filter(|r| cell(&header, r, "row_type") == "SOLD_LOT")
        .map(|r| dec(cell(&header, r, "realized_pnl")))
        .sum();
    assert_ne!(repeated, sold, "sell-level columns repeat per lot row; summing them double-counts");
}

#[tokio::test]
async fn every_row_kind_reaches_the_file_and_nulls_are_empty_cells() {
    let (_, body) = csv_body(1, "strategy=FIFO&from=2024-01-01&to=2024-06-30").await;
    let (header, rows) = parse(&body);

    let kinds: Vec<&str> = rows.iter().map(|r| cell(&header, r, "row_type")).collect();
    assert!(kinds.contains(&"SOLD_LOT"), "{kinds:?}");
    assert!(kinds.contains(&"OPEN_LOT"), "{kinds:?}");
    assert!(kinds.contains(&"UNMATCHED"), "{kinds:?}");

    let unmatched = rows.iter().find(|r| cell(&header, r, "row_type") == "UNMATCHED").unwrap();
    assert_eq!(cell(&header, unmatched, "unmatched_qty"), "20.0");
    assert_eq!(cell(&header, unmatched, "lot_pnl"), "", "nulls are empty cells, not `0` or `n/a`");
    assert_eq!(cell(&header, unmatched, "market_value"), "");

    // A comma-free fixture, so the only way to see quoting is this cell.
    assert!(
        body.contains("\"'=cmd|'/c calc'!A1\""),
        "a free-text cell with a formula lead is quoted and apostrophe-guarded",
    );
    assert!(body.contains("'-SYM"), "and so is a symbol");
    assert!(!body.contains("e-"), "money is written verbatim and never in exponent notation");
}

#[tokio::test]
async fn two_dispatches_over_unchanged_data_are_byte_identical() {
    // Goal 4, and the reason the file carries no generated-at column: a timestamp would
    // make this unreachable and would make every regenerated file a different file.
    let pool = setup_db(&disabled_config()).await;
    let port_id = csv_fixture(&pool).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();
    let path = format!("/api/portfolios/{port_id}/tax-summary?strategy=FIFO&from=2024-01-01&to=2024-06-30&format=csv");

    let first = client.get(&path).dispatch().await.into_string().await.unwrap();
    let second = client.get(&path).dispatch().await.into_string().await.unwrap();
    assert_eq!(first, second);
}

#[tokio::test]
async fn format_csv_without_a_window_is_a_422_naming_format() {
    // The legacy summary shape has no rows to write, and answering with "everything,
    // priced at the latest close" would quietly answer a different question than the file
    // claims to answer.
    let pool = setup_db(&disabled_config()).await;
    let port_id = csv_fixture(&pool).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();

    let resp = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?format=csv"))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::UnprocessableEntity);
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
    assert!(body["error"].as_str().unwrap().contains("format"));

    let unknown = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?from=2024-01-01&format=json"))
        .dispatch()
        .await;
    assert_eq!(unknown.status(), Status::UnprocessableEntity);
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cd backend_rust && cargo test --test report_csv_test`
Expected: FAIL to compile — `backend_rust::report_csv` does not exist.

- [ ] **Step 3: Write `report_csv.rs`**

Create `backend_rust/src/report_csv.rs`:

```rust
//! Server-side CSV for the dated tax report.
//!
//! Deliberately not under `src/api_routes/`: the access gate's Rule A requires every file
//! in that directory to import `crate::access` and call a `require*` function, and a
//! serializer has no business calling one.
//!
//! Row 1 is always the header row and no comment row precedes it. That is evidence rather
//! than taste: `detectColumns` (`frontend/src/utils/csvParser.ts:25`) normalises every cell
//! of row 1 against `FIELD_ALIASES`, so a leading `#` row maps every column to `null` and
//! an import silently yields nothing. Caveats therefore ride as columns, exactly as the
//! single-sell export already does (`sellPnlCsv.ts:13-16`), and the split caveat comes from
//! the payload (`ReportContext.splits_note`) rather than from a string typed in here.
//!
//! There is no generated-at column on purpose: two dispatches over unchanged data must be
//! byte-identical, which is Goal 4 and is only testable because the file is server-side.

use crate::schemas::{ReportResponse, ReportRow};
use rust_decimal::Decimal;

/// The header, in order: run context, then the asset and its pricing provenance, then the
/// asset's server-computed totals, then the sell, the lot, and the open-lot cells.
///
/// The per-asset totals ride as columns so a reader can check the file against them without
/// recomputing anything, and so a spreadsheet can be reconciled against the screen.
/// `lot_pnl` is the one column whose sum over `SOLD_LOT` rows is a figure at all
/// (`AssetTotals.realized_in_window`); every sell-level column repeats on each lot row and
/// must not be summed, and every `asset_*` column is already a total.
pub const CSV_COLUMNS: [&str; 49] = [
    // run context
    "row_type", "portfolio_id", "portfolio_name", "base_currency", "strategy",
    "threshold_days", "window_from", "window_to", "splits_note",
    // asset and pricing provenance
    "asset_id", "symbol", "asset_type", "asset_currency", "fx_applied", "split_applied",
    "price_source", "price_date_used", "fx_rate_date_used",
    // this asset's server-computed totals
    "asset_realized_in_window", "asset_realized_cumulative", "asset_unrealized_at_end",
    "asset_market_value", "asset_cost_basis",
    // sell context, repeated on each of the sell's lot rows
    "sell_transaction_id", "sell_date", "sold_qty", "sell_price_in_asset_currency",
    "sell_fee_in_asset_currency", "proceeds", "cost_basis", "realized_pnl", "matched_qty",
    "unmatched_qty",
    // the buy lot a sell consumed
    "buy_transaction_id", "buy_date", "buy_price_in_asset_currency", "unit_cost",
    "lot_matched_qty", "lot_qty_before_match", "lot_cost_basis", "lot_proceeds", "lot_pnl",
    "holding_days", "term",
    // the lot still open at the window end
    "original_qty", "remaining_qty", "market_value", "unrealized_gain_loss", "unrealized_roi",
];

/// Characters that make Excel and Google Sheets run a cell instead of showing it. A symbol
/// reaches the file as free text — `csvValidator` checks only that it is non-empty and the
/// backend stores it uppercased and unfiltered — so it is attacker-chosen by the time it is
/// exported. The leading apostrophe is what makes a spreadsheet render the cell as text.
/// Applied to every text cell rather than to an enumerated set: none of the literal cells
/// this file writes starts with one of these characters, so the guard is free.
const FORMULA_LEAD: [char; 6] = ['=', '+', '-', '@', '\t', '\r'];

fn text_cell(value: &str) -> String {
    let guarded = if value.starts_with(FORMULA_LEAD) {
        format!("'{value}")
    } else {
        value.to_string()
    };
    // RFC 4180: a cell containing a comma, a quote or a newline is quoted, and inner
    // quotes are doubled.
    if guarded.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

/// Money and quantities, verbatim from the `Decimal`. Never through `f64`, and never with
/// an exponent: `Decimal`'s display is fixed-point at the stored scale.
fn decimal_cell(value: Option<Decimal>) -> String {
    value.map(|d| d.to_string()).unwrap_or_default()
}

/// A CSV row as cells, already escaped and joined with `,`.
fn line(cells: impl IntoIterator<Item = String>) -> String {
    let mut out = cells.into_iter().collect::<Vec<_>>().join(",");
    out.push_str("\r\n");
    out
}

/// The whole file, `\r\n`-terminated per row (RFC 4180, and what Excel expects).
pub fn render(report: &ReportResponse) -> String {
    let mut out = line(CSV_COLUMNS.iter().map(|c| c.to_string()));

    for row in &report.rows {
        // The asset's totals are looked up, never computed: the CSV is a projection of the
        // payload, so the numbers in the file are the numbers the route produced.
        let totals = report.asset_totals.iter().find(|t| t.asset_id == row.asset_id);
        out.push_str(&render_row(report, row, totals));
    }

    out
}

fn render_row(
    report: &ReportResponse,
    row: &ReportRow,
    totals: Option<&crate::schemas::AssetTotals>,
) -> String {
    let context = &report.context;
    line([
        text_cell(row.row_type.as_str()),
        context.portfolio_id.to_string(),
        text_cell(&context.portfolio_name),
        text_cell(&context.base_currency),
        text_cell(&context.strategy),
        context.threshold_days.to_string(),
        context.requested_from.map(|d| d.to_string()).unwrap_or_default(),
        context.requested_to.to_string(),
        text_cell(&context.splits_note),
        row.asset_id.to_string(),
        text_cell(&row.symbol),
        text_cell(&row.asset_type),
        text_cell(&row.asset_currency),
        totals.map(|t| t.fx_applied).unwrap_or(false).to_string(),
        totals.map(|t| t.split_applied).unwrap_or(false).to_string(),
        row.price_source.map(|s| s.as_str().to_string()).unwrap_or_default(),
        row.price_date_used.map(|d| d.to_string()).unwrap_or_default(),
        row.fx_rate_date_used.map(|d| d.to_string()).unwrap_or_default(),
        decimal_cell(totals.map(|t| t.realized_in_window)),
        decimal_cell(totals.map(|t| t.realized_cumulative)),
        decimal_cell(totals.map(|t| t.unrealized_at_end)),
        decimal_cell(totals.map(|t| t.market_value)),
        decimal_cell(totals.map(|t| t.cost_basis)),
        row.sell_transaction_id.map(|v| v.to_string()).unwrap_or_default(),
        row.sell_date.map(|d| d.to_rfc3339()).unwrap_or_default(),
        decimal_cell(row.sold_qty),
        decimal_cell(row.sell_price),
        decimal_cell(row.sell_fee),
        decimal_cell(row.proceeds),
        decimal_cell(row.cost_basis),
        decimal_cell(row.realized_pnl),
        decimal_cell(row.matched_qty),
        decimal_cell(row.unmatched_qty),
        row.buy_transaction_id.map(|v| v.to_string()).unwrap_or_default(),
        row.buy_date.map(|d| d.to_rfc3339()).unwrap_or_default(),
        decimal_cell(row.buy_price),
        decimal_cell(row.unit_cost),
        decimal_cell(row.lot_matched_qty),
        decimal_cell(row.lot_qty_before_match),
        decimal_cell(row.lot_cost_basis),
        decimal_cell(row.lot_proceeds),
        decimal_cell(row.lot_pnl),
        row.holding_days.map(|v| v.to_string()).unwrap_or_default(),
        row.term.clone().map(|t| text_cell(&t)).unwrap_or_default(),
        decimal_cell(row.original_qty),
        decimal_cell(row.remaining_qty),
        decimal_cell(row.market_value),
        decimal_cell(row.unrealized_gain_loss),
        decimal_cell(row.unrealized_roi),
    ])
}
```

`fx_applied` and `split_applied` are per asset, not per row, so they are read off the asset's totals — the row `render_row` already has in hand for the five total columns. No field is added to `ReportRow`, and the CSV is still a projection of the payload: the totals vector is part of it.

Then add to `backend_rust/src/lib.rs`, after `pub mod access;`:

```rust
pub mod report_csv;
```

This is the one line this plan adds to a file the spec says it does not edit, and the reason is in *Deviations from the spec*: a module that must not live under `api_routes/` still has to be declared somewhere.

- [ ] **Step 4: Add the `format` parameter to the route**

In `backend_rust/src/api_routes/analytics.rs`: extend the `TaxSummaryResponse` enum and its `Responder`, add `parse_format`, and thread it through the handler.

```rust
pub enum TaxSummaryResponse {
    Legacy(Json<serde_json::Value>),
    Report(Json<ReportResponse>),
    Csv { body: String, filename: String },
}

impl<'r> Responder<'r, 'static> for TaxSummaryResponse {
    fn respond_to(self, request: &'r Request<'_>) -> rocket::response::Result<'static> {
        match self {
            TaxSummaryResponse::Legacy(json) => json.respond_to(request),
            TaxSummaryResponse::Report(json) => json.respond_to(request),
            TaxSummaryResponse::Csv { body, filename } => Response::build()
                .status(Status::Ok)
                // `ContentType::new` rather than a constant: Rocket has no CSV constant
                // (`rocket_http-0.5.1/src/header/content_type.rs:230`).
                .header(ContentType::new("text", "csv"))
                .raw_header("Content-Disposition", format!("attachment; filename=\"{filename}\""))
                .sized_body(body.len(), Cursor::new(body))
                .ok(),
        }
    }
}

/// `csv` or nothing. The default is JSON, and JSON is the documented surface: the OpenAPI
/// document describes the typed payload, and `?format=csv` is the same bytes in another
/// envelope, described in the operation's summary rather than as a second content type for
/// status 200 (which utoipa's response map, keyed by status, cannot express).
fn parse_format(format: Option<&str>, dated: bool) -> Result<bool, ReportError> {
    match format {
        None => Ok(false),
        Some("csv") if dated => Ok(true),
        Some("csv") => Err(ReportError::validation("format", "`csv` needs `from` or `to`")),
        Some(other) => Err(ReportError::validation(
            "format",
            &format!("`{other}` is not a known format; only `csv` is accepted"),
        )),
    }
}
```

In the handler: add `format: Option<String>` as the last parameter, change the route attribute to `#[get("/<id>/tax-summary?<strategy>&<threshold_days>&<from>&<to>&<format>")]`, and after `let window = parse_window(...)`, add:

```rust
    let csv = parse_format(format.as_deref(), window.is_some())?;
```

The dated arm becomes:

```rust
        Some(window) => {
            let report = build_tax_report(
                pool, currency_service.inner(), &portfolio, &assets,
                &strategy_val, threshold_val, &window, REPORT_DEADLINE,
            )
            .await?;
            if csv {
                let filename = format!(
                    "tax-report-{}-{}-{}.csv",
                    report.context.portfolio_id,
                    report.context.requested_from.map(|d| d.to_string()).unwrap_or_else(|| "inception".to_string()),
                    report.context.requested_to,
                );
                Ok(TaxSummaryResponse::Csv { body: report_csv::render(&report), filename })
            } else {
                Ok(TaxSummaryResponse::Report(Json(report)))
            }
        }
```

An error in CSV mode returns the typed JSON body, not a CSV: `ReportError::respond_to` is the only error path, and the frontend does not offer a download URL when the JSON probe fails.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd backend_rust && cargo test --test report_csv_test`

Expected: PASS. If `the_headers_are_the_columns_order_and_no_comment_row_precedes_them` fails on the disposition header, check `CSV_COLUMNS`' length against the number of cells `render_row` writes — a mismatch moves every column one place and is the failure mode this test exists for.

- [ ] **Step 6: Regenerate the contract and run every gate**

```bash
cd backend_rust
cargo run --bin backend_rust -- --print-openapi > ../docs/openapi/openapi.json
cargo test && bash scripts/check_access_gate.sh && cargo clippy --all-targets
cd ../frontend && npm run test && npm run lint && npm run typecheck
```

Expected: all green. `report_csv.rs` is not under `src/api_routes/`, so the gate's Rule A does not apply to it and its printed handler count is unchanged.

- [ ] **Step 7: Commit**

```bash
git add backend_rust/src/report_csv.rs backend_rust/src/lib.rs backend_rust/src/schemas.rs \
        backend_rust/src/api_routes/analytics.rs backend_rust/tests/report_csv_test.rs \
        docs/openapi/openapi.json frontend/src/types.ts frontend/src/types/api.d.ts
git commit -m "feat(api): serve the dated tax report as CSV from the same route"
```

(`frontend/src/types/api.d.ts` is regenerated from the committed contract with `npm run generate-types`, so the generated file and the committed one cannot disagree. It has no importers, so it cannot break a renderer.)

---

## Task 6: The report tab — fetch, controls, download, error surface

The dated report is a sixth tab: `ReportControls` (the window, the presets, the download link, the error panel) and `TaxReportTable` (the run context, the five totals, the rows). Both live in `ReportControls.tsx`, the one frontend file the spec's list gives this work; the report's own rendering has no file of its own, so it shares this one as a second export rather than adding a component the spec did not sanction.

Three things this task is the home of:

- **App holds the payload, separately from `taxSummary`.** The normalized summary is never the source for anything here, and the dated fetch is its own request.
- **The report is renderable only while it matches the current selection.** A fetch in flight, a portfolio switch, or a strategy change must not leave the previous window's numbers on screen under the new page's heading — the statement's bug, and the reason it keys its payload.
- **The download is an anchor, not a fetch.** The same `href` the JSON probe used, plus `format=csv`: the bytes the server produced reach the disk unchanged, and the session cookie rides along because `/api` is same-origin in dev and prod (`nginx.conf:12`).

**Files:**
- Create: `frontend/src/components/ReportControls.tsx`
- Modify: `frontend/src/App.tsx` (state, the fetch effect, the nav list, the tab branch)
- Modify: `frontend/src/test/mocks/handlers.ts` (branch the existing `tax-summary` handler on the window)
- Test: `frontend/src/test/components/ReportControls.test.tsx` (props-driven)
- Test: `frontend/src/test/components/AppReport.test.tsx` (MSW, the wiring)

**Interfaces:**
- Consumes: `TaxReport`, `ReportErrorBody`, `ReportRow`, `ReportRowKind` (Task 3), `formatCurrency` / `formatPercent` (`utils/formatters.ts`, both of which accept decimal **strings**), `apiFetch` and `API_BASE`.
- Produces: `ReportControls`, `TaxReportTable`, `currentYearToDate`, `buildReportQuery`.

- [ ] **Step 1: Write the failing tests**

Create `frontend/src/test/components/ReportControls.test.tsx`:

```tsx
import { describe, it, expect, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import ReportControls, { TaxReportTable, currentYearToDate } from '../../components/ReportControls'
import type { ReportErrorBody, TaxReport } from '../../types'

const window2024 = { from: '2024-01-01', to: '2024-06-30' }

/**
 * Totals that deliberately disagree with the rows: `lot_pnl` sums to 1800.00 while
 * `realized_in_window` says 1234.56. A renderer that adds rows up shows 1800.00 and fails.
 * The money is strings on purpose, exactly as the server sends it — the report is the one
 * payload that is not normalized through `number`, so a renderer that tried to add these
 * would concatenate them and be visible immediately.
 */
function fixture(): TaxReport {
  return {
    context: {
      portfolio_id: 1, portfolio_name: 'Retirement', base_currency: 'USD', strategy: 'FIFO',
      threshold_days: 30, requested_from: '2024-01-01', requested_to: '2024-06-30',
      splits_note: 'Splits are as recorded.',
    },
    asset_totals: [{
      asset_id: 1, symbol: 'AAPL', asset_type: 'STOCK', asset_currency: 'USD',
      realized_in_window: '1234.56', realized_cumulative: '9999.99',
      unrealized_at_end: '2500.00', market_value: '12500.00', cost_basis: '10000.00',
      price_date_used: '2024-06-28', fx_rate_date_used: null, price_source: 'database',
      fx_applied: false, split_applied: false,
    }],
    totals: {
      realized_in_window: '1234.56', realized_cumulative: '9999.99',
      unrealized_at_end: '2500.00', market_value: '12500.00', cost_basis: '10000.00',
    },
    rows: [
      {
        row_type: 'SOLD_LOT', asset_id: 1, symbol: 'AAPL', asset_type: 'STOCK', asset_currency: 'USD',
        price_date_used: '2024-06-28', fx_rate_date_used: null, price_source: 'database',
        sell_transaction_id: 3, sell_date: '2024-05-01T00:00:00Z', sold_qty: '80.0',
        sell_price: '130.0', sell_fee: '0.0', proceeds: '7800.0', cost_basis: '6000.0',
        realized_pnl: '1800.0', matched_qty: '60.0', unmatched_qty: '20.0',
        buy_transaction_id: 1, buy_date: '2024-01-01T00:00:00Z', buy_price: '100.0',
        unit_cost: '100.0', lot_matched_qty: '60.0', lot_qty_before_match: '60.0',
        lot_cost_basis: '6000.0', lot_proceeds: '7800.0', lot_pnl: '1800.0',
        holding_days: 121, term: null, original_qty: null, remaining_qty: null,
        market_value: null, unrealized_gain_loss: null, unrealized_roi: null,
      },
    ],
  }
}

describe('currentYearToDate', () => {
  it('defaults to January 1 through today, not December 31', () => {
    // A default ending on December 31 is a future date for most of the year, and the route
    // rejects a future date with a 422 — so the default would be an error, not a default.
    const w = currentYearToDate(new Date('2026-02-14T00:00:00Z'))
    expect(w.from).toBe('2026-01-01')
    expect(w.to).toBe('2026-02-14')
  })
})

describe('ReportControls', () => {
  it('shows the window it was given and links the download at the same window', () => {
    render(
      <ReportControls
        window={window2024}
        onWindowChange={() => {}}
        csvHref="/api/portfolios/1/tax-summary?from=2024-01-01&to=2024-06-30&strategy=FIFO&threshold_days=30&format=csv"
        onRetry={() => {}}
        report={null}
        error={null}
        loading={false}
      />,
    )
    expect(screen.getByLabelText('From')).toHaveValue('2024-01-01')
    expect(screen.getByLabelText('To')).toHaveValue('2024-06-30')
    expect(screen.getByTestId('download-csv')).toHaveAttribute(
      'href',
      '/api/portfolios/1/tax-summary?from=2024-01-01&to=2024-06-30&strategy=FIFO&threshold_days=30&format=csv',
    )
  })

  it('offers a quarter that has not happened yet as unavailable', async () => {
    const onWindowChange = vi.fn()
    render(
      <ReportControls
        window={{ from: '', to: '' }}
        onWindowChange={onWindowChange}
        csvHref=""
        onRetry={() => {}}
        report={null}
        error={null}
        loading={false}
        today={new Date('2026-02-14T00:00:00Z')}
      />,
    )
    // Q1 is in the past, Q3 is not. A preset that filled 2026-07-01..2026-09-30 would be a
    // 422, and a preset that silently clamped its end would be an invented date.
    await userEvent.click(screen.getByRole('button', { name: 'Q1' }))
    expect(onWindowChange).toHaveBeenCalledWith({ from: '2026-01-01', to: '2026-03-31' })
    expect(screen.getByRole('button', { name: 'Q3' })).toBeDisabled()
  })

  it('reports a 422 by naming the symbols and the count of what was omitted', () => {
    const error: ReportErrorBody = {
      error: 'no price or rate exists at or before the requested date',
      missing_prices: ['AAA', 'BBB'],
      missing_rates: [],
      missing_prices_total: 22,
      missing_rates_total: 0,
      retryable: false,
    }
    render(
      <ReportControls window={window2024} onWindowChange={() => {}} csvHref="" onRetry={() => {}}
        report={null} error={error} loading={false} />,
    )
    expect(screen.getByTestId('report-error')).toHaveTextContent('AAA, BBB')
    expect(screen.getByTestId('report-error')).toHaveTextContent('20 more')
  })

  it('offers a retry when the deadline trips', async () => {
    const onRetry = vi.fn()
    const error: ReportErrorBody = {
      error: 'the report exceeded its 20 second deadline',
      missing_prices: [], missing_rates: [], missing_prices_total: 0, missing_rates_total: 0,
      retryable: true,
    }
    render(
      <ReportControls window={window2024} onWindowChange={() => {}} csvHref="" onRetry={onRetry}
        report={null} error={error} loading={false} />,
    )
    await userEvent.click(screen.getByRole('button', { name: /try again/i }))
    expect(onRetry).toHaveBeenCalled()
  })
})

describe('TaxReportTable', () => {
  it('prints the payload totals and never a sum of the rows', () => {
    // Review Focus 2. The rows' `lot_pnl` sums to 1800.00 and the payload's total is
    // 1234.56; the screen must show the server's figure.
    render(<TaxReportTable report={fixture()} />)
    expect(screen.getByTestId('total-realized-in-window')).toHaveTextContent('$1,234.56')
    expect(screen.getByTestId('total-realized-in-window')).not.toHaveTextContent('1,800.00')
  })

  it('names the window that was actually applied and the close it used', () => {
    render(<TaxReportTable report={fixture()} />)
    expect(screen.getByTestId('report-context')).toHaveTextContent('2024-01-01')
    expect(screen.getByTestId('report-context')).toHaveTextContent('2024-06-30')
    expect(screen.getByTestId('row-0-price-date')).toHaveTextContent('2024-06-28')
  })
})
```

Create `frontend/src/test/components/AppReport.test.tsx`:

```tsx
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { http, HttpResponse } from 'msw'
import App from '../../App'
import { server } from '../mocks/server'

/** The dated branch of the shared handler, with the query the tab sent. */
const seen: string[] = []

function datedHandler() {
  return http.get('/api/portfolios/:id/tax-summary', ({ request }) => {
    const url = new URL(request.url)
    seen.push(url.search)
    if (!url.searchParams.has('from') && !url.searchParams.has('to')) {
      return HttpResponse.json({ currency: 'USD', total_portfolio_value: 5000, total_realized_pnl: 0, total_unrealized_pnl: 0, assets: [] })
    }
    return HttpResponse.json(REPORT_FIXTURE)
  })
}

describe('the tax report tab', () => {
  beforeEach(() => { seen.length = 0; vi.stubGlobal('alert', vi.fn()); vi.stubGlobal('confirm', vi.fn(() => true)) })
  afterEach(() => vi.unstubAllGlobals())

  it('fetches the dated variant with the current year to date when the tab is opened', async () => {
    server.use(datedHandler())
    render(<App />)
    await userEvent.click(await screen.findByText('Tax Report'))
    await waitFor(() => expect(screen.getByTestId('total-realized-in-window')).toBeInTheDocument())

    const dated = seen.find((q) => q.includes('from=') || q.includes('to='))
    expect(dated).toBeDefined()
    expect(dated).toContain(`to=${new Date().getFullYear()}-`)
    // The tab's own fetch is dated; the undated summary the rest of the app reads is a
    // separate request and stays undated.
    expect(seen.filter((q) => !q.includes('from=') && !q.includes('to=')).length).toBeGreaterThan(0)
  })

  it('does not render a report that belongs to another portfolio', async () => {
    // Review Focus 3: portfolio 2's report never resolves, so after the switch the state
    // still holds portfolio 1's payload while `selectedId` has moved on. A page with B's
    // numbers under A's heading is the failure this guard exists for.
    server.use(datedHandler())
    server.use(http.get('/api/portfolios/2/tax-summary', () => new Promise(() => {})))
    render(<App />)
    await userEvent.click(await screen.findByText('Tax Report'))
    await waitFor(() => expect(screen.getByTestId('total-realized-in-window')).toBeInTheDocument())

    await userEvent.selectOptions(screen.getByRole('combobox'), 'Retirement Fund')
    await waitFor(() => expect(screen.queryByTestId('total-realized-in-window')).toBeNull())
  })
})

const REPORT_FIXTURE = { /* the `fixture()` object from ReportControls.test.tsx, inlined */ }
```

`REPORT_FIXTURE` is the same object `ReportControls.test.tsx` builds, with `context.portfolio_id` matching the portfolio the shared handlers return (`handlers.ts:14` gives portfolio 1).

- [ ] **Step 2: Run them to verify they fail**

Run: `cd frontend && npx vitest run src/test/components/ReportControls.test.tsx src/test/components/AppReport.test.tsx`
Expected: FAIL — `../../components/ReportControls` does not exist, and `App` has no `Tax Report` nav item.

- [ ] **Step 3: Write the component**

Create `frontend/src/components/ReportControls.tsx`. The shape of it, with the load-bearing parts spelled out:

```tsx
import { formatCurrency, formatPercent } from '../utils/formatters';
import type { ReportErrorBody, ReportRow, TaxReport } from '../types';

export interface ReportWindow { from: string; to: string }

/** Local-date ISO, not `toISOString()`: the latter shifts the date across midnight UTC. */
const isoDate = (d: Date) =>
  `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;

/**
 * The default window: the current year to date. Not January 1 to December 31 -- a December
 * 31 `to` is a future date for most of the year and the route answers 422, so the default
 * would be an error rather than a default.
 */
export function currentYearToDate(today: Date = new Date()): ReportWindow {
  return { from: `${today.getFullYear()}-01-01`, to: isoDate(today) };
}

/** Four quarters plus the current year, each a window the route will accept. */
export function quarterPresets(today: Date): { label: string; window: ReportWindow }[] {
  const year = today.getFullYear();
  const quarters: ReportWindow[] = [
    { from: `${year}-01-01`, to: `${year}-03-31` },
    { from: `${year}-04-01`, to: `${year}-06-30` },
    { from: `${year}-07-01`, to: `${year}-09-30` },
    { from: `${year}-10-01`, to: `${year}-12-31` },
  ];
  return [
    ...quarters.map((window, i) => ({ label: `Q${i + 1}`, window })),
    { label: 'This year to date', window: currentYearToDate(today) },
  ];
}

/**
 * The query the download link and the probe share, so the file cannot describe a different
 * window from the one on screen. An empty input omits its parameter rather than inventing a
 * date: `from` absent means inception and `to` absent means today, and inventing either
 * silently changes the number.
 */
export function buildReportQuery(
  portfolioId: number, window: ReportWindow, strategy: string, thresholdDays: number,
): string {
  const qs = new URLSearchParams();
  if (window.from) qs.set('from', window.from);
  if (window.to) qs.set('to', window.to);
  qs.set('strategy', strategy);
  qs.set('threshold_days', String(thresholdDays));
  return `/portfolios/${portfolioId}/tax-summary?${qs.toString()}`;
}
```

`ReportControls` renders, in order: the two `<input type="date">` fields (labelled `From` / `To` through `<label htmlFor>`, because the tests query by label), the five preset buttons (each `disabled` when its window ends after `today`, with `title` explaining why), an `<a data-testid="download-csv" className="btn btn-primary" href={csvHref}>` that is rendered only when `csvHref` is non-empty, the error panel, and the loading state. The error panel:

```tsx
      {error && (
        <div data-testid="report-error" role="alert" className="glass-card">
          <p>{error.missing_prices.length > 0
            ? `No price for ${error.missing_prices.join(', ')}`
            : error.missing_rates.length > 0
              ? `No rate for ${error.missing_rates.map((r) => `${r.from}→${r.to}`).join(', ')}`
              : error.error}</p>
          {error.missing_prices_total > error.missing_prices.length && (
            <p>{`and ${error.missing_prices_total - error.missing_prices.length} more`}</p>
          )}
          {error.retryable
            ? <button className="btn btn-secondary" onClick={onRetry}>Try again</button>
            : <p>The range or the strategy has to change before this can be answered.</p>}
        </div>
      )}
```

`TaxReportTable` renders the run context (`data-testid="report-context"`, one line naming the
window that was actually applied, the strategy, the threshold, and the split caveat), the
five totals from `report.totals` (each with its own `data-testid`), and one table row per
`report.rows` entry — `Symbol`, `Kind` (`row_type`), `Date`, `Quantity`, `Value`,
`Gain/Loss`, mapped per kind:

```tsx
function cells(row: ReportRow) {
  switch (row.row_type) {
    case 'SOLD_LOT':
      return { date: row.sell_date, qty: row.lot_matched_qty, value: row.lot_proceeds, gain: row.lot_pnl };
    case 'OPEN_LOT':
      return { date: row.buy_date, qty: row.remaining_qty, value: row.market_value, gain: row.unrealized_gain_loss };
    case 'UNMATCHED':
      // No money exists for this row, and an em dash is what the null means.
      return { date: row.sell_date, qty: row.unmatched_qty, value: null, gain: null };
  }
}
```

The first row's price-date cell carries `data-testid="row-0-price-date"` (index-derived, like
`PortfolioDetail`'s lot rows), and every money cell goes through `formatCurrency` / `formatPercent`,
which accept the payload's exact strings. **No `reduce`, no `+`, no arithmetic of any kind in this
file** — the em-dash mapping above is the only conditional, and it decides what to *show*, never
what to compute.

- [ ] **Step 4: Wire the sixth tab**

In `frontend/src/App.tsx`:

1. Imports: `ReportControls, TaxReportTable, currentYearToDate, buildReportQuery` from `./components/ReportControls`; `Receipt` from `lucide-react`; `ReportErrorBody, TaxReport` from `./types`.
2. State and the fetch, next to the tax state (not inside `fetchPortfolioData` — the report is its own request, and the Holdings and Dashboard tabs never pay for it):

```tsx
  const [reportWindow, setReportWindow] = useState<ReportWindow>(() => currentYearToDate());
  // Keyed by what the payload describes, so a switch, a strategy change or an in-flight
  // fetch cannot leave the previous window's numbers on screen under a new page heading.
  // Derived rather than cleared from an effect: five places change selectedId, and a reset
  // that has to be remembered in each one is a reset that gets forgotten.
  const [report, setReport] = useState<{ key: string; payload: TaxReport } | null>(null);
  const [reportError, setReportError] = useState<{ key: string; body: ReportErrorBody } | null>(null);
  const [reportLoading, setReportLoading] = useState(false);
  const [reportAttempt, setReportAttempt] = useState(0);

  const reportKey = `${selectedId}|${reportWindow.from}|${reportWindow.to}|${strategy}|${thresholdDays}`;
  const reportQuery = selectedId === null
    ? ''
    : buildReportQuery(selectedId, reportWindow, strategy, thresholdDays);

  useEffect(() => {
    // Only on the report tab, and only with a window: without one the route answers with the
    // legacy summary, which is a different question from the one this tab asks.
    if (activeTab !== 'report' || selectedId === null) return;
    if (!reportWindow.from && !reportWindow.to) return;
    const key = reportKey;
    const query = reportQuery;
    let cancelled = false;
    const load = async () => {
      setReportLoading(true);
      setReportError(null);
      try {
        const res = await apiFetch(`${API_BASE}${query}`);
        const body = await res.json();
        if (cancelled) return;
        if (res.ok) setReport({ key, payload: body as TaxReport });
        else setReportError({ key, body: body as ReportErrorBody });
      } catch (err) {
        if (!cancelled) setReportError({ key, body: { error: String(err), missing_prices: [], missing_rates: [], missing_prices_total: 0, missing_rates_total: 0, retryable: false } });
      } finally {
        if (!cancelled) setReportLoading(false);
      }
    };
    load();
    return () => { cancelled = true; };
    // `reportKey` and `reportQuery` are derived inside the effect, not listed here: extra
    // derived dependencies are what `react-hooks/exhaustive-deps` reports as unnecessary,
    // and the primitives below are what actually change.
  }, [activeTab, selectedId, reportWindow.from, reportWindow.to, strategy, thresholdDays, reportAttempt]);
```

3. The nav item, after the statement's, using the same `selectTab` so opening the report closes
   the sell breakdown like every other tab:

```tsx
          <li className={`nav-item ${activeTab === 'report' ? 'active' : ''}`} onClick={() => selectTab('report')}>
            <Receipt size={18} /> Tax Report
          </li>
```

4. The branch, as a sibling of the statement branch — a fragment, no layout wrapper, for the same
   reason the statement's comment gives:

```tsx
            {activeTab === 'report' && (
              <>
                <ReportControls
                  window={reportWindow}
                  onWindowChange={setReportWindow}
                  csvHref={reportQuery ? `${reportQuery}&format=csv` : ''}
                  onRetry={() => setReportAttempt((n) => n + 1)}
                  report={report?.key === reportKey ? report.payload : null}
                  error={reportError?.key === reportKey ? reportError.body : null}
                  loading={reportLoading}
                />
                {report?.key === reportKey && <TaxReportTable report={report.payload} />}
              </>
            )}
```

5. `DEFAULT_TAB` and the tab-count comment above it stay as they are: five tabs plus this one is six, and the comment enumerates names rather than a count.

- [ ] **Step 5: Branch the MSW handler**

In `frontend/src/test/mocks/handlers.ts`, the existing `tax-summary` handler (`:121-123`) becomes a branch on the query. This is what the review calls for — a branch on the existing handler, not a second route, because MSW matches by path:

```ts
  http.get('/api/portfolios/:id/tax-summary', ({ request }) => {
    const url = new URL(request.url)
    // The dated report and the legacy summary are the same route. Without this branch the
    // report tab would receive the summary body and its renderer would show nothing, which
    // is exactly the failure the dated tests exist to catch.
    if (url.searchParams.has('from') || url.searchParams.has('to')) {
      return HttpResponse.json(REPORT_FIXTURE)
    }
    return HttpResponse.json({ currency: 'USD', total_portfolio_value: 5000, total_realized_pnl: 0, total_unrealized_pnl: 0, assets: [] })
  }),
```

`REPORT_FIXTURE` is declared once near the top of `handlers.ts` with `portfolio_id: 1`, matching the portfolio the file's other handlers return. `handlers.test.ts` asserts handler *behaviour* and never the handler count, so nothing there moves.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cd frontend && npx vitest run src/test/components/ReportControls.test.tsx src/test/components/AppReport.test.tsx`
Expected: PASS.

- [ ] **Step 7: Run every gate**

```bash
cd frontend && npm run test && npm run lint && npm run typecheck
```

Expected: all green, including the statement's own suite (`AppStatement.test.tsx`, `PortfolioStatement.test.tsx`, `PrintCss.test.ts`) — this task adds a tab and does not touch `print.css`, which hides chrome by the statement root's *position*, not by class.

- [ ] **Step 8: Commit**

```bash
git add frontend/src/components/ReportControls.tsx frontend/src/App.tsx \
        frontend/src/test/mocks/handlers.ts frontend/src/test/components/ReportControls.test.tsx \
        frontend/src/test/components/AppReport.test.tsx
git commit -m "feat(frontend): add the dated tax report tab with a CSV download"
```

---

## Task 7: Docs — the release note, the manual download, the concepts correction

Three documents, one commit. The report's own verification is split by design: everything numeric is a gate, and the one thing no gate can see is a real browser carrying a session cookie into a download.

**Files:**
- Create: `docs/release-notes/2026-11-09-tax-report.md`
- Modify: `docs/auth-testing.md` (append to §5, beside the statement's print walkthrough)
- Modify: `docs/superpowers/specs/2026-11-09-tax-report-and-statement-concepts.md` (the amended non-goal)

**Interfaces:**
- Consumes: everything above.
- Produces: nothing.

- [ ] **Step 1: Write the release note**

Create `docs/release-notes/2026-11-09-tax-report.md` covering:

- **What ships.** `GET /api/portfolios/<id>/tax-summary?from=&to=` answers a dated report: every sell inside the window at lot granularity, every lot still open at the window's end with its unrealized gain at that date, and per-asset and portfolio totals. `?format=csv` returns the same report as `text/csv`; a new Tax Report tab renders it and links the download. No route was added; absent `from`/`to` the endpoint behaves exactly as it did.
- **The one figure that moves.** `TaxLot.original_qty` is now the quantity originally bought and split-adjusted, instead of the already-decremented quantity. Holdings' **Original Qty** column was printing **Remaining Qty** in both columns; it is correct from this release. `TaxLot` also gains `buy_transaction_id`. Nothing else on an existing screen moves.
- **Strategy validation is strict on the dated path only.** `?strategy=average_cost&to=2024-06-30` is a 422 naming the accepted values; without a window the same input still falls back to FIFO, and the response now echoes `FIFO` rather than the string the client sent.
- **The columns that may be summed.** In the CSV, `lot_pnl` filtered to `row_type=SOLD_LOT` is the only column whose sum is a figure — it equals that asset's `asset_realized_in_window`, which the file also carries. Every sell-level column (`proceeds`, `cost_basis`, `realized_pnl`, `sold_qty`, ...) repeats on each of that sell's lot rows and must not be summed; every `asset_*` column is already a total. The disclosure lives here rather than in the file, because anything in the file's first row breaks the importer it is meant to warn.
- **Splits.** The dated path does not sync splits (issue #50). Split-adjusted history is as recorded, the run context and the CSV both say so, and a split recorded after a quarter is filed legitimately changes that quarter's cost basis on regeneration.
- **The first request after a deploy may 503 once.** A cold multi-currency portfolio can exceed the 20 s deadline on its first report and succeed on retry, because that attempt warmed the per-pair series cache. That is the design working, not a bug — without this sentence it reads as one.
- **Reproducibility.** Two downloads over unchanged data are byte-identical, and there is deliberately no generated-at column to break that. The JSON payload is the documented public surface; the CSV is the same payload in another envelope.
- **Not in this release.** No PDF, no tax computation, no cross-portfolio report, no asset subset.

- [ ] **Step 2: Add the manual download step**

Append to `docs/auth-testing.md` §5, after the statement's print walkthrough:

```markdown
### Downloading the tax report

Still signed in through the provider (this is the step no gate can take: the download is a
plain `<a href>`, so the session cookie is what makes it work, and nothing in CI logs in).

1. Open a portfolio, click **Tax Report**. The window defaults to the current year to date.
2. Click **Download CSV**. The file arrives as `tax-report-<id>-<from>-<to>.csv`, named by the
   server's `Content-Disposition`, not by the browser's guess at the URL.
3. Open it in a spreadsheet: row 1 is the header row, there is no `#` row above it, and
   `splits_note` is a column. Check `lot_pnl` filtered to `SOLD_LOT` against
   `asset_realized_in_window` on the same rows — that is the reconciliation the file is for.
4. Copy the URL, add `&format=csv` if it is not there, and load it with `curl --cookie` in a
   shell that holds the session: the bytes must match the browser's exactly.
5. Print the report tab (Ctrl-P) and confirm it prints like the other tabs — the whole shell,
   no print stylesheet — because the statement's `> *:not([data-print="statement"])` rule does
   not match a page with no statement on it.
```

- [ ] **Step 3: Correct the predecessor's amended non-goal**

In `docs/superpowers/specs/2026-11-09-tax-report-and-statement-concepts.md`, the amendment at the end of the *Out-of-Scope Non-Goals* paragraph still says a per-user in-flight report slot and a per-request outbound budget are "now in scope". Both were deleted (decision D2) and replaced by the route's elapsed deadline. Replace the amended sentence with:

```markdown
  *Amended 2026-11-09 by the report design:* the no-machinery clause is narrowed exactly as
  far as one wall-clock bound. The reports carry a 20 s elapsed deadline, checked between
  outbound calls and failing closed with the typed 503; the earlier per-user in-flight slot
  and per-request outbound budget were removed on review, because a slot bounds concurrency
  rather than cost and the budget duplicated a bound the per-pair series cache already
  enforces. Batching frameworks, concurrent fan-out and new timeout knobs remain out of scope.
```

- [ ] **Step 4: Run every gate one last time**

```bash
cd backend_rust && cargo test && bash scripts/check_access_gate.sh && cargo clippy --all-targets && cargo build --release
cargo run --bin backend_rust -- --print-openapi | diff - ../docs/openapi/openapi.json
cd ../frontend && npm run test && npm run lint && npm run typecheck
```

Expected: all green, and the OpenAPI diff empty.

- [ ] **Step 5: Commit**

```bash
git add docs/release-notes/2026-11-09-tax-report.md docs/auth-testing.md \
        docs/superpowers/specs/2026-11-09-tax-report-and-statement-concepts.md
git commit -m "docs: tax report release note, download walkthrough and the amended non-goal"
```

---

## Spec target-file coverage

Every file in the spec's *Target Files to Create/Modify* list, with where it lands. Anything marked *out* is out because the spec says so, and the reason is in the spec.

| Spec file | Landed in | Note |
| :-- | :-- | :-- |
| 1, 2 `currency_service.rs`, `lib.rs` | Task 5 (one line), otherwise out | The pricing-layer half is merged (PR #51). `lib.rs` gains `pub mod report_csv;` and nothing else — see *Deviations* |
| 3 `report_guard.rs` | out | Deleted (D2): the route's deadline replaces it |
| 4 `tax_engine.rs` | Tasks 1, 2 | The `TaxLot` fix, then `replay_at` |
| 5 `schemas.rs` | Tasks 3, 4, 5 | The payload types; then `AssetTotals::empty`/`ReportTotals::zero` |
| 6 `analytics.rs` | Tasks 4, 5 | The dated route, then `format=csv` |
| 7 `openapi.rs` | Task 3 | Registration and the retyped 200 |
| 8 `tests/tax_report_test.rs` | Tasks 3, 4 | Created with the payload tests, extended with the route tests |
| 9 `tests/isolation_test.rs` | Task 4 | The dated variant, which is the real access gate |
| 10 `route_coverage_test.rs`, `auth_test.rs` | Task 4 | No route added, so the counts stand; `auth_test` gains the dated pass and loses a stale comment |
| 11 reconciliation tests | out | Moved to the pricing-layer document with the dated-FX fix |
| 12 `docs/openapi/openapi.json` | Tasks 1, 3, 5 | Regenerated in each schema-changing commit |
| 13 `api.d.ts` / `types.ts` | Tasks 1, 3 | `types.ts` carries the hand-written types; `api.d.ts` is regenerated and has no importers |
| 14 `report_csv.rs` | Task 5 | Outside `src/api_routes/`, so the access gate does not apply to it |
| 15 `tests/report_csv_test.rs` | Task 5 | Bytes, columns, totals, reproducibility |
| 16 frontend `decimal.ts` report path | out | No frontend builder exists: the server writes the file, and `formatCurrency` accepts strings |
| 17 `App.tsx` | Task 6 | Sixth tab, its own fetch, keyed payload |
| 18 `ReportControls.tsx` | Task 6 | Controls plus, as a second export, the on-screen report |
| 19-23 statement and print | out | Shipped on this branch by `2026-11-09-portfolio-statement-print.md` |
| 24 sell-P&L footnote | out | Moved to the pricing-layer document with the dated-FX fix |
| 25 this plan | done | — |
| `analytics_test.rs` | Task 1 | The failing test for `original_qty` |
| `PortfolioDetail.tsx` | Task 1 (fixture only) | The component already renders both columns; only the value was wrong |
| `PortfolioDetail.test.tsx` | Task 1 | Fixture gains `buy_transaction_id`, and the two quantities are pulled apart |
| `package.json`, `main.tsx` | out | Shipped with the statement (`e2e:statement`, the `print.css` import) |
| `reviews/2026-11-09-tax-report.md` | out | The record already exists as `docs/superpowers/reviews/2026-11-09-tax-report/` |
| concepts doc non-goal | Task 7 | The amendment still calls the deleted guard "in scope" |

## Deviations from the spec

Each of these is a place where the spec's text and the code as it now stands do not agree, or where a decision had to be made to make a task writable. None of them re-opens a settled value.

1. **The undated path keeps its legacy response shape.** The typed `ReportResponse` is returned only when `from` or `to` is present. The spec's file 7 says to replace `body = serde_json::Value` with the typed response; doing that for *both* paths breaks two shipped screens, because the statement and `PortfolioDetail` read `assets[].tax_lots`, `current_shares` and `total_portfolio_value`, none of which the report payload carries — and the spec's own *Route* decision says "absent `from` and `to`, the endpoint behaves exactly as it does today". The committed contract therefore documents the dated shape for 200, and the operation's description states the undated form.
2. **`backend_rust/src/lib.rs` gains one line.** The spec says nothing in this document edits `lib.rs`, and that is about the pricing-layer work that used to live in the file. `report_csv.rs` must not live under `src/api_routes/` (the access gate's Rule A), and a module has to be declared in its parent, so `pub mod report_csv;` lands there.
3. **`AssetTotals` carries `asset_type` and `asset_currency`,** which the spec's shape block omits. The CSV writes `sell_price_in_asset_currency` and `buy_price_in_asset_currency` beside base-currency money; without the asset's currency named on the row, those columns are unlabelled. A tax file that mixes currencies without saying so fails Goal 5's "no column is silently substituted".
4. **The per-asset totals ride in the CSV** as `asset_realized_in_window` and its four siblings. The spec's file 14 column description does not mention them, and its *The payload* section does: "The CSV writes the server-computed totals verbatim and states which columns are legal to sum". A reader cannot check the file against a total the file does not carry.
5. **`unrealized_roi` is nullable.** The spec says "nulls are nulls" for the payload and does not give the open-lot ROI's zero-cost behaviour. This plan follows `SellPnlResponse.roi`, whose documented rule is null when the cost basis is zero — rather than `TaxLot.latent_roi`, which writes a zero and cannot be told from a genuine 0%.
6. **Two spec test cases are recorded as unwritable, not faked.** "A missing price returns 422 naming every missing symbol" and "an unreachable provider returns 503" both need a provider seam that does not exist: a seeded database is either fresh — and answers — or stale, and stale routes to the hardcoded provider URL. Task 4 Step 9 names what is pinned instead (the body shape via validation, the caps and the pair rule on `MissingItems`, the mapping on `pricing_failure_status`, and the deadline's 503 with `retryable`). Writing them as integration tests would mean reaching the network.
7. **The frontend's on-screen report shares `ReportControls.tsx`.** The spec's list gives this work files 17 and 18 and nothing else, and a report view needs somewhere to live; it is a second export in the same file rather than a sixth component.
8. **`route_coverage_test.rs` is not modified.** No route is added, so its hard counts (15/15/13) must stay exactly as they are; the dated variant is a second parameterised pass in `auth_test.rs`, not a new entry.
9. **No `docs/superpowers/reviews/2026-11-09-tax-report.md`.** The convention's record for this design already exists as the directory `docs/superpowers/reviews/2026-11-09-tax-report/` (five stage reports plus the consolidated review), and the spec's forced-files table lists it without noticing.

## What this plan does not include

- **No statement or print work.** Shipped on this branch; the spec's Goals 6 and its files 19-23 belong to `2026-11-09-portfolio-statement-print.md`.
- **No pricing-layer work.** Merged as PR #51; this plan consumes it and edits neither `currency_service.rs`'s behaviour nor `lib.rs`'s startup.
- **No new route, no new dependency, no new table or column.** The schema change is one field addition on `TaxLot` and a new set of response types; `init_db` is untouched, so there is no DDL and no migration.
- **No frontend CSV builder, and no string-preserving path in `decimal.ts`.** The server writes the file.
- **No cost guard, no 429, no asset subset, no split sync, no undated-path strict validation.** All four were decided against (D2, D5, D6, Alternative G).
- **No end-to-end browser test for the report.** CI has no browser job (issue #45); the CSV is reproducible with `curl`, and the one thing curl cannot prove — that a real browser carries the session cookie into an anchor download — is the Task 7 manual step.
- **No verification of the access level by the grep-based gate.** It cannot see a parameter-dependent path on an existing route; `isolation_test.rs` is the gate, as the spec says.

## Final gate

```bash
cd backend_rust
cargo test
bash scripts/check_access_gate.sh
cargo clippy --all-targets
cargo build --release
cargo run --bin backend_rust -- --print-openapi | diff - ../docs/openapi/openapi.json
cd ../frontend
npm run test
npm run lint
npm run typecheck
```

Then the manual step in Task 7 Step 2 — a real OIDC login and a real browser download — which no gate above performs.
