//! Pricing tests that must not reach the internet.
//!
//! Three of these lived in `src/engines/stats_engine.rs` as unit tests and reached
//! `query1.finance.yahoo.com` on every run. They asked for symbols Yahoo does not
//! know and treated the resulting 404 as "the provider has no data"; one of them,
//! `a_provider_outage_prices_from_the_stored_window_instead_of_zero`, proved a
//! provider outage with a real outbound request. They were integration tests by
//! behaviour — a database and a provider — so they live here now, beside the mock,
//! and the outage is a local 404 rather than whatever the network happens to do.
//!
//! Bodies are verbatim from the unit tests. Only the service construction changed,
//! from `CurrencyService::new()` to one pointed at [`MockYahoo`].

mod common;

use backend_rust::engines::stats_engine::StatsEngine;
use backend_rust::models::{Asset, Transaction};
use backend_rust::services::currency_service::{CurrencyService, PRICE_WINDOW_FRESH_DAYS};
use chrono::{DateTime, Utc};
use common::MockYahoo;
use rust_decimal::Decimal;
use sqlx::SqlitePool;
use std::str::FromStr;

/// A `historical_prices` table with nothing in it — what each moved test assumed
/// when it was a unit test.
async fn empty_price_table() -> SqlitePool {
    let pool = SqlitePool::connect(":memory:").await.unwrap();
    sqlx::query("CREATE TABLE IF NOT EXISTS historical_prices (symbol TEXT, date DATE, close_price TEXT)")
        .execute(&pool)
        .await
        .unwrap();
    pool
}

/// One close, in Yahoo's chart envelope. The service takes the last non-null entry
/// of `indicators.quote[0].close`.
const MOCKED_CHART: &str = r#"{"chart":{"result":[{"timestamp":[],"indicators":{"quote":[{"close":[42.50]}]}}],"error":null}}"#;

#[tokio::test]
async fn a_mocked_close_prices_the_portfolio_so_the_success_path_is_hermetic_too() {
    // The 404 direction proves the seam fails honestly. This proves it succeeds
    // honestly: a close served locally has to survive URL construction, the
    // request, and response parsing to land in the portfolio value. That is the
    // half of the pricing path the outage tests do not cover, and the half
    // v0.0.6-M5's release notes had to describe as untested.
    let pool = empty_price_table().await;
    let yahoo = MockYahoo::serving([("MOCKED", MOCKED_CHART)]);
    let svc = CurrencyService::with_base_url(yahoo.base_url());

    let asset = Asset {
        id: 1,
        portfolio_id: 1,
        symbol: "MOCKED".to_string(),
        name: "Mocked".to_string(),
        asset_type: "STOCK".to_string(),
        sector: None,
        currency: "USD".to_string(),
        isin: None,
    };
    let buy = Transaction {
        id: 1,
        asset_id: 1,
        r#type: "BUY".to_string(),
        quantity: "10.0".to_string(),
        price: "100.0".to_string(),
        fee: "0.0".to_string(),
        date: DateTime::parse_from_rfc3339("2024-06-01T00:00:00Z").unwrap().with_timezone(&Utc),
    };

    let result = StatsEngine::calculate_portfolio_performance(&pool, &[asset], &[buy], "USD", &svc)
        .await
        .unwrap();
    let value = result.get("metrics").unwrap().get("portfolio_value").unwrap()
        .as_str().unwrap();

    assert_eq!(
        Decimal::from_str(value).unwrap(),
        Decimal::from_str("425.00").unwrap(),
        "10 shares at the served close of 42.50; anything else means the mock was bypassed"
    );
    assert!(
        yahoo.paths().iter().any(|p| p.contains("/v8/finance/chart/MOCKED")),
        "the request must actually have been made, not short-circuited"
    );
}

