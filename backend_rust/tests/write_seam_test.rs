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

/// The constraint behind the check, asserted rather than assumed.
///
/// `a_duplicate_symbol_is_reported_as_a_collision_not_a_database_error` proves the service is
/// careful. It cannot prove anything is underneath it, and for the whole life of this schema
/// nothing was: the check was a `SELECT`-then-`INSERT` with a window, so two concurrent
/// writers each stored a row for one symbol and split its cost basis. This bypasses the
/// service entirely and asks Postgres, which is the only way to tell a real constraint from a
/// careful-looking convention.
///
/// The lowercase spelling is the point of the second insert. The index is on
/// `upper(symbol)`, so `acme` beside `ACME` is a violation; an index on the bare column would
/// have allowed it and looked exactly as reassuring in a `\d` listing.
#[tokio::test]
async fn the_schema_rejects_a_second_asset_for_one_symbol_in_one_portfolio() {
    let (pool, user) = db_with_user("asset_symbol_unique").await;
    let pid = seed_portfolio(&pool, user, "Seam", "USD").await;

    let mut conn = pool.acquire().await.unwrap();
    assets::insert(&mut conn, pid, &asset_create("ACME"))
        .await
        .expect("first insert");
    drop(conn);

    for spelling in ["ACME", "acme", "Acme"] {
        let dup = sqlx::query(
            "INSERT INTO assets (portfolio_id, symbol, name, asset_type, currency)
             VALUES ($1, $2, $3, 'EQUITY', 'USD')",
        )
        .bind(pid)
        .bind(spelling)
        .bind("ACME Corp")
        .execute(&pool)
        .await;
        assert!(
            dup.is_err(),
            "`{spelling}` beside the stored `ACME` must be rejected by the index, not appended"
        );
        // Vacuous-pass guard: without this, a typo in the INSERT (a NOT NULL column left out,
        // most likely) makes every iteration "fail" for a reason that has nothing to do with
        // uniqueness, and the test reports success while proving nothing.
        let e = dup.err().unwrap().to_string();
        assert!(
            e.contains("duplicate key value") && e.contains("uq_assets_portfolio_symbol"),
            "`{spelling}` was refused, but not by the index: {e}"
        );
    }

    // And the rejected writes left nothing behind.
    let (rows,): (i64,) = sqlx::query_as("SELECT count(*) FROM assets WHERE portfolio_id = $1")
        .bind(pid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 1, "one symbol, one row");
}

/// The same symbol in a different portfolio is a different asset, and must stay insertable.
///
/// The index leads with `portfolio_id` for exactly this reason. A unique index on `symbol`
/// alone would be a plausible-looking mistake — it would pass every test in this file except
/// this one, and it would make it impossible to hold the same security in two portfolios,
/// which is the ordinary case for anyone who tracks a tax-free wrapper separately.
#[tokio::test]
async fn the_same_symbol_in_two_portfolios_is_two_assets() {
    let (pool, user) = db_with_user("asset_symbol_scoped_per_portfolio").await;
    let one = seed_portfolio(&pool, user, "Taxable", "USD").await;
    let two = seed_portfolio(&pool, user, "Wrapper", "USD").await;

    let mut conn = pool.acquire().await.unwrap();
    assets::insert(&mut conn, one, &asset_create("ACME"))
        .await
        .expect("first portfolio");
    assets::insert(&mut conn, two, &asset_create("ACME"))
        .await
        .expect("the same symbol in a second portfolio is not a collision");
}

/// A symbol stored in a spelling the write path never produces is still found by the check.
///
/// This is the regression test for changing the lookup to `upper(symbol) = $2`. Such a row is
/// reachable — a hand-authored restore bundle inserts `&a.symbol` untouched, and so does any
/// psql session — and while the bare-column comparison missed it, re-importing that symbol
/// sailed past the check and hit the index as a raw database error: right answer, wrong layer,
/// and a 500 where the API has a 400 with a name.
///
/// It is also why `0006` does not normalize the column. Normalizing would have made this row
/// unreachable by detaching its price history instead.
#[tokio::test]
async fn a_lowercase_row_is_found_by_the_check_rather_than_slipping_past_it() {
    let (pool, user) = db_with_user("asset_symbol_legacy_case").await;
    let pid = seed_portfolio(&pool, user, "Seam", "USD").await;

    sqlx::query(
        "INSERT INTO assets (portfolio_id, symbol, name, asset_type, currency)
         VALUES ($1, 'acme', 'ACME Corp', 'EQUITY', 'USD')",
    )
    .bind(pid)
    .execute(&pool)
    .await
    .unwrap();

    let mut conn = pool.acquire().await.unwrap();
    let err = assets::insert(&mut conn, pid, &asset_create("ACME"))
        .await
        .expect_err("the legacy lowercase row is the same asset and must be reported as one");
    assert_eq!(
        err,
        WriteError::SymbolExists,
        "a collision the service can name, not a database error from the index"
    );
}
