mod common;

use common::{build_rocket, disabled_config, setup_db, DEFAULT_USER, seed_portfolio, seed_asset, seed_transaction, seed_transaction_at, seed_price, seed_split_sync};
use chrono::{Datelike, NaiveDate, Utc};
use rocket::http::Status;
use rocket::local::asynchronous::Client;
use sqlx::sqlite::SqlitePool;

#[tokio::test]
async fn test_tax_summary_empty_portfolio() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "Empty Tax", "USD").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get(format!("/api/portfolios/{}/tax-summary?strategy=FIFO&threshold_days=30", port_id)).dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed["assets"], serde_json::json!([]));
}

#[tokio::test]
async fn test_tax_summary_with_assets_fifo() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "FIFO Tax", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "AAPL", "Apple").await;
    seed_transaction(&pool, asset_id, "BUY", 100.0, 150.0, 0.0).await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get(format!("/api/portfolios/{}/tax-summary?strategy=FIFO&threshold_days=30", port_id)).dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed["strategy"], "FIFO");
    assert_eq!(parsed["assets"].as_array().unwrap().len(), 1);
    // Verify individual asset summary fields
    let asset = &parsed["assets"][0];
    assert_eq!(asset["symbol"], "AAPL");
    assert_eq!(asset["current_shares"], "100.0");
    // Price-derived fields are dynamic (live quote) but must serialize as strings
    assert!(asset["market_value"].is_string());
    assert!(asset["unrealized_pnl"].is_string());
    assert_eq!(asset["realized_pnl"], "0");
    assert!(asset["tax_lots"].is_array());
    // Verify tax lot data is present
    let lots = asset["tax_lots"].as_array().unwrap();
    assert_eq!(lots.len(), 1);
    let lot = &lots[0];
    assert!(lot["buy_date"].is_string());
    assert_eq!(lot["buy_price"], "150.0");
    assert_eq!(lot["original_qty"], "100.0");
    assert_eq!(lot["remaining_qty"], "100.0");
}

#[tokio::test]
async fn test_tax_summary_with_assets_lifo() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "LIFO Tax", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "GOOGL", "Google").await;
    seed_transaction(&pool, asset_id, "BUY", 50.0, 200.0, 0.0).await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get(format!("/api/portfolios/{}/tax-summary?strategy=LIFO&threshold_days=30", port_id)).dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed["strategy"], "LIFO");
    assert_eq!(parsed["assets"].as_array().unwrap().len(), 1);
    let asset = &parsed["assets"][0];
    assert_eq!(asset["symbol"], "GOOGL");
}

#[tokio::test]
async fn test_tax_summary_404() {
    let pool = setup_db(&disabled_config()).await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get("/api/portfolios/9999/tax-summary?strategy=FIFO&threshold_days=30").dispatch().await;
    assert_eq!(resp.status(), Status::NotFound);
}

#[tokio::test]
async fn test_performance_404() {
    let pool = setup_db(&disabled_config()).await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get("/api/portfolios/9999/performance").dispatch().await;
    assert_eq!(resp.status(), Status::NotFound);
}

#[tokio::test]
async fn test_performance_endpoint_fields() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "Perf Fields", "USD").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get(format!("/api/portfolios/{}/performance", port_id)).dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    // Verify metrics fields exist
    let metrics = &parsed["metrics"];
    assert!(metrics.get("volatility").is_some());
    assert!(metrics.get("sharpe_ratio").is_some());
    assert!(metrics.get("beta").is_some());
    assert!(metrics.get("portfolio_value").is_some());
    // Verify history is an array
    assert!(parsed["history"].is_array());
    // Verify correlation_matrix exists
    assert!(parsed.get("correlation_matrix").is_some());
}

#[tokio::test]
async fn test_performance_endpoint() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "Perf Test", "USD").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get(format!("/api/portfolios/{}/performance", port_id)).dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    assert!(parsed.get("metrics").is_some());
    assert!(parsed.get("history").is_some());
}

#[tokio::test]
async fn test_performance_value_reflects_split_transaction() {
    let (port_id, pool, _today) = seeded_performance_fixture().await;

    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get(format!("/api/portfolios/{}/performance", port_id)).dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    let metrics = &parsed["metrics"];
    let value = metrics.get("portfolio_value").unwrap().as_str().unwrap();
    // 100 shares before a 2:1 split become 200; the final value at the split-adjusted
    // price of 50.0 is 200 * 50 = 10000, not 100 * 50 = 5000.
    assert_eq!(value.parse::<f64>().unwrap(), 10000.0);
}

