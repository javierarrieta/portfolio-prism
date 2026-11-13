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

use common::{
    build_rocket, disabled_config, seed_asset, seed_daily_prices, seed_portfolio, seed_transaction,
    setup_db,
};
use rocket::data::{Data, Limits, ToByteUnit};
use rocket::http::{Header, Status};
use rocket::local::asynchronous::Client;
use rocket::serde::json::Json;
use rocket::{get, post, routes};
use serde_json::{json, Value};

use backend_rust::api_routes::backup::{BackupError, lock_response};
use backend_rust::backup::export::{self as backup_export, DbFailure};
use backend_rust::backup::restore::{self as backup_restore, ApplyError, Mode};
use backend_rust::backup::{BackupConfig, MAX_BUNDLE_BYTES, MAX_BUNDLE_ROWS, parse_and_validate};

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

/// Price rows for two symbols, only one of which `seed_two_portfolios` puts in the
/// selection. The unheld one is what makes "prices are for the selection, not for the
/// whole `historical_prices` table" a checked claim rather than an assumption.
async fn seed_two_symbols(pool: &sqlx::SqlitePool) {
    seed_daily_prices(pool, "AAPL", "2024-01-01", "2024-01-03", "150.00").await;
    seed_daily_prices(pool, "MSFT", "2024-01-01", "2024-01-02", "90.00").await;
}

#[tokio::test]
async fn every_on_spelling_includes_only_the_prices_the_selection_holds() {
    // Rocket's own `bool` form field accepts neither `1` (what the UI sends) nor a
    // value it would report as an error, so these spellings used to arrive as `None`
    // and answer a 200 with the requested section missing.
    let pool = setup_db(&disabled_config()).await;
    let (one, two) = seed_two_portfolios(&pool).await;
    seed_two_symbols(&pool).await;
    let c = client(pool).await;

    for spelling in ["1", "true", "yes", "TRUE"] {
        let path = format!("/api/backup/export?portfolio_ids={one}&include_prices={spelling}");
        let res = c.get(&path).header(session()).dispatch().await;
        assert_eq!(res.status(), Status::Ok, "{spelling}");
        let body: Value = res.into_json().await.unwrap();
        let prices = body["prices"]
            .as_array()
            .unwrap_or_else(|| panic!("include_prices={spelling} must include prices: {body}"));
        // Three days of AAPL, and no MSFT: the selection holds AAPL and nothing else.
        assert_eq!(prices.len(), 3, "include_prices={spelling}: {body}");
        assert!(prices.iter().all(|p| p["symbol"] == "AAPL"), "{spelling}: {body}");
        assert_eq!(prices[0]["date"], "2024-01-01", "{spelling}");
        assert_eq!(prices[0]["close_price"], "150.00", "{spelling}");
        assert!(
            !body.to_string().contains("MSFT"),
            "include_prices={spelling} leaked a symbol the selection does not hold: {body}"
        );
    }

    // The empty portfolio asks for prices and gets none: the section follows the
    // selection's holdings, not the whole `historical_prices` table.
    let path = format!("/api/backup/export?portfolio_ids={two}&include_prices=1");
    let res = c.get(&path).header(session()).dispatch().await;
    assert_eq!(res.status(), Status::Ok);
    let body: Value = res.into_json().await.unwrap();
    assert!(
        body.get("prices").is_none(),
        "a portfolio with no assets has no prices to include: {body}"
    );
}

#[tokio::test]
async fn every_off_spelling_omits_the_price_section() {
    // The mirror half: an explicit no must stay a no, and an omitted section is
    // skipped rather than serialised as `[]`.
    let pool = setup_db(&disabled_config()).await;
    let (one, _) = seed_two_portfolios(&pool).await;
    seed_two_symbols(&pool).await;
    let c = client(pool).await;

    for spelling in ["0", "false", "no"] {
        let path = format!("/api/backup/export?portfolio_ids={one}&include_prices={spelling}");
        let res = c.get(&path).header(session()).dispatch().await;
        assert_eq!(res.status(), Status::Ok, "{spelling}");
        let body: Value = res.into_json().await.unwrap();
        assert!(body.get("prices").is_none(), "include_prices={spelling}: {body}");
    }
}

#[tokio::test]
async fn an_unrecognised_include_prices_is_a_422_not_a_quiet_omission() {
    // A value in neither accepted set is the caller's error, said out loud. `on` and
    // `off` are rocket's spellings, not this flag's: one rule for both flags means the
    // accepted set is `1/true/yes` and `0/false/no` here and in the environment.
    let pool = setup_db(&disabled_config()).await;
    let (one, _) = seed_two_portfolios(&pool).await;
    let c = client(pool).await;

    for spelling in ["maybe", "2", "on", "off", "truest", ""] {
        let path = format!("/api/backup/export?portfolio_ids={one}&include_prices={spelling}");
        let res = c.get(&path).header(session()).dispatch().await;
        assert_eq!(res.status(), Status::UnprocessableEntity, "include_prices={spelling}");
        let body: Value = res.into_json().await.unwrap();
        assert!(
            body["error"].as_str().unwrap().contains("include_prices"),
            "include_prices={spelling}: {body}"
        );
    }
}

#[tokio::test]
async fn a_repeated_query_parameter_is_refused_not_half_read() {
    // One class, both parameters: rocket collapses a repeated `Option` field to
    // `None` (`ErrorKind::Duplicate` swallowed by `Option::finalize`), so a duplicate
    // reaches the handler as if it had not been sent. For `portfolio_ids` that reads as
    // "no selection"; for `include_prices` it reads as "off", which is the same silent
    // omission as a value that fails to parse. Both are refused outright.
    let pool = setup_db(&disabled_config()).await;
    let (one, two) = seed_two_portfolios(&pool).await;
    // Seeded so the duplicate's stakes are real: the failure refused here is a 200 with
    // a section missing, and there has to be a section that could go missing.
    seed_two_symbols(&pool).await;
    let c = client(pool).await;

    let path = format!("/api/backup/export?portfolio_ids={one}&portfolio_ids={two}");
    let res = c.get(&path).header(session()).dispatch().await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
    let body: Value = res.into_json().await.unwrap();
    // Not the "name at least one portfolio" message: the caller named two.
    assert!(body["error"].as_str().unwrap().contains("only once"), "{body}");

    // The refusal is about the repetition, not about naming several: the documented
    // comma form still exports both.
    let path = format!("/api/backup/export?portfolio_ids={one},{two}");
    let res = c.get(&path).header(session()).dispatch().await;
    assert_eq!(res.status(), Status::Ok);
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["portfolios"].as_array().unwrap().len(), 2);

    // `include_prices` twice, whether the two values agree or not: a caller who asked
    // for prices must never be handed a bundle without them.
    for second in ["false", "0", "true"] {
        let path =
            format!("/api/backup/export?portfolio_ids={one}&include_prices=1&include_prices={second}");
        let res = c.get(&path).header(session()).dispatch().await;
        assert_eq!(res.status(), Status::UnprocessableEntity, "second value {second}");
        let body: Value = res.into_json().await.unwrap();
        assert!(
            body["error"].as_str().unwrap().contains("include_prices"),
            "second value {second}: {body}"
        );
        assert!(
            body["error"].as_str().unwrap().contains("only once"),
            "second value {second}: {body}"
        );
    }
}

