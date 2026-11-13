use backend_rust::backup::BackupConfig;
use backend_rust::build_rocket;
use backend_rust::get_cors_options;
use backend_rust::parse_allowed_origins;
mod common;

use common::offline_pricing_service;

use backend_rust::auth::finalise_auth;
use backend_rust::init_db;
use rocket::http::{Header, Status};
use rocket::local::asynchronous::Client;
use sqlx::SqlitePool;

#[tokio::test]
async fn test_cors_allowed_origin() {
    let pool = SqlitePool::connect(":memory:").await.unwrap();
    let cfg = common::disabled_config();
    init_db(&pool, &cfg).await.unwrap();
    let auth = finalise_auth(&pool, &cfg).await.unwrap();
    let currency_service = offline_pricing_service();

    let allowed_origin: &'static str =
        Box::leak("http://localhost:5173".to_string().into_boxed_str());
    let cors = get_cors_options(vec![allowed_origin.to_string()]);
    let rocket = build_rocket(pool, currency_service, cors, auth, BackupConfig::default());
    let client = Client::tracked(rocket).await.expect("valid rocket");

    let response = client
        .get("/api/portfolios/")
        .header(Header::new("Origin", allowed_origin))
        .dispatch()
        .await;

    assert_eq!(response.status(), Status::Ok);
    assert_eq!(
        response.headers().get_one("Access-Control-Allow-Origin"),
        Some(allowed_origin)
    );
}

#[tokio::test]
async fn a_disallowed_origin_is_answered_before_authentication_gets_a_turn() {
    // `rocket_cors` is attached before the auth fairing, and on an Origin it does not
    // allow it rewrites the request to its own `403` route. The rewritten path is not
    // under `/api/`, so `is_protected` is false and the auth layer never sees the
    // request. Nothing leaks — the handler does not run — but the consequence is
    // operational and severe: browsers send `Origin` on same-origin `POST`/`PATCH`/
    // `DELETE`, so a deployment that leaves `ALLOWED_ORIGINS` at its localhost default
    // answers 403 to every write from the public site, and `apiFetch` only redirects
    // on 401, so the user sees a silent failure.
    let pool = common::setup_db(&common::oidc_config()).await;
    let client = Client::tracked(common::build_rocket_oidc_with_cors(
        pool.clone(),
        vec!["https://elsewhere.example".to_string()],
    ))
    .await
    .expect("valid rocket");

    let resp = client
        .post("/api/portfolios/")
        .header(Header::new("Origin", "https://portfolio.example.com"))
        .json(&serde_json::json!({"name": "x"}))
        .dispatch()
        .await;
    assert_eq!(
        resp.status(),
        Status::Forbidden,
        "a disallowed Origin must be rejected by CORS, not converted into a 401"
    );

    // With the origin allowed, the same unauthenticated request reaches the auth
    // layer, which is the ordering that makes the fairing's prefix rule meaningful.
    let allowed = Client::tracked(common::build_rocket_oidc(pool.clone()))
        .await
        .expect("valid rocket");
    let resp = allowed
        .post("/api/portfolios/")
        .header(Header::new("Origin", "http://localhost:5173"))
        .json(&serde_json::json!({"name": "x"}))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Unauthorized);
}

#[tokio::test]
async fn test_cors_disallowed_origin() {
    let pool = SqlitePool::connect(":memory:").await.unwrap();
    let cfg = common::disabled_config();
    init_db(&pool, &cfg).await.unwrap();
    let auth = finalise_auth(&pool, &cfg).await.unwrap();
    let currency_service = offline_pricing_service();

    let allowed_origin: &'static str =
        Box::leak("http://localhost:5173".to_string().into_boxed_str());
    let cors = get_cors_options(vec![allowed_origin.to_string()]);
    let rocket = build_rocket(pool, currency_service, cors, auth, BackupConfig::default());
    let client = Client::tracked(rocket).await.expect("valid rocket");

    let disallowed_origin: &'static str =
        Box::leak("https://example.com".to_string().into_boxed_str());

    let response = client
        .options("/api/portfolios/")
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
    let currency_service = offline_pricing_service();

    let allowed_origins_raw = vec![
        "http://localhost:5173".to_string(),
        "https://my-app.com".to_string(),
    ];
    let allowed_origins_static: Vec<&'static str> = allowed_origins_raw
        .iter()
        .map(|s| Box::leak(s.clone().into_boxed_str()) as &'static str)
        .collect();

    let cors = get_cors_options(allowed_origins_raw.clone());
    let rocket = build_rocket(pool, currency_service, cors, auth, BackupConfig::default());
    let client = Client::tracked(rocket).await.expect("valid rocket");

    // Test first origin
    let resp1 = client
        .get("/api/portfolios/")
        .header(Header::new("Origin", allowed_origins_static[0]))
        .dispatch()
        .await;
    assert_eq!(resp1.status(), Status::Ok);
    assert_eq!(
        resp1.headers().get_one("Access-Control-Allow-Origin"),
        Some(allowed_origins_static[0])
    );

    // Test second origin
    let resp2 = client
        .get("/api/portfolios/")
        .header(Header::new("Origin", allowed_origins_static[1]))
        .dispatch()
        .await;
    assert_eq!(resp2.status(), Status::Ok);
    assert_eq!(
        resp2.headers().get_one("Access-Control-Allow-Origin"),
        Some(allowed_origins_static[1])
    );
}

#[tokio::test]
async fn test_cors_trailing_comma_origin_is_allowed() {
    // Regression: "https://app,": used to yield an empty entry, rocket_cors
    // rejected it, and get_cors_options panicked before the server bound a port.
    let pool = SqlitePool::connect(":memory:").await.unwrap();
    let cfg = common::disabled_config();
    init_db(&pool, &cfg).await.unwrap();
    let auth = finalise_auth(&pool, &cfg).await.unwrap();
    let currency_service = offline_pricing_service();

    let cors = get_cors_options(parse_allowed_origins(Some("http://localhost:5173,")));
    let rocket = build_rocket(pool, currency_service, cors, auth, BackupConfig::default());
    let client = Client::tracked(rocket).await.expect("valid rocket");

    let response = client
        .get("/api/portfolios/")
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
    let currency_service = offline_pricing_service();

    let cors = get_cors_options(parse_allowed_origins(Some("")));
    let rocket = build_rocket(pool, currency_service, cors, auth, BackupConfig::default());
    let client = Client::tracked(rocket).await.expect("valid rocket");

    let response = client
        .get("/api/portfolios/")
        .header(Header::new("Origin", "http://127.0.0.1:5173"))
        .dispatch()
        .await;

    assert_eq!(response.status(), Status::Ok);
    assert_eq!(
        response.headers().get_one("Access-Control-Allow-Origin"),
        Some("http://127.0.0.1:5173")
    );
}
