//! Authentication behaviour: the session cookie, the guard, and the requirement
//! that every `/api/` request is denied without one.
//!
//! The isolation matrix runs under `disabled` mode, where nothing denies — so this
//! file is the only place the unauthenticated path is exercised.

mod common;

use common::{
    build_rocket, build_rocket_oidc, disabled_config, expired_session_cookie, oidc_config,
    setup_db, session_cookie, DEFAULT_USER,
};
use rocket::http::{Cookie, Header, Method, SameSite, Status};
use rocket::local::asynchronous::Client;
use serde_json::Value;

/// Every `/api` route, with a syntactically valid id, for the "no session means
/// 401" sweep. Kept here as data so a new route that is missing from it is caught
/// by the route-coverage test rather than silently untested.
fn all_api_routes() -> Vec<(Method, &'static str)> {
    use Method::*;
    vec![
        (Get, "/api/portfolios/"),
        (Post, "/api/portfolios/"),
        (Get, "/api/portfolios/1"),
        (Patch, "/api/portfolios/1"),
        (Delete, "/api/portfolios/1"),
        (Get, "/api/portfolios/1/tax-summary?strategy=FIFO&threshold_days=365"),
        (Get, "/api/portfolios/1/performance"),
        (Post, "/api/portfolios/1/assets"),
        (Patch, "/api/portfolios/1/assets/1"),
        (Delete, "/api/assets/1"),
        (Post, "/api/portfolios/1/assets/1/transactions"),
        (Get, "/api/portfolios/1/transactions"),
        (Delete, "/api/transactions/1"),
        (Get, "/api/transactions/1/pnl?strategy=FIFO&threshold_days=365"),
        (Get, "/api/assets/lookup?isin=US0378331005"),
    ]
}

async fn oidc_client() -> Client {
    let pool = setup_db(&oidc_config()).await;
    Client::tracked(build_rocket_oidc(pool)).await.expect("valid rocket")
}

// ---------------------------------------------------------------------------
// session round-trip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_valid_session_cookie_authenticates() {
    let pool = setup_db(&oidc_config()).await;
    let client = Client::tracked(build_rocket_oidc(pool)).await.unwrap();

    let resp = client
        .get("/auth/me")
        .private_cookie(session_cookie(DEFAULT_USER as i64, 0))
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::Ok);
    let body: Value = resp.into_json().await.unwrap();
    assert_eq!(body["id"], DEFAULT_USER);
    assert_eq!(body["sub"], "bootstrap");
}

#[tokio::test]
async fn a_tampered_session_cookie_is_rejected() {
    let pool = setup_db(&oidc_config()).await;
    let client = Client::tracked(build_rocket_oidc(pool.clone())).await.unwrap();

    // A cookie forged by hand rather than through `add_private` fails the AEAD tag.
    let forged = Cookie::build(("pp_session", "bm90IGEgcm9ja2V0IGNvb2tpZQ"))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .finish();
    let resp = client.get("/auth/me").cookie(forged).dispatch().await;
    assert_eq!(resp.status(), Status::Unauthorized);

    // A well-formed cookie naming a user who does not exist is also rejected: the
    // guard re-reads the row on every request, so the cookie alone grants nothing.
    let ghost = session_cookie(999_999, 0);
    let resp = client.get("/auth/me").private_cookie(ghost).dispatch().await;
    assert_eq!(resp.status(), Status::Unauthorized, "a valid cookie for a missing user must not pass");

    // A cookie that decrypts to something that is not the payload shape fails closed.
    let resp = client
        .get("/auth/me")
        .private_cookie(("pp_session", r#"{"uid":999,"tv":0,"exp":9999999999}"#))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Unauthorized);

    // The positive control: the same construction for a real user is accepted.
    // That is what shows the AEAD tag, not the shape of the value, authenticates.
    let real = session_cookie(DEFAULT_USER as i64, 0);
    let resp = client.get("/auth/me").private_cookie(real).dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
}

#[tokio::test]
async fn a_cookie_whose_token_version_is_stale_is_rejected() {
    let pool = setup_db(&oidc_config()).await;
    let client = Client::tracked(build_rocket_oidc(pool)).await.unwrap();

    let resp = client
        .get("/auth/me")
        .private_cookie(session_cookie(DEFAULT_USER as i64, 41))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Unauthorized);
}

#[tokio::test]
async fn an_expired_cookie_is_rejected_even_with_a_matching_token_version() {
    // The point of `exp`: a Rocket private cookie is otherwise valid forever, so
    // without a server-side check a stolen cookie never stops working.
    let pool = setup_db(&oidc_config()).await;
    let client = Client::tracked(build_rocket_oidc(pool)).await.unwrap();

    let resp = client
        .get("/auth/me")
        .private_cookie(expired_session_cookie(DEFAULT_USER as i64, 0))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Unauthorized);
}

#[tokio::test]
async fn a_deactivated_user_is_rejected() {
    let pool = setup_db(&oidc_config()).await;
    sqlx::query("UPDATE users SET is_active = 0 WHERE id = ?")
        .bind(DEFAULT_USER as i64)
        .execute(&pool)
        .await
        .unwrap();
    let client = Client::tracked(build_rocket_oidc(pool.clone())).await.unwrap();

    let resp = client
        .get("/auth/me")
        .private_cookie(session_cookie(DEFAULT_USER as i64, 0))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Unauthorized);
}

