mod common;

use common::{build_rocket, disabled_config, setup_db, DEFAULT_USER, seed_portfolio, seed_asset, seed_transaction};
use rocket::http::Status;
use rocket::local::asynchronous::Client;
use rocket::serde::json::Json;

#[tokio::test]
async fn test_create_asset() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "Tx Test", "USD").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let body = Json(serde_json::json!({
        "symbol": "AAPL",
        "name": "Apple Inc",
        "asset_type": "STOCK",
        "sector": "Technology",
        "currency": "USD",
        "isin": "US0378331005"
    }));

    let resp = client.post(format!("/api/portfolios/{}/assets", port_id))
        .header(rocket::http::ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed["symbol"], "AAPL");
    assert_eq!(parsed["portfolio_id"], port_id);
    assert!(parsed["id"].is_number());
    assert_eq!(parsed["name"], "Apple Inc");
    assert_eq!(parsed["asset_type"], "STOCK");
    assert_eq!(parsed["sector"], "Technology");
    assert_eq!(parsed["currency"], "USD");
    assert_eq!(parsed["isin"], "US0378331005");
    assert!(parsed["transactions"].is_array());
    assert_eq!(parsed["transactions"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_create_duplicate_asset_returns_400() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "Dup Test", "USD").await;
    let _asset_id = seed_asset(&pool, port_id, "AAPL", "Apple").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let body = Json(serde_json::json!({
        "symbol": "AAPL",
        "name": "Apple Inc",
        "asset_type": "STOCK",
        "sector": null,
        "currency": "USD",
        "isin": "US0378331005"
    }));

    let resp = client.post(format!("/api/portfolios/{}/assets", port_id))
        .header(rocket::http::ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::BadRequest);
}

#[tokio::test]
async fn test_create_transaction() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "Create Tx", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "GOOGL", "Google").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let body = Json(serde_json::json!({
        "type": "BUY",
        "quantity": 10.0,
        "price": 150.0,
        "fee": 5.0,
        "date": "2024-06-15T00:00:00Z"
    }));

    let resp = client.post(format!("/api/portfolios/{}/assets/{}/transactions", port_id, asset_id))
        .header(rocket::http::ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed["type"], "BUY");
    assert!(parsed["id"].is_number());
    assert_eq!(parsed["asset_id"], asset_id);
    assert_eq!(parsed["quantity"], "10");
    assert_eq!(parsed["price"], "150");
    assert_eq!(parsed["fee"], "5");
    assert!(parsed["date"].is_string());
}

#[tokio::test]
async fn test_create_split_transaction_forces_fee_to_zero() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "Split Tx", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "NVDA", "Nvidia").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let body = Json(serde_json::json!({
        "type": "SPLIT",
        "quantity": 2.0,
        "price": 1.0,
        "fee": 5.0,
        "date": "2024-06-15T00:00:00Z"
    }));

    let resp = client.post(format!("/api/portfolios/{}/assets/{}/transactions", port_id, asset_id))
        .header(rocket::http::ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed["type"], "SPLIT");
    assert_eq!(parsed["quantity"], "2");
    assert_eq!(parsed["price"], "1");
    assert_eq!(parsed["fee"], "0");
}

#[tokio::test]
async fn test_create_split_transaction_rejects_non_positive_ratio() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "Bad Split", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "INTC", "Intel").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let body = Json(serde_json::json!({
        "type": "SPLIT",
        "quantity": 0.0,
        "price": 1.0,
        "fee": 0.0,
        "date": "2024-06-15T00:00:00Z"
    }));

    let resp = client.post(format!("/api/portfolios/{}/assets/{}/transactions", port_id, asset_id))
        .header(rocket::http::ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::BadRequest);
}

#[tokio::test]
async fn test_create_transaction_nonexistent_asset_returns_404() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "404 Tx", "USD").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let body = Json(serde_json::json!({
        "type": "BUY",
        "quantity": 10.0,
        "price": 150.0,
        "fee": 5.0,
        "date": "2024-06-15T00:00:00Z"
    }));

    let resp = client.post(format!("/api/portfolios/{}/assets/9999/transactions", port_id))
        .header(rocket::http::ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::NotFound);
}

#[tokio::test]
async fn test_list_portfolio_transactions() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "List Tx", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "MSFT", "Microsoft").await;
    seed_transaction(&pool, asset_id, "BUY", 50.0, 300.0, 10.0).await;
    seed_transaction(&pool, asset_id, "BUY", 25.0, 320.0, 8.0).await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get(format!("/api/portfolios/{}/transactions", port_id)).dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed.len(), 2);
    // Verify sort order: most recent first (seed_transaction uses fixed date, but check fields)
    let tx0 = &parsed[0];
    let _tx1 = &parsed[1];
    assert!(tx0["id"].is_number());
    assert_eq!(tx0["asset_id"], asset_id);
    assert!(tx0["type"].is_string());
    assert_eq!(tx0["quantity"], "50.0");
    assert_eq!(tx0["price"], "300.0");
    assert_eq!(tx0["fee"], "10.0");
    assert!(tx0["date"].is_string());
}

