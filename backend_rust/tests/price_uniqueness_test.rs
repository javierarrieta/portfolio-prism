use std::str::FromStr;

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
    // The plain index `init_db` used to create before the unique index superseded it.
    // Without it the "the old index is gone" assertion below is true whether or not
    // the drop exists, which is the whole assertion's point.
    sqlx::query("CREATE INDEX idx_historical_prices_symbol_date ON historical_prices(symbol, date)")
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

/// The shape of the owner's real `portfolio.db` before this change, run through the
/// whole of `init_db` rather than through `enforce_price_uniqueness` on its own.
///
/// Three things only meet here: the legacy REAL `close_price` (pre-decimal), duplicate
/// `(symbol, date)` rows, and the ordering constraint that `migrate_decimal_columns`
/// renames `close_price` on a dedicated connection under `PRAGMA legacy_alter_table`
/// *before* the dedupe runs. The rename needs that connection because of a stale
/// per-connection schema cache that only misbehaves on a file-backed database, which
/// is why this test cannot be `:memory:` either.
#[tokio::test]
async fn init_db_boots_a_legacy_database_that_already_holds_duplicates() {
    let tmp = std::env::temp_dir().join(format!(
        "portfolio_price_uniqueness_test_{}.db",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&tmp);
    let pool = sqlx::sqlite::SqlitePool::connect(&format!("sqlite:{}?mode=rwc", tmp.display()))
        .await
        .unwrap();

    sqlx::query(
        "CREATE TABLE historical_prices (
            symbol TEXT NOT NULL,
            date DATE NOT NULL,
            close_price REAL NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .unwrap();
    // The plain index `init_db` used to create, so the drop is exercised by the real
    // boot sequence and not only by the fixture in this file.
    sqlx::query("CREATE INDEX idx_historical_prices_symbol_date ON historical_prices(symbol, date)")
        .execute(&pool)
        .await
        .unwrap();
    for (symbol, date, price) in [
        ("AAPL", "2024-01-02", 100.0_f64),
        ("AAPL", "2024-01-02", 101.5), // duplicate, written later, must survive
        ("AAPL", "2024-01-03", 102.0),
        ("MSFT", "2024-01-02", 300.0),
        ("MSFT", "2024-01-02", 301.0),
        ("MSFT", "2024-01-02", 302.0), // duplicate, written latest, must survive
    ] {
        sqlx::query("INSERT INTO historical_prices (symbol, date, close_price) VALUES (?, ?, ?)")
            .bind(symbol)
            .bind(date)
            .bind(price)
            .execute(&pool)
            .await
            .unwrap();
    }

    let cfg = backend_rust::auth::StartupConfig::from_env_with(&|key| {
        (key == "AUTH_MODE").then(|| "disabled".to_string())
    })
    .expect("AUTH_MODE=disabled parses in a debug build");

    // The spec's mitigation for "a bad dedupe bricks boot": a duplicate the dedupe
    // misses makes the unique index fail, `init_db` returns `Err`, and the app
    // refuses to start. Boot succeeding is the assertion, not a precondition.
    backend_rust::init_db(&pool, &cfg)
        .await
        .expect("init_db must boot a legacy database that already holds duplicates");

    let groups: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT symbol, date(date), COUNT(*) FROM historical_prices GROUP BY symbol, date",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(groups.len(), 3, "three distinct (symbol, date) pairs were seeded");
    assert!(
        groups.iter().all(|(_, _, n)| *n == 1),
        "exactly one row per (symbol, date) must survive boot; got {groups:?}"
    );

    // The `MAX(rowid)` survivor, readable as a decimal after the column became TEXT.
    let (aapl,): (String,) = sqlx::query_as(
        "SELECT close_price FROM historical_prices WHERE symbol = 'AAPL' AND date(date) = '2024-01-02'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        backend_rust::db_types::str_to_decimal(&aapl),
        rust_decimal::Decimal::from_str("101.5").unwrap()
    );
    let affinity: Vec<(i64, String, String, i64, Option<String>, i64)> =
        sqlx::query_as("PRAGMA table_info(historical_prices)")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        affinity.iter().find(|r| r.1 == "close_price").unwrap().2,
        "TEXT",
        "the decimal migration and the dedupe must both have run, in that order"
    );

    let indexes: Vec<(i64, String, i64)> = sqlx::query_as("PRAGMA index_list(historical_prices)")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(
        indexes.iter().any(|(_, name, unique)| name == "uq_historical_prices_symbol_date" && *unique == 1),
        "the unique index must exist after a real boot; have {:?}",
        indexes.iter().map(|(_, n, _)| n).collect::<Vec<_>>()
    );
    assert!(
        !indexes.iter().any(|(_, name, _)| name == "idx_historical_prices_symbol_date"),
        "the redundant plain index must be gone after a real boot"
    );

    // A second boot on the migrated database is a no-op, not an error.
    backend_rust::init_db(&pool, &cfg)
        .await
        .expect("a second init_db on the migrated database must succeed");
    let n: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM historical_prices")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n.0, 3);

    pool.close().await;
    let _ = std::fs::remove_file(&tmp);
}