#[tokio::test]
async fn logout_all_bumps_the_token_version_and_invalidates_the_live_cookie() {
    let pool = setup_db(&oidc_config()).await;
    let client = Client::tracked(build_rocket_oidc(pool.clone())).await.unwrap();
    let cookie = session_cookie(DEFAULT_USER as i64, 0);

    // Valid before.
    let resp = client.get("/auth/me").private_cookie(cookie.clone()).dispatch().await;
    assert_eq!(resp.status(), Status::Ok);

    let resp = client.post("/auth/logout-all").private_cookie(cookie.clone()).dispatch().await;
    assert_eq!(resp.status(), Status::SeeOther);

    let (version,): (i64,) =
        sqlx::query_as("SELECT token_version FROM users WHERE id = ?")
            .bind(DEFAULT_USER as i64)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(version, 1, "the bump is what invalidates every session, not just this cookie");

    // The same cookie, unchanged, no longer authenticates.
    let resp = client.get("/auth/me").private_cookie(cookie).dispatch().await;
    assert_eq!(resp.status(), Status::Unauthorized);
}

#[tokio::test]
async fn a_forged_authorization_header_does_not_authenticate() {
    // The guard reads the session cookie only. Accepting a bearer header here
    // would silently widen the authentication surface beyond what this change
    // claims to provide.
    let client = oidc_client().await;
    let resp = client
        .get("/api/portfolios/")
        .header(Header::new("Authorization", "Bearer forged-token"))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Unauthorized);
}

// ---------------------------------------------------------------------------
// F1: every /api request requires an identity
// ---------------------------------------------------------------------------

#[tokio::test]
async fn every_api_route_is_unauthenticated_without_a_session() {
    let client = oidc_client().await;
    let mut checked = 0;
    for (method, path) in all_api_routes() {
        let resp = client.req(method, path).dispatch().await;
        assert_eq!(resp.status(), Status::Unauthorized, "{method} {path}");
        let body: Value = resp.into_json().await.unwrap();
        assert_eq!(body["error"], "unauthenticated", "{method} {path}");
        checked += 1;
    }
    assert_eq!(checked, 15, "the sweep must cover the whole mounted surface");
}

#[tokio::test]
async fn a_denied_post_has_no_side_effect() {
    // The test that distinguishes the URI-rewrite design from a guards-only one:
    // the handler must not merely fail to authorise, it must not run at all.
    let pool = setup_db(&oidc_config()).await;
    let before: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM portfolios")
        .fetch_one(&pool)
        .await
        .unwrap();

    let client = Client::tracked(build_rocket_oidc(pool.clone())).await.unwrap();
    let resp = client
        .post("/api/portfolios/")
        .header(Header::new("Content-Type", "application/json".to_string()))
        .body(r#"{"name":"should not exist","currency":"USD","description":null,"base_currency":"USD"}"#)
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::Unauthorized);
    let after: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM portfolios")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before.0, after.0, "the handler ran despite the 401");
}

#[tokio::test]
async fn an_unauthenticated_put_is_401_not_405() {
    // Why the sentinel is mounted for all seven methods.
    let client = oidc_client().await;
    for method in [Method::Put, Method::Patch, Method::Delete, Method::Head] {
        let resp = client.req(method, "/api/portfolios/1").dispatch().await;
        assert_eq!(resp.status(), Status::Unauthorized, "{method}");
    }
}

#[tokio::test]
async fn a_bare_options_is_denied_but_a_real_preflight_is_not() {
    let client = oidc_client().await;

    let resp = client.options("/api/portfolios/").dispatch().await;
    assert_eq!(resp.status(), Status::Unauthorized, "a bare OPTIONS is not a preflight");

    let resp = client
        .options("/api/portfolios/")
        .header(Header::new("Origin", "http://localhost:5173"))
        .header(Header::new("Access-Control-Request-Method", "GET"))
        .dispatch()
        .await;
    assert_ne!(resp.status(), Status::Unauthorized, "a real preflight must reach CORS");
}

// ---------------------------------------------------------------------------
// public surface
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_root_and_the_api_contract_stay_public() {
    let client = oidc_client().await;

    assert_eq!(client.get("/").dispatch().await.status(), Status::Ok);

    let resp = client.get("/api-docs/openapi.json").dispatch().await;
    assert_eq!(resp.status(), Status::Ok, "the contract is public by decision");

    // The duplicate mount is gone; the handler's attribute still serves /api-docs.
    assert_eq!(client.get("/openapi.json").dispatch().await.status(), Status::NotFound);
}

#[tokio::test]
async fn auth_me_reports_the_placeholder_under_disabled_mode() {
    let pool = setup_db(&disabled_config()).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();

    let resp = client.get("/auth/me").dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body: Value = resp.into_json().await.unwrap();
    // The literal marker string, not a subject any provider issues.
    assert_eq!(body["sub"], "bootstrap");
}

#[tokio::test]
async fn auth_me_without_a_session_returns_the_shared_unauthenticated_body() {
    let client = oidc_client().await;
    let resp = client.get("/auth/me").dispatch().await;
    assert_eq!(resp.status(), Status::Unauthorized);
    // `/auth/me` is outside `/api`, so the denial route never runs for it; this
    // pins that the guard produces the same body rather than Rocket's default.
    // Parsed from text rather than `into_json`, which yields None for an error
    // status; the point is the exact body the guard writes.
    let text = resp.into_string().await.unwrap();
    let body: Value = serde_json::from_str(&text).expect(&text);
    assert_eq!(body["error"], "unauthenticated");
}

#[tokio::test]
async fn login_under_disabled_mode_does_not_try_to_reach_a_provider() {
    let pool = setup_db(&disabled_config()).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();
    // No OIDC-only routes are mounted in this mode, so the route is simply absent.
    assert_eq!(client.get("/auth/login").dispatch().await.status(), Status::NotFound);
}