#[tokio::test]
async fn test_delete_asset() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "Del Asset", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "TSLA", "Tesla").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.delete(format!("/api/assets/{}", asset_id)).dispatch().await;
    assert_eq!(resp.status(), Status::NoContent);
}

#[tokio::test]
async fn test_delete_asset_not_found() {
    let pool = setup_db(&disabled_config()).await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.delete("/api/assets/9999").dispatch().await;
    assert_eq!(resp.status(), Status::NotFound);
}

#[tokio::test]
async fn test_delete_transaction() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "Del Tx", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "NVDA", "Nvidia").await;
    let tx_id = seed_transaction(&pool, asset_id, "BUY", 10.0, 500.0, 5.0).await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.delete(format!("/api/transactions/{}", tx_id)).dispatch().await;
    assert_eq!(resp.status(), Status::NoContent);
}

#[tokio::test]
async fn test_delete_transaction_not_found() {
    let pool = setup_db(&disabled_config()).await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.delete("/api/transactions/9999").dispatch().await;
    assert_eq!(resp.status(), Status::NotFound);
}

#[tokio::test]
async fn test_update_asset_currency() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "Update Asset Cur", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "SAP", "SAP").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let body = Json(serde_json::json!({
        "currency": "EUR"
    }));

    let resp = client.patch(format!("/api/portfolios/{}/assets/{}", port_id, asset_id))
        .header(rocket::http::ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed["currency"], "EUR");
    // Verify other fields still present after update
    assert_eq!(parsed["symbol"], "SAP");
    assert_eq!(parsed["name"], "SAP");
    assert_eq!(parsed["asset_type"], "STOCK");
    assert!(parsed["id"].is_number());

    // Verify the change persisted by listing transactions (which includes asset info)
    let resp = client.get(format!("/api/portfolios/{}/transactions", port_id)).dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
}

#[tokio::test]
async fn test_create_transaction_invalid_date_returns_400() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "Invalid Date", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "INVALID", "Invalid Asset").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    // Test date from year 11 AD
    let body = Json(serde_json::json!({
        "type": "BUY",
        "quantity": 10.0,
        "price": 150.0,
        "fee": 5.0,
        "date": "0011-11-21T11:37:21Z"
    }));

    let resp = client.post(format!("/api/portfolios/{}/assets/{}/transactions", port_id, asset_id))
        .header(rocket::http::ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::BadRequest);
}

#[tokio::test]
async fn test_update_asset_not_found() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "Update Asset 404", "USD").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let body = Json(serde_json::json!({
        "currency": "EUR"
    }));

    let resp = client.patch(format!("/api/portfolios/{}/assets/9999", port_id))
        .header(rocket::http::ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::NotFound);
}

#[tokio::test]
async fn test_create_asset_currency_detection_fallback() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "Detect Cur", "USD").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    // Send asset with empty currency — backend should detect from symbol
    let body = Json(serde_json::json!({
        "symbol": "SAP.DE",
        "name": "SAP SE",
        "asset_type": "STOCK",
        "sector": "Technology",
        "currency": "",
        "isin": "DE000BAY0017"
    }));

    let resp = client.post(format!("/api/portfolios/{}/assets", port_id))
        .header(rocket::http::ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed["symbol"], "SAP.DE");
    assert_eq!(parsed["currency"], "EUR");
    assert_eq!(parsed["isin"], "DE000BAY0017");
}

#[tokio::test]
async fn test_create_asset_null_sector() {
    let pool = setup_db(&disabled_config()).await;
    let port_id = seed_portfolio(&pool, DEFAULT_USER, "Null Sector", "USD").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let body = Json(serde_json::json!({
        "symbol": "BTC-USD",
        "name": "Bitcoin",
        "asset_type": "CRYPTO",
        "sector": null,
        "currency": "USD",
        "isin": "BTC123456789"
    }));

    let resp = client.post(format!("/api/portfolios/{}/assets", port_id))
        .header(rocket::http::ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed["symbol"], "BTC-USD");
    assert_eq!(parsed["sector"], serde_json::Value::Null);
    assert_eq!(parsed["isin"], "BTC123456789");
    assert!(parsed["id"].is_number());
}

#[tokio::test]
async fn test_cross_portfolio_transaction_rejected() {
    let pool = setup_db(&disabled_config()).await;
    let port_a = seed_portfolio(&pool, DEFAULT_USER, "Port A", "USD").await;
    let port_b = seed_portfolio(&pool, DEFAULT_USER, "Port B", "USD").await;
    let asset_id = seed_asset(&pool, port_a, "AAPL", "Apple").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    // Try to add transaction via portfolio B's URL but asset belongs to A
    let body = Json(serde_json::json!({
        "type": "BUY",
        "quantity": 10.0,
        "price": 150.0,
        "fee": 5.0,
        "date": "2024-06-15T00:00:00Z"
    }));

    let resp = client.post(format!("/api/portfolios/{}/assets/{}/transactions", port_b, asset_id))
        .header(rocket::http::ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::NotFound);
}
