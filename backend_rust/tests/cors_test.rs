use backend_rust::parse_allowed_origins;
use backend_rust::get_cors_options;
use backend_rust::build_rocket;
mod common;

use backend_rust::init_db;
use backend_rust::auth::finalise_auth;
use backend_rust::services::currency_service::CurrencyService;
use rocket::http::{Status, Header};
use rocket::local::asynchronous::Client;
use sqlx::SqlitePool;

#[tokio::test]
async fn test_cors_allowed_origin() {
    let pool = SqlitePool::connect(":memory:").await.unwrap();
    let cfg = common::disabled_config();
    init_db(&pool, &cfg).await.unwrap();
    let auth = finalise_auth(&pool, &cfg).await.unwrap();
    let currency_service = CurrencyService::new();
    
    let allowed_origin: &'static str = Box::leak("http://localhost:5173".to_string().into_boxed_str());
    let cors = get_cors_options(vec![allowed_origin.to_string()]);
    let rocket = build_rocket(pool, currency_service, cors, auth);
    let client = Client::tracked(rocket).await.expect("valid rocket");

    let response = client.get("/api/portfolios/")
        .header(Header::new("Origin", allowed_origin))
        .dispatch()
        .await;

    assert_eq!(response.status(), Status::Ok);
    assert_eq!(response.headers().get_one("Access-Control-Allow-Origin"), Some(allowed_origin));
}

#[tokio::test]
async fn test_cors_disallowed_origin() {
    let pool = SqlitePool::connect(":memory:").await.unwrap();
    let cfg = common::disabled_config();
    init_db(&pool, &cfg).await.unwrap();
    let auth = finalise_auth(&pool, &cfg).await.unwrap();
    let currency_service = CurrencyService::new();
    
    let allowed_origin: &'static str = Box::leak("http://localhost:5173".to_string().into_boxed_str());
    let cors = get_cors_options(vec![allowed_origin.to_string()]);
    let rocket = build_rocket(pool, currency_service, cors, auth);
    let client = Client::tracked(rocket).await.expect("valid rocket");

    let disallowed_origin: &'static str = Box::leak("https://example.com".to_string().into_boxed_str());
    
    let response = client.options("/api/portfolios/")
        .header(Header::new("Origin", disallowed_origin))
        .header(Header::new("Access-Control-Request-Method", "POST"))
        .dispatch()
        .await;

    assert_eq!(response.status(), Status::Forbidden);
}

#[tokio::test]
async fn test_cors_multiple_origins() {
    let pool = SqlitePool::connect(":memory:").await.unwrap();
    let cfg = common::disabled_config();
    init_db(&pool, &cfg).await.unwrap();
    let auth = finalise_auth(&pool, &cfg).await.unwrap();
    let currency_service = CurrencyService::new();
    
    let allowed_origins_raw = vec!["http://localhost:5173".to_string(), "https://my-app.com".to_string()];
    let allowed_origins_static: Vec<&'static str> = allowed_origins_raw.iter()
        .map(|s| Box::leak(s.clone().into_boxed_str()) as &'static str)
        .collect();
        
    let cors = get_cors_options(allowed_origins_raw.clone());
    let rocket = build_rocket(pool, currency_service, cors, auth);
    let client = Client::tracked(rocket).await.expect("valid rocket");

    // Test first origin
    let resp1 = client.get("/api/portfolios/")
        .header(Header::new("Origin", allowed_origins_static[0]))
        .dispatch()
        .await;
    assert_eq!(resp1.status(), Status::Ok);
    assert_eq!(resp1.headers().get_one("Access-Control-Allow-Origin"), Some(allowed_origins_static[0]));

    // Test second origin
    let resp2 = client.get("/api/portfolios/")
        .header(Header::new("Origin", allowed_origins_static[1]))
        .dispatch()
        .await;
    assert_eq!(resp2.status(), Status::Ok);
    assert_eq!(resp2.headers().get_one("Access-Control-Allow-Origin"), Some(allowed_origins_static[1]));
}

#[tokio::test]
async fn test_cors_trailing_comma_origin_is_allowed() {
    // Regression: "https://app,": used to yield an empty entry, rocket_cors
    // rejected it, and get_cors_options panicked before the server bound a port.
    let pool = SqlitePool::connect(":memory:").await.unwrap();
    let cfg = common::disabled_config();
    init_db(&pool, &cfg).await.unwrap();
    let auth = finalise_auth(&pool, &cfg).await.unwrap();
    let currency_service = CurrencyService::new();

    let cors = get_cors_options(parse_allowed_origins(Some("http://localhost:5173,")));
    let rocket = build_rocket(pool, currency_service, cors, auth);
    let client = Client::tracked(rocket).await.expect("valid rocket");

    let response = client.get("/api/portfolios/")
        .header(Header::new("Origin", "http://localhost:5173"))
        .dispatch()
        .await;

    assert_eq!(response.status(), Status::Ok);
    assert_eq!(
        response.headers().get_one("Access-Control-Allow-Origin"),
        Some("http://localhost:5173")
    );
}

#[tokio::test]
async fn test_cors_blank_env_falls_back_to_dev_defaults() {
    // An empty allowlist would answer 403 to every browser while the process
    // looked healthy, so a blank value must restore the dev defaults instead.
    let pool = SqlitePool::connect(":memory:").await.unwrap();
    let cfg = common::disabled_config();
    init_db(&pool, &cfg).await.unwrap();
    let auth = finalise_auth(&pool, &cfg).await.unwrap();
    let currency_service = CurrencyService::new();

    let cors = get_cors_options(parse_allowed_origins(Some("")));
    let rocket = build_rocket(pool, currency_service, cors, auth);
    let client = Client::tracked(rocket).await.expect("valid rocket");

    let response = client.get("/api/portfolios/")
        .header(Header::new("Origin", "http://127.0.0.1:5173"))
        .dispatch()
        .await;

    assert_eq!(response.status(), Status::Ok);
    assert_eq!(
        response.headers().get_one("Access-Control-Allow-Origin"),
        Some("http://127.0.0.1:5173")
    );
}
