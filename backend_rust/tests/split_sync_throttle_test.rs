//! The split-sync throttle is keyed per asset (#38).
//!
//! Keyed on the symbol, the throttle let the first asset holding a symbol suppress
//! every other asset holding it for 30 days — while which splits actually get
//! inserted is decided by *the calling asset's* earliest transaction date. So the
//! first asset could sync successfully, insert nothing relevant because the split
//! predated its own first trade, record the throttle anyway, and leave a second
//! portfolio priced pre-split on a screen whose entire purpose is tax figures.
//!
//! All hermetic: the provider stand-in serves a canned split event, so the corporate
//! action is real to the code and costs no network.

mod common;

use backend_rust::models::Asset;
use backend_rust::migrate_split_sync_key;
use backend_rust::services::currency_service::CurrencyService;
use common::{
    build_rocket_pricing, disabled_config, seed_asset, seed_portfolio, seed_split_sync,
    seed_transaction_at, setup_db, MockYahoo, DEFAULT_USER,
};
use rocket::http::{ContentType, Status};
use rocket::local::asynchronous::Client;
use sqlx::SqlitePool;

/// The corporate action: a 2:1 split on 2024-06-01, in Yahoo's chart envelope.
/// `1717200000` is 2024-06-01T00:00:00Z.
const SPLIT_ME: &str = r#"{"chart":{"result":[{"timestamp":[],"indicators":{"quote":[{"close":[]}]},"events":{"splits":{"1717200000":{"date":1717200000,"numerator":2.0,"denominator":1.0,"splitRatio":"2:1"}}}}],"error":null}}"#;

const SYMBOL: &str = "SPLITME";
async fn split_rows(pool: &SqlitePool, asset_id: i32) -> Vec<String> {
    sqlx::query_as::<_, (String,)>(
        "SELECT date(date) FROM transactions WHERE asset_id = ? AND type = 'SPLIT' ORDER BY date(date)",
    )
    .bind(asset_id)
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|(d,)| d)
    .collect()
}

fn asset(id: i32, portfolio_id: i32) -> Asset {
    Asset {
        id,
        portfolio_id,
        symbol: SYMBOL.to_string(),
        name: "Split me".to_string(),
        asset_type: "STOCK".to_string(),
        sector: None,
        currency: "USD".to_string(),
        isin: None,
    }
}

#[tokio::test]
async fn two_portfolios_holding_the_same_symbol_each_get_their_own_split() {
    // The common case, and the one the symbol key broke outright. Two portfolios both
    // holding the same symbol, both bought before a split. `apply_split_events`
    // inserts rows for the *calling* asset only, so a symbol-level throttle lets the
    // first portfolio's sync mark the symbol done and the second portfolio never
    // receives its own split rows — wrong cost basis, silently, for 30 days.
    //
    // Note this is ordinary usage, not a corner: the duplicate-asset guard is scoped
    // per portfolio, so "US Large Cap" and "Retirement" both holding AAPL is normal.
    let pool = setup_db(&disabled_config()).await;

    let port_a = seed_portfolio(&pool, DEFAULT_USER, "Taxable", "USD").await;
    let asset_a = seed_asset(&pool, port_a, SYMBOL, "Split me").await;
    seed_transaction_at(&pool, asset_a, "BUY", 10.0, 60.0, 0.0, "2024-01-01T00:00:00Z").await;

    let port_b = seed_portfolio(&pool, DEFAULT_USER, "Retirement", "USD").await;
    let asset_b = seed_asset(&pool, port_b, SYMBOL, "Split me").await;
    seed_transaction_at(&pool, asset_b, "BUY", 10.0, 65.0, 0.0, "2024-02-01T00:00:00Z").await;

    let yahoo = MockYahoo::serving([(SYMBOL, SPLIT_ME)]);
    let client = Client::tracked(build_rocket_pricing(pool.clone(), DEFAULT_USER, yahoo.base_url()))
        .await
        .unwrap();

    for port in [port_a, port_b] {
        let resp = client
            .post(format!("/api/portfolios/{}/sync-splits", port))
            .header(ContentType::JSON)
            .dispatch()
            .await;
        assert_eq!(resp.status(), Status::Ok);
    }

    for (name, id) in [("A", asset_a), ("B", asset_b)] {
        assert_eq!(
            split_rows(&pool, id).await,
            vec!["2024-06-01".to_string()],
            "portfolio {name} holds the symbol and bought before the split, so it must get its own split row"
        );
    }
}

