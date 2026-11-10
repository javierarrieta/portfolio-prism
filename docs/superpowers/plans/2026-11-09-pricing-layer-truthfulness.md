# Pricing Layer Truthfulness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make past-dated pricing honest — FX that resolves and is actually dated, one close per symbol per date, and "no data" distinguishable from "could not ask".

**Architecture:** Four changes inside the existing `CurrencyService`, in a forced order. Uniqueness first so writes stop appending; then FX symbology plus a pure on-or-before selection rule; then a dated `get_rate` backed by a per-pair series cache and the `historical_prices` table; then typed pricing results with a stated database-first read order. No new route, no new table, no new dependency.

**Tech Stack:** Rust 2021, Rocket 0.5, SQLx 0.8 (SQLite), rust_decimal 1.35, chrono 0.4, tokio. Frontend: React + TypeScript, Vitest.

**Spec:** `docs/superpowers/specs/2026-11-09-pricing-layer-truthfulness.md` — this plan implements it. The review that shaped it is in `docs/superpowers/reviews/2026-11-09-tax-report/`.

## Global Constraints

- **Branch:** `fix/pricing-layer-truthfulness`, off `origin/main`. Not `main`, not the design branch.
- **No new dependency.** `backend_rust/Cargo.toml` has **no `[dev-dependencies]` section at all** — no mock server, no wiremock, no mockito. Tests are network-free by construction (pure functions and seeded databases), not by interception. Do not add a mocking crate to fix that; it is the constraint that keeps the suite honest.
- **No network in any test.** Goal 5. A test that reaches `query1.finance.yahoo.com` is a failed test even when it passes.
- **Money is `rust_decimal::Decimal`.** Compare by value, never by serialized string — `"2000"`, `"2000.0"` and `"2000.000"` are equal numbers with different scale. `tests/sell_pnl_test.rs:11-22` already provides the `dec()` and `d()` helpers; copy that pattern rather than inventing one.
- **Schema changes are idempotent DDL inside `init_db`** (`backend_rust/src/lib.rs:118-206`), run on every boot. There is no `migrations/` directory and no `sqlx::migrate!`. Editing a `CREATE TABLE` string changes nothing on an existing database.
- **The undated screen keeps its current numbers** except where the FX fix necessarily moves them. Permissiveness that exists today (`stats_engine.rs:158` `.unwrap_or(Decimal::ONE)`, `analytics.rs:73` treating a failed price as zero) is preserved deliberately; see Open Question 2 in the spec, resolved here as "keep".
- **All backend commands run from `backend_rust/`.** `cargo test`, `bash scripts/check_access_gate.sh`, `cargo clippy --all-targets`, `cargo build --release`. CI runs the first, second and fourth — **not clippy**.
- **Each task ships as its own commit, and each commit message names which existing figures move and why** (Goal 6). A commit that moves numbers without saying so is a review failure regardless of test results.

## Review Focus

These are the input classes the spec implies that no single task's happy-path tests exercise. Each line gets a test in the task that owns the code.

1. **A date with no close within the lookback** — a delisted fund or a currency pair with a data gap. Expected: an error naming the symbol and the date, never a silent `Decimal::ZERO`, because zero is a plausible price and would be filed as one.
2. **A non-trading day** — a Saturday or a market holiday. Expected: the last close strictly before it, and the date actually used is visible to the caller rather than inferred from the requested date.
3. **A legacy database that already holds duplicate `(symbol, date)` rows** — the owner's real `portfolio.db`. Expected: boot succeeds, exactly one row per pair survives, and which one survives is deterministic. If the dedupe misses a duplicate, `CREATE UNIQUE INDEX` fails, `init_db` returns `Err` and the app refuses to start.
4. **Provider unreachable on a dated lookup** — expected: distinguishable from "no data exists", and the undated endpoints behave exactly as they do today.
5. **Same-currency and unusual pairs** — `USD→USD` must short-circuit to exactly one with the requested date, and a pair like `GBP→JPY` must produce `GBPJPY=X`, not `GBP=JPY`.

---

### Task 1: One close per symbol per date (P2)

**Files:**
- Modify: `backend_rust/src/lib.rs` (add `enforce_price_uniqueness`, call it from `init_db` after `migrate_decimal_columns` at `:185`)
- Modify: `backend_rust/src/services/currency_service.rs:395-404` (`cache_price`) and `:486-499` (the batch insert inside `get_historical_prices`) — both become real upserts
- Modify: `backend_rust/src/services/currency_service.rs:722` and `:745` — the two hand-rolled `CREATE TABLE historical_prices` statements in the unit tests gain the unique index
- Test: `backend_rust/tests/price_uniqueness_test.rs` (new)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `pub async fn enforce_price_uniqueness(pool: &SqlitePool) -> anyhow::Result<()>` in `backend_rust::` (re-exported from `lib.rs`, same visibility style as `pub async fn migrate_decimal_columns`). Produces the invariant every later task relies on: at most one row per `(symbol, date)`, and `INSERT … ON CONFLICT(symbol,date) DO UPDATE` as the only write form.

- [ ] **Step 1: Write the failing migration test**

Create `backend_rust/tests/price_uniqueness_test.rs`:

```rust
// Legacy shape: no primary key, no unique constraint, REAL close_price (pre-decimal).
// Duplicate (symbol, date) rows are the whole point: they are what the current
// INSERT OR REPLACE has been appending for every fetch since the table was created.
async fn create_legacy_prices(pool: &sqlx::sqlite::SqlitePool) {
    sqlx::query(
        "CREATE TABLE historical_prices (
            symbol TEXT NOT NULL,
            date DATE NOT NULL,
            close_price TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await
    .unwrap();

    for (symbol, date, price) in [
        ("AAPL", "2024-01-02", "100.0"),
        ("AAPL", "2024-01-02", "101.5"), // duplicate, written later, must survive
        ("AAPL", "2024-01-03", "102.0"),
        ("MSFT", "2024-01-02", "300.0"),
        ("MSFT", "2024-01-02", "301.0"), // duplicate, written later, must survive
        ("MSFT", "2024-01-02", "302.0"), // duplicate, written latest, must survive
    ] {
        sqlx::query("INSERT INTO historical_prices (symbol, date, close_price) VALUES (?, ?, ?)")
            .bind(symbol)
            .bind(date)
            .bind(price)
            .execute(pool)
            .await
            .unwrap();
    }
}

async fn row_count(pool: &sqlx::sqlite::SqlitePool) -> i64 {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM historical_prices")
        .fetch_one(pool)
        .await
        .unwrap();
    n
}

#[tokio::test]
async fn dedupe_leaves_one_row_per_symbol_and_date() {
    let pool = sqlx::sqlite::SqlitePool::connect(":memory:").await.unwrap();
    create_legacy_prices(&pool).await;
    assert_eq!(row_count(&pool).await, 6);

    backend_rust::enforce_price_uniqueness(&pool).await.unwrap();

    assert_eq!(row_count(&pool).await, 2);
    let (aapl,): (String,) = sqlx::query_as(
        "SELECT close_price FROM historical_prices WHERE symbol = 'AAPL' AND date = '2024-01-02'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(aapl, "101.5");
    let (msft,): (String,) = sqlx::query_as(
        "SELECT close_price FROM historical_prices WHERE symbol = 'MSFT' AND date = '2024-01-02'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(msft, "302.0");
}

#[tokio::test]
async fn uniqueness_is_idempotent_and_the_index_exists_afterwards() {
    let pool = sqlx::sqlite::SqlitePool::connect(":memory:").await.unwrap();
    create_legacy_prices(&pool).await;

    backend_rust::enforce_price_uniqueness(&pool).await.unwrap();
    // Second boot on an already-migrated database must be a no-op, not an error:
    // init_db runs on every process start.
    backend_rust::enforce_price_uniqueness(&pool).await.unwrap();
    assert_eq!(row_count(&pool).await, 2);

    let indexes: Vec<(i64, String, i64)> =
        sqlx::query_as("PRAGMA index_list(historical_prices)")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(
        indexes.iter().any(|(_, name, unique)| name == "uq_historical_prices_symbol_date" && *unique == 1),
        "unique index missing; have {:?}",
        indexes.iter().map(|(_, n, _)| n).collect::<Vec<_>>()
    );
    assert!(
        !indexes.iter().any(|(_, name, _)| name == "idx_historical_prices_symbol_date"),
        "the redundant plain index should have been dropped"
    );
}

#[tokio::test]
async fn a_duplicate_insert_is_rejected_by_the_index() {
    let pool = sqlx::sqlite::SqlitePool::connect(":memory:").await.unwrap();
    create_legacy_prices(&pool).await;
    backend_rust::enforce_price_uniqueness(&pool).await.unwrap();

    // A bare INSERT of an existing pair must fail. This is what proves the index is
    // doing the work rather than the application being careful.
    let dup = sqlx::query("INSERT INTO historical_prices (symbol, date, close_price) VALUES (?, ?, ?)")
        .bind("AAPL")
        .bind("2024-01-02")
        .bind("999.0")
        .execute(&pool)
        .await;
    assert!(dup.is_err(), "duplicate (symbol, date) must be rejected");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test price_uniqueness_test`
