//! Route-level behaviour for export and restore.
//!
//! Field-level validation lives in `src/backup/mod.rs`'s own unit tests; this
//! file is about what the routes do — status codes, headers, what got written.
//!
//! The request-size tests at the bottom mount their own probe route onto the rocket
//! `build_rocket` returns. The bundle cap is **per route** — a handler opens its
//! `Data` stream under `MAX_BUNDLE_BYTES` rather than a global `Limits` entry being
//! raised for one endpoint — so the probes pin both halves: the cap is exact, and
//! no global limit was touched. The lock probe after them uses the same technique
//! for the module's `503` responder, which no real route can be made to return from
//! a test.

mod common;

use common::{build_rocket, disabled_config, seed_asset, seed_portfolio, seed_transaction, setup_db};
use rocket::data::{Data, Limits, ToByteUnit};
use rocket::http::{Header, Status};
use rocket::local::asynchronous::Client;
use rocket::serde::json::Json;
use rocket::{get, post, routes};
use serde_json::{json, Value};

use backend_rust::api_routes::backup::{BackupError, lock_response};
use backend_rust::backup::{BackupConfig, MAX_BUNDLE_BYTES};

/// The session header the existing tests use to run as the placeholder user.
async fn client(pool: sqlx::SqlitePool) -> Client {
    Client::tracked(build_rocket(pool, common::DEFAULT_USER)).await.unwrap()
}

fn session() -> Header<'static> {
    let (name, value) = common::session_cookie(1, 0);
    Header::new(name, value)
}

#[tokio::test]
async fn capabilities_reports_the_limits_the_ui_must_respect() {
    let pool = setup_db(&disabled_config()).await;
    let c = client(pool).await;

    let res = c.get("/api/backup/capabilities").header(session()).dispatch().await;
    assert_eq!(res.status(), Status::Ok);
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["schema_versions"], json!([1]));
    assert_eq!(body["allow_price_restore"], json!(false), "off by default");
    assert_eq!(body["max_bytes"], json!(8 * 1024 * 1024));
    assert_eq!(body["max_rows"], json!(50_000));
}

#[tokio::test]
async fn capabilities_requires_a_session() {
    let pool = setup_db(&disabled_config()).await;
    let rocket = common::build_rocket_oidc(pool);
    let c = Client::tracked(rocket).await.unwrap();
    let res = c.get("/api/backup/capabilities").dispatch().await;
    assert_eq!(res.status(), Status::Unauthorized);
}

#[tokio::test]
async fn the_managed_backup_config_is_the_one_build_rocket_was_given() {
    // Rocket 0.5.1's `manage` panics on a type it already holds, so a
    // `BackupConfig::default()` managed inside `build_rocket` crashed `main` at
    // startup — and had it not panicked, the default would have won and
    // `BACKUP_ALLOW_PRICE_RESTORE` could never have taken effect. The config is a
    // parameter now, and this asserts the value that comes out the other side
    // through the handler: re-introducing a default-managed config fails a test
    // here instead of the release binary at boot.
    let pool = setup_db(&disabled_config()).await;
    let on = BackupConfig { allow_price_restore: true };
    let rocket = common::build_rocket_with_backup_config(pool, common::DEFAULT_USER, on);
    let c = Client::tracked(rocket).await.unwrap();

    assert!(
        c.rocket().state::<BackupConfig>().unwrap().allow_price_restore,
        "build_rocket managed something other than the config it was given"
    );

    let res = c.get("/api/backup/capabilities").header(session()).dispatch().await;
    assert_eq!(res.status(), Status::Ok);
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(
        body["allow_price_restore"], json!(true),
        "the handler read a different BackupConfig than the one passed in"
    );
}

// ---------------------------------------------------------------------------
// The request-size cap.
//
// `MAX_BUNDLE_BYTES` is 8 MiB while Rocket's own `bytes` default is 8 KiB, so a
// bundle cannot arrive through a `Vec<u8>` guard, and raising a global limit to
// suit one endpoint would widen the body ceiling of every other guard in the app.
// The pattern Tasks 4-6 use is therefore a `Data` guard opened under
// `MAX_BUNDLE_BYTES` per route. These probes stand in for that guard, mounted on
// the rocket `build_rocket` returns and outside `/api`, which is not the surface
// under test.
// ---------------------------------------------------------------------------

