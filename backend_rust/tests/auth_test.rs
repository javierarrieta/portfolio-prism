//! Authentication behaviour: the session cookie, the guard, and the requirement
//! that every `/api/` request is denied without one.
//!
//! The isolation matrix runs under `disabled` mode, where nothing denies — so this
//! file is the only place the unauthenticated path is exercised.

mod common;

use common::{
    DEFAULT_USER, build_rocket, build_rocket_oidc, disabled_config, expired_session_cookie,
    oidc_cfg, oidc_config, session_cookie, setup_db,
};
use rocket::http::{Cookie, Header, Method, SameSite, Status};
use rocket::local::asynchronous::Client;
use serde_json::Value;

/// Every `/api` route the router actually holds, with `<param>` filled by a
/// syntactically valid id.
///
/// Derived from the ignited router rather than written out, so a route added
/// without touching this file is swept automatically. A hand-maintained copy here
/// and the inventory in `route_coverage_test.rs` were two lists that could each
/// agree with themselves while neither matched the mount.
async fn all_api_routes() -> Vec<(Method, String)> {
    let pool = setup_db(&oidc_config()).await;
    let rocket = build_rocket_oidc(pool)
        .ignite()
        .await
        .expect("rocket ignites");
    rocket
        .routes()
        .filter(|r| r.uri.path().starts_with("/api/"))
        .map(|r| {
            let path = r
                .uri
                .to_string()
                .split('?')
                .next()
                .unwrap()
                .replace("<id>", "1")
                .replace("<portfolio_id>", "1")
                .replace("<asset_id>", "1");
            // The tax routes default `strategy` and `threshold_days`; the parameters are
            // here so the sweep exercises the same shape the UI sends, not because the
            // route requires them.
            if path.ends_with("/tax-summary") || path.ends_with("/pnl") {
                (r.method, format!("{path}?strategy=FIFO&threshold_days=365"))
            } else if path.starts_with("/api/assets/lookup") {
                (r.method, format!("{path}?isin=US0378331005"))
            } else {
                (r.method, path)
            }
        })
        .collect()
}

async fn oidc_client() -> Client {
    let pool = setup_db(&oidc_config()).await;
    Client::tracked(build_rocket_oidc(pool))
        .await
        .expect("valid rocket")
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
    let client = Client::tracked(build_rocket_oidc(pool.clone()))
        .await
        .unwrap();

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
    let resp = client
        .get("/auth/me")
        .private_cookie(ghost)
        .dispatch()
        .await;
    assert_eq!(
        resp.status(),
        Status::Unauthorized,
        "a valid cookie for a missing user must not pass"
    );

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
    let client = Client::tracked(build_rocket_oidc(pool.clone()))
        .await
        .unwrap();

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
    let client = Client::tracked(build_rocket_oidc(pool.clone()))
        .await
        .unwrap();
    let cookie = session_cookie(DEFAULT_USER as i64, 0);

    // Valid before.
    let resp = client
        .get("/auth/me")
        .private_cookie(cookie.clone())
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Ok);

    let resp = client
        .post("/auth/logout-all")
        .private_cookie(cookie.clone())
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::SeeOther);

    let (version,): (i64,) = sqlx::query_as("SELECT token_version FROM users WHERE id = ?")
        .bind(DEFAULT_USER as i64)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        version, 1,
        "the bump is what invalidates every session, not just this cookie"
    );

    // The same cookie, unchanged, no longer authenticates.
    let resp = client
        .get("/auth/me")
        .private_cookie(cookie)
        .dispatch()
        .await;
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
    for (method, path) in all_api_routes().await {
        let resp = client.req(method, path.as_str()).dispatch().await;
        assert_eq!(resp.status(), Status::Unauthorized, "{method} {path}");
        let body: Value = resp.into_json().await.unwrap();
        assert_eq!(body["error"], "unauthenticated", "{method} {path}");
        checked += 1;
    }
    assert_eq!(
        checked, 18,
        "the sweep must cover the whole mounted surface"
    );
}

