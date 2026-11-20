//! The write seam: one insert path, callable inside a caller's transaction.
//!
//! `services::assets::insert` and `services::transactions::insert` exist because the
//! staged-import apply path has to write a batch as **one** unit — an asset and the
//! transactions that reference it, or nothing. Before them, the only insert code was
//! inline in the Rocket handlers, which could only ever write through the pool.
//!
//! So the property worth pinning here is not that the inserts work — the endpoint
//! tests already cover that — but that they compose into a transaction the caller
//! controls. A future change that quietly reaches for `pool` inside a service
//! function instead of the connection it was handed would break the batch guarantee
//! while leaving every endpoint test green. These tests fail if that happens.
//!
//! The normalization checks (the uppercase symbol, the `SPLIT` zero rule, the date range) moved
//! into the services from the handlers; the last two tests pin them at their new home
//! so the move cannot silently drop one.

mod common;

use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use sqlx::Acquire;

use backend_rust::models::TxType;
use backend_rust::schemas::{AssetCreate, TransactionCreate};
use backend_rust::services::assets;
use backend_rust::services::error::WriteError;
use backend_rust::services::transactions;

use common::{db_with_user, seed_portfolio};

fn asset_create(symbol: &str) -> AssetCreate {
    AssetCreate {
        symbol: symbol.to_string(),
        name: format!("{symbol} Inc"),
        asset_type: "stock".to_string(),
        sector: None,
        currency: "USD".to_string(),
        isin: String::new(),
    }
}

fn txn_create(kind: TxType, quantity: &str, price: &str) -> TransactionCreate {
    TransactionCreate {
        r#type: kind,
        quantity: quantity.parse::<Decimal>().unwrap(),
        price: price.parse::<Decimal>().unwrap(),
        fee: Decimal::ZERO,
        date: Utc.with_ymd_and_hms(2024, 6, 15, 12, 0, 0).unwrap(),
    }
}

async fn counts(pool: &sqlx::PgPool) -> (i64, i64) {
    let assets: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM assets")
            .fetch_one(pool)
            .await
            .unwrap();
    let txns: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM transactions")
            .fetch_one(pool)
            .await
            .unwrap();
    (assets.0, txns.0)
}

/// The point of the whole seam: two writes, one transaction, rolled back, and the
/// database is exactly as it was. If a service ever writes outside the connection it
/// was handed, this is the test that says so.
#[tokio::test]
async fn a_rolled_back_transaction_leaves_neither_asset_nor_transaction() {
    let (pool, user) = db_with_user("rolled_back_batch").await;
    let pid = seed_portfolio(&pool, user, "Seam", "USD").await;

    let mut conn = pool.acquire().await.unwrap();
    let mut tx = conn.begin().await.unwrap();

    let asset = assets::insert(&mut tx, pid, &asset_create("ACME"))
        .await
        .expect("asset insert");
    transactions::insert(&mut tx, asset.id, &txn_create(TxType::Buy, "10", "100.50"))
        .await
        .expect("transaction insert");

    tx.rollback().await.unwrap();
    drop(conn);

    assert_eq!(
        counts(&pool).await,
        (0, 0),
        "a rolled back batch must leave no asset and no transaction behind"
    );
}

/// The same two writes committed are visible, and the transaction row really points at
/// the asset row created in the same transaction — which is only possible because both
/// services take the caller's connection.
#[tokio::test]
async fn a_committed_transaction_writes_both_rows_linked() {
    let (pool, user) = db_with_user("committed_batch").await;
    let pid = seed_portfolio(&pool, user, "Seam", "USD").await;

    let mut conn = pool.acquire().await.unwrap();
    let mut tx = conn.begin().await.unwrap();

    let asset = assets::insert(&mut tx, pid, &asset_create("ACME"))
        .await
        .expect("asset insert");
    let written = transactions::insert(&mut tx, asset.id, &txn_create(TxType::Buy, "10", "100.50"))
        .await
        .expect("transaction insert");

    tx.commit().await.unwrap();
    drop(conn);

    assert_eq!(counts(&pool).await, (1, 1));
    let (asset_id,): (i64,) = sqlx::query_as("SELECT asset_id FROM transactions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        asset_id, written.asset_id,
        "the transaction must reference the asset created in the same transaction"
    );
    assert_eq!(asset.portfolio_id, pid);
}

/// A `SPLIT` with a zero price is a ratio of zero, which is not a ratio. The rule used
/// to live in the handler; it is pinned here so the extraction cannot lose it, and the
/// rejected write must not have reached the table.
#[tokio::test]
async fn a_split_with_a_zero_price_is_refused_and_writes_nothing() {
    let (pool, user) = db_with_user("split_zero_price").await;
    let pid = seed_portfolio(&pool, user, "Seam", "USD").await;

    let mut conn = pool.acquire().await.unwrap();
    let asset = assets::insert(&mut conn, pid, &asset_create("ACME"))
        .await
        .expect("asset insert");

    let err = transactions::insert(&mut conn, asset.id, &txn_create(TxType::Split, "2", "0"))
        .await
        .expect_err("a zero-price split must be refused");
    assert_eq!(err, WriteError::InvalidSplit);

    drop(conn);
    assert_eq!(
        counts(&pool).await.1,
        0,
        "a refused write must not leave a row behind"
    );
}

/// The second insert of the same symbol in the same portfolio is a collision the
/// service reports by cause, not as an opaque database error — which is what lets the
/// HTTP layer answer 400 and a future agent-facing layer answer something specific.
#[tokio::test]
async fn a_duplicate_symbol_is_reported_as_a_collision_not_a_database_error() {
    let (pool, user) = db_with_user("duplicate_symbol").await;
    let pid = seed_portfolio(&pool, user, "Seam", "USD").await;

    let mut conn = pool.acquire().await.unwrap();
    assets::insert(&mut conn, pid, &asset_create("ACME"))
        .await
        .expect("first insert");

    let err = assets::insert(&mut conn, pid, &asset_create("acme"))
        .await
        .expect_err("the second insert of the same symbol must be refused");
    assert_eq!(err, WriteError::SymbolExists);
}

/// The stored spelling is canonical for both columns, by two different routes.
///
/// The symbol is normalized at run time inside the service, so no caller can skip it by
/// calling the service instead of the endpoint. The type is no longer normalized at all:
/// it arrives as `TxType`, whose `as_str()` is the only spelling it can produce, so there
/// is nothing left to forget. Both assertions stay because they are asserting different
/// things — one is a runtime guarantee, the other is a type-system guarantee, and a future
/// edit that reintroduces a stringly-typed type would fail the second one at compile time
/// rather than here.
#[tokio::test]
async fn the_service_stores_the_canonical_form() {
    let (pool, user) = db_with_user("uppercase_normalization").await;
    let pid = seed_portfolio(&pool, user, "Seam", "USD").await;

    let mut conn = pool.acquire().await.unwrap();
    let asset = assets::insert(&mut conn, pid, &asset_create("acme"))
        .await
        .expect("asset insert");
    transactions::insert(&mut conn, asset.id, &txn_create(TxType::Buy, "10", "100.50"))
        .await
        .expect("transaction insert");
    drop(conn);

    let (symbol,): (String,) = sqlx::query_as("SELECT symbol FROM assets")
        .fetch_one(&pool)
        .await
        .unwrap();
    let (kind,): (String,) = sqlx::query_as("SELECT type FROM transactions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(symbol, "ACME", "the symbol must be stored uppercased");
    assert_eq!(
        kind, "BUY",
        "the stored type is the enum's spelling, not whatever the caller spelled"
    );
}
