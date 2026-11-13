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
use backend_rust::backup::{BackupConfig, MAX_BUNDLE_BYTES, MAX_BUNDLE_ROWS};

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
