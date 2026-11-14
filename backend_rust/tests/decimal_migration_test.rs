//! Money and quantity are stored as text and come back exactly.
//!
//! This file used to test `migrate_decimal_columns`, which rewrote SQLite's `REAL`
//! columns to `TEXT` and had four recovery paths for a rename interrupted halfway.
//! Postgres has no such history: `0001_init.sql` declares the columns `text` from the
//! start, and `scripts/check_migrations.sh` fails the build if any of them is not
//! `text` — so there is nothing left here to migrate and nothing left to recover.
//!
//! What the migration was *for* is still worth asserting, and is the reason the columns
//! are text at all: a binary float cannot hold 9.99, and a portfolio whose cost basis
//! drifts by a rounding unit per row is wrong on tax figures. So this file now checks
//! the invariant directly against the real schema — write a value the API accepts, read
//! it back through the same decoder the routes use, and require it to be identical.

use backend_rust::db::DbPool;
use backend_rust::db_types::str_to_decimal;
use rust_decimal::Decimal;
use std::str::FromStr;

mod common;
use common::{db_with_user, seed_asset, seed_portfolio};

/// The values that break a float: not representable in binary, and each one a real
/// price or fee a broker would send.
const EXACT_VALUES: &[&str] = &[
    "9.99", "0.1", "0.3", "0.000000000000000001", "1234567890.123456789", "100.0",
];

async fn stored(pool: &DbPool, user: i64, column: &str, value: &str) -> String {
    let portfolio = seed_portfolio(pool, user, "Exact", "USD").await;
    let asset = seed_asset(pool, portfolio, "EXACT", "Exact").await;

    // `seed_transaction` takes floats, which is the very thing under test, so the row
    // is written by hand with the text the column actually holds.
    let (got,): (String,) = sqlx::query_as(&format!(
        "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) \
         VALUES ($1, 'BUY', $2, '1', '0', '2024-01-01T00:00:00Z') RETURNING {column}",
    ))
    .bind(asset)
    .bind(value)
    .fetch_one(pool)
    .await
    .unwrap();
    got
}

#[tokio::test]
async fn a_value_written_as_text_is_read_back_identically() {
    let (pool, user) = db_with_user("a_value_written_as_text_is_read_back_identically").await;

    for value in EXACT_VALUES {
        let raw = stored(&pool, user, "quantity", value).await;
        assert_eq!(
            &raw, value,
            "{value} must come back from a text column as the same string"
        );
        assert_eq!(
            str_to_decimal(&raw),
            Decimal::from_str(value).unwrap(),
            "{value} must decode to the decimal it was written as"
        );
    }
}

#[tokio::test]
async fn the_same_holds_for_price_and_fee() {
    // Three separate columns, three separate chances for a schema change to quietly
    // make one of them a numeric type again.
    let (pool, user) = db_with_user("the_same_holds_for_price_and_fee").await;
    let portfolio = seed_portfolio(&pool, user, "Exact", "USD").await;
    let asset = seed_asset(&pool, portfolio, "EXACT", "Exact").await;

    let (qty, price, fee): (String, String, String) = sqlx::query_as(
        "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) \
         VALUES ($1, 'BUY', '3.14159', '9.99', '0.0075', '2024-01-01T00:00:00Z') \
         RETURNING quantity, price, fee",
    )
    .bind(asset)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(str_to_decimal(&qty), Decimal::from_str("3.14159").unwrap());
    assert_eq!(str_to_decimal(&price), Decimal::from_str("9.99").unwrap());
    assert_eq!(str_to_decimal(&fee), Decimal::from_str("0.0075").unwrap());
}