/// `n` transactions on one asset in a single statement. A loop of 50k inserts would
/// dominate the suite; a recursive CTE is one round trip.
async fn seed_many_transactions(pool: &sqlx::SqlitePool, asset_id: i32, n: usize) {
    sqlx::query(
        "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) \
         WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt WHERE x < ?) \
         SELECT ?, 'BUY', '1', '10.0', '0.0', '2024-06-15T00:00:00+00:00' FROM cnt",
    )
    .bind(n as i64)
    .bind(asset_id)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn a_selection_over_the_row_ceiling_is_refused_by_the_route() {
    // The ceiling only protects anyone if the *route* refuses: a function that can
    // name the cap is not the same as a caller being told, and a bundle over it is one
    // restore will reject after the download.
    let pool = setup_db(&disabled_config()).await;
    let (one, _) = seed_two_portfolios(&pool).await;
    let asset = seed_asset(&pool, one, "AAPL", "Apple").await;
    seed_many_transactions(&pool, asset, MAX_BUNDLE_ROWS).await;
    let c = client(pool).await;

    let path = format!("/api/backup/export?portfolio_ids={one}");
    let res = c.get(&path).header(session()).dispatch().await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
    let body: Value = res.into_json().await.unwrap();
    let err = body["error"].as_str().unwrap();
    assert!(err.contains(&MAX_BUNDLE_ROWS.to_string()), "{err}");
    assert!(err.contains("export fewer portfolios"), "{err}");
}

#[tokio::test]
async fn a_bundle_over_the_byte_ceiling_is_refused_by_the_route() {
    // `row_count` counts portfolios, assets and transactions — never price rows — so
    // the row ceiling alone lets a dated export through that restore then refuses on
    // size. The serialised bundle is checked against the same byte ceiling the upload
    // enforces. A long asset name is the cheapest way to be big on this axis; what is
    // under test is the check, not which field made the bundle large.
    let pool = setup_db(&disabled_config()).await;
    let (one, _) = seed_two_portfolios(&pool).await;
    seed_asset(&pool, one, "BIG", &"x".repeat(MAX_BUNDLE_BYTES + 1)).await;
    let c = client(pool).await;

    let path = format!("/api/backup/export?portfolio_ids={one}");
    let res = c.get(&path).header(session()).dispatch().await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
    let body: Value = res.into_json().await.unwrap();
    let err = body["error"].as_str().unwrap();
    assert!(err.contains("byte"), "{err}");
    assert!(err.contains(&MAX_BUNDLE_BYTES.to_string()), "{err}");
    assert!(err.contains("export fewer portfolios"), "{err}");
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

// ---------------------------------------------------------------------------
// The database-failure half of the `503`.
//
// The handler's `DbFailure::Lock => lock_response()` arm cannot be driven over
// HTTP from a test: `require` reads the same tables `build` does, so anything that
// makes the database refuse a read denies the request at `require` (a 500 from
// `access.rs`) before the arm is reached. Each half is pinned where it can be:
// `build()` producing `Lock` below, and the response that `Lock` becomes in
// `a_lock_is_a_503_that_says_when_to_retry` above.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_exhausted_pool_is_a_lock_not_an_outage() {
    // Under SQLite a reader rarely lives long enough to meet `SQLITE_BUSY`; the
    // likelier way to be kept out is every connection in the pool being checked out.
    // That is just as retryable, so it must arrive as the same failure the busy
    // timeout does rather than a bare 500 with no `Retry-After`.
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_millis(50))
        .connect(":memory:")
        .await
        .unwrap();
    backend_rust::init_db(&pool, &disabled_config()).await.unwrap();
    let pid = seed_portfolio(&pool, common::DEFAULT_USER, "One", "USD").await;

    // The pool's only connection, held: nothing else can reach the database.
    let held = pool.acquire().await.unwrap();
    let err = backup_export::build(&pool, &[pid], false)
        .await
        .expect_err("a pool with no free connection cannot answer");
    assert!(matches!(err, DbFailure::Lock), "expected Lock, got {err:?}");
    assert_eq!(err.status(), Status::ServiceUnavailable);

    // And it was the exhaustion: with the connection back, the same call answers.
    drop(held);
    let bundle = backup_export::build(&pool, &[pid], false).await.unwrap();
    assert_eq!(bundle.portfolios.len(), 1);
}

// ---------------------------------------------------------------------------
// Preview.
//
// Preview is the step a user must pass through before anything is written, so it
// carries three obligations no other route here has: it must answer every kind of
// bad input with a status the caller can act on (a bundle is user-authored text, so
// "not JSON" is a 422 and never a 500), it must describe the caller's own state and
// nothing else (a bundle names portfolios by bundle-local refs, which are never
// database ids), and above all it must write nothing — including the previews that
// end in a refusal, and the one mode whose whole subject is deletion.
// ---------------------------------------------------------------------------