/// The same surface, minus the one route that would make a real outbound HTTP
/// call if it were allowed to reach its handler. `GET /api/assets/lookup` is
/// covered by the unauthenticated sweep above; reaching it here would make the
/// suite depend on the network and on a third-party quota.
async fn reachable_api_routes() -> Vec<(Method, String)> {
    all_api_routes()
        .await
        .into_iter()
        .filter(|(_, path)| !path.starts_with("/api/assets/lookup"))
        .collect()
}

/// The same tax route again, dated. `from`/`to` select a *response shape* on an existing
/// route rather than a route of their own, so the sweep above does not cover them: no new
/// entry exists to sweep. The window is denied before the handler runs.
#[tokio::test]
async fn every_tax_route_denies_the_dated_variant_without_a_session() {
    let client = oidc_client().await;
    let dated = "/api/portfolios/1/tax-summary?strategy=FIFO&threshold_days=365&from=2024-01-01&to=2024-12-31";
    assert_eq!(client.get(dated).dispatch().await.status(), Status::Unauthorized);
    // The bare window too: `strategy` and `threshold_days` are optional, so a request
    // that names neither must not fall outside the check.
    let csvish = "/api/portfolios/1/tax-summary?from=2024-01-01&to=2024-12-31";
    assert_eq!(client.get(csvish).dispatch().await.status(), Status::Unauthorized);
}

#[tokio::test]
async fn every_api_route_reaches_a_handler_with_a_session() {
    // The mirror of the sweep above, and the test that was missing when the
    // implementation shipped. "Every /api route answers 401 without a cookie" is
    // equally satisfied by a build that answers 401 *with* one — which is exactly
    // the bug this file now pins: the callback used to mint sessions at
    // `token_version = 0` regardless of the row, so the first `logout-all` locked a
    // user out of all 16 routes forever and every test still passed.
    let client = oidc_client().await;
    let mut checked = 0;
    for (method, path) in reachable_api_routes().await {
        let resp = client
            .req(method, path.as_str())
            .private_cookie(session_cookie(DEFAULT_USER as i64, 0))
            .dispatch()
            .await;
        assert_ne!(
            resp.status(),
            Status::Unauthorized,
            "{method} {path} rejected a valid session"
        );
        checked += 1;
    }
    assert_eq!(
        checked, 17,
        "the sweep covers the mounted surface bar the network route"
    );
}

#[tokio::test]
async fn logging_out_everywhere_does_not_lock_the_user_out() {
    // `logout-all` is the only revocation lever the system has, and it was also an
    // account lock: it raises `users.token_version`, while the callback kept issuing
    // `tv = 0`, so every later login was rejected and nothing could lower the column
    // again. The session must be issued at the version the guard is about to demand.
    let pool = setup_db(&oidc_config()).await;
    let cfg = oidc_cfg();
    let client = Client::tracked(build_rocket_oidc(pool.clone()))
        .await
        .expect("valid rocket");

    let first = backend_rust::auth::oidc::provision_or_update(
        &pool,
        &cfg.issuer,
        "sub-lockout",
        None,
        None,
        None,
    )
    .await
    .expect("provision");
    assert_eq!(first.token_version, 0, "a new identity starts at version 0");

    // Minted through the same helper the callback uses, so this fails if the
    // callback's seam is wrong — not merely if the database is.
    let before = first.session_pair();
    assert_eq!(
        client
            .get("/auth/me")
            .private_cookie(before.clone())
            .dispatch()
            .await
            .status(),
        Status::Ok
    );
    assert_eq!(
        client
            .post("/auth/logout-all")
            .private_cookie(before)
            .dispatch()
            .await
            .status(),
        Status::SeeOther
    );

    // What a second login does: the callback re-provisions, then issues at whatever
    // version the row now carries.
    let again = backend_rust::auth::oidc::provision_or_update(
        &pool,
        &cfg.issuer,
        "sub-lockout",
        None,
        None,
        None,
    )
    .await
    .expect("re-provision");
    assert_eq!(
        again.token_version, 1,
        "provision reports the bumped version"
    );

    let after = again.session_pair();
    let resp = client
        .get("/auth/me")
        .private_cookie(after)
        .dispatch()
        .await;
    assert_eq!(
        resp.status(),
        Status::Ok,
        "a login after logout-all must still work; a rejection here means the session \
         was issued at a version the guard will not accept"
    );
}