#[tokio::test]
async fn a_portfolio_that_bought_before_the_split_is_not_suppressed_by_one_that_bought_after() {
    // The issue's stated scenario, and the case that proves the coverage half of the
    // throttle is doing work rather than riding along.
    //
    // Portfolio A bought after the split, so the split correctly predates it and
    // nothing is inserted — but that sync still records a throttle row. Portfolio B
    // bought before the split and genuinely needs the row.
    let pool = setup_db(&disabled_config()).await;

    // A: first trade 2024-09-01, after the 2024-06-01 split.
    let port_a = seed_portfolio(&pool, DEFAULT_USER, "After the split", "USD").await;
    let asset_a = seed_asset(&pool, port_a, SYMBOL, "Split me").await;
    seed_transaction_at(&pool, asset_a, "BUY", 10.0, 120.0, 0.0, "2024-09-01T00:00:00Z").await;

    // B: first trade 2024-01-01, before the split.
    let port_b = seed_portfolio(&pool, DEFAULT_USER, "Before the split", "USD").await;
    let asset_b = seed_asset(&pool, port_b, SYMBOL, "Split me").await;
    seed_transaction_at(&pool, asset_b, "BUY", 10.0, 60.0, 0.0, "2024-01-01T00:00:00Z").await;

    let yahoo = MockYahoo::serving([(SYMBOL, SPLIT_ME)]);
    let client = Client::tracked(build_rocket_pricing(pool.clone(), DEFAULT_USER, yahoo.base_url()))
        .await
        .unwrap();

    // A syncs first. This is the step that used to poison the well.
    let resp = client
        .post(format!("/api/portfolios/{}/sync-splits", port_a))
        .header(ContentType::JSON)
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Ok);
    assert!(
        split_rows(&pool, asset_a).await.is_empty(),
        "the split predates A's first trade, so A must get nothing"
    );

    // B syncs. B must get the split regardless of A having synced first.
    let resp = client
        .post(format!("/api/portfolios/{}/sync-splits", port_b))
        .header(ContentType::JSON)
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Ok);
    assert_eq!(
        split_rows(&pool, asset_b).await,
        vec!["2024-06-01".to_string()],
        "B bought before the split and must receive the split row even though A synced first"
    );
}

#[tokio::test]
async fn a_transaction_older_than_the_last_sync_reopens_the_throttle() {
    // The other half of the throttle condition. A fresh 30-day window is only
    // evidence that nothing new is owed if it still covers the asset's history.
    // Record a trade older than anything the last sync saw and the splits predating
    // the old earliest are missing, so the window must not suppress the sync.
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "Late entry", "USD").await;
    let asset_id = seed_asset(&pool, port_id, SYMBOL, "Split me").await;
    seed_transaction_at(&pool, asset_id, "BUY", 10.0, 120.0, 0.0, "2024-09-01T00:00:00Z").await;

    let yahoo = MockYahoo::serving([(SYMBOL, SPLIT_ME)]);
    let svc = CurrencyService::with_base_url(yahoo.base_url());
    let a = asset(asset_id, port_id);

    assert_eq!(
        svc.sync_splits_for_asset(&a, &pool).await.unwrap(),
        0,
        "the split predates the only trade, so the first sync correctly inserts nothing"
    );

    // The user remembers an older trade and adds it.
    seed_transaction_at(&pool, asset_id, "BUY", 10.0, 60.0, 0.0, "2024-01-01T00:00:00Z").await;

    assert_eq!(
        svc.sync_splits_for_asset(&a, &pool).await.unwrap(),
        1,
        "a trade older than the last sync's coverage must reopen the throttle"
    );
    assert_eq!(split_rows(&pool, asset_id).await, vec!["2024-06-01".to_string()]);
}