/// The smallest bundle that passes validation: one portfolio, one asset, one
/// transaction. `name` is the only string in it a test can point at.
fn bundle_json(name: &str) -> String {
    json!({
        "format": "portfolio-prism-backup",
        "schema_version": 1,
        "created_at": "2026-11-13T00:00:00Z",
        "portfolios": [{
            "ref": "p1", "name": name, "description": null,
            "currency": "USD", "base_currency": "USD",
            "assets": [{
                "ref": "a1", "symbol": "AAPL", "name": "Apple", "asset_type": "STOCK",
                "sector": null, "currency": "USD", "isin": null,
                "transactions": [{ "type": "BUY", "quantity": "10.0", "price": "150.00",
                                   "fee": "1.00", "date": "2024-01-15T00:00:00Z" }]
            }]
        }]
    })
    .to_string()
}

/// The same bundle as a mutable `Value`, for a test that needs one field changed.
///
/// Parsed rather than wrapped: `json!(bundle_json(..))` turns the *string* into a
/// `Value::String`, and editing a field on that panics in serde before the request is
/// ever sent — which is a confusing failure for a test whose subject is the response.
fn bundle_value(name: &str) -> Value {
    serde_json::from_str(&bundle_json(name)).expect("the fixture is JSON")
}

/// A bundle of the same shape carrying a price section: prices are flat, global,
/// and the one part of a bundle that belongs to no portfolio.
fn bundle_json_with_prices(name: &str) -> String {
    bundle_priced(name, "AAPL")
}

/// The same bundle with a caller-chosen price symbol: a test that applies the price
/// section several times needs a distinct `(symbol, date)` each time, because
/// `ON CONFLICT DO NOTHING` makes a repeat insert insert nothing.
fn bundle_priced(name: &str, symbol: &str) -> String {
    let mut b = bundle_value(name);
    b["prices"] = json!([{ "symbol": symbol, "date": "2024-01-01", "close_price": "150.00" }]);
    b.to_string()
}

#[tokio::test]
async fn preview_counts_what_replace_would_delete_and_insert() {
    let pool = setup_db(&disabled_config()).await;
    seed_two_portfolios(&pool).await;
    let c = client(pool).await;

    let res = c
        .post("/api/backup/preview?mode=replace")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle_json("Restored"))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["mode"], "replace");
    assert_eq!(body["delete"]["portfolios"], 2, "both owned portfolios go");
    assert_eq!(body["delete"]["assets"], 1);
    assert_eq!(body["delete"]["transactions"], 1);
    assert_eq!(body["insert"]["portfolios"], 1);
    assert_eq!(body["insert"]["assets"], 1);
    assert_eq!(body["insert"]["transactions"], 1);
}

#[tokio::test]
async fn preview_in_copy_mode_deletes_nothing() {
    let pool = setup_db(&disabled_config()).await;
    seed_two_portfolios(&pool).await;
    let c = client(pool).await;
    let res = c
        .post("/api/backup/preview?mode=copy")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle_json("Restored"))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["mode"], "copy");
    assert_eq!(body["delete"]["portfolios"], 0);
    assert_eq!(body["delete"]["transactions"], 0);
    // Copy still inserts: "deletes nothing" is not the same as "does nothing".
    assert_eq!(body["insert"]["portfolios"], 1);
}

#[tokio::test]
async fn preview_writes_nothing() {
    let pool = setup_db(&disabled_config()).await;
    seed_two_portfolios(&pool).await;
    let before: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM portfolios")
        .fetch_one(&pool)
        .await
        .unwrap();
    let c = client(pool.clone()).await;
    let res = c
        .post("/api/backup/preview?mode=replace")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle_json("Restored"))
        .dispatch()
        .await;
    // Asserted, not assumed: a preview that refused with a 4xx and wrote nothing
    // would pass a counts-only check for the wrong reason.
    assert_eq!(res.status(), Status::Ok);
    let after: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM portfolios")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before.0, after.0);
}

/// Every row a preview could conceivably touch, counted. `historical_prices` is in
/// the list because a bundle can carry prices and a preview that wrote them would be
/// one tenant writing the market data every other tenant's dated reports read. The
/// id sum is there because counts alone would miss a delete-and-reinsert of the same
/// number of rows.
async fn snapshot(pool: &sqlx::SqlitePool) -> Vec<(String, i64)> {
    let mut out: Vec<(String, i64)> = Vec::new();
    for table in ["portfolios", "assets", "transactions", "historical_prices"] {
        let (n,): (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(pool)
            .await
            .unwrap();
        out.push((table.to_string(), n));
    }
    let (sum,): (i64,) = sqlx::query_as("SELECT COALESCE(SUM(id), 0) FROM portfolios")
        .fetch_one(pool)
        .await
        .unwrap();
    out.push(("portfolios.id_sum".to_string(), sum));
    out
}

#[tokio::test]
async fn no_preview_writes_anything_whether_it_succeeds_or_refuses() {
    // `preview_writes_nothing` is the happy path. This is the whole surface: the
    // preview that would replace everything, the one whose bundle is invalid, the
    // one from a newer build, the one that is not JSON, and the one too big to read.
    // A refusal that wrote is the worst possible failure here, because the user has
    // been told the answer is "look at the plan first".
    let pool = setup_db(&disabled_config()).await;
    seed_two_portfolios(&pool).await;
    seed_daily_prices(&pool, "AAPL", "2024-01-01", "2024-01-02", "150.00").await;
    let before = snapshot(&pool).await;
    let c = client(pool.clone()).await;

    let mut newer = bundle_value("Restored");
    newer["schema_version"] = json!(2);
    let mut invalid = bundle_value("Restored");
    invalid["portfolios"][0]["assets"][0]["transactions"][0]["price"] = json!("not-a-number");

    let previews: Vec<(&str, &str, String, Status)> = vec![
        ("a replace of everything", "replace", bundle_json("Restored"), Status::Ok),
        ("a copy with prices", "copy", bundle_json_with_prices("Restored"), Status::Ok),
        ("an invalid bundle", "copy", invalid.to_string(), Status::UnprocessableEntity),
        ("a newer schema_version", "copy", newer.to_string(), Status::UnprocessableEntity),
        ("not JSON", "copy", "not json".to_string(), Status::UnprocessableEntity),
        ("over the byte cap", "copy", body_of(MAX_BUNDLE_BYTES + 1), Status::PayloadTooLarge),
    ];
    for (what, mode, body, expected) in previews {
        let path = format!("/api/backup/preview?mode={mode}");
        let res = c
            .post(&path)
            .header(session())
            .header(rocket::http::ContentType::JSON)
            .body(body)
            .dispatch()
            .await;
        // Each case answers what it is meant to, so a case that silently stopped
        // exercising its own path (an "invalid" bundle that validated, say) is caught
        // here rather than by the snapshot below.
        assert_eq!(res.status(), expected, "{what} via {path}");
    }

    assert_eq!(snapshot(&pool).await, before, "a preview must write nothing");
}

#[tokio::test]
async fn an_unknown_mode_is_a_422() {
    let pool = setup_db(&disabled_config()).await;
    let c = client(pool).await;
    let res = c
        .post("/api/backup/preview?mode=merge")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle_json("x"))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
    let body: Value = res.into_json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("mode"), "{body}");
}

