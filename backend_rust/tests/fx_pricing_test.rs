//! The FX rate contract: a rate is dated, the pair symbology is Yahoo's, and a
//! missing rate is an error rather than a silent zero.
//!
//! These tests are hermetic — no network, ever. The seeded FX rows are what make
//! that possible (`EURUSD=X` for the rate tests, `USDEUR=X` for the engine test):
//! the new `get_rate` consults the database before the provider, so a seeded window
//! answers with no outbound call at all.

mod common;

use common::{offline_pricing_service, seed_asset_currency, seed_portfolio, seed_transaction_at, DEFAULT_USER};
use backend_rust::auth::StartupConfig;
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use sqlx::sqlite::SqlitePool;
use std::str::FromStr;

use backend_rust::services::currency_service::{no_rate_error, PricingError};

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
    let cfg = StartupConfig::from_env_with(&|key| (key == "AUTH_MODE").then(|| "disabled".to_string()))
        .expect("AUTH_MODE=disabled parses in a debug build");
    backend_rust::init_db(&pool, &cfg).await.unwrap();
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

    let svc = offline_pricing_service();
    let got = svc.get_rate("EUR", "USD", ts("2024-03-09"), &pool).await.unwrap();

    assert_eq!(got.rate, dec("0.9125"));
    assert_eq!(got.date_used, NaiveDate::parse_from_str("2024-03-08", "%Y-%m-%d").unwrap());
}

#[tokio::test]
async fn get_rate_does_not_return_todays_rate_for_a_past_date() {
    // The bug this task exists to fix: the old get_rate keyed its cache on the
    // requested date and then returned the last close in the series, so the stored
    // value looked correctly dated and was not.
    //
    // The stored close sits three days before the requested date, not twenty-three:
    // a window whose newest row stops short of the date is incomplete coverage, so
    // `get_rate` sends it to the provider rather than serving a close from inside the
    // lookback (see `rate_from_db`). Three days is inside `PRICE_WINDOW_FRESH_DAYS`,
    // so this window is honest and answers hermetically, and the claim is unchanged:
    // the latest close in the table must not price a past date.
    let pool = pool_with_prices().await;
    seed_pair(
        &pool,
        "EURUSD=X",
        &[("2024-03-28", "0.9125"), ("2026-11-09", "1.1000")],
    )
    .await;

    let svc = offline_pricing_service();
    let got = svc.get_rate("EUR", "USD", ts("2024-03-31"), &pool).await.unwrap();

    assert_eq!(got.rate, dec("0.9125"), "a past date must not pick up the latest close");
    assert_eq!(got.date_used, NaiveDate::parse_from_str("2024-03-28", "%Y-%m-%d").unwrap());
}

#[tokio::test]
async fn no_rate_error_names_symbol_date_and_bound() {
    // Review Focus 1, tested at the message rather than the fetch. The gap case
    // cannot be produced hermetically at this level: the fetch window (365 days) is
    // wider than the lookback (30), so any row the lookback would reject is also
    // outside the window, which means a database miss and a real provider call. The
    // rule itself is pinned hermetically in Task 2 by
    // select_rate_rejects_a_gap_wider_than_the_lookback; what is pinned here is that
    // the error a user sees carries enough to act on.
    let err = no_rate_error("EURUSD=X", NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(), 30);
    let msg = err.to_string();
    assert!(msg.contains("EURUSD=X"), "error should name the symbol, got: {msg}");
    assert!(msg.contains("2024-03-31"), "error should name the date, got: {msg}");
    assert!(msg.contains("30"), "error should name the bound, got: {msg}");
}

/// Goal 5: a past-dated multi-currency conversion asserted through the engine, with
/// every price and rate seeded. The portfolio base currency and the asset currency
/// differ here, which no fixture in the suite did before: nothing single-currency
/// can distinguish a conversion priced at each transaction's own date from one
/// priced at a single undated rate.
#[tokio::test]
async fn engine_converts_a_past_dated_sell_at_the_dated_rate() {
    use backend_rust::engines::tax_engine::TaxLotEngine;

    let pool = pool_with_prices().await;

    // Portfolio base EUR, asset priced in USD: the conversion is USD -> EUR.
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "FX", "EUR").await;
    let asset_id = seed_asset_currency(&pool, port_id, "AAPL", "Apple", "USD").await;
    seed_transaction_at(&pool, asset_id, "BUY", 10.0, 100.0, 0.0, "2024-01-02T00:00:00Z").await;
    seed_transaction_at(&pool, asset_id, "SELL", 10.0, 130.0, 0.0, "2024-03-08T00:00:00Z").await;

    let today = Utc::now().date_naive();
    let today_str = today.format("%Y-%m-%d").to_string();
    // USD -> EUR closes: 0.8700 on the buy date, 0.9125 on the sell date, and today's
    // rate for the market-value leg, which marks a current price and therefore
    // converts at the valuation date. Without the last row this test would ask the
    // live provider for `USDEUR=X` and stop being hermetic.
    seed_pair(
        &pool,
        "USDEUR=X",
        &[("2024-01-02", "0.8700"), ("2024-03-08", "0.9125"), (&today_str, "1.0000")],
    )
    .await;
    // The asset's own close, as the route-level caller would read it before calling
    // the engine. `calculate_lots` takes the current price as an argument, so this
    // row completes the fixture rather than being read by the assertion below.
    seed_pair(&pool, "AAPL", &[(today_str.as_str(), "130.00")]).await;

    let txs = sqlx::query_as::<_, backend_rust::models::Transaction>(
        "SELECT * FROM transactions WHERE asset_id = ?",
    )
    .bind(asset_id)
    .fetch_all(&pool)
    .await
    .unwrap();

    let svc = offline_pricing_service();
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