/// The read pattern, reported rather than answered: the byte count that arrived and
/// whether it was the whole body. Rocket never refuses a `Data` request of its own
/// accord, so turning a capped read into the module's `413` is the handler's job —
/// Tasks 4-6 must do it, and `parse_and_validate`'s own byte check behind that is
/// defense-in-depth, not the first line of defence.
#[post("/data", data = "<body>")]
async fn probe_data(body: Data<'_>) -> String {
    let capped = body
        .open(MAX_BUNDLE_BYTES.bytes())
        .into_bytes()
        .await
        .expect("opening under a limit does not fail the read");
    format!("{}:{}", capped.value.len(), capped.is_complete())
}

async fn probe_client(pool: sqlx::SqlitePool) -> Client {
    let rocket = build_rocket(pool, common::DEFAULT_USER).mount("/probe", routes![probe_data]);
    Client::tracked(rocket).await.unwrap()
}

/// A body of `len` bytes. Content is irrelevant: these tests are about size.
fn body_of(len: usize) -> String {
    "x".repeat(len)
}

#[tokio::test]
async fn a_body_of_exactly_max_bundle_bytes_arrives_whole() {
    let pool = setup_db(&disabled_config()).await;
    let c = probe_client(pool).await;

    let res = c
        .post("/probe/data")
        .body(body_of(MAX_BUNDLE_BYTES))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    assert_eq!(
        res.into_string().await.unwrap(),
        format!("{MAX_BUNDLE_BYTES}:true"),
        "a bundle of exactly the cap must reach validation whole, not be cut off"
    );
}

#[tokio::test]
async fn one_byte_more_arrives_capped_for_the_handler_to_refuse() {
    // The cap is exact, and Rocket does not refuse this request: the handler is
    // handed `MAX_BUNDLE_BYTES` with `is_complete() == false` and must answer with
    // the module's `413` rather than restoring a truncated bundle.
    let pool = setup_db(&disabled_config()).await;
    let c = probe_client(pool).await;

    let res = c
        .post("/probe/data")
        .body(body_of(MAX_BUNDLE_BYTES + 1))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok, "the probe reports the cap; the handler decides");
    assert_eq!(
        res.into_string().await.unwrap(),
        format!("{MAX_BUNDLE_BYTES}:false"),
        "the cap is not the one the handler opened with"
    );
}

// ---------------------------------------------------------------------------
// Export.
//
// The route is the only place a caller can name a portfolio id and get rows back,
// so these pin the shape of the download and the two rules that make it safe: an
// empty selection is never a bundle, and only the selected portfolios appear.
// The cross-user half of that second rule lives in `tests/isolation_test.rs`,
// because the static access gate cannot see `WHERE portfolio_id = ?`.
// ---------------------------------------------------------------------------

/// Seed: portfolio "One" with one asset and one buy; portfolio "Two" with nothing.
async fn seed_two_portfolios(pool: &sqlx::SqlitePool) -> (i32, i32) {
    let one = seed_portfolio(pool, common::DEFAULT_USER, "One", "USD").await;
    let two = seed_portfolio(pool, common::DEFAULT_USER, "Two", "EUR").await;
    let asset = seed_asset(pool, one, "AAPL", "Apple").await;
    seed_transaction(pool, asset, "BUY", 10.0, 150.0, 1.0).await;
    (one, two)
}

#[tokio::test]
async fn export_contains_only_the_selected_portfolios() {
    let pool = setup_db(&disabled_config()).await;
    let (one, two) = seed_two_portfolios(&pool).await;
    let c = client(pool).await;

    let path = format!("/api/backup/export?portfolio_ids={one}");
    let res = c.get(&path).header(session()).dispatch().await;
    assert_eq!(res.status(), Status::Ok);
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["portfolios"].as_array().unwrap().len(), 1);
    assert_eq!(body["portfolios"][0]["name"], "One");
    assert_eq!(body["portfolios"][0]["assets"][0]["symbol"], "AAPL");
    assert_eq!(body["portfolios"][0]["assets"][0]["transactions"][0]["quantity"], "10.0");
    assert!(body.get("prices").is_none(), "prices are opt-in and must be absent");
    let _ = two;
}

