mod common;

use common::{build_rocket, disabled_config, setup_db, DEFAULT_USER, seed_portfolio, seed_asset};
use rocket::http::Status;
use rocket::local::asynchronous::Client;
use rocket::serde::json::Json;

#[tokio::test]
async fn test_create_portfolio() {
    let pool = setup_db(&disabled_config()).await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let body = Json(serde_json::json!({
        "name": "Test Portfolio",
        "description": null,
        "currency": "USD"
    }));

    let resp = client.post("/api/portfolios/")
        .header(rocket::http::ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed["name"], "Test Portfolio");
    assert_eq!(parsed["currency"], "USD");
    assert!(parsed["assets"].is_array());
    assert_eq!(parsed["assets"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_list_portfolios() {
    let pool = setup_db(&disabled_config()).await;
    let id = seed_portfolio(&pool, DEFAULT_USER, "List Test", "USD").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get("/api/portfolios/").dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0]["id"], id);
    assert!(parsed[0]["assets"].is_array());
}

#[tokio::test]
async fn test_create_portfolio_with_description() {
    let pool = setup_db(&disabled_config()).await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let body = Json(serde_json::json!({
        "name": "Desc Test",
        "description": "A portfolio with description",
        "currency": "USD"
    }));

    let resp = client.post("/api/portfolios/")
        .header(rocket::http::ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed["name"], "Desc Test");
    assert_eq!(parsed["description"], "A portfolio with description");
    assert!(parsed["assets"].is_array());
}

#[tokio::test]
async fn test_list_portfolios_empty_db() {
    let pool = setup_db(&disabled_config()).await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get("/api/portfolios/").dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed.len(), 0);
}

#[tokio::test]
async fn test_list_portfolios_multi_asset() {
    let pool = setup_db(&disabled_config()).await;
    let id = seed_portfolio(&pool, DEFAULT_USER, "Multi Asset", "USD").await;
    seed_asset(&pool, id, "AAPL", "Apple").await;
    seed_asset(&pool, id, "GOOGL", "Google").await;
    seed_asset(&pool, id, "MSFT", "Microsoft").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get("/api/portfolios/").dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&body_str).unwrap();
    let portfolio = parsed.iter().find(|p| p["id"].as_i64().unwrap() == id as i64).unwrap();
    let assets = portfolio["assets"].as_array().unwrap();
    assert_eq!(assets.len(), 3);
    let symbols: Vec<&serde_json::Value> = assets.iter().map(|a| &a["symbol"]).collect();
    assert!(symbols.contains(&&serde_json::json!("AAPL")));
    assert!(symbols.contains(&&serde_json::json!("GOOGL")));
    assert!(symbols.contains(&&serde_json::json!("MSFT")));
}

#[tokio::test]
async fn test_get_portfolio_by_id() {
    let pool = setup_db(&disabled_config()).await;
    let id = seed_portfolio(&pool, DEFAULT_USER, "Get Test", "EUR").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get(format!("/api/portfolios/{}", id)).dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed["name"], "Get Test");
    assert_eq!(parsed["currency"], "EUR");
    assert!(parsed["assets"].is_array());
}

#[tokio::test]
async fn test_get_portfolio_not_found() {
    let pool = setup_db(&disabled_config()).await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get("/api/portfolios/9999").dispatch().await;
    assert_eq!(resp.status(), Status::NotFound);
}

#[tokio::test]
async fn test_delete_portfolio() {
    let pool = setup_db(&disabled_config()).await;
    let id = seed_portfolio(&pool, DEFAULT_USER, "Delete Test", "USD").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.delete(format!("/api/portfolios/{}", id)).dispatch().await;
    assert_eq!(resp.status(), Status::NoContent);

    // Verify it's gone
    let resp = client.get(format!("/api/portfolios/{}", id)).dispatch().await;
    assert_eq!(resp.status(), Status::NotFound);
}

#[tokio::test]
async fn test_delete_portfolio_with_assets_returns_conflict() {
    let pool = setup_db(&disabled_config()).await;
    let id = seed_portfolio(&pool, DEFAULT_USER, "Delete With Assets", "USD").await;
    seed_asset(&pool, id, "AAPL", "Apple").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.delete(format!("/api/portfolios/{}", id)).dispatch().await;
    assert_eq!(resp.status(), Status::Conflict);

    // Verify portfolio still exists
    let resp = client.get(format!("/api/portfolios/{}", id)).dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
}

#[tokio::test]
async fn test_delete_portfolio_not_found() {
    let pool = setup_db(&disabled_config()).await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.delete("/api/portfolios/9999").dispatch().await;
    assert_eq!(resp.status(), Status::NotFound);
}

#[tokio::test]
async fn test_update_portfolio_currency() {
    let pool = setup_db(&disabled_config()).await;
    let id = seed_portfolio(&pool, DEFAULT_USER, "Update Cur", "USD").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let body = Json(serde_json::json!({
        "currency": "EUR"
    }));

    let resp = client.patch(format!("/api/portfolios/{}", id))
        .header(rocket::http::ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed["currency"], "EUR");
    assert!(parsed["assets"].is_array());

    // Verify the change persisted
    let resp = client.get(format!("/api/portfolios/{}", id)).dispatch().await;
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed["currency"], "EUR");
    assert!(parsed["assets"].is_array());
}

#[tokio::test]
async fn test_update_portfolio_not_found() {
    let pool = setup_db(&disabled_config()).await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let body = Json(serde_json::json!({
        "currency": "EUR"
    }));

    let resp = client.patch("/api/portfolios/9999")
        .header(rocket::http::ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::NotFound);
}

#[tokio::test]
async fn test_list_portfolios_includes_assets() {
    let pool = setup_db(&disabled_config()).await;
    let id = seed_portfolio(&pool, DEFAULT_USER, "Asset Test", "USD").await;
    seed_asset(&pool, id, "AAPL", "Apple Inc").await;
    seed_asset(&pool, id, "BTC-USD", "Bitcoin").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get("/api/portfolios/").dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&body_str).unwrap();
    let portfolio = parsed.iter().find(|p| p["id"].as_i64().unwrap() == id as i64).unwrap();
    assert_eq!(portfolio["assets"].as_array().unwrap().len(), 2);
    assert_eq!(portfolio["assets"][0]["symbol"], "AAPL");
    assert_eq!(portfolio["assets"][1]["symbol"], "BTC-USD");
}

#[tokio::test]
async fn test_get_portfolio_includes_assets() {
    let pool = setup_db(&disabled_config()).await;
    let id = seed_portfolio(&pool, DEFAULT_USER, "Single Asset Test", "USD").await;
    seed_asset(&pool, id, "SPY", "SPDR S&P 500").await;
    let rocket = build_rocket(pool, DEFAULT_USER);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get(format!("/api/portfolios/{}", id)).dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    let assets = parsed["assets"].as_array().unwrap();
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0]["symbol"], "SPY");
}