#[tokio::test]
async fn an_absent_mode_is_a_422_naming_the_two_that_exist() {
    // There is no default mode. Guessing one would answer a question the caller did
    // not ask, and the two answers differ by "delete everything you own".
    let pool = setup_db(&disabled_config()).await;
    let c = client(pool).await;
    for path in ["/api/backup/preview", "/api/backup/preview?mode="] {
        let res = c
            .post(path)
            .header(session())
            .header(rocket::http::ContentType::JSON)
            .body(bundle_json("x"))
            .dispatch()
            .await;
        assert_eq!(res.status(), Status::UnprocessableEntity, "{path}");
        let body: Value = res.into_json().await.unwrap();
        let err = body["error"].as_str().unwrap();
        assert!(err.contains("copy") && err.contains("replace"), "{path}: {err}");
    }
}

#[tokio::test]
async fn a_repeated_mode_is_refused_not_half_read() {
    // The same class `a_repeated_query_parameter_is_refused_not_half_read` pins on
    // export: rocket collapses a repeated `Option` field into `None`, so
    // `?mode=copy&mode=replace` would otherwise arrive as "no mode at all" and be
    // answered as a missing parameter rather than as the contradiction it is.
    let pool = setup_db(&disabled_config()).await;
    let c = client(pool).await;
    let res = c
        .post("/api/backup/preview?mode=copy&mode=replace")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle_json("x"))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
    let body: Value = res.into_json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("only once"), "{body}");
}

#[tokio::test]
async fn a_non_json_upload_is_a_422_not_a_500() {
    // Review Focus #4.
    let pool = setup_db(&disabled_config()).await;
    let c = client(pool).await;
    let res = c
        .post("/api/backup/preview?mode=copy")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body("not json")
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
    let body: Value = res.into_json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("not valid JSON"));
}

#[tokio::test]
async fn a_bundle_from_a_newer_build_is_refused_by_preview() {
    // Review Focus #2: preview is where the user learns it, before anything is written.
    let pool = setup_db(&disabled_config()).await;
    let c = client(pool).await;
    let mut b = bundle_value("x");
    b["schema_version"] = json!(2);
    let res = c
        .post("/api/backup/preview?mode=copy")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(b.to_string())
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["accepted"], json!([1]));
}

#[tokio::test]
async fn a_bundle_that_fails_validation_names_its_paths() {
    // The plan is the user's last look before a write, so a refusal has to say which
    // field is wrong — the same body shape the restore route will produce, so a
    // client parses validation failures the same way whichever route answered.
    let pool = setup_db(&disabled_config()).await;
    let c = client(pool).await;
    let mut b = bundle_value("x");
    b["portfolios"][0]["assets"][0]["transactions"][0]["price"] = json!("not-a-number");
    let res = c
        .post("/api/backup/preview?mode=copy")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(b.to_string())
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["error"], "the bundle failed validation");
    assert_eq!(
        body["errors"][0]["path"],
        "portfolios[0].assets[0].transactions[0].price"
    );
}

#[tokio::test]
async fn an_upload_over_the_byte_cap_is_the_module_s_413() {
    // Rocket never refuses a `Data` upload of its own accord, so the ceiling is only
    // real if the route that opens the stream enforces it. The probes above pin the
    // read; this pins the route doing the conversion.
    let pool = setup_db(&disabled_config()).await;
    let c = client(pool).await;
    let res = c
        .post("/api/backup/preview?mode=copy")
        .header(session())
        .body(body_of(MAX_BUNDLE_BYTES + 1))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::PayloadTooLarge);
    let body: Value = res.into_json().await.unwrap();
    let err = body["error"].as_str().unwrap();
    assert!(err.contains(&MAX_BUNDLE_BYTES.to_string()), "{err}");
}

#[tokio::test]
async fn preview_reports_the_price_section_and_warns_about_a_replace() {
    // The warnings are the two things a user is least likely to have thought of:
    // that `replace` deletes portfolios the bundle says nothing about, and that a
    // bundle carrying prices is carrying shared market data rather than their own.
    let pool = setup_db(&disabled_config()).await;
    seed_two_portfolios(&pool).await;
    let c = client(pool).await;

    let res = c
        .post("/api/backup/preview?mode=replace")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle_json_with_prices("Restored"))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["schema_version"], 1);
    assert_eq!(body["prices"], 1);
    let warnings = body["warnings"].as_array().unwrap();
    assert!(
        warnings.iter().any(|w| w.as_str().unwrap().contains("replace will delete")),
        "{warnings:?}"
    );
    assert!(
        warnings.iter().any(|w| w.as_str().unwrap().contains("price history")),
        "{warnings:?}"
    );

    // Copy with no prices: nothing to warn about, and an empty list rather than a
    // missing key, so the UI renders the same component either way.
    let res = c
        .post("/api/backup/preview?mode=copy")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle_json("Restored"))
        .dispatch()
        .await;
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["warnings"], json!([]), "{body}");
    assert_eq!(body["prices"], 0);
}