#[tokio::test]
async fn export_is_a_download_and_is_never_cached() {
    // A cached bundle is a silent data-loss bug: the user restores last week's
    // file believing it is today's.
    let pool = setup_db(&disabled_config()).await;
    let (one, _) = seed_two_portfolios(&pool).await;
    let c = client(pool).await;
    let path = format!("/api/backup/export?portfolio_ids={one}");
    let res = c.get(&path).header(session()).dispatch().await;
    assert_eq!(res.headers().get_one("Cache-Control"), Some("no-store"));
    let cd = res.headers().get_one("Content-Disposition").unwrap();
    assert!(cd.starts_with("attachment; filename=\"portfolio-prism-backup-"), "{cd}");
    assert!(cd.ends_with(".json\""), "{cd}");
}

#[tokio::test]
async fn an_empty_selection_is_a_422_not_an_empty_bundle() {
    // Review Focus #5. An empty bundle must never be produced, because someone
    // will eventually restore it.
    let pool = setup_db(&disabled_config()).await;
    let c = client(pool).await;
    // The whitespace case is percent-encoded on purpose: a raw space in the request
    // target is refused by Rocket's URI parser with a `400` before any route runs,
    // so it would test the parser rather than this rule. `%20` decodes to the same
    // one-character value the handler sees.
    for path in ["/api/backup/export", "/api/backup/export?portfolio_ids=", "/api/backup/export?portfolio_ids=%20"] {
        let res = c.get(path).header(session()).dispatch().await;
        assert_eq!(res.status(), Status::UnprocessableEntity, "{path}");
    }
}

#[tokio::test]
async fn a_malformed_id_list_is_a_422() {
    let pool = setup_db(&disabled_config()).await;
    let c = client(pool).await;
    let res = c.get("/api/backup/export?portfolio_ids=1,abc").header(session()).dispatch().await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
}

#[tokio::test]
async fn a_duplicate_id_is_exported_once() {
    let pool = setup_db(&disabled_config()).await;
    let (one, _) = seed_two_portfolios(&pool).await;
    let c = client(pool).await;
    let path = format!("/api/backup/export?portfolio_ids={one},{one}");
    let res = c.get(&path).header(session()).dispatch().await;
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["portfolios"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn no_global_body_limit_is_raised_for_the_bundle() {
    // The other half of the ruling: because the cap is per route, every other
    // guard in the app still runs under Rocket's own defaults.
    let pool = setup_db(&disabled_config()).await;
    let rocket = build_rocket(pool, common::DEFAULT_USER)
        .ignite()
        .await
        .expect("rocket ignites");
    let limits = &rocket.config().limits;

    assert_eq!(limits.get("bytes"), Some(Limits::BYTES));
    assert_eq!(limits.get("json"), Some(Limits::JSON));
    assert_eq!(
        limits.get("data"),
        None,
        "no global `data` limit: the bundle cap belongs to the route that needs it"
    );
}

// ---------------------------------------------------------------------------
// The lock responder.
//
// `DbFailure::Lock` is the one failure this module cannot provoke over HTTP from a
// test: it needs the database to answer `SQLITE_BUSY`/`SQLITE_LOCKED`, and a
// portfolio cannot delete itself between the handler's `require` and its read. So
// the responder is pinned the way the size cap above is — the module's own value,
// mounted on a probe route outside `/api`, asserted as what a client receives. The
// real route reaches this exact code through `lock_response()` in
// `api_routes::backup::export`, which is the only thing that turns a `DbFailure`
// into a response.
// ---------------------------------------------------------------------------

/// The module's lock failure, answered verbatim.
#[get("/lock")]
async fn probe_lock() -> Result<Json<Value>, BackupError> {
    Err(lock_response())
}

#[tokio::test]
async fn a_lock_is_a_503_that_says_when_to_retry() {
    // A `503` without `Retry-After` tells a client nothing about whether the
    // failure was permanent, and the header can only be checked on a real response.
    let pool = setup_db(&disabled_config()).await;
    let rocket = build_rocket(pool, common::DEFAULT_USER).mount("/probe", routes![probe_lock]);
    let c = Client::tracked(rocket).await.unwrap();

    let res = c.get("/probe/lock").dispatch().await;
    assert_eq!(res.status(), Status::ServiceUnavailable);
    let retry = res.headers().get_one("Retry-After").expect("a lock must say when to retry");
    assert!(retry.parse::<u32>().is_ok(), "Retry-After must be a number of seconds: {retry:?}");
    let body: Value = res.into_json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("busy"), "{body}");
}