/// The shared performance fixture: a USD portfolio holding one asset that had a 2:1
/// split, a split-adjusted close of 50.0 seeded from a year before the first BUY
/// through today, and `split_sync` pinned so the route's split fetch stays offline.
///
/// The seed skips Saturdays and Sundays deliberately: seeding every date leaves no
/// calendar gap, so the non-trading-day path is never exercised. With the provider
/// fallback gone this seed is the only thing keeping these two tests offline, and the
/// gaps are what make that claim mean something. Today is the one weekend day still
/// seeded, because the endpoint reports the portfolio value as of today and the
/// fixture has to cover the date it reads.
async fn seeded_performance_fixture() -> (i32, SqlitePool, NaiveDate) {
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "Perf Split", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "SPLITINTTEST", "Split Int").await;

    let symbol = "SPLITINTTEST";
    let today = Utc::now().date_naive();
    let split_date = NaiveDate::from_ymd_opt(2024, 6, 1).unwrap();
    let buy_date = NaiveDate::from_ymd_opt(2024, 1, 10).unwrap();
    let start_date = buy_date - chrono::Duration::days(365);

    sqlx::query_as::<_, (i32,)>(
        "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) VALUES (?, 'BUY', ?, ?, '0', ?) RETURNING id"
    )
    .bind(asset_id)
    .bind("100.0")
    .bind("100.0")
    .bind(buy_date.and_hms_opt(0, 0, 0).unwrap().and_utc())
    .fetch_one(&pool)
    .await
    .unwrap();

    sqlx::query_as::<_, (i32,)>(
        "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) VALUES (?, 'SPLIT', ?, ?, '0', ?) RETURNING id"
    )
    .bind(asset_id)
    .bind("2.0")
    .bind("1.0")
    .bind(split_date.and_hms_opt(0, 0, 0).unwrap().and_utc())
    .fetch_one(&pool)
    .await
    .unwrap();

    // Seed split-adjusted (continuous) prices so the engine uses them instead of a
    // live Yahoo fetch, and throttle split sync for this symbol so no network is hit.
    let mut curr = start_date;
    while curr <= today {
        let weekday = curr.weekday();
        let is_weekend = weekday == chrono::Weekday::Sat || weekday == chrono::Weekday::Sun;
        if !is_weekend || curr == today {
            sqlx::query(
                "INSERT INTO historical_prices (symbol, date, close_price) VALUES (?, ?, ?)
                     ON CONFLICT(symbol, date) DO UPDATE SET close_price = excluded.close_price",
            )
            .bind(symbol)
            .bind(curr)
            .bind("50.0")
            .execute(&pool)
            .await
            .unwrap();
        }
        match curr.succ_opt() {
            Some(next) => curr = next,
            None => break,
        }
    }
    sqlx::query("INSERT OR REPLACE INTO split_sync (symbol, last_synced_at) VALUES (?, ?)")
        .bind(symbol)
        .bind(today)
        .execute(&pool)
        .await
        .unwrap();

    (port_id, pool, today)
}

#[tokio::test]
async fn performance_resolves_a_weekend_from_the_prior_weekday_close() {
    // Review Focus 2 through the route rather than the helper: the seeded series has
    // no weekend rows, so a weekend in the window must be answered by Friday's close
    // rather than a hole or a network call.
    let (port_id, pool, _today) = seeded_performance_fixture().await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();
    let resp = client
        .get(format!("/api/portfolios/{}/performance", port_id))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Ok);
    let body: serde_json::Value = resp.into_json().await.unwrap();
    let history = body["history"].as_array().unwrap();
    assert!(
        !history.is_empty(),
        "performance history must be populated from the gapped seed"
    );

    // The endpoint reports weekly points, each one its week's last day with a value.
    // The week holding the first BUY starts Monday 2024-01-08 and has no close on the
    // 13th or the 14th, so its point is Friday the 12th: 200 split-adjusted shares at
    // 50.0. If a weekend hole propagated instead, the point would be dated the 14th
    // and worth nothing.
    let buy_week = history
        .iter()
        .find(|week| week["date"].as_str() == Some("2024-01-12"))
        .expect("the week holding the first BUY must report Friday's close");
    assert_eq!(buy_week["value"].as_str().unwrap().parse::<f64>().unwrap(), 10000.0);
}

#[tokio::test]
async fn a_lot_reports_the_quantity_that_was_bought_and_the_buy_it_came_from() {
    // Before this change `original_qty` and `remaining_qty` are both the already
    // decremented quantity, so the Holdings table prints the same number twice and the
    // originally-bought quantity is not recoverable from the response at all.
    //
    // Both seeds are load-bearing for hermeticity: `get_price` answers from today's row
    // instead of fetching, and `sync_splits_for_asset` returns on its cache check
    // instead of fetching. Neither call is mocked, so without them this test hits the
    // network and takes seconds.
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "Lot qty", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "LOTQTY", "Lot qty").await;
    let buy = seed_transaction_at(&pool, asset_id, "BUY", 60.0, 100.0, 0.0, "2024-01-01T00:00:00Z").await;
    seed_transaction_at(&pool, asset_id, "SELL", 20.0, 130.0, 0.0, "2024-12-01T00:00:00Z").await;
    seed_price(&pool, "LOTQTY", Utc::now().date_naive(), "110.0").await;
    seed_split_sync(&pool, "LOTQTY").await;

    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();
    let resp = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?strategy=FIFO&threshold_days=30"))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Ok);
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
    let lot = &body["assets"][0]["tax_lots"][0];

    assert_eq!(lot["buy_transaction_id"], buy, "the lot must name the buy it came from");
    assert_eq!(lot["original_qty"], "60.0", "60 were bought");
    assert_eq!(lot["remaining_qty"], "40.0", "20 were sold");
}