#[tokio::test]
async fn a_throttle_within_the_window_and_covering_the_history_suppresses_the_fetch() {
    // The throttle still throttles. Without this the fix could pass by simply never
    // skipping, which would multiply provider calls against a shared quota.
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "Already covered", "USD").await;
    let asset_id = seed_asset(&pool, port_id, SYMBOL, "Split me").await;
    seed_transaction_at(&pool, asset_id, "BUY", 10.0, 60.0, 0.0, "2024-01-01T00:00:00Z").await;

    seed_split_sync(&pool, asset_id).await;

    let yahoo = MockYahoo::serving([(SYMBOL, SPLIT_ME)]);
    let svc = CurrencyService::with_base_url(yahoo.base_url());

    assert_eq!(
        svc.sync_splits_for_asset(&asset(asset_id, port_id), &pool).await.unwrap(),
        0,
        "a seeded throttle covering the asset's whole history must short-circuit"
    );
    assert!(
        yahoo.paths().is_empty(),
        "the throttle must return before any request is made; saw {:?}",
        yahoo.paths()
    );
}

#[tokio::test]
async fn deleting_an_asset_drops_its_throttle_row() {
    // Not a correctness requirement — `assets.id` is AUTOINCREMENT, so the id is
    // never handed to a later asset. This is what keeps the throttle table from
    // keeping one permanent row per asset the user ever deleted.
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "Deletion", "USD").await;
    let asset_id = seed_asset(&pool, port_id, SYMBOL, "Split me").await;
    seed_split_sync(&pool, asset_id).await;

    let client = Client::tracked(build_rocket_pricing(
        pool.clone(),
        DEFAULT_USER,
        common::mock_pricing_url(),
    ))
    .await
    .unwrap();
    let resp = client.delete(format!("/api/assets/{}", asset_id)).dispatch().await;
    assert_eq!(resp.status(), Status::NoContent);

    let left: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM split_sync WHERE asset_id = ?")
        .bind(asset_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left.0, 0, "the throttle row must not outlive the asset");
}

#[tokio::test]
async fn the_migration_replaces_the_symbol_keyed_table() {
    // `CREATE TABLE IF NOT EXISTS` will not alter an existing table, so an old-shaped
    // `split_sync` left in place would keep the bug while the code above it looked
    // fixed. The migration detects the old column and replaces the table.
    let pool = SqlitePool::connect(":memory:").await.unwrap();
    sqlx::query(
        "CREATE TABLE split_sync (symbol TEXT PRIMARY KEY, last_synced_at DATE NOT NULL)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO split_sync (symbol, last_synced_at) VALUES ('OLD', '2026-01-01')")
        .execute(&pool)
        .await
        .unwrap();

    migrate_split_sync_key(&pool).await.unwrap();

    let columns = sqlx::query("PRAGMA table_info(split_sync)").fetch_all(&pool).await.unwrap();
    let names: Vec<String> = columns
        .iter()
        .map(|r| sqlx::Row::try_get::<String, _>(r, "name").unwrap())
        .collect();
    assert!(names.contains(&"asset_id".to_string()), "got {names:?}");
    assert!(names.contains(&"covered_from".to_string()), "got {names:?}");
    assert!(!names.contains(&"symbol".to_string()), "got {names:?}");

    // Idempotent: running it again on the new shape is a no-op, not a drop.
    migrate_split_sync_key(&pool).await.unwrap();
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM split_sync")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count.0, 0);
}