#[tokio::test]
async fn a_preview_describes_only_the_caller_s_own_rows() {
    // Cross-tenant: a bundle names portfolios by bundle-local refs, which are labels
    // and never database ids, so a bundle cannot address another user's rows — and
    // the counts it is answered with are its own. `replace` describing a foreign
    // portfolio would be a promise to delete what the caller does not own.
    let pool = setup_db(&disabled_config()).await;
    seed_two_portfolios(&pool).await;
    let other = common::seed_user(&pool, "user-b").await;
    let theirs = seed_portfolio(&pool, other, "Theirs", "USD").await;
    let their_asset = seed_asset(&pool, theirs, "BBB", "Asset B").await;
    seed_transaction(&pool, their_asset, "SELL", 1.0, 2.0, 0.0).await;

    // A bundle that names the foreign portfolio's name and reuses its id-shaped label,
    // which is exactly the thing a hand-edited file would try.
    let foreign = json!({
        "format": "portfolio-prism-backup", "schema_version": 1,
        "created_at": "2026-11-13T00:00:00Z",
        "portfolios": [{
            "ref": format!("p{theirs}"), "name": "Theirs", "description": null,
            "currency": "USD", "base_currency": "USD",
            "assets": [{
                "ref": format!("a{their_asset}"), "symbol": "BBB", "name": "Asset B",
                "asset_type": "STOCK", "sector": null, "currency": "USD", "isin": null,
                "transactions": []
            }]
        }]
    })
    .to_string();

    let c = client(pool).await; // runs as DEFAULT_USER, who owns none of B's rows
    let res = c
        .post("/api/backup/preview?mode=replace")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(foreign)
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let body: Value = res.into_json().await.unwrap();
    // The caller's own two portfolios, and none of B's: B's one asset and one
    // transaction are absent from every count.
    assert_eq!(body["delete"]["portfolios"], 2, "{body}");
    assert_eq!(body["delete"]["assets"], 1, "{body}");
    assert_eq!(body["delete"]["transactions"], 1, "{body}");
    // And the plan is counts, not rows: nothing from either side's data appears.
    let text = body.to_string();
    assert!(!text.contains("Asset B") && !text.contains("Theirs"), "{text}");
}

#[tokio::test]
async fn preview_requires_a_session() {
    let pool = setup_db(&disabled_config()).await;
    let rocket = common::build_rocket_oidc(pool);
    let c = Client::tracked(rocket).await.unwrap();
    let res = c
        .post("/api/backup/preview?mode=copy")
        .body(bundle_json("x"))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Unauthorized);
}

// ---------------------------------------------------------------------------
// Restore — copy mode.
//
// Export hands back what the caller already had; restore is the first backup route
// that *writes*, so everything it does has to be proved against the database rather
// than against a response body. Four things are pinned here that a naive
// implementation gets wrong: the rows land beside the caller's existing ones rather
// than replacing them; every parent/child link is rebuilt from freshly allocated ids
// (a bundle `ref` is a label and must never become a database id); a failure
// anywhere leaves the database byte-for-byte as it was; and the caller owns every row
// written, whatever the file claims.
// ---------------------------------------------------------------------------

/// Row counts for the three tables a restore writes.
async fn rows(pool: &sqlx::SqlitePool) -> (i64, i64, i64) {
    let p: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM portfolios").fetch_one(pool).await.unwrap();
    let a: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM assets").fetch_one(pool).await.unwrap();
    let t: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM transactions").fetch_one(pool).await.unwrap();
    (p.0, a.0, t.0)
}

#[tokio::test]
async fn copy_mode_adds_alongside_and_assigns_new_ids() {
    let pool = setup_db(&disabled_config()).await;
    let (one, _) = seed_two_portfolios(&pool).await;
    let before = rows(&pool).await;
    let c = client(pool.clone()).await;

    let res = c.post("/api/backup/restore?mode=copy").header(session())
        .header(rocket::http::ContentType::JSON).body(bundle_json("Restored")).dispatch().await;
    assert_eq!(res.status(), Status::Ok);
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["inserted"]["portfolios"], 1);
    assert_eq!(body["inserted"]["transactions"], 1);

    let after = rows(&pool).await;
    assert_eq!(after, (before.0 + 1, before.1 + 1, before.2 + 1), "copy touches nothing");

    // The new portfolio is owned by the caller, and its asset and transaction are
    // reachable through it — that is the ref rewiring, not merely the row counts.
    let new_id = body["portfolio_ids"][0].as_i64().unwrap();
    assert_ne!(new_id, one as i64, "a bundle ref must never become a database id");
    let chain: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM transactions WHERE asset_id IN \
         (SELECT id FROM assets WHERE portfolio_id = ?)")
        .bind(new_id as i32).fetch_one(&pool).await.unwrap();
    assert_eq!(chain.0, 1, "the transaction must hang off the new asset under the new portfolio");

    let owner: (Option<i64>,) = sqlx::query_as("SELECT owner_id FROM portfolios WHERE id = ?")
        .bind(new_id as i32).fetch_one(&pool).await.unwrap();
    assert_eq!(owner.0, Some(common::DEFAULT_USER as i64));
}

#[tokio::test]
async fn a_round_trip_through_export_and_restore_is_lossless() {
    // The test that proves the format is sufficient — what backup features most
    // often fail at.
    let source = setup_db(&disabled_config()).await;
    let (one, _) = seed_two_portfolios(&source).await;
    let exported: String = {
        let c = Client::tracked(build_rocket(source.clone(), common::DEFAULT_USER)).await.unwrap();
        let path = format!("/api/backup/export?portfolio_ids={one}");
        let res = c.get(&path).header(session()).dispatch().await;
        res.into_string().await.unwrap()
    };

    let target = setup_db(&disabled_config()).await;
    let c = Client::tracked(build_rocket(target.clone(), common::DEFAULT_USER)).await.unwrap();
    let res = c.post("/api/backup/restore?mode=copy").header(session())
        .header(rocket::http::ContentType::JSON).body(exported.clone()).dispatch().await;
    assert_eq!(res.status(), Status::Ok, "{:?}", res.into_string().await);

    // Re-export from the target and compare, ignoring the informational timestamp
    // and the ids that are target-local by construction.
    let new_id: (i32,) = sqlx::query_as("SELECT id FROM portfolios WHERE name = 'One'")
        .fetch_one(&target).await.unwrap();
    let path = format!("/api/backup/export?portfolio_ids={}", new_id.0);
    let res = c.get(&path).header(session()).dispatch().await;
    let mut reexported: Value = res.into_json().await.unwrap();
    let mut original: Value = serde_json::from_str(&exported).unwrap();
    // `ref` is bundle-local and `created_at` is informational; both are the only
    // bytes in a bundle that are allowed to differ across a round trip.
    for v in [&mut original, &mut reexported] {
        v["created_at"] = json!("");
        v["portfolios"][0]["ref"] = json!("");
        v["portfolios"][0]["assets"][0]["ref"] = json!("");
    }
    assert_eq!(original, reexported);
}