#[tokio::test]
async fn test_multicurrency_asset_with_conversion() {
    let pool = empty_price_table().await;
    let yahoo = MockYahoo::start();
    let svc = CurrencyService::with_base_url(yahoo.base_url());

    let asset = Asset {
        id: 1,
        portfolio_id: 1,
        symbol: "NESF".to_string(),
        name: "Nestle".to_string(),
        asset_type: "STOCK".to_string(),
        sector: None,
        currency: "CHF".to_string(),
        isin: None,
    };
    let tx = Transaction {
        id: 1,
        asset_id: 1,
        r#type: "BUY".to_string(),
        quantity: "10.0".to_string(),
        price: "100.0".to_string(),
        fee: "0.0".to_string(),
        date: DateTime::parse_from_rfc3339("2024-06-01T00:00:00Z").unwrap().with_timezone(&Utc),
    };

    let result = StatsEngine::calculate_portfolio_performance(
        &pool,
        &[asset],
        &[tx],
        "USD",
        &svc,
    ).await.unwrap();

    let metrics = result.get("metrics").unwrap();
    let value = metrics.get("portfolio_value").unwrap().as_str().unwrap();
    assert_eq!(value, "0");
}
#[tokio::test]
async fn test_fallback_to_current_price_when_no_historical_data() {
    let pool = empty_price_table().await;
    let yahoo = MockYahoo::start();
    let svc = CurrencyService::with_base_url(yahoo.base_url());

    let asset = Asset {
        id: 1,
        portfolio_id: 1,
        symbol: "TESTFALLBACK".to_string(),
        name: "Test Fallback".to_string(),
        asset_type: "STOCK".to_string(),
        sector: None,
        currency: "USD".to_string(),
        isin: None,
    };
    let tx = Transaction {
        id: 1,
        asset_id: 1,
        r#type: "BUY".to_string(),
        quantity: "10.0".to_string(),
        price: "50.0".to_string(),
        fee: "0.0".to_string(),
        date: DateTime::parse_from_rfc3339("2024-06-01T00:00:00Z").unwrap().with_timezone(&Utc),
    };

    let result = StatsEngine::calculate_portfolio_performance(
        &pool,
        &[asset],
        &[tx],
        "USD",
        &svc,
    ).await.unwrap();

    let metrics = result.get("metrics").unwrap();
    let value = metrics.get("portfolio_value").unwrap().as_str().unwrap();
    assert_eq!(value, "0");
}
#[tokio::test]
async fn a_provider_outage_prices_from_the_stored_window_instead_of_zero() {
    // I3. `get_historical_prices` now returns `Err` where it used to hand back the
    // database rows, and an outage must not price the portfolio at zero: the
    // pre-branch code served the stale rows, and the honest error has to be
    // handled here, in the consumer, rather than by making the return type lie
    // again.
    //
    // The stored window stops one day past the freshness bound, so the provider is
    // asked for the tail; the symbol is one the provider cannot serve, so the
    // outage is real whichever way the request fails. What the fix has to produce
    // is the stored closes in the series instead of nothing at all.
    let pool = empty_price_table().await;

    let today = Utc::now().date_naive();
    let symbol = "NOSUCHSYMBOL-OUTAGE";
    let newest = today
        - chrono::Duration::days(PRICE_WINDOW_FRESH_DAYS + 1);
    let mut date = newest - chrono::Duration::days(4);
    while date <= newest {
        sqlx::query("INSERT INTO historical_prices (symbol, date, close_price) VALUES (?, ?, ?)")
            .bind(symbol)
            .bind(date)
            .bind("3.00")
            .execute(&pool)
            .await
            .unwrap();
        date = date.succ_opt().unwrap();
    }

    let asset = Asset {
        id: 1,
        portfolio_id: 1,
        symbol: symbol.to_string(),
        name: "Outage".to_string(),
        asset_type: "STOCK".to_string(),
        sector: None,
        currency: "USD".to_string(),
        isin: None,
    };
    let buy = Transaction {
        id: 1,
        asset_id: 1,
        r#type: "BUY".to_string(),
        quantity: "100.0".to_string(),
        price: "100.0".to_string(),
        fee: "0.0".to_string(),
        date: (today - chrono::Duration::days(30)).and_hms_opt(0, 0, 0).unwrap().and_utc(),
    };

    let yahoo = MockYahoo::start();
    let svc = CurrencyService::with_base_url(yahoo.base_url());
    let result = StatsEngine::calculate_portfolio_performance(
        &pool,
        &[asset],
        &[buy],
        "USD",
        &svc,
    )
    .await
    .unwrap();

    // 100 shares x 3.00 = 300 on every date the stored window covers. Without the
    // fallback the window is discarded, `get_price` fails for the same reason, and
    // every point in the series is 0.
    let history = result.get("history").unwrap().as_array().unwrap();
    let values: Vec<Decimal> = history
        .iter()
        .filter_map(|entry| {
            entry.get("value").and_then(|v| v.as_str()).and_then(|s| Decimal::from_str(s).ok())
        })
        .collect();
    let stale = Decimal::from_str("300").unwrap();
    assert!(
        values.contains(&stale),
        "the stored closes must still price the series during an outage; got {values:?}"
    );
    assert_eq!(
        values.iter().rev().find(|v| **v > Decimal::ZERO),
        Some(&stale),
        "the last priced point must be the stored close, not zero; got {values:?}"
    );
}
