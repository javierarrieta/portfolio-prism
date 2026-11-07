mod common;

use common::{build_rocket, disabled_config, setup_db, DEFAULT_USER, seed_portfolio, seed_asset, seed_transaction};
use rocket::http::Status;
use rocket::local::asynchronous::Client;

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
    use chrono::{NaiveDate, Utc};

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
        sqlx::query("INSERT OR REPLACE INTO historical_prices (symbol, date, close_price) VALUES (?, ?, ?)")
            .bind(symbol)
            .bind(curr)
            .bind("50.0")
            .execute(&pool)
            .await
            .unwrap();
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
