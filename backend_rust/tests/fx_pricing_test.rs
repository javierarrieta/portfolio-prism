//! The FX rate contract: a rate is dated, the pair symbology is Yahoo's, and a
//! missing rate is an error rather than a silent zero.
//!
//! These tests are hermetic — no network, ever. The seeded `EURUSD=X` rows are
//! what make that possible: the new `get_rate` consults the database before the
//! provider, so a seeded window answers with no outbound call at all.

use backend_rust::auth::StartupConfig;
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

    let svc = CurrencyService::new();
    let got = svc.get_rate("EUR", "USD", ts("2024-03-09"), &pool).await.unwrap();

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
    let got = svc.get_rate("EUR", "USD", ts("2024-03-31"), &pool).await.unwrap();

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
    let err = no_rate_error("EURUSD=X", NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(), 30);
    let msg = err.to_string();
    assert!(msg.contains("EURUSD=X"), "error should name the symbol, got: {msg}");
    assert!(msg.contains("2024-03-31"), "error should name the date, got: {msg}");
    assert!(msg.contains("30"), "error should name the bound, got: {msg}");
}

#[tokio::test]
async fn get_rate_short_circuits_the_same_currency() {
    // Review Focus 5, and the only existing get_rate behaviour the suite pins
    // (currency_service.rs same-currency tests): same currency is exactly one.
    let pool = pool_with_prices().await;
    let svc = CurrencyService::new();
    let got = svc.get_rate("USD", "USD", ts("2020-01-01"), &pool).await.unwrap();
    assert_eq!(got.rate, Decimal::ONE);
    assert_eq!(got.date_used, NaiveDate::parse_from_str("2020-01-01", "%Y-%m-%d").unwrap());
}