#[tokio::test]
async fn market_value_is_converted_at_the_valuation_date_not_the_last_transaction_date() {
    // The market-value leg of `calculate_lots` marks the position at *today's* price,
    // so it has to convert at today's rate. Converting it at the rate recorded on the
    // latest transaction date mixes a current price with a stale exchange rate — and
    // after `get_rate` became honest about dates, that is exactly what it did.
    //
    // The two rates are deliberately far apart, and the seeded pair is the only thing
    // that keeps this hermetic: the stale one sits on the latest transaction date, the
    // fresh one on today.
    use backend_rust::engines::tax_engine::TaxLotEngine;

    let pool = pool_with_prices().await;

    let port_id = seed_portfolio(&pool, DEFAULT_USER, "FX", "EUR").await;
    let asset_id = seed_asset_currency(&pool, port_id, "AAPL", "Apple", "USD").await;
    seed_transaction_at(&pool, asset_id, "BUY", 10.0, 100.0, 0.0, "2024-01-02T00:00:00Z").await;
    seed_transaction_at(&pool, asset_id, "BUY", 10.0, 110.0, 0.0, "2024-03-08T00:00:00Z").await;

    let today = Utc::now().date_naive();
    let today_str = today.format("%Y-%m-%d").to_string();
    seed_pair(
        &pool,
        "USDEUR=X",
        &[
            ("2024-01-02", "0.8700"),
            // The rate at the latest transaction date: 20 shares at 130 would mark at
            // 2372.50 EUR if the market-value leg used it.
            ("2024-03-08", "0.9125"),
            // Today's rate: the correct one for a current price. Not 1.0000 on
            // purpose — at parity an unconverted leg produces the same number.
            // 20 shares x 130 USD x 0.9500 = 2470.00 EUR.
            (&today_str, "0.9500"),
        ],
    )
    .await;

    let txs = sqlx::query_as::<_, backend_rust::models::Transaction>(
        "SELECT * FROM transactions WHERE asset_id = ?",
    )
    .bind(asset_id)
    .fetch_all(&pool)
    .await
    .unwrap();

    let svc = offline_pricing_service();
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

    // 20 shares x 130 USD x 0.9500 = 2470 EUR. At the 2024-03-08 rate it is 2372.50;
    // unconverted it is 2600.
    assert_eq!(
        summary.market_value,
        dec("2470"),
        "the market-value leg must convert today's price at today's rate"
    );
    assert_eq!(
        summary.current_price,
        dec("123.5"),
        "the quoted current price converts at today's rate"
    );
}

#[tokio::test]
async fn get_rate_short_circuits_the_same_currency() {
    // Review Focus 5, and the only existing get_rate behaviour the suite pins
    // (currency_service.rs same-currency tests): same currency is exactly one.
    let pool = pool_with_prices().await;
    let svc = offline_pricing_service();
    let got = svc.get_rate("USD", "USD", ts("2020-01-01"), &pool).await.unwrap();
    assert_eq!(got.rate, Decimal::ONE);
    assert_eq!(got.date_used, NaiveDate::parse_from_str("2020-01-01", "%Y-%m-%d").unwrap());
}

#[tokio::test]
async fn get_rate_short_circuits_a_case_difference() {
    // The cache key and the FX symbology both uppercase, so a case difference is the
    // same currency. Compared raw, "eur" -> "EUR" asked the provider for EUREUR=X.
    let pool = pool_with_prices().await;
    let svc = offline_pricing_service();
    let got = svc.get_rate("eur", "EUR", ts("2020-01-01"), &pool).await.unwrap();
    assert_eq!(got.rate, Decimal::ONE);
}

#[tokio::test]
async fn get_price_answers_from_the_database_without_the_provider() {
    // What is pinned here is existing behaviour now carried by the new `Result`:
    // `get_price` already read today's row before this change, so the point is that a
    // seeded row answers with no outbound call and without an error. The endpoint
    // whose order really changed was `get_historical_prices` — provider-first with a
    // database fallback — and that half is pinned by
    // performance_resolves_a_weekend_from_the_prior_weekday_close in
    // tests/analytics_test.rs and by the freshness tests in the service's own unit
    // tests.
    let pool = pool_with_prices().await;
    let today = chrono::Utc::now().date_naive();
    seed_pair(&pool, "SEEDED", &[(today.format("%Y-%m-%d").to_string().as_str(), "42.0")]).await;
    let svc = offline_pricing_service();
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
