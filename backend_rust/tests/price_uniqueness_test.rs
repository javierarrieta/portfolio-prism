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

    // Three distinct pairs survive the six seeded rows: AAPL/2024-01-02 (x2),
    // AAPL/2024-01-03, MSFT/2024-01-02 (x3).
    assert_eq!(row_count(&pool).await, 3);
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
    assert_eq!(row_count(&pool).await, 3);

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