#[tokio::test]
async fn a_restored_portfolio_produces_the_same_tax_result() {
    // Proves the rewiring is semantically right, not just present: the tax engine
    // reaches transactions through assets through portfolios.
    let source = setup_db(&disabled_config()).await;
    let (one, _) = seed_two_portfolios(&source).await;
    let c = Client::tracked(build_rocket(source.clone(), common::DEFAULT_USER)).await.unwrap();
    let path = format!("/api/portfolios/{one}/tax-summary");
    let original: Value = c.get(&path)
        .header(session()).dispatch().await.into_json().await.unwrap();

    // The bundle has to come from the source, which is the side that owns portfolio
    // `one`; the target owns nothing yet.
    let exported: String = {
        let sc = Client::tracked(build_rocket(source, common::DEFAULT_USER)).await.unwrap();
        let path = format!("/api/backup/export?portfolio_ids={one}");
        sc.get(&path).header(session()).dispatch().await.into_string().await.unwrap()
    };
    let target = setup_db(&disabled_config()).await;
    let c = Client::tracked(build_rocket(target, common::DEFAULT_USER)).await.unwrap();
    let res = c.post("/api/backup/restore?mode=copy").header(session())
        .header(rocket::http::ContentType::JSON).body(exported).dispatch().await;
    let body: Value = res.into_json().await.unwrap();
    let new_id = body["portfolio_ids"][0].as_i64().unwrap() as i32;

    let path = format!("/api/portfolios/{new_id}/tax-summary");
    let restored: Value = c.get(&path)
        .header(session()).dispatch().await.into_json().await.unwrap();
    assert_eq!(original, restored);
}

#[tokio::test]
async fn a_bundle_with_no_portfolios_is_refused_by_restore() {
    // Review Focus #5, on the write path this time.
    let pool = setup_db(&disabled_config()).await;
    seed_two_portfolios(&pool).await;
    let before = rows(&pool).await;
    let c = client(pool.clone()).await;
    let empty = json!({ "format": "portfolio-prism-backup", "schema_version": 1,
        "created_at": "2026-11-13T00:00:00Z", "portfolios": [] }).to_string();
    let res = c.post("/api/backup/restore?mode=replace").header(session())
        .header(rocket::http::ContentType::JSON).body(empty).dispatch().await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
    assert_eq!(rows(&pool).await, before, "an empty bundle must never be able to wipe a user");
}

// ---------------------------------------------------------------------------
// The `apply_prices` parameter.
//
// The same class of bug `include_prices` on export had twice: rocket's `Option`
// query field silently swallows both an unparseable value and a duplicated
// parameter, and on this route both collapses read as "prices off" — a 200 that
// quietly did something other than what was asked. The rule is the server flag's
// rule (`1/true/yes`, `0/false/no`), and asking for prices the server forbids is a
// 422 rather than a shrug.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn every_apply_prices_on_spelling_applies_the_price_section() {
    let pool = setup_db(&disabled_config()).await;
    let on = BackupConfig { allow_price_restore: true };
    let c = Client::tracked(common::build_rocket_with_backup_config(
        pool.clone(),
        common::DEFAULT_USER,
        on,
    ))
    .await
    .unwrap();

    // A distinct symbol per spelling: two rows for one `(symbol, date)` is one row,
    // so a repeated symbol would make three of these four calls answer exactly like
    // a refusal.
    for (spelling, symbol) in [("1", "AAPL"), ("true", "MSFT"), ("yes", "SPY"), ("TRUE", "NVDA")] {
        let path = format!("/api/backup/restore?mode=copy&apply_prices={spelling}");
        let res = c
            .post(&path)
            .header(session())
            .header(rocket::http::ContentType::JSON)
            .body(bundle_priced("Priced", symbol))
            .dispatch()
            .await;
        assert_eq!(res.status(), Status::Ok, "apply_prices={spelling}");
        let body: Value = res.into_json().await.unwrap();
        assert_eq!(body["prices_inserted"], 1, "apply_prices={spelling}: {body}");
    }

    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM historical_prices")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 4, "one row per accepted on-spelling, no more and no less");
}

#[tokio::test]
async fn every_apply_prices_off_spelling_and_an_absent_parameter_write_no_prices() {
    // The mirror half, including the default: an absent parameter means off, because
    // writing shared `historical_prices` is opt-in and never a guess.
    let pool = setup_db(&disabled_config()).await;
    let on = BackupConfig { allow_price_restore: true };
    let c = Client::tracked(common::build_rocket_with_backup_config(
        pool.clone(),
        common::DEFAULT_USER,
        on,
    ))
    .await
    .unwrap();

    let mut paths: Vec<String> = ["0", "false", "no"]
        .iter()
        .map(|s| format!("/api/backup/restore?mode=copy&apply_prices={s}"))
        .collect();
    paths.push("/api/backup/restore?mode=copy".to_string());

    for path in paths {
        let res = c
            .post(&path)
            .header(session())
            .header(rocket::http::ContentType::JSON)
            .body(bundle_json_with_prices("Priced"))
            .dispatch()
            .await;
        assert_eq!(res.status(), Status::Ok, "{path}");
        let body: Value = res.into_json().await.unwrap();
        assert_eq!(body["prices_inserted"], 0, "{path}: {body}");
        // The portfolios still restored: "no prices" is not "no restore".
        assert_eq!(body["inserted"]["portfolios"], 1, "{path}: {body}");
    }

    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM historical_prices")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0, "an off spelling must not write market data");
}

#[tokio::test]
async fn an_unrecognised_apply_prices_is_a_422_not_a_quiet_ignore() {
    let pool = setup_db(&disabled_config()).await;
    let on = BackupConfig { allow_price_restore: true };
    let c = Client::tracked(common::build_rocket_with_backup_config(
        pool,
        common::DEFAULT_USER,
        on,
    ))
    .await
    .unwrap();

    for spelling in ["maybe", "2", "on", "off", "truest", ""] {
        let path = format!("/api/backup/restore?mode=copy&apply_prices={spelling}");
        let res = c
            .post(&path)
            .header(session())
            .header(rocket::http::ContentType::JSON)
            .body(bundle_json_with_prices("Priced"))
            .dispatch()
            .await;
        assert_eq!(res.status(), Status::UnprocessableEntity, "apply_prices={spelling}");
        let body: Value = res.into_json().await.unwrap();
        assert!(
            body["error"].as_str().unwrap().contains("apply_prices"),
            "apply_prices={spelling}: {body}"
        );
    }
}