Expected: FAIL — `error[E0425]: cannot find function 'enforce_price_uniqueness' in the crate root`.

- [ ] **Step 3: Implement `enforce_price_uniqueness`**

In `backend_rust/src/lib.rs`, immediately after `migrate_decimal_columns` (whose definition ends at `:417`), add:

```rust
/// Collapse duplicate `historical_prices` rows and make `(symbol, date)` unique.
///
/// The table was created with no primary key and only a non-unique index
/// (`lib.rs:170-176`), so `INSERT OR REPLACE` always inserted and every fetch
/// appended. Reads have no `ORDER BY`, which made "the close on date X" an
/// arbitrary row among N.
///
/// The tie-break is `MAX(rowid)`: the table has no write timestamp, so the most
/// recently written row is the only deterministic survivor available. That is a
/// deliberate choice about Yahoo's revisions around corporate actions, and it is
/// what makes an as-of price reproducible from here on.
///
/// Must run AFTER `migrate_decimal_columns`, which renames this table's
/// `close_price` column on a dedicated connection.
pub async fn enforce_price_uniqueness(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        "DELETE FROM historical_prices WHERE rowid NOT IN (
             SELECT MAX(rowid) FROM historical_prices GROUP BY symbol, date
         )",
    )
    .execute(pool)
    .await?;

    // Superseded by the unique index below; leaving it costs a write for no read.
    sqlx::query("DROP INDEX IF EXISTS idx_historical_prices_symbol_date")
        .execute(pool)
        .await?;

    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS uq_historical_prices_symbol_date
             ON historical_prices(symbol, date)",
    )
    .execute(pool)
    .await?;

    Ok(())
}
```

- [ ] **Step 4: Call it from `init_db`, after the decimal migration**

In `backend_rust/src/lib.rs`, the `init_db` body currently reads (`:170-185`):

```rust
    sqlx::query("CREATE TABLE IF NOT EXISTS historical_prices (
        symbol TEXT NOT NULL,
        date DATE NOT NULL,
        close_price TEXT NOT NULL
    )").execute(pool).await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_historical_prices_symbol_date ON historical_prices(symbol, date)")
        .execute(pool)
        .await?;

    sqlx::query("CREATE TABLE IF NOT EXISTS split_sync (
        symbol TEXT PRIMARY KEY,
        last_synced_at DATE NOT NULL
    )").execute(pool).await?;

    migrate_decimal_columns(pool).await?;
```

Replace the plain-index statement and add the call after the decimal migration:

```rust
    sqlx::query("CREATE TABLE IF NOT EXISTS historical_prices (
        symbol TEXT NOT NULL,
        date DATE NOT NULL,
        close_price TEXT NOT NULL
    )").execute(pool).await?;

    sqlx::query("CREATE TABLE IF NOT EXISTS split_sync (
        symbol TEXT PRIMARY KEY,
        last_synced_at DATE NOT NULL
    )").execute(pool).await?;

    migrate_decimal_columns(pool).await?;

    // After the decimal migration: it renames close_price on this same table, and a
    // dedupe that runs before it operates on a schema about to change.
    enforce_price_uniqueness(pool).await?;
```

- [ ] **Step 5: Run the migration tests to verify they pass**

Run: `cargo test --test price_uniqueness_test`
Expected: PASS, 3 tests.

- [ ] **Step 6: Convert both write paths to real upserts**

`INSERT OR REPLACE` was already a no-op without the constraint; with the constraint it deletes and re-inserts, which changes `rowid` and would silently reorder the tie-break on a database that somehow still holds duplicates. Use an explicit upsert.

In `backend_rust/src/services/currency_service.rs`, `cache_price` (`:395-404`) becomes:

```rust
    async fn cache_price(&self, symbol: &str, date: NaiveDate, price: Decimal, pool: &SqlitePool) {
        let _ = sqlx::query(
            "INSERT INTO historical_prices (symbol, date, close_price) VALUES (?, ?, ?)
                 ON CONFLICT(symbol, date) DO UPDATE SET close_price = excluded.close_price",
        )
        .bind(symbol)
        .bind(date)
        .bind(crate::db_types::decimal_to_str(&price))
        .execute(pool)
        .await;
    }
```

And the batch insert inside `get_historical_prices` (`:486-499`) becomes:

```rust
        if !daily_prices.is_empty() {
            let mut tx = pool.begin().await?;
            for (date, price) in &daily_prices {
                let _ = sqlx::query(
                    "INSERT INTO historical_prices (symbol, date, close_price) VALUES (?, ?, ?)
                         ON CONFLICT(symbol, date) DO UPDATE SET close_price = excluded.close_price",
                )
                .bind(symbol)
                .bind(*date)
                .bind(crate::db_types::decimal_to_str(price))
                .execute(&mut *tx)
                .await;
            }
            let _ = tx.commit().await;
        }
```

- [ ] **Step 7: Give the hand-rolled test schemas the index**

`currency_service.rs:722` and `:745` each `CREATE TABLE historical_prices` themselves, bypassing `init_db`. Assertions written against a schema the production path does not produce prove nothing. In both places, replace

```rust
        sqlx::query("CREATE TABLE IF NOT EXISTS historical_prices (symbol TEXT, date DATE, close_price TEXT)")
            .execute(&pool)
            .await
            .unwrap();
```

with

```rust
        sqlx::query(
            "CREATE TABLE historical_prices (symbol TEXT NOT NULL, date DATE NOT NULL, close_price TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE UNIQUE INDEX uq_historical_prices_symbol_date ON historical_prices(symbol, date)",
        )
        .execute(&pool)
        .await
        .unwrap();
```

- [ ] **Step 8: Run the whole backend suite**

Run: `cargo test`
Expected: PASS. If `tests/analytics_test.rs` fails here, note the failure and continue — that suite seeds with `INSERT OR REPLACE`, which is still valid SQL; it is Task 5 that changes its read behaviour.

- [ ] **Step 9: Commit**

```bash
git add backend_rust/src/lib.rs backend_rust/src/services/currency_service.rs backend_rust/tests/price_uniqueness_test.rs
git commit -m "fix(pricing): make historical_prices unique per symbol and date

P2. The table had no primary key and only a non-unique index, so INSERT OR
REPLACE always appended and reads without ORDER BY returned an arbitrary row.
Dedupe with MAX(rowid) as the tie-break, add the unique index, drop the plain
index, and make both write paths real upserts.

Figures moved: none yet. No read path changed; this only stops the table
accumulating duplicates. It must land before the dated FX work, whose range
fetch would otherwise append roughly 250 rows per symbol per call."
```

---

### Task 2: FX symbology, range request, and the on-or-before rule