#[tokio::test]
async fn a_database_failure_is_not_reported_as_a_denial() {
    // A 401 here tells the browser to send the user to the identity provider, and
    // `apiFetch` obeys — so during a transient `SQLITE_BUSY` (reachable: the read
    // endpoints write) a signed-in user is bounced to the provider, comes back, and
    // is bounced again. An outage has to look like an outage.
    let pool = setup_db(&oidc_config()).await;
    let client = Client::tracked(build_rocket_oidc(pool.clone()))
        .await
        .expect("valid rocket");
    sqlx::query("DROP TABLE users")
        .execute(&pool)
        .await
        .unwrap();

    let resp = client
        .get("/api/portfolios/")
        .private_cookie(session_cookie(DEFAULT_USER as i64, 0))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::ServiceUnavailable);
    let body: Value = resp.into_json().await.unwrap();
    assert_eq!(body["error"], "service_unavailable");
}

#[tokio::test]
async fn a_malformed_session_cookie_is_a_denial_not_a_500() {
    // The cookie is attacker-controlled bytes. Anything that is not the payload we
    // wrote must read as "no session", not as a server error and not as a panic.
    let client = oidc_client().await;
    for junk in [
        "not json at all",
        "",
        "[]",
        "{\"uid\":\"a string\",\"tv\":0,\"exp\":9999999999}",
        "{\"uid\":1,\"tv\":\"x\",\"exp\":9999999999}",
        "{\"uid\":1}",
    ] {
        let resp = client
            .get("/api/portfolios/")
            .private_cookie((backend_rust::auth::session::SESSION_COOKIE, junk))
            .dispatch()
            .await;
        assert_eq!(resp.status(), Status::Unauthorized, "for cookie {junk:?}");
    }
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

    let client = Client::tracked(build_rocket_oidc(pool.clone()))
        .await
        .unwrap();
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
    assert_eq!(
        resp.status(),
        Status::Unauthorized,
        "a bare OPTIONS is not a preflight"
    );

    let resp = client
        .options("/api/portfolios/")
        .header(Header::new("Origin", "http://localhost:5173"))
        .header(Header::new("Access-Control-Request-Method", "GET"))
        .dispatch()
        .await;
    assert_ne!(
        resp.status(),
        Status::Unauthorized,
        "a real preflight must reach CORS"
    );
    // `!= 401` alone would also be satisfied by a 500 or by the fairing never
    // running, so pin what a preflight actually is: the browser needs the method
    // list, and that is the whole purpose of the exemption.
    assert!(
        (200..300).contains(&resp.status().code),
        "a preflight should succeed, got {}",
        resp.status()
    );
    assert!(
        resp.headers()
            .get_one("Access-Control-Allow-Methods")
            .is_some(),
        "a preflight that names no methods is useless to the browser that sent it"
    );
}

// ---------------------------------------------------------------------------
// public surface
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_root_and_the_api_contract_stay_public() {
    let client = oidc_client().await;

    assert_eq!(client.get("/").dispatch().await.status(), Status::Ok);

    let resp = client.get("/api-docs/openapi.json").dispatch().await;
    assert_eq!(
        resp.status(),
        Status::Ok,
        "the contract is public by decision"
    );

    // The duplicate mount is gone; the handler's attribute still serves /api-docs.
    assert_eq!(
        client.get("/openapi.json").dispatch().await.status(),
        Status::NotFound
    );
}

#[tokio::test]
async fn auth_me_reports_the_placeholder_under_disabled_mode() {
    let pool = setup_db(&disabled_config()).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER))
        .await
        .unwrap();

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
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER))
        .await
        .unwrap();
    // No OIDC-only routes are mounted in this mode, so the route is simply absent.
    assert_eq!(
        client.get("/auth/login").dispatch().await.status(),
        Status::NotFound
    );
}