#[tokio::test]
async fn a_repeated_restore_parameter_is_refused_not_half_read() {
    // Both parameters, for the reason `only_once` documents: a repeated `Option`
    // field arrives as `None`, and on this route `None` means "no mode" for one and
    // "do not touch prices" for the other. Neither is what the caller asked.
    let pool = setup_db(&disabled_config()).await;
    let c = client(pool).await;

    let res = c
        .post("/api/backup/restore?mode=copy&mode=replace")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle_json("Restored"))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
    let body: Value = res.into_json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("only once"), "{body}");

    let res = c
        .post("/api/backup/restore?mode=copy&apply_prices=1&apply_prices=0")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle_json_with_prices("Restored"))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
    let body: Value = res.into_json().await.unwrap();
    let err = body["error"].as_str().unwrap();
    assert!(err.contains("only once") && err.contains("apply_prices"), "{body}");
}

#[tokio::test]
async fn asking_to_apply_prices_while_the_flag_is_off_is_a_422() {
    // Never a silently ignored request: the caller asked for something this server
    // refuses to do, and a 200 would tell them it happened.
    let pool = setup_db(&disabled_config()).await;
    let before = rows(&pool).await;
    let c = client(pool.clone()).await; // `build_rocket`'s config: the flag is off
    let path = "/api/backup/restore?mode=copy&apply_prices=1";

    let res = c
        .post(path)
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle_json_with_prices("Priced"))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
    let body: Value = res.into_json().await.unwrap();
    assert!(
        body["error"].as_str().unwrap().contains("price restore disabled"),
        "{body}"
    );
    assert_eq!(rows(&pool).await, before, "the refusal writes nothing at all");

    // A bundle with no price section asks for nothing the flag forbids, so the same
    // request is an ordinary 200 rather than a refusal over nothing.
    let res = c
        .post(path)
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle_json("No prices"))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    assert_eq!(rows(&pool).await, (before.0 + 1, before.1 + 1, before.2 + 1));
}

// ---------------------------------------------------------------------------
// The one-transaction guarantee.
//
// `apply` claims a bundle that fails partway writes nothing. Over HTTP every failure
// a bundle can provoke is caught by validation before the transaction opens, which
// is the user-visible half of the guarantee and is tested as such. The transaction
// itself can only be driven into a mid-write failure by calling `apply` with a
// bundle that skipped validation — which is exactly the caller the claim has to
// survive, so that is how the second test is written.
// ---------------------------------------------------------------------------

/// Two portfolios, each with one asset and one transaction: enough for a failure in
/// the second to be *after* real writes in the first.
fn two_portfolio_bundle() -> String {
    let one = &bundle_value("First")["portfolios"][0];
    let mut two = one.clone();
    two["ref"] = json!("p2");
    two["name"] = json!("Second");
    two["assets"][0]["ref"] = json!("a2");
    two["assets"][0]["transactions"][0]["date"] = json!("2024-01-02T00:00:00Z");
    json!({
        "format": "portfolio-prism-backup", "schema_version": 1,
        "created_at": "2026-11-13T00:00:00Z",
        "portfolios": [one, two],
    })
    .to_string()
}

#[tokio::test]
async fn a_bundle_whose_second_portfolio_is_invalid_writes_nothing() {
    let pool = setup_db(&disabled_config()).await;
    seed_two_portfolios(&pool).await;
    let before = rows(&pool).await;
    let c = client(pool.clone()).await;

    let mut bundle = bundle_value("Two portfolios");
    let mut second = bundle_value("Second")["portfolios"][0].clone();
    second["ref"] = json!("p2");
    second["assets"][0]["ref"] = json!("a2");
    second["assets"][0]["transactions"][0]["price"] = json!("not-a-number");
    bundle["portfolios"] = json!([bundle["portfolios"][0].clone(), second]);

    let res = c
        .post("/api/backup/restore?mode=copy")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle.to_string())
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(
        body["errors"][0]["path"],
        "portfolios[1].assets[0].transactions[0].price",
        "{body}"
    );
    assert_eq!(rows(&pool).await, before, "a half-valid bundle is a half-restored one");
}

#[tokio::test]
async fn a_write_that_fails_after_the_first_portfolio_rolls_everything_back() {
    let pool = setup_db(&disabled_config()).await;
    let before = rows(&pool).await;
    // Parsed as a real bundle first, so the only thing wrong with it is the one field
    // corrupted below: everything else is exactly what the route would have written.
    let mut bundle = parse_and_validate(two_portfolio_bundle().as_bytes())
        .expect("the fixture is a valid bundle");
    bundle.portfolios[1].assets[0].transactions[0].date = "not-a-date".into();

    let err = backup_restore::apply(
        &pool,
        common::DEFAULT_USER,
        &bundle,
        Mode::Copy,
        false,
        &BackupConfig::default(),
    )
    .await
    .expect_err("a date no parser accepts stops the write");
    assert!(matches!(err, ApplyError::Db(DbFailure::Other)), "{err:?}");
    assert_eq!(err.status(), Status::InternalServerError);
    assert_eq!(
        rows(&pool).await,
        before,
        "the first portfolio, its asset and its transaction must roll back with the second"
    );
}