**Files:**
- Modify: `backend_rust/src/services/currency_service.rs` (new constants, `fx_symbol`, `select_rate`; unit tests in the existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nothing beyond Task 1's invariant.
- Produces: `pub(crate) fn select_rate(series: &[(NaiveDate, Decimal)], date: NaiveDate, lookback_days: i64) -> Option<(NaiveDate, Decimal)>`, `pub(crate) fn fx_symbol(from: &str, to: &str) -> String`, and the constants `FX_SERIES_WINDOW_DAYS: i64 = 365`, `FX_LOOKBACK_DAYS: i64 = 30`. Task 3 consumes all four.

- [ ] **Step 1: Write the failing unit tests for the selection rule**

In `backend_rust/src/services/currency_service.rs`, inside the existing `mod tests`, add. These are the Review Focus 1 and 2 tests, and they are pure — no pool, no network:

```rust
    fn series(pairs: &[(&str, &str)]) -> Vec<(NaiveDate, Decimal)> {
        pairs
            .iter()
            .map(|(d, p)| {
                (
                    NaiveDate::parse_from_str(d, "%Y-%m-%d").unwrap(),
                    Decimal::from_str(p).unwrap(),
                )
            })
            .collect()
    }

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn select_rate_uses_the_exact_date_when_present() {
        let s = series(&[("2024-03-01", "0.9000"), ("2024-03-04", "0.9050")]);
        let (used, rate) = select_rate(&s, day("2024-03-04"), FX_LOOKBACK_DAYS).unwrap();
        assert_eq!(used, day("2024-03-04"));
        assert_eq!(rate, Decimal::from_str("0.9050").unwrap());
    }

    #[test]
    fn select_rate_falls_back_to_the_prior_trading_day() {
        // Review Focus 2: 2024-03-09 is a Saturday; the Friday close is the answer,
        // and the caller learns which date was used instead of assuming its own.
        let s = series(&[("2024-03-07", "0.9100"), ("2024-03-08", "0.9125")]);
        let (used, rate) = select_rate(&s, day("2024-03-09"), FX_LOOKBACK_DAYS).unwrap();
        assert_eq!(used, day("2024-03-08"));
        assert_eq!(rate, Decimal::from_str("0.9125").unwrap());
    }

    #[test]
    fn select_rate_rejects_a_date_before_every_close() {
        let s = series(&[("2024-03-07", "0.9100")]);
        assert_eq!(select_rate(&s, day("2024-03-01"), FX_LOOKBACK_DAYS), None);
    }

    #[test]
    fn select_rate_rejects_a_gap_wider_than_the_lookback() {
        // Review Focus 1: a delisted pair whose last close is three years old must
        // produce no rate at all rather than a stale one.
        let s = series(&[("2021-01-04", "0.8800")]);
        assert_eq!(select_rate(&s, day("2024-03-09"), FX_LOOKBACK_DAYS), None);
    }

    #[test]
    fn select_rate_accepts_a_gap_exactly_at_the_lookback_edge() {
        let date = day("2024-03-09");
        let edge = date - chrono::Duration::days(FX_LOOKBACK_DAYS);
        let s = series(&[("2024-02-08", "0.8900")]);
        assert_eq!(edge, day("2024-02-08"), "fixture must sit on the boundary");
        assert_eq!(select_rate(&s, date, FX_LOOKBACK_DAYS).map(|(d, _)| d), Some(edge));
    }

    #[test]
    fn select_rate_handles_an_unsorted_series_and_an_empty_one() {
        let s = series(&[("2024-03-08", "0.9125"), ("2024-03-06", "0.9075"), ("2024-03-07", "0.9100")]);
        assert_eq!(select_rate(&s, day("2024-03-09"), FX_LOOKBACK_DAYS).map(|(d, _)| d), Some(day("2024-03-08")));
        assert_eq!(select_rate(&[], day("2024-03-09"), FX_LOOKBACK_DAYS), None);
    }

    #[test]
    fn fx_symbol_uses_yahoo_pair_symbology() {
        // Review Focus 5: the live endpoint returns "No data found" for GBP=JPY and
        // data for GBPJPY=X. The old form is why no foreign-currency asset priced.
        assert_eq!(fx_symbol("EUR", "USD"), "EURUSD=X");
        assert_eq!(fx_symbol("GBP", "JPY"), "GBPJPY=X");
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test --lib select_rate`
Expected: FAIL — `cannot find function 'select_rate' in this scope`.

- [ ] **Step 3: Implement the constants, `fx_symbol` and `select_rate`**

In `backend_rust/src/services/currency_service.rs`, next to the existing `MAX_RETRIES` / `RATE_LIMIT_DELAY` constants near the top of the file:

```rust
/// How far back a first fetch for a pair reaches. One wide window per pair, so a
/// year of dated lookups costs a single outbound call instead of one per distinct
/// transaction date.
pub(crate) const FX_SERIES_WINDOW_DAYS: i64 = 365;

/// How stale a close may be and still answer for a date. Thirty days is generous
/// for a currency pair, which trades nearly every day, and tight enough that a
/// delisted or mis-symbolled pair produces an error instead of a years-old rate
/// silently used as a tax figure.
pub(crate) const FX_LOOKBACK_DAYS: i64 = 30;

/// Yahoo's FX pair symbology. `EUR=USD` returns "No data found"; `EURUSD=X`
/// returns data. `get_rate` built the first form (`currency_service.rs:235`),
/// which is why every asset whose currency differed from the portfolio's base
/// currency failed to price at all.
pub(crate) fn fx_symbol(from: &str, to: &str) -> String {
    format!("{}{}=X", from.to_uppercase(), to.to_uppercase())
}

/// The last close on or before `date`, within `lookback_days`. Returns the date
/// actually used alongside the value: a caller that reports a figure "as of" a
/// date must be able to say which date priced it, and silent substitution is the
/// failure this whole change exists to remove.
///
/// Pure and order-independent on purpose, so the rule is testable without a pool
/// or a network.
pub(crate) fn select_rate(
    series: &[(NaiveDate, Decimal)],
    date: NaiveDate,
    lookback_days: i64,
) -> Option<(NaiveDate, Decimal)> {
    let earliest = date.checked_sub_signed(chrono::Duration::days(lookback_days))?;
    series
        .iter()
        .filter(|(d, _)| *d <= date && *d >= earliest)
        .max_by_key(|(d, _)| *d)
        .cloned()
}
```

- [ ] **Step 4: Run them to verify they pass**

Run: `cargo test --lib select_rate fx_symbol`
Expected: PASS, 7 tests.

- [ ] **Step 5: Commit**

```bash
git add backend_rust/src/services/currency_service.rs
git commit -m "feat(pricing): add FX pair symbology and the on-or-before rate rule

Not yet wired in. select_rate is the rule (last close on or before the date,
within a 30-day lookback, returning the date used) and fx_symbol builds
EURUSD=X rather than the EUR=USD form the live endpoint rejects.

Figures moved: none, nothing calls these yet."
```

---

### Task 3: `get_rate` that is actually dated

**Files:**
- Modify: `backend_rust/src/services/currency_service.rs:121-127` (cache field type), `:226-262` (`get_rate` rewrite)
- Modify: `backend_rust/src/engines/tax_engine.rs` — `replay` (`:390-397`, gains a pool), `:403`, `:551`, `calculate_lots` (`:536-546`, gains a pool), `sell_breakdown` (`:619-627`, gains a pool), and the in-file test helpers at `:29`, `:208`, `:342`, `:348`
- Modify: `backend_rust/src/api_routes/analytics.rs:75` and `:239` — the two production callers
- Modify: `backend_rust/src/engines/stats_engine.rs:155-160`
- Modify: `backend_rust/src/services/currency_service.rs:700-716` (the three same-currency `get_rate` unit tests)
- Test: `backend_rust/tests/fx_pricing_test.rs` (new)

**The blast radius, stated up front.** `get_rate` needs a pool to read the database, and `CurrencyService::new()` takes no arguments at 24 call sites, so the pool cannot ride on the service. Threading it as a parameter reaches four engine functions and two route handlers: `replay` (callers `:557` inside `calculate_lots` and `:629` inside `sell_breakdown`), `calculate_lots` (callers `analytics.rs:75` and the in-file tests at `:29` and `:342`), and `sell_breakdown` (caller `analytics.rs:239`). Every one of those can reach a `&SqlitePool` at its call site. This is the widest part of the change; it is mechanical, and it is why this task is third rather than first.

**Interfaces:**
- Consumes: `fx_symbol`, `select_rate`, `FX_SERIES_WINDOW_DAYS`, `FX_LOOKBACK_DAYS` from Task 2; the unique index from Task 1, which is what lets the range fetch persist without duplicating.
- Produces: `pub struct RateAt { pub rate: Decimal, pub date_used: NaiveDate }` and the new signature `pub async fn get_rate(&self, from: &str, to: &str, date: DateTime<Utc>) -> Result<RateAt>`. Every caller in the crate uses `.rate`. Task 4 and the tax-report design both consume `date_used`.

- [ ] **Step 1: Write the failing hermetic test**

Create `backend_rust/tests/fx_pricing_test.rs`. The seeded `EURUSD=X` rows are what make this network-free: the new `get_rate` consults the database before the provider, so a seeded window never issues a request.

```rust
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use sqlx::sqlite::SqlitePool;
use std::str::FromStr;

use backend_rust::services::currency_service::{no_rate_error, CurrencyService};

async fn seed_pair(pool: &SqlitePool, symbol: &str, rows: &[(&str, &str)]) {
    for (date, price) in rows {
        sqlx::query(
            "INSERT INTO historical_prices (symbol, date, close_price) VALUES (?, ?, ?)
                 ON CONFLICT(symbol, date) DO UPDATE SET close_price = excluded.close_price",
        )
        .bind(symbol)
        .bind(NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap())
        .bind(*price)
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn pool_with_prices() -> SqlitePool {
    let pool = SqlitePool::connect(":memory:").await.unwrap();
    backend_rust::init_db(&pool, &backend_rust::StartupConfig::default_for_test())
        .await
        .unwrap();
    pool
}

fn ts(date: &str) -> DateTime<Utc> {
    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

#[tokio::test]
async fn get_rate_returns_the_seeded_past_dated_rate() {
    let pool = pool_with_prices().await;
    // A Friday close and nothing after it: the Saturday request must resolve to it.
    seed_pair(&pool, "EURUSD=X", &[("2024-03-07", "0.9100"), ("2024-03-08", "0.9125")]).await;

    let svc = CurrencyService::new();
    let got = svc.get_rate("EUR", "USD", ts("2024-03-09")).await.unwrap();

    assert_eq!(got.rate, dec("0.9125"));
    assert_eq!(got.date_used, NaiveDate::parse_from_str("2024-03-08", "%Y-%m-%d").unwrap());
}

#[tokio::test]
async fn get_rate_does_not_return_todays_rate_for_a_past_date() {
    // The bug this task exists to fix: the old get_rate keyed its cache on the
    // requested date and then returned the last close in the series, so the stored
    // value looked correctly dated and was not.
    let pool = pool_with_prices().await;
    seed_pair(
        &pool,
        "EURUSD=X",
        &[("2024-03-08", "0.9125"), ("2026-11-09", "1.1000")],
    )
    .await;

    let svc = CurrencyService::new();
    let got = svc.get_rate("EUR", "USD", ts("2024-03-31")).await.unwrap();

    assert_eq!(got.rate, dec("0.9125"), "a past date must not pick up the latest close");
    assert_eq!(got.date_used, NaiveDate::parse_from_str("2024-03-08", "%Y-%m-%d").unwrap());
}

#[tokio::test]
async fn a_rate_beyond_the_lookback_names_the_symbol_and_the_date() {
    // Review Focus 1, tested at the message rather than the fetch. The gap case
    // cannot be produced hermetically at this level: the fetch window (365 days) is
    // wider than the lookback (30), so any row the lookback would reject is also
    // outside the window, which means a database miss and a real provider call. The
    // rule itself is pinned hermetically in Task 2 by
    // select_rate_rejects_a_gap_wider_than_the_lookback; what is pinned here is that
    // the error a user sees carries enough to act on.
    let err = no_rate_error("EURUSD=X", "2024-03-31", 30);
    let msg = err.to_string();
    assert!(msg.contains("EURUSD=X"), "error should name the symbol, got: {msg}");
    assert!(msg.contains("2024-03-31"), "error should name the date, got: {msg}");
    assert!(msg.contains("30"), "error should name the bound, got: {msg}");
}

#[tokio::test]
async fn get_rate_short_circuits_the_same_currency() {
    // Review Focus 5, and the only existing get_rate behaviour the suite pins
    // (currency_service.rs:700-716): same currency is exactly one.
    let pool = pool_with_prices().await;
    let svc = CurrencyService::new();
    let got = svc.get_rate("USD", "USD", ts("2020-01-01")).await.unwrap();
    assert_eq!(got.rate, Decimal::ONE);
    assert_eq!(got.date_used, NaiveDate::parse_from_str("2020-01-01", "%Y-%m-%d").unwrap());
    let _ = &pool;
}
```

`pool_with_prices` needs a `StartupConfig` that does not require OIDC. `tests/common/mod.rs:15` already has `disabled_config()`; it is a test-module helper, not exported from the library, so use it by copying the pattern: replace the body of `pool_with_prices` with the same two calls `setup_db` makes —

```rust
async fn pool_with_prices() -> SqlitePool {
    let pool = SqlitePool::connect(":memory:").await.unwrap();
    let cfg = StartupConfig::from_env_with(&|key| (key == "AUTH_MODE").then(|| "disabled".to_string()))
        .expect("AUTH_MODE=disabled parses in a debug build");
    backend_rust::init_db(&pool, &cfg).await.unwrap();
    pool
}
```

Those are the exact two calls `tests/common/mod.rs:71-76` (`setup_db`) and `:15-20` (`disabled_config`) make together; `StartupConfig::from_env_with` is the real constructor and there is no `disabled()` or `default_for_test()` on it. If this file declares `mod common;` (Task 4 adds it), replace the whole body with `common::setup_db(&common::disabled_config()).await` instead of duplicating.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --test fx_pricing_test`
Expected: FAIL — `no method 'get_rate'` mismatch or `type ... has no member 'rate'`, because `get_rate` still returns a bare `Decimal`.

- [ ] **Step 3: Change the cache to a per-pair dated series**

`CurrencyService`'s cache field (`:125`) becomes:

```rust
pub struct CurrencyService {
    client: Client,
    timeout: Duration,
    cache: Arc<RwLock<HashMap<(String, String), FxSeries>>>,
    last_request: Mutex<Option<Instant>>,
    eodhd_api_key: Option<String>,
}

/// One fetched window of a pair's dated closes. Keyed by pair, not by pair-and-date:
/// the old key (`(from, to, date)`) meant every distinct transaction date was its own
/// outbound call, and the value was never the date it claimed.
///
/// The cache is process-local and dies with the process, so it needs no key version:
/// entries written by the old buggy code cannot outlive a deploy. The persistent store
/// is `historical_prices`, and it is versioned by symbology — `EURUSD=X` rows cannot
/// have been written by the `EUR=USD` code path.
struct FxSeries {
    /// First date of the fetched window. A request earlier than this needs a new
    /// fetch rather than a wrong answer from a window that never covered it.
    window_start: NaiveDate,
    series: Vec<(NaiveDate, Decimal)>,
}

/// Bound on cached pairs. Unbounded is not an option in a 256Mi pod once past dates
/// became reachable; 64 pairs is far beyond any portfolio in this app, and the
/// eviction is a full clear rather than an LRU because a miss is one cheap call.
const MAX_FX_SERIES_CACHED: usize = 64;
```

`CurrencyService::new()` (`:131-143`) initialises `cache: Arc::new(RwLock::new(HashMap::new()))` unchanged — the type changed, the expression did not.

- [ ] **Step 4: Rewrite `get_rate`**

Replace `get_rate` (`:226-262`) with:

```rust
    /// The FX rate to convert `from_curr` into `to_curr` for `date`: the last close
    /// on or before that date, within `FX_LOOKBACK_DAYS`, together with the date
    /// actually used.
    ///
    /// Read order is database first, provider second. That is what makes a past-dated
    /// conversion testable without a network, and it is the same order
    /// `get_historical_prices` is moving to.
    pub async fn get_rate(
        &self,
        from_curr: &str,
        to_curr: &str,
        date: DateTime<Utc>,
    ) -> Result<RateAt> {
        let date_naive = date.date_naive();

        if from_curr == to_curr {
            return Ok(RateAt { rate: Decimal::ONE, date_used: date_naive });
        }

        let key = (from_curr.to_uppercase(), to_curr.to_uppercase());

        {
            let cache = self.cache.read().await;
            if let Some(fx) = cache.get(&key) {
                if date_naive >= fx.window_start {
                    if let Some((date_used, rate)) = select_rate(&fx.series, date_naive, FX_LOOKBACK_DAYS) {
                        return Ok(RateAt { rate, date_used });
                    }
                }
            }
        }

        let symbol = fx_symbol(from_curr, to_curr);
        let window_start = date_naive - chrono::Duration::days(FX_SERIES_WINDOW_DAYS);

        // Database first: a seeded or previously fetched window answers with no
        // outbound call at all.
        if let Some(hit) = self.rate_from_db(&symbol, window_start, date_naive).await? {
            return Ok(hit);
        }

        // get_historical_prices already builds the range URL, parses the chart
        // response, persists the series through the upsert, and merges database rows.
        // Reusing it is why this change is small.
        let series = self
            .get_historical_prices(&symbol, window_start, date_naive, pool)
            .await?;

        {
            let mut cache = self.cache.write().await;
            if cache.len() >= MAX_FX_SERIES_CACHED && !cache.contains_key(&key) {
                cache.clear();
            }
            cache.insert(
                key,
                FxSeries { window_start: series.first().map(|(d, _)| *d).unwrap_or(window_start), series: series.clone() },
            );
        }

        select_rate(&series, date_naive, FX_LOOKBACK_DAYS)
            .map(|(date_used, rate)| RateAt { rate, date_used })
            .ok_or_else(|| {
                anyhow!(
                    "No {} close on or before {} within {} days",
                    symbol,
                    date_naive,
                    FX_LOOKBACK_DAYS
                )
            })
    }
```

**Two things this sketch needs settled, not improvised.** First, `get_rate` has no `pool` parameter today and `CurrencyService::new()` takes no arguments at 24 call sites, so the pool cannot ride on the service without touching all 24. The two options are: add a `pool: &SqlitePool` parameter to `get_rate` and update its three call sites (`tax_engine.rs:403`, `tax_engine.rs:551`, `stats_engine.rs:158`) — each of which can reach a pool — or store an `Option<SqlitePool>` on the service. **Choose the parameter.** The signature is:

```rust
    pub async fn get_rate(
        &self,
        from_curr: &str,
        to_curr: &str,
        date: DateTime<Utc>,
        pool: &SqlitePool,
    ) -> Result<RateAt>
```

and the `get_historical_prices` call passes `pool`.

Second, add the helper the sketch calls, next to `get_rate`:

```rust
    async fn rate_from_db(
        &self,
        symbol: &str,
        window_start: NaiveDate,
        date: NaiveDate,
        pool: &SqlitePool,
    ) -> Result<Option<RateAt>> {
        let rows: Vec<(NaiveDate, String)> = sqlx::query_as(
            "SELECT date, close_price FROM historical_prices
                 WHERE symbol = ? AND date >= ? AND date <= ? ORDER BY date",
        )
        .bind(symbol)
        .bind(window_start)
        .bind(date)
        .fetch_all(pool)
        .await?;

        let series: Vec<(NaiveDate, Decimal)> = rows
            .into_iter()
            .map(|(d, p)| (d, crate::db_types::str_to_decimal(&p)))
            .collect();

        Ok(select_rate(&series, date, FX_LOOKBACK_DAYS).map(|(date_used, rate)| RateAt { rate, date_used }))
    }
```

and the public result type, above the `impl` block:

```rust
/// A rate and the date that produced it. The date is not decoration: a report that
/// says "as of 2024-03-31" has to be able to state that a Friday close priced it.
#[derive(Debug, Clone, PartialEq)]
pub struct RateAt {
    pub rate: Decimal,
    pub date_used: NaiveDate,
}

/// The error for "this pair has no close near that date", built separately from the
/// `ok_or_else` above so the message is testable without a provider. Public so the
/// integration test can assert on it; `select_rate` stays `pub(crate)`.
pub fn no_rate_error(symbol: &str, date: NaiveDate, lookback_days: i64) -> anyhow::Error {
    anyhow!(
        "No {} close on or before {} within {} days",
        symbol,
        date,
        lookback_days
    )
}
```

and the `ok_or_else` in `get_rate` becomes `.ok_or_else(|| no_rate_error(&symbol, date_naive, FX_LOOKBACK_DAYS))`.

- [ ] **Step 5: Update every caller**

`replay` (`tax_engine.rs:390-397`) gains `pool: &SqlitePool` as its second parameter, after `sorted_txs`. Its body at `:403` becomes:

```rust
            let tx_rate = if asset_currency != base_currency {
                currency_service
                    .get_rate(asset_currency, base_currency, tx.date, pool)
                    .await?
                    .rate
            } else {
                Decimal::ONE
            };
```

`calculate_lots` (`:536-546`) gains `pool: &SqlitePool` as its final parameter. Its own FX call at `:551` becomes:

```rust
            let rate = currency_service
                .get_rate(asset_currency, base_currency, latest_date, pool)
                .await?
                .rate;
```

and its `Self::replay(...)` call at `:557` passes `pool`. `sell_breakdown` (`:619-627`) gains `pool: &SqlitePool` as its final parameter and passes it at `:629`.

Production callers, both in `backend_rust/src/api_routes/analytics.rs` — `:75` (`calculate_lots`, inside the per-asset loop, where `pool.inner()` is already what `:73` uses) and `:239` (`sell_breakdown`). Both already hold a pool; pass `pool.inner()`.

`backend_rust/src/engines/stats_engine.rs:155-160`, keeping today's permissiveness exactly:

```rust
                    let rate = currency_service
                        .get_rate(&asset.currency, base_currency, date_utc, pool)
                        .await
                        .map(|r| r.rate)
                        .unwrap_or(Decimal::ONE);
```

`backend_rust/src/services/currency_service.rs:700-716`, the three same-currency unit tests, gain a pool and `.rate`:

```rust
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query("CREATE TABLE historical_prices (symbol TEXT NOT NULL, date DATE NOT NULL, close_price TEXT NOT NULL)")
            .execute(&pool).await.unwrap();
        let result = svc.get_rate("USD", "USD", date, &pool).await;
```

and the two `GBP` assertions become `svc.get_rate("GBP", "GBP", date1, &pool).await.unwrap().rate`.

The in-file engine tests in `tax_engine.rs` (`run_engine` at `:29`, the `sell_breakdown` helper at `:208`, and the direct calls at `:342` and `:348`) all pass `"USD"`/`"USD"`, which short-circuits before any query, so a bare `&SqlitePool::connect(":memory:").await.unwrap()` at each call site is sufficient and correct — but say so in a one-line comment, because a reader will otherwise look for seeded prices that are not there.

- [ ] **Step 6: Run the tests**

Run: `cargo test --test fx_pricing_test && cargo test --lib currency_service`
Expected: PASS. `get_rate_returns_the_seeded_past_dated_rate` and `get_rate_does_not_return_todays_rate_for_a_past_date` pass with no network because the seeded window answers from the database, and `a_rate_beyond_the_lookback_names_the_symbol_and_the_date` is a pure message test.

- [ ] **Step 7: Run the whole suite and record what moved**

Run: `cargo test`
Expected: `sell_pnl_test.rs` still passes — every fixture there is USD/USD, which short-circuits, and `sell_pnl_test.rs:67` asserts `fx_applied == false`. If any test moves, that is a real finding: write down which figure, and which fixture made it multi-currency, before continuing.

- [ ] **Step 8: Commit**

```bash
git add backend_rust/src/services/currency_service.rs backend_rust/src/engines/tax_engine.rs backend_rust/src/engines/stats_engine.rs backend_rust/tests/fx_pricing_test.rs
git commit -m "fix(pricing): date FX properly and resolve pairs at all

P1. get_rate built EUR=USD, which Yahoo answers with No data found, and then
returned the latest close in the series regardless of the date argument, so the
cached value looked correctly dated and was not. It now builds EURUSD=X, reads
the database before the provider, caches a dated series per pair rather than a
value per pair-and-date, and returns the date it used.

Figures moved: every foreign-currency realized and unrealized number, on
tax-summary and performance. No existing test fixture is multi-currency, so the
suite does not show the movement; the release note must. stats_engine keeps its
unwrap_or(ONE) permissiveness deliberately."
```

---

### Task 4: The hermetic past-dated multi-currency conversion test

**Files:**
- Modify: `backend_rust/tests/common/mod.rs` (add `seed_asset_currency`; `seed_asset` at `:125-138` hardcodes `'USD'`)
- Test: `backend_rust/tests/fx_pricing_test.rs` (extend)

**Interfaces:**
- Consumes: `get_rate` returning `RateAt`, the `EURUSD=X` symbology, and the unique index, all from Tasks 1-3.
- Produces: `pub async fn seed_asset_currency(pool: &SqlitePool, portfolio_id: i32, symbol: &str, name: &str, currency: &str) -> i32` in the shared test module, and the pattern for seeding a throttled `split_sync` row so a route test stays offline.

- [ ] **Step 1: Add the multi-currency seeding helper**

In `backend_rust/tests/common/mod.rs`, after `seed_asset` (`:125-138`), add. `seed_asset` stays as-is — eleven existing tests call it and its hardcoded `'USD'` is what keeps them single-currency:

```rust
/// `seed_asset` hardcodes `'USD'`, which is why no fixture in the suite can tell a
/// correct dated-FX implementation from the broken one. Multi-currency tests use
/// this instead.
pub async fn seed_asset_currency(
    pool: &SqlitePool,
    portfolio_id: i32,
    symbol: &str,
    name: &str,
    currency: &str,
) -> i32 {
    let (id,): (i32,) = sqlx::query_as(
        "INSERT INTO assets (portfolio_id, symbol, name, asset_type, currency) VALUES (?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(portfolio_id)
    .bind(symbol)
    .bind(name)
    .bind("STOCK")
    .bind(currency)
    .fetch_one(pool)
    .await
    .unwrap();
    id
}
```

Match the exact column list and `asset_type` value used by `seed_asset` immediately above it; copy from there rather than trusting this listing.

- [ ] **Step 2: Write the failing end-to-end test**

Append to `backend_rust/tests/fx_pricing_test.rs`:

```rust
/// Goal 5: a past-dated multi-currency conversion asserted through the engine, with
/// every price and rate seeded. This is the test that did not exist, and the reason
/// the rest of this document is verifiable.
#[tokio::test]
async fn engine_converts_a_past_dated_sell_at_the_dated_rate() {
    use backend_rust::engines::tax_engine::TaxLotEngine;

    let pool = pool_with_prices().await;

    // Portfolio base EUR, asset priced in USD: the conversion is USD -> EUR.
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "FX", "EUR").await;
    let asset_id = seed_asset_currency(&pool, port_id, "AAPL", "Apple", "USD").await;
    seed_transaction_at(&pool, asset_id, "BUY", 10.0, 100.0, 0.0, "2024-01-02T00:00:00Z").await;
    seed_transaction_at(&pool, asset_id, "SELL", 10.0, 130.0, 0.0, "2024-03-08T00:00:00Z").await;

    // USD -> EUR closes: 0.8700 on the buy date, 0.9125 on the sell date.
    seed_pair(&pool, "USDEUR=X", &[("2024-01-02", "0.8700"), ("2024-03-08", "0.9125")]).await;
    // Today's close for market value, so the unrealized leg does not reach the network.
    let today = chrono::Utc::now().date_naive();
    seed_pair(&pool, "AAPL", &[(today.format("%Y-%m-%d").to_string().as_str(), "130.00")]).await;

    let txs = sqlx::query_as::<_, backend_rust::models::Transaction>(
        "SELECT * FROM transactions WHERE asset_id = ?",
    )
    .bind(asset_id)
    .fetch_all(&pool)
    .await
    .unwrap();

    let svc = CurrencyService::new();
    let summary = TaxLotEngine::calculate_lots(
        "AAPL",
        "STOCK",
        &txs,
        dec("130.00"),
        "USD",
        "EUR",
        &svc,
        "FIFO",
        30,
        &pool,
    )
    .await
    .unwrap();

    // Proceeds 1300 USD at 0.9125 = 1186.25 EUR; cost 1000 USD at 0.8700 = 870.00 EUR.
    // Realized = 316.25 EUR. Under the old code both legs used one undated rate and
    // the difference between the two dates never existed.
    assert_eq!(summary.realized_pnl, dec("316.25"));
}
```

Add to the file's imports: `mod common;` plus `use common::{DEFAULT_USER, seed_portfolio, seed_transaction_at, seed_asset_currency};` — the same shape `sell_pnl_test.rs:1-3` uses. Confirm `calculate_lots`'s real parameter list in `tax_engine.rs:536-546` before writing the call; the order above mirrors `analytics.rs:75-84` plus the `pool` parameter Task 3 added.

- [ ] **Step 3: Run it and check the arithmetic is the point**

Run: `cargo test --test fx_pricing_test`
Expected: PASS with `316.25`. If it fails on scale (`316.2500`), the comparison is already value-based via `dec()`; if it fails on value, print `summary.realized_pnl` and check which rate each leg used before changing the expectation — the expectation is derived by hand above, not guessed from output.

- [ ] **Step 4: Prove no network was touched**

Run: `cargo test --test fx_pricing_test` with the network disabled, e.g.

```bash
env -u http_proxy -u https_proxy HTTP_PROXY= HTTPS_PROXY= NO_PROXY='*' cargo test --test fx_pricing_test
```

Expected: PASS. A test that silently reaches Yahoo would fail or hang here; this is the check that Goal 5 is real rather than asserted.

- [ ] **Step 5: Commit**

```bash
git add backend_rust/tests/common/mod.rs backend_rust/tests/fx_pricing_test.rs
git commit -m "test(pricing): hermetic past-dated multi-currency conversion

Goal 5. Every fixture in the suite was single-currency - seed_asset hardcodes
USD and the portfolio base to the same string - so nothing could distinguish a
correct dated-FX implementation from the broken one. Adds seed_asset_currency
and an engine-level assertion that a January buy and a March sell convert at
their own dates' rates.

Figures moved: none."
```

---

### Task 5: Typed pricing results and a stated read order (P4)

**Files:**
- Modify: `backend_rust/src/services/currency_service.rs` — `get_price` (`:267-300`), `get_historical_prices` (`:407-520`) and its five swallow sites (`:429`, `:442`, `:449`, `:467`, `:502`)
- Modify: `backend_rust/src/api_routes/analytics.rs:73`
- Modify: `backend_rust/src/engines/stats_engine.rs:138-147` and `:176-186`
- Test: `backend_rust/tests/analytics_test.rs` (gapped seed for the non-trading-day case)

**Interfaces:**
- Consumes: Task 1's uniqueness, Task 3's database-first pattern.
- Produces:

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum PricingError {
    NoData,
    ProviderUnavailable(String),
}
pub async fn get_price(&self, symbol: &str, isin: Option<&str>, pool: &SqlitePool) -> Result<Decimal, PricingError>;
```

- [ ] **Step 1: Write the failing no-data test**

Append to `backend_rust/tests/fx_pricing_test.rs`:

```rust
#[tokio::test]
async fn get_price_answers_from_the_database_without_the_provider() {
    // The hermetic half of Review Focus 4: a seeded row answers with no outbound
    // call, which is what the previous provider-first order could not promise.
    let pool = pool_with_prices().await;
    let today = chrono::Utc::now().date_naive();
    seed_pair(&pool, "SEEDED", &[(today.format("%Y-%m-%d").to_string().as_str(), "42.0")]).await;
    let svc = CurrencyService::new();
    assert_eq!(svc.get_price("SEEDED", None, &pool).await.unwrap(), dec("42.0"));
}

#[test]
fn the_two_pricing_failures_are_distinguishable() {
    // Review Focus 1 and 4 at the type level. The end-to-end version of this needs
    // a mock provider, and this crate has no dev-dependencies to build one with; see
    // Out of Scope. What is pinned here is that the two failures are different values
    // with different messages, so a caller can branch on them and a user can be told
    // something different — which is the whole point of the change.
    assert_ne!(PricingError::NoData, PricingError::ProviderUnavailable("timeout".into()));
    assert_eq!(PricingError::NoData.to_string(), "no price data available");
    assert!(PricingError::ProviderUnavailable("timeout".into())
        .to_string()
        .contains("timeout"));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --test fx_pricing_test get_price`
Expected: FAIL — `get_price` returns `Decimal`, which has no `unwrap`, and `PricingError` does not exist yet. Add `PricingError` to the file's `use backend_rust::services::currency_service::{CurrencyService, PricingError};` line when you write the tests.

- [ ] **Step 3: Add the error type and change `get_price`**

In `currency_service.rs`, next to `RateAt`:

```rust
/// "There is no close for this symbol" and "we could not ask" are different failures
/// with different user actions. `get_price` returned `Decimal::ZERO` for both
/// (`currency_service.rs:267`), and zero is a plausible price.
#[derive(Debug, Clone, PartialEq)]
pub enum PricingError {
    NoData,
    ProviderUnavailable(String),
}

impl std::fmt::Display for PricingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PricingError::NoData => write!(f, "no price data available"),
            PricingError::ProviderUnavailable(reason) => write!(f, "price provider unavailable: {reason}"),
        }
    }
}
```

`get_price` keeps its database-first shape and changes only its returns. Replace the trailing `Decimal::ZERO` (`:299`) with `Err(PricingError::NoData)`, each successful return with `Ok(price)`, and the `fetch_yahoo_price` result check with a provider-unreachable case:

```rust
    pub async fn get_price(
        &self,
        symbol: &str,
        isin: Option<&str>,
        pool: &SqlitePool,
    ) -> Result<Decimal, PricingError> {
        let today = chrono::Utc::now().date_naive();
        let existing = sqlx::query_as::<_, HistoricalPrice>(
            "SELECT * FROM historical_prices WHERE symbol = ? AND date = ?",
        )
        .bind(symbol)
        .bind(today)
        .fetch_optional(pool)
        .await
        .map_err(|e| PricingError::ProviderUnavailable(e.to_string()))?;

        if let Some(hp) = existing {
            return Ok(crate::db_types::str_to_decimal(&hp.close_price));
        }

        let price = self.fetch_yahoo_price(symbol).await;
        if price > Decimal::ZERO {
            self.cache_price(symbol, today, price, pool).await;
            return Ok(price);
        }

        if let Some(isin) = isin {
            if !isin.is_empty() {
                if let Some(eodhd_key) = &self.eodhd_api_key {
                    let price = self.fetch_eodhd_price(isin, eodhd_key).await;
                    if price > Decimal::ZERO {
                        self.cache_price(symbol, today, price, pool).await;
                        return Ok(price);
                    }
                }
            }
        }

        Err(PricingError::NoData)
    }
```

Note what changes here too: `fetch_yahoo_price` (`:302`) returns a bare `Decimal` and yields zero on failure, so a provider outage would otherwise land on `NoData` rather than `ProviderUnavailable`. It has exactly one caller — `get_price` at `:281`, verified with `grep -rn "fetch_yahoo_price" src/` — so **change it in this task** to return `Result<Decimal, PricingError>`, mapping a request error to `ProviderUnavailable(e.to_string())` and an absent or non-positive close to `NoData`, and replace the `price > Decimal::ZERO` test with `Ok(price)`. That is what makes the two error cases real rather than nominal; leaving it would ship an enum whose second variant is only ever produced by a database error.

- [ ] **Step 4: Stop swallowing in `get_historical_prices`, and state the read order**

The five `return self.get_historical_prices_from_db(...)` fallbacks (`:429`, `:442`, `:449`, `:467`, `:502`) are the accident that `tests/analytics_test.rs:169-171` depends on. Replace the network-first-then-fallback shape with database-first, provider-on-miss:

```rust
    /// Daily closes for `symbol` in `[start_date, end_date]`.
    ///
    /// Read order, stated rather than inherited: **the database answers if it holds
    /// any row in the window**; the provider is asked only when it holds none. The
    /// previous order was provider-first with a database fallback on every failure,
    /// which is the only reason the existing hermetic pricing test worked
    /// (`tests/analytics_test.rs:169-171`). With the fallback gone, a seeded window
    /// must answer on its own.
    ///
    /// `Err(ProviderUnavailable)` means we could not ask. `Err(NoData)` means we asked
    /// and there is nothing.
    pub async fn get_historical_prices(
        &self,
        symbol: &str,
        start_date: NaiveDate,
        end_date: NaiveDate,
        pool: &SqlitePool,
    ) -> Result<Vec<(NaiveDate, Decimal)>, PricingError> {
        let db_prices = self
            .get_historical_prices_from_db(symbol, start_date, end_date, pool)
            .await
            .map_err(|e| PricingError::ProviderUnavailable(e.to_string()))?;
        if !db_prices.is_empty() {
            return Ok(db_prices);
        }

        // ... existing range fetch, unchanged, but each former fallback becomes:
        //   fetch error            -> Err(PricingError::ProviderUnavailable(e.to_string()))
        //   non-success status     -> Err(PricingError::ProviderUnavailable(format!("status {status}")))
        //   parse failure          -> Err(PricingError::ProviderUnavailable(e.to_string()))
        //   no chart result        -> Err(PricingError::NoData)
        //   empty daily_prices     -> Err(PricingError::NoData)
        // and the final merge keeps returning Ok(merged) with the rows it persisted.
    }
```

The comment block marks the five sites and what each becomes; the surrounding fetch and parse code is already correct and stays. `get_historical_prices_from_db` keeps returning `Result<Vec<…>>` with the anyhow error, converted here.

- [ ] **Step 5: Update the two consumers without moving a number**

`backend_rust/src/api_routes/analytics.rs:73` — today a failed price is `Decimal::ZERO` and the endpoint still returns 200. Preserve that exactly:

```rust
        let current_price = match currency_service
            .get_price(&asset.symbol, asset.isin.as_deref(), pool.inner())
            .await
        {
            Ok(price) => price,
            Err(e) => {
                eprintln!("WARN: no price for {}: {}", asset.symbol, e);
                Decimal::ZERO
            }
        };
```

`backend_rust/src/engines/stats_engine.rs:138-147` — the `match` on `get_historical_prices` keeps its shape; only the error binding type changes, so the existing `Err(e) => { eprintln!(…); }` arm is still correct. `:176-186` — the `get_price` call inside the `if !has_prices` block:

```rust
                let current_price = match currency_service.get_price(symbol, isin, pool).await {
                    Ok(price) => price,
                    Err(e) => {
                        eprintln!("WARN: no price for {}: {}", symbol, e);
                        Decimal::ZERO
                    }
                };
                if current_price > Decimal::ZERO {
```

which is byte-for-byte today's behaviour: a failure contributes nothing.

- [ ] **Step 6: Add the gapped seed and the non-trading-day assertion**

`tests/analytics_test.rs:169-183` seeds **every** date, so it has no calendar gaps and cannot exercise a non-trading day (Review Focus 2). In that seeding loop, skip Saturdays and Sundays:

```rust
    let mut curr = start_date;
    while curr <= today {
        // Skip weekends deliberately: seeding every date leaves no calendar gap, so
        // the non-trading-day path is never exercised. With the provider fallback gone
        // this seed is now the only thing keeping the test offline.
        let weekday = curr.weekday();
        if weekday != chrono::Weekday::Sat && weekday != chrono::Weekday::Sun {
            sqlx::query(
                "INSERT INTO historical_prices (symbol, date, close_price) VALUES (?, ?, ?)
                     ON CONFLICT(symbol, date) DO UPDATE SET close_price = excluded.close_price",
            )
            .bind(symbol)
            .bind(curr)
            .bind("50.0")
            .execute(&pool)
            .await
            .unwrap();
        }
        match curr.succ_opt() {
            Some(next) => curr = next,
            None => break,
        }
    }
```

Then assert the weekend case directly, in the same file:

```rust
#[tokio::test]
async fn performance_resolves_a_weekend_from_the_prior_weekday_close() {
    // Review Focus 2 through the route rather than the helper: the seeded series has
    // no weekend rows, so a weekend in the window must be answered by Friday's close
    // rather than a hole or a network call.
    let (port_id, pool, today) = seeded_performance_fixture().await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();
    let resp = client
        .get(format!("/api/portfolios/{}/performance", port_id))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Ok);
    let body: serde_json::Value = resp.into_json().await.unwrap();
    assert!(
        !body["history"].as_array().unwrap().is_empty(),
        "performance history must be populated from the gapped seed"
    );
    let _ = today;
}
```

`seeded_performance_fixture()` is the existing setup inside `analytics_test.rs` around `:120-190`; extract it into a helper function rather than duplicating the seeding, and have the existing test call it too.

- [ ] **Step 7: Run everything**

Run: `cargo test`
Expected: PASS. Then `cargo clippy --all-targets` (not in CI, run locally) and `bash scripts/check_access_gate.sh` — unchanged by this work, run to prove it.

- [ ] **Step 8: Commit**

```bash
git add backend_rust/src/services/currency_service.rs backend_rust/src/api_routes/analytics.rs backend_rust/src/engines/stats_engine.rs backend_rust/tests/analytics_test.rs backend_rust/tests/fx_pricing_test.rs
git commit -m "fix(pricing): distinguish no data from provider unavailable

P4. get_price returned Decimal::ZERO for both, and get_historical_prices
swallowed its error at five sites and handed back the database fallback as Ok -
which is the only reason the existing hermetic pricing test worked. Read order is
now stated: the database answers when it holds the window, the provider is asked
only on a miss.

Figures moved: none on the undated endpoints. analytics.rs and stats_engine both
treat a pricing failure exactly as before (zero, contribute nothing); the type
change is so the dated report can fail loudly. The analytics seed now skips
weekends so the non-trading-day path is actually exercised."
```

---

### Task 6: Correct the conversion footnote

**Files:**
- Modify: `frontend/src/components/SellPnlModal.tsx:283`
- Test: `frontend/src/components/__tests__/SellPnlModal.test.tsx` (extend the assertion at `:59`)

**Interfaces:**
- Consumes: Task 3's guarantee that the rate used is the one on or before the sell date.
- Produces: nothing consumed by other tasks.

- [ ] **Step 1: Write the failing assertion**

`SellPnlModal.test.tsx:59` currently asserts only `toHaveTextContent('converted to EUR')`, which is true of both the old and the new sentence. Tighten it:

```tsx
    expect(
      screen.getByTestId('fx-note').textContent
    ).toMatch(/rate recorded on or before the sell date/);
```

If the element at `SellPnlModal.tsx:283` has no `data-testid`, add `data-testid="fx-note"` to it in Step 3 — the modal already uses `data-testid` for `strategy-note` at `:209`, so this follows the existing pattern rather than inventing one.

- [ ] **Step 2: Run it to verify it fails**

Run from `frontend/`: `npx vitest run src/components/__tests__/SellPnlModal.test.tsx`
Expected: FAIL — the rendered text still says "at the latest available rate".

- [ ] **Step 3: Correct the sentence**

`frontend/src/components/SellPnlModal.tsx:283` currently reads:

```tsx
                  Cost basis and proceeds were converted to {data.currency} at the latest available {data.asset_currency}→{data.currency} rate.
```

Replace with:

```tsx
                  <span data-testid="fx-note">Cost basis and proceeds were converted to {data.currency} at the {data.asset_currency}→{data.currency} rate recorded on or before the sell date.</span>
```

"Latest available" became false in Task 3 and was false before it too, since the old code returned the latest close while labelling it with the requested date.

- [ ] **Step 4: Run the frontend checks**

Run from `frontend/`: `npx vitest run src/components/__tests__/SellPnlModal.test.tsx && npm run lint && npm run typecheck`
Expected: PASS. If `node_modules` is absent in the working environment, run `npm ci` first and say so in the report — do not skip the check.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/SellPnlModal.tsx frontend/src/components/__tests__/SellPnlModal.test.tsx
git commit -m "fix(frontend): the FX footnote no longer claims the latest rate

The modal said the conversion used the latest available rate. After the pricing
fix it uses the rate recorded on or before the sell date, and before it the code
returned the latest close while labelling it with the requested date, so the
sentence was never true.

Figures moved: none."
```

---

## Final Gate

- [ ] `cargo test` from `backend_rust/` — all green, including `price_uniqueness_test` and `fx_pricing_test`.
- [ ] `bash scripts/check_access_gate.sh` from `backend_rust/` — unchanged, and proven unchanged.
- [ ] `cargo clippy --all-targets` from `backend_rust/` — clean. **CI does not run clippy**, so this only happens if someone runs it.
- [ ] `cargo build --release` from `backend_rust/`.
- [ ] `npm run lint`, `npm run typecheck`, `npm test` from `frontend/`.
- [ ] No test reaches the network: re-run `cargo test` with proxies nulled and the suite still passes.
- [ ] Each of the four figure-moving commits names its delta in the message, and the release note for the branch lists: every foreign-currency realized and unrealized figure changes; the close used for a duplicated `(symbol, date)` is the most recently written one.

## Out of Scope for This Plan

Named so nobody assumes they are handled:

- **`SQLITE_BUSY` on the write path.** No WAL, no `busy_timeout`, and the dedupe plus the upsert both write. Unchanged here; the tax-report design must not assume writes are safe.
- **Split sync running inside read paths**, and the non-transactional count-then-insert behind it. Issue #50.
- **The `split_sync` throttle keyed per symbol**, so one asset's sync can suppress another's for thirty days. Issue #38.
- **Integration tests that make live Yahoo calls.** Issue #37. This plan adds hermetic tests; it does not convert the existing offenders.
- **Whether `stats_engine` should stop defaulting a failed rate to one.** Left permissive deliberately; making it honest moves the performance endpoint's numbers and is a separate decision.
- **An end-to-end test of a provider outage.** Distinguishing "provider down" from "no data" is tested at the type and message level, not through a real failing fetch, because `backend_rust/Cargo.toml` has no dev-dependencies and this plan adds none. The first person to add a mock HTTP server to this crate should replace those two tests with a fetch-level assertion.
