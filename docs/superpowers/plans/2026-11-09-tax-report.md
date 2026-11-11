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
        // Review Focus 1. The realized leg converts each transaction at its own date
        // (1.00 here) and the market-value leg converts at `valuation_date` (1.25).
        // A replay that converted both at the transaction date would report 12500.0
        // ... no: it would report 10000.0 market value and no gain at all, and every
        // number would still look plausible.
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