// ---------------------------------------------------------------------------
// Who a restore is allowed to write.
//
// A bundle is data, not a grant: it names nothing by database id, and every row a
// restore writes takes its owner from the session. `replace` is the same rule with a
// delete in front of it, and the delete is scoped by the caller's own id alone.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_restore_writes_the_caller_and_never_another_user_s_rows() {
    let pool = setup_db(&disabled_config()).await;
    seed_two_portfolios(&pool).await;
    let other = common::seed_user(&pool, "user-b").await;
    let theirs = seed_portfolio(&pool, other, "Theirs", "USD").await;
    let their_asset = seed_asset(&pool, theirs, "BBB", "Asset B").await;
    seed_transaction(&pool, their_asset, "SELL", 1.0, 2.0, 0.0).await;
    let before = rows(&pool).await;

    // A bundle that reuses B's id-shaped refs and copies B's names — the shape a
    // hand-edited file would try, and the one a bundle exported by B and handed over
    // arrives as.
    let foreign = json!({
        "format": "portfolio-prism-backup", "schema_version": 1,
        "created_at": "2026-11-13T00:00:00Z",
        "portfolios": [{
            "ref": format!("p{theirs}"), "name": "Theirs", "description": null,
            "currency": "USD", "base_currency": "USD",
            "assets": [{
                "ref": format!("a{their_asset}"), "symbol": "BBB", "name": "Asset B",
                "asset_type": "STOCK", "sector": null, "currency": "USD", "isin": null,
                "transactions": [{ "type": "BUY", "quantity": "1", "price": "1",
                                   "fee": "0", "date": "2024-01-15T00:00:00Z" }]
            }]
        }]
    })
    .to_string();

    let c = client(pool.clone()).await; // runs as DEFAULT_USER, who owns none of B's rows
    let res = c
        .post("/api/backup/restore?mode=copy")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(foreign)
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let body: Value = res.into_json().await.unwrap();
    let new_id = body["portfolio_ids"][0].as_i64().unwrap();
    assert_ne!(new_id, theirs as i64, "a foreign ref must not become an id");

    // The caller owns what the caller restored...
    let owner: (Option<i64>,) = sqlx::query_as("SELECT owner_id FROM portfolios WHERE id = ?")
        .bind(new_id as i32)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(owner.0, Some(common::DEFAULT_USER as i64));

    // ...and B's rows are untouched, in count and in owner.
    let theirs_now: (i64, i64, i64) = sqlx::query_as(
        "SELECT \
         (SELECT COUNT(*) FROM portfolios WHERE owner_id = ?), \
         (SELECT COUNT(*) FROM assets WHERE portfolio_id IN \
          (SELECT id FROM portfolios WHERE owner_id = ?)), \
         (SELECT COUNT(*) FROM transactions WHERE asset_id IN \
          (SELECT id FROM assets WHERE portfolio_id IN \
           (SELECT id FROM portfolios WHERE owner_id = ?)))",
    )
    .bind(other as i64)
    .bind(other as i64)
    .bind(other as i64)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(theirs_now, (1, 1, 1), "another user's rows are neither read nor written");
    assert_eq!(rows(&pool).await, (before.0 + 1, before.1 + 1, before.2 + 1));
}

#[tokio::test]
async fn a_bundle_ref_never_reaches_a_row_even_when_it_is_blank() {
    // Validation rejects a *duplicate* ref but nothing rejects an empty or
    // whitespace-only one, and portfolio and asset refs share one namespace — so at
    // most one portfolio and one asset in a bundle can be blank, and this is that
    // bundle. It restores, because the write path never reads a ref: every link comes
    // from the id the insert that made the row returned.
    let pool = setup_db(&disabled_config()).await;
    let before = rows(&pool).await;
    let mut bundle = bundle_value("Blank refs");
    bundle["portfolios"][0]["ref"] = json!("");
    bundle["portfolios"][0]["assets"][0]["ref"] = json!("   ");
    let c = client(pool.clone()).await;

    let res = c
        .post("/api/backup/restore?mode=copy")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle.to_string())
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let body: Value = res.into_json().await.unwrap();
    let new_id = body["portfolio_ids"][0].as_i64().unwrap();

    // The chain is intact, which is only true if the links were built from ids: a
    // blank ref used as a key would have hung the transaction off nothing.
    let chain: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM transactions WHERE asset_id IN \
         (SELECT id FROM assets WHERE portfolio_id = ?)",
    )
    .bind(new_id as i32)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(chain.0, 1);
    // And no column holds a ref: the name is what the bundle's `name` said.
    let (name,): (String,) = sqlx::query_as("SELECT name FROM portfolios WHERE id = ?")
        .bind(new_id as i32)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(name, "Blank refs");
    assert_eq!(rows(&pool).await, (before.0 + 1, before.1 + 1, before.2 + 1));
}

#[tokio::test]
async fn replace_leaves_no_orphan_split_sync_rows() {
    // `split_sync.asset_id` is a plain INTEGER PRIMARY KEY with **no** foreign key to
    // `assets`, so `PRAGMA foreign_keys` does not reach it: a delete that forgot it
    // would leave one row per deleted asset forever, and the next asset handed that id
    // would inherit a stranger's split-sync throttle. The delete order is the other
    // half — `foreign_keys` is ON and no FK here cascades, so children go first.
    let pool = setup_db(&disabled_config()).await;
    let (one, _) = seed_two_portfolios(&pool).await;
    let asset = seed_asset(&pool, one, "MSFT", "Microsoft").await;
    common::seed_split_sync(&pool, asset).await;
    let (before,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM split_sync")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, 1, "the fixture must actually have a row to orphan");

    let c = client(pool.clone()).await;
    let res = c
        .post("/api/backup/restore?mode=replace")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle_json("Only"))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);

    let orphans: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM split_sync WHERE asset_id NOT IN (SELECT id FROM assets)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(orphans.0, 0, "a replace must not strand split_sync rows");
}

#[tokio::test]
async fn restore_requires_a_session() {
    let pool = setup_db(&disabled_config()).await;
    let rocket = common::build_rocket_oidc(pool);
    let c = Client::tracked(rocket).await.unwrap();
    let res = c
        .post("/api/backup/restore?mode=copy")
        .body(bundle_json("x"))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Unauthorized);
}

#[tokio::test]
async fn restore_refuses_an_upload_it_cannot_read_or_parse() {
    // The same two refusals `preview` gives, on the route that writes: an upload cut
    // off by the cap is the module's `413` rather than a truncated bundle restored,
    // and a file that is not JSON is the caller's `422` rather than a `500`.
    let pool = setup_db(&disabled_config()).await;
    seed_two_portfolios(&pool).await;
    let before = rows(&pool).await;
    let c = client(pool.clone()).await;

    let res = c
        .post("/api/backup/restore?mode=copy")
        .header(session())
        .body(body_of(MAX_BUNDLE_BYTES + 1))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::PayloadTooLarge);

    let res = c
        .post("/api/backup/restore?mode=copy")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body("not json")
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::UnprocessableEntity);

    assert_eq!(rows(&pool).await, before, "neither refusal writes");
}
