//! One close per `(symbol, date)`, enforced by the schema.
//!
//! This used to be a migration test: SQLite shipped a `historical_prices` with no
//! primary key and no unique constraint, so `INSERT OR REPLACE` matched on rowid and
//! appended a second row for every fetch, and `init_db` had a dedupe step to clean up
//! the mess before adding the index. Postgres has none of that history — the table is
//! created with `PRIMARY KEY (symbol, date)` in `0001_init.sql` and there is no
//! pre-existing database to repair.
//!
//! What remains worth testing is the part the application depends on: that a duplicate
//! really is rejected rather than that the code is careful, and that the upsert the
//! pricing layer uses is an update and not an insert. The *shape* of the constraint is
//! `scripts/check_migrations.sh`'s job; this file is about behaviour.

mod common;

use common::{seed_price, setup_db, test_pool};

#[tokio::test]
async fn a_duplicate_insert_is_rejected_by_the_schema() {
    let pool = setup_db(&common::disabled_config()).await;
    seed_price(&pool, "AAPL", common::FULLY_COVERED, "100.0").await;

    let dup = sqlx::query(
        "INSERT INTO historical_prices (symbol, date, close_price) VALUES ($1, $2, $3)",
    )
    .bind("AAPL")
    .bind(common::FULLY_COVERED)
    .bind("999.0")
    .execute(&pool)
    .await;
    assert!(
        dup.is_err(),
        "a second close for the same (symbol, date) must be rejected by the primary key"
    );

    // And the row the first write put there is untouched by the rejected second one.
    let (close,): (String,) = sqlx::query_as(
        "SELECT close_price FROM historical_prices WHERE symbol = $1 AND date = $2",
    )
    .bind("AAPL")
    .bind(common::FULLY_COVERED)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(close, "100.0");
}

#[tokio::test]
async fn the_upsert_the_pricing_layer_uses_replaces_rather_than_appends() {
    // `seed_price` is the same `ON CONFLICT (symbol, date) DO UPDATE` the pricing
    // layer writes with, so this is the write path and not a paraphrase of it.
    let pool = test_pool("the_upsert_the_pricing_layer_uses").await;
    seed_price(&pool, "AAPL", common::FULLY_COVERED, "100.0").await;
    seed_price(&pool, "AAPL", common::FULLY_COVERED, "101.5").await;

    let (rows, close): (i64, String) = sqlx::query_as(
        "SELECT COUNT(*), MAX(close_price) FROM historical_prices WHERE symbol = $1 AND date = $2",
    )
    .bind("AAPL")
    .bind(common::FULLY_COVERED)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rows, 1, "an updated fetch must not leave the old one behind");
    assert_eq!(close, "101.5");
}
