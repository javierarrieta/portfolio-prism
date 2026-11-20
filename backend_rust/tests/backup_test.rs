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

use backend_rust::ensure_bootstrap_owner;
use common::{
    build_rocket,
    bundle_json,
    bundle_with_size,
    db_with_user,
    disabled_config,
    placeholder_id,
    pool_options,
    seed_asset,
    seed_daily_prices,
    seed_portfolio,
    seed_transaction,
    test_pool_url,
};
use sqlx::Connection as _;
use rocket::data::{Data, Limits, ToByteUnit};
use rocket::http::{Header, Status};
use rocket::local::asynchronous::Client;
use rocket::serde::json::Json;
use rocket::{get, post, routes};
use serde_json::{json, Value};

use backend_rust::api_routes::backup::{BackupError, lock_response};
use backend_rust::db::DbPool;
use backend_rust::backup::export::{self as backup_export, DbFailure};
use backend_rust::backup::restore::{self as backup_restore, ApplyError, Mode};
use backend_rust::backup::{BackupConfig, MAX_BUNDLE_BYTES, MAX_BUNDLE_ROWS, parse_and_validate};

/// The session header the existing tests use to run as the caller they seeded.
async fn client(pool: DbPool, user: i64) -> Client {
    Client::tracked(build_rocket(pool, user)).await.unwrap()
}

/// The same, with the price-restore flag on — the only thing a test can do about it:
/// `build_rocket_with_backup_config` documents why the flag is injected, not exported.
async fn price_restore_client(pool: DbPool, user: i64) -> Client {
    Client::tracked(common::build_rocket_with_backup_config(
        pool,
        user,
        BackupConfig { allow_price_restore: true },
    ))
    .await
    .unwrap()
}

fn session() -> Header<'static> {
    common::session_header()
}

#[tokio::test]
async fn capabilities_reports_the_limits_the_ui_must_respect() {
    let (pool, user) = db_with_user("capabilities_reports_the_limits_the_ui_must_respect").await;
    let c = client(pool, user).await;

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
    let (pool, _) = db_with_user("capabilities_requires_a_session").await;
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
    let (pool, user) = db_with_user("the_managed_backup_config_is_the_one_build_rocket_was_given").await;
    let on = BackupConfig { allow_price_restore: true };
    let rocket = common::build_rocket_with_backup_config(pool, user, on);
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

async fn probe_client(pool: DbPool, user: i64) -> Client {
    let rocket = build_rocket(pool, user).mount("/probe", routes![probe_data]);
    Client::tracked(rocket).await.unwrap()
}

/// A body of `len` bytes. Content is irrelevant: these tests are about size.
fn body_of(len: usize) -> String {
    "x".repeat(len)
}

#[tokio::test]
async fn a_body_of_exactly_max_bundle_bytes_arrives_whole() {
    let (pool, user) = db_with_user("a_body_of_exactly_max_bundle_bytes_arrives_whole").await;
    let c = probe_client(pool, user).await;

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
    let (pool, user) = db_with_user("one_byte_more_arrives_capped_for_the_handler_to_refuse").await;
    let c = probe_client(pool, user).await;

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
async fn seed_two_portfolios(pool: &DbPool, user: i64) -> (i64, i64) {
    let one = seed_portfolio(pool, user, "One", "USD").await;
    let two = seed_portfolio(pool, user, "Two", "EUR").await;
    let asset = seed_asset(pool, one, "AAPL", "Apple").await;
    seed_transaction(pool, asset, "BUY", 10.0, 150.0, 1.0).await;
    (one, two)
}

#[tokio::test]
async fn export_contains_only_the_selected_portfolios() {
    let (pool, user) = db_with_user("export_contains_only_the_selected_portfolios").await;
    let (one, two) = seed_two_portfolios(&pool, user).await;
    let c = client(pool, user).await;

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
    let (pool, user) = db_with_user("export_is_a_download_and_is_never_cached").await;
    let (one, _) = seed_two_portfolios(&pool, user).await;
    let c = client(pool, user).await;
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
    let (pool, user) = db_with_user("an_empty_selection_is_a_422_not_an_empty_bundle").await;
    let c = client(pool, user).await;
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
    let (pool, user) = db_with_user("a_malformed_id_list_is_a_422").await;
    let c = client(pool, user).await;
    let res = c.get("/api/backup/export?portfolio_ids=1,abc").header(session()).dispatch().await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
}

#[tokio::test]
async fn a_duplicate_id_is_exported_once() {
    let (pool, user) = db_with_user("a_duplicate_id_is_exported_once").await;
    let (one, _) = seed_two_portfolios(&pool, user).await;
    let c = client(pool, user).await;
    let path = format!("/api/backup/export?portfolio_ids={one},{one}");
    let res = c.get(&path).header(session()).dispatch().await;
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["portfolios"].as_array().unwrap().len(), 1);
}

/// Price rows for two symbols, only one of which `seed_two_portfolios` puts in the
/// selection. The unheld one is what makes "prices are for the selection, not for the
/// whole `historical_prices` table" a checked claim rather than an assumption.
async fn seed_two_symbols(pool: &DbPool) {
    seed_daily_prices(pool, "AAPL", "2024-01-01", "2024-01-03", "150.00").await;
    seed_daily_prices(pool, "MSFT", "2024-01-01", "2024-01-02", "90.00").await;
}

#[tokio::test]
async fn every_on_spelling_includes_only_the_prices_the_selection_holds() {
    // Rocket's own `bool` form field accepts neither `1` (what the UI sends) nor a
    // value it would report as an error, so these spellings used to arrive as `None`
    // and answer a 200 with the requested section missing.
    let (pool, user) = db_with_user("every_on_spelling_includes_only_the_prices_the_selection_holds").await;
    let (one, two) = seed_two_portfolios(&pool, user).await;
    seed_two_symbols(&pool).await;
    let c = client(pool, user).await;

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
    let (pool, user) = db_with_user("every_off_spelling_omits_the_price_section").await;
    let (one, _) = seed_two_portfolios(&pool, user).await;
    seed_two_symbols(&pool).await;
    let c = client(pool, user).await;

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
    let (pool, user) = db_with_user("an_unrecognised_include_prices_is_a_422_not_a_quiet_omission").await;
    let (one, _) = seed_two_portfolios(&pool, user).await;
    let c = client(pool, user).await;

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
    let (pool, user) = db_with_user("a_repeated_query_parameter_is_refused_not_half_read").await;
    let (one, two) = seed_two_portfolios(&pool, user).await;
    // Seeded so the duplicate's stakes are real: the failure refused here is a 200 with
    // a section missing, and there has to be a section that could go missing.
    seed_two_symbols(&pool).await;
    let c = client(pool, user).await;

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
async fn seed_many_transactions(pool: &DbPool, asset_id: i64, n: usize) {
    sqlx::query(
        "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) \
         WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt WHERE x < $1) \
         SELECT $2, 'BUY', '1', '10.0', '0.0', '2024-06-15T00:00:00+00:00' FROM cnt",
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
    let (pool, user) = db_with_user("a_selection_over_the_row_ceiling_is_refused_by_the_route").await;
    let (one, _) = seed_two_portfolios(&pool, user).await;
    let asset = seed_asset(&pool, one, "AAPL", "Apple").await;
    seed_many_transactions(&pool, asset, MAX_BUNDLE_ROWS).await;
    let c = client(pool, user).await;

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
    let (pool, user) = db_with_user("a_bundle_over_the_byte_ceiling_is_refused_by_the_route").await;
    let (one, _) = seed_two_portfolios(&pool, user).await;
    seed_asset(&pool, one, "BIG", &"x".repeat(MAX_BUNDLE_BYTES + 1)).await;
    let c = client(pool, user).await;

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
    let (pool, user) = db_with_user("no_global_body_limit_is_raised_for_the_bundle").await;
    let rocket = build_rocket(pool, user)
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
// A `503` and its `Retry-After` are only checkable on a real response, so the
// responder is pinned the way the size cap above is — the module's own value, mounted
// on a probe route outside `/api`, asserted as what a client receives. The real routes
// reach this exact code through `lock_response()`, which is the only way to build a
// `BackupError` with a `503` at all (`BackupError`'s fields are private). The route that
// writes reaches it from a real serialisation failure, proven end to end in
// `tests/restore_concurrency_test.rs::a_concurrent_replace_restore_is_a_503_with_retry_after`.
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
    let (pool, user) = db_with_user("a_lock_is_a_503_that_says_when_to_retry").await;
    let rocket = build_rocket(pool, user).mount("/probe", routes![probe_lock]);
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
// Two arms turn a `DbFailure::Lock` into the response, and they are not equally
// reachable from a test:
//
// * `build()` producing a `Lock` at all is driven below, by a pool with no free
//   connection. `PoolTimedOut` is the retryable database condition a test can actually
//   provoke here.
// * The response a `Lock` becomes is pinned by `a_lock_is_a_503_that_says_when_to_retry`
//   on a probe route.
//
// What *is* now pinned, and what is not, about `restore`'s arm:
//
// * `restore`'s `ApplyError::Db(DbFailure::Lock) => lock_response()` arm is driven over
//   HTTP for real by `tests/restore_concurrency_test.rs`, which races two `replace`
//   restores and requires the loser to answer `503` with `Retry-After`. That is the
//   Postgres replacement for the SQLite test the engine switch deleted: two writers on
//   the same rows abort with `40001`, because `restore::apply` runs `SERIALIZABLE`.
// * The pool-timeout half of the same arm is **not** reachable over HTTP, and the reason
//   is specific rather than incidental: the auth guard reads `users` before the handler
//   runs, so a pool with nothing free is answered a `500` by the guard and never reaches
//   `apply`. Proving it would mean holding a connection at an instant *between* the
//   guard and the handler, which no test can observe. A `40P01` deadlock cannot be
//   provoked on purpose either: the victim is whichever transaction's wait completes the
//   cycle, and a restore that blocks on its first statement holds nothing yet.
//
// TODO(Task 7, accepted gap): the two bullets above are the whole of what is proven.
// `restore`'s `Lock` arm is proven for a serialisation failure and permanently unproven
// for `PoolTimedOut`, for the reason in the previous paragraph — do not re-derive it, and
// do not "fix" it by softening the guard, which answers a database failure as a `500`
// exactly as `src/access.rs` requires. The `PoolTimedOut` half of the rule is pinned
// where it can be: `classify` maps it (unit test `pool_exhaustion_is_busy_and_a_plain_
// error_is_not`), and `build()` produces the `Lock` (below).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_exhausted_pool_is_a_lock_not_an_outage() {
    // The likeliest way to be kept out of a healthy Postgres is every connection in the
    // pool being checked out. That is just as retryable as a lock wait, so it must
    // arrive as the same failure rather than a bare 500 with no `Retry-After`.
    let url = test_pool_url("an_exhausted_pool_is_a_lock").await;
    let pool = pool_options()
        .max_connections(1)
        // 2s, not the 50ms this used to be. `acquire_timeout` bounds the *initial*
        // `connect()` as much as any later acquire, and 50ms is not a budget in which a
        // fresh Postgres connection can be relied on to complete: under CI's parallel test
        // binaries the setup occasionally died with `PoolTimedOut` before the test had
        // asserted anything (PR #66's first red run, `backup_test.rs:573` unwrapping a
        // `PoolTimedOut` on `.connect`). The short timeout was here to make the held
        // connection below fail fast, and 2s still does that — it is the same arm, just
        // not racing the connection open against a 50ms clock.
        .acquire_timeout(std::time::Duration::from_secs(2))
        .connect(&url)
        .await
        .unwrap();
    ensure_bootstrap_owner(&pool, &disabled_config()).await.unwrap();
    let owner = placeholder_id(&pool).await.expect("the placeholder owner");
    let pid = seed_portfolio(&pool, owner, "One", "USD").await;

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

#[tokio::test]
async fn a_read_only_export_does_not_queue_behind_a_writer() {
    // `export::build` wraps its reads in a `REPEATABLE READ` transaction, and the claim
    // this test makes is the one about locking rather than visibility: an open writer
    // blocks nothing. A reader neither waits for the lock nor sees the uncommitted write.
    //
    // Changing `backup_export::EXPORT_ISOLATION`, or `begin_read_tx`, to anything
    // that takes a write lock no read needs — `BEGIN READ ONLY FOR UPDATE`, or a `BEGIN`
    // that acquires one — fails here.
    //
    // What this test cannot pin, said plainly rather than glossed: that every read in
    // `build` shares one snapshot. Uncommitted data is invisible to a reader at any
    // isolation level, and no test can commit at a chosen moment *between* two of
    // `build`'s own queries. That half is pinned from inside the transaction instead, in
    // `tests/export_snapshot_test.rs`.
    let url = test_pool_url("a_read_only_export_does_not_queue").await;
    let pool = pool_options().connect(&url).await.unwrap();
    ensure_bootstrap_owner(&pool, &disabled_config()).await.unwrap();
    let owner = placeholder_id(&pool).await.expect("the placeholder owner");
    seed_two_portfolios(&pool, owner).await;
    let (one, _) = seed_two_portfolios(&pool, owner).await;

    // A second pool, because the point is that a *different* session is mid-write.
    let holder_pool = pool_options().connect(&url).await.unwrap();
    let mut holder = holder_pool.acquire().await.unwrap();
    let mut tx = holder.begin().await.unwrap();
    sqlx::query("UPDATE portfolios SET name = 'Renamed mid-flight' WHERE id = $1")
        .bind(one)
        .execute(&mut *tx)
        .await
        .unwrap();

    let c = client(pool.clone(), owner).await;
    let path = format!("/api/backup/export?portfolio_ids={one}");
    let res = c.get(&path).header(session()).dispatch().await;
    let status = res.status();
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(status, Status::Ok, "a reader must not queue behind a writer: {body}");
    assert_eq!(
        body["portfolios"][0]["name"], "One",
        "and an uncommitted rename must not be half-visible"
    );

    tx.rollback().await.unwrap();
    drop(holder);
    holder_pool.close().await;
    pool.close().await;
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

/// The smallest bundle that passes validation, as a mutable `Value`, for a test that
/// needs one field changed.
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
    let (pool, user) = db_with_user("preview_counts_what_replace_would_delete_and_insert").await;
    seed_two_portfolios(&pool, user).await;
    let c = client(pool, user).await;

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
    assert_eq!(body["delete"]["counts"]["portfolios"], 2, "both owned portfolios go");
    assert_eq!(body["delete"]["counts"]["assets"], 1);
    assert_eq!(body["delete"]["counts"]["transactions"], 1);
    assert_eq!(body["insert"]["counts"]["portfolios"], 1);
    assert_eq!(body["insert"]["counts"]["assets"], 1);
    assert_eq!(body["insert"]["counts"]["transactions"], 1);
}

#[tokio::test]
async fn preview_in_copy_mode_deletes_nothing() {
    let (pool, user) = db_with_user("preview_in_copy_mode_deletes_nothing").await;
    seed_two_portfolios(&pool, user).await;
    let c = client(pool, user).await;
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
    assert_eq!(body["delete"]["counts"]["portfolios"], 0);
    assert_eq!(body["delete"]["counts"]["transactions"], 0);
    // Copy still inserts: "deletes nothing" is not the same as "does nothing".
    assert_eq!(body["insert"]["counts"]["portfolios"], 1);
}

#[tokio::test]
async fn preview_writes_nothing() {
    let (pool, user) = db_with_user("preview_writes_nothing").await;
    seed_two_portfolios(&pool, user).await;
    let before: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM portfolios")
        .fetch_one(&pool)
        .await
        .unwrap();
    let c = client(pool.clone(), user).await;
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
async fn snapshot(pool: &DbPool) -> Vec<(String, i64)> {
    let mut out: Vec<(String, i64)> = Vec::new();
    for table in ["portfolios", "assets", "transactions", "historical_prices"] {
        let (n,): (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(pool)
            .await
            .unwrap();
        out.push((table.to_string(), n));
    }
    // `::bigint` because `SUM` of a `BIGINT` column is `NUMERIC` in Postgres, and
    // `NUMERIC` does not decode into `i64`. The sum is a fingerprint, not a money
    // amount, so narrowing it back is the point rather than a shortcut.
    let (sum,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(id), 0)::bigint FROM portfolios",
    )
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
    let (pool, user) = db_with_user("no_preview_writes_anything_whether_it_succeeds_or_refuses").await;
    seed_two_portfolios(&pool, user).await;
    seed_daily_prices(&pool, "AAPL", "2024-01-01", "2024-01-02", "150.00").await;
    let before = snapshot(&pool).await;
    let c = client(pool.clone(), user).await;

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
    let (pool, user) = db_with_user("an_unknown_mode_is_a_422").await;
    let c = client(pool, user).await;
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
    let (pool, user) = db_with_user("an_absent_mode_is_a_422_naming_the_two_that_exist").await;
    let c = client(pool, user).await;
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
    let (pool, user) = db_with_user("a_repeated_mode_is_refused_not_half_read").await;
    let c = client(pool, user).await;
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
    let (pool, user) = db_with_user("a_non_json_upload_is_a_422_not_a_500").await;
    let c = client(pool, user).await;
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
    let (pool, user) = db_with_user("a_bundle_from_a_newer_build_is_refused_by_preview").await;
    let c = client(pool, user).await;
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
    let (pool, user) = db_with_user("a_bundle_that_fails_validation_names_its_paths").await;
    let c = client(pool, user).await;
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
async fn two_splits_on_one_utc_day_are_a_422_naming_the_asset_and_the_day() {
    // `uq_split_asset_date` allows one SPLIT per asset per UTC calendar day, and it is
    // the only thing that enforces that — `apply` inserts plainly, so the second row of
    // a pair raised `23505 unique_violation`, `classify` mapped it to `Other`, and the
    // caller got a 500 for a row in their own file *after* `preview` had said the bundle
    // was fine. The two answers now agree because `collect` refuses the pair.
    //
    // It failed safe before this check, and still does: `apply` is one SERIALIZABLE
    // transaction, so the `23505` rolled back every row the bundle had already written.
    // That property is kept and is not what the 422 replaces — a rollback is the right
    // outcome for a bundle that cannot be applied, but it is a `500` telling a user their
    // own data is a server bug, and `preview` had already promised otherwise.
    let (pool, user) = db_with_user("two_splits_on_one_utc_day").await;
    let c = client(pool, user).await;
    let mut b = bundle_value("x");
    // Same UTC day, different instants, so this is not merely the same row twice.
    b["portfolios"][0]["assets"][0]["transactions"] = json!([
        { "type": "SPLIT", "quantity": "2.0", "price": "1.0", "fee": "0.00",
          "date": "2024-01-15T00:00:00Z" },
        { "type": "SPLIT", "quantity": "3.0", "price": "1.0", "fee": "0.00",
          "date": "2024-01-15T18:00:00Z" },
    ]);

    let res = c
        .post("/api/backup/preview?mode=copy")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(b.to_string())
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::UnprocessableEntity, "a duplicate SPLIT day is a client data error, never a 500");
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["error"], "the bundle failed validation");
    let errors = body["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1, "exactly the second split is at fault: {body}");
    // Names the asset (which one, by path) and the day (which day, in the reason).
    assert_eq!(errors[0]["path"], "portfolios[0].assets[0].transactions[1].date");
    let reason = errors[0]["reason"].as_str().unwrap();
    assert!(reason.contains("2024-01-15"), "the reason must name the day: {reason}");
    assert!(reason.contains("SPLIT"), "the reason must name what collided: {reason}");
}

#[tokio::test]
async fn two_splits_on_different_utc_days_are_accepted_even_in_the_same_local_date() {
    // The control, and the half that a naive date comparison gets wrong: the rule is the
    // UTC calendar day because that is what `(date AT TIME ZONE 'UTC')::date` buckets
    // on. These two share a local date but not a UTC one, so the index does not see a
    // collision and neither may `collect` — otherwise a valid bundle becomes unrestorable.
    let (pool, user) = db_with_user("two_splits_different_utc_days").await;
    let c = client(pool, user).await;
    let mut b = bundle_value("x");
    b["portfolios"][0]["assets"][0]["transactions"] = json!([
        { "type": "SPLIT", "quantity": "2.0", "price": "1.0", "fee": "0.00",
          "date": "2024-01-15T01:00:00+05:00" },
        { "type": "SPLIT", "quantity": "3.0", "price": "1.0", "fee": "0.00",
          "date": "2024-01-15T20:00:00+05:00" },
    ]);

    let res = c
        .post("/api/backup/preview?mode=copy")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(b.to_string())
        .dispatch()
        .await;
    let status = res.status();
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(status, Status::Ok, "2024-01-14T20:00Z and 2024-01-15T15:00Z are different UTC days: {body}");
    assert_eq!(body["insert"]["counts"]["transactions"], 2);
}

#[tokio::test]
async fn an_upload_over_the_byte_cap_is_the_module_s_413() {
    // Rocket never refuses a `Data` upload of its own accord, so the ceiling is only
    // real if the route that opens the stream enforces it. The probes above pin the
    // read; this pins the route doing the conversion.
    let (pool, user) = db_with_user("an_upload_over_the_byte_cap_is_the_module_s_413").await;
    let c = client(pool, user).await;
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
    let (pool, user) = db_with_user("preview_reports_the_price_section_and_warns_about_a_replace").await;
    seed_two_portfolios(&pool, user).await;
    let c = client(pool, user).await;

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
async fn a_replace_plan_names_the_split_sync_rows_it_clears() {
    // `Counts` has three fields because a bundle has rows in three tables. `replace`
    // clears a fourth: `split_sync` throttle rows, whose `asset_id` carries no foreign
    // key, so they are deleted explicitly rather than cascaded. A plan that
    // under-reports what it is about to delete is the one thing a preview must not do,
    // and the delete set is not visible in the counts (`restore::plan_for`).
    let (pool, user) = db_with_user("a_replace_plan_names_the_split_sync_rows_it_clears").await;
    let one = seed_portfolio(&pool, user, "One", "USD").await;
    let asset = seed_asset(&pool, one, "AAPL", "Apple").await;
    common::seed_split_sync(&pool, asset).await;
    let c = client(pool, user).await;

    let res = c.post("/api/backup/preview?mode=replace").header(session())
        .header(rocket::http::ContentType::JSON).body(bundle_json("Restored")).dispatch().await;
    assert_eq!(res.status(), Status::Ok);
    let warnings: Value = res.into_json().await.unwrap();
    let warnings = warnings["warnings"].as_array().unwrap().clone();
    assert!(
        warnings.iter().any(|w| w.as_str().unwrap().contains("split-sync")),
        "the plan must say the throttle rows go too: {warnings:?}"
    );
}

#[tokio::test]
async fn a_preview_describes_only_the_caller_s_own_rows() {
    // Cross-tenant: a bundle names portfolios by bundle-local refs, which are labels
    // and never database ids, so a bundle cannot address another user's rows — and
    // the counts it is answered with are its own. `replace` describing a foreign
    // portfolio would be a promise to delete what the caller does not own.
    //
    // The fixture below deliberately impersonates user B — it reuses B's portfolio
    // name, B's asset name and B's id-shaped refs, because that is exactly what a
    // hand-edited file would try. Do not "clean up" those names: with them removed
    // this test would still pass while proving nothing about impersonation.
    let (pool, user) = db_with_user("a_preview_describes_only_the_caller_s_own_rows").await;
    seed_two_portfolios(&pool, user).await;
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

    let c = client(pool, user).await; // runs as user, who owns none of B's rows
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
    assert_eq!(body["delete"]["counts"]["portfolios"], 2, "{body}");
    assert_eq!(body["delete"]["counts"]["assets"], 1, "{body}");
    assert_eq!(body["delete"]["counts"]["transactions"], 1, "{body}");
    // The live side — the half read from the database — names only the caller's own
    // rows. Both halves are load-bearing: the negative is the leak check, and the
    // positive is what stops the negative passing vacuously. A delete side that listed
    // nothing at all would satisfy `!contains` exactly as happily as one that filtered
    // by owner correctly, and a broken owner filter is indistinguishable from a broken
    // delete side from the outside.
    let live = body["delete"].to_string();
    assert!(
        !live.contains("Asset B") && !live.contains("Theirs"),
        "the plan must not name another tenant's rows: {live}"
    );
    assert!(
        live.contains("One") && live.contains("Two"),
        "the caller's own portfolios must be listed, or the check above proves nothing: {live}"
    );
    // The bundle side is the caller's own upload reflected back, so it *does* name
    // "Theirs" and "Asset B" — the caller typed them into the file. Asserting the echo
    // is what separates "the bundle's rows are listed" from "they were silently
    // dropped"; it is not a leak, because the only reader of a preview is the caller
    // who uploaded it.
    let uploaded = body["insert"].to_string();
    assert!(
        uploaded.contains("Theirs") && uploaded.contains("Asset B"),
        "the insert side echoes the uploaded bundle: {uploaded}"
    );
}

#[tokio::test]
async fn preview_names_the_portfolios_and_assets_it_will_insert() {
    // The counts alone are not a preview: a user with 40 portfolios cannot decide
    // whether "2 portfolios, 6 assets" is the file they meant to upload. `insert`
    // carries the names behind its numbers, and `delete` carries the same shape even
    // when it is empty, so the panel renders both sides with one component.
    let (pool, user) = db_with_user("preview-detail-insert").await;
    let c = client(pool, user).await;
    // One transaction's price is zeroed, so the flag aggregation has something to
    // report. `bundle_with_size` builds a clean file, and a clean file's `asset_flags`
    // is `{}` whether or not the aggregation ever ran — asserting on it would pin shape
    // and nothing else. The aggregation itself (over every asset, not just the listed
    // ones) is pinned by the unit test
    // `backup::detail::tests::asset_flags_are_counted_over_every_asset_not_just_the_shown_ones`;
    // what this one pins is that a set flag survives the trip through the route.
    let body = {
        let mut bundle: Value = serde_json::from_str(&bundle_with_size(2, 3, 4)).unwrap();
        bundle["portfolios"][0]["assets"][0]["transactions"][0]["price"] = json!("0.00");
        bundle.to_string()
    };

    let res = c
        .post("/api/backup/preview?mode=copy")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(body)
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let plan: Value = res.into_json().await.unwrap();

    assert_eq!(plan["insert"]["counts"]["portfolios"], 2);
    assert_eq!(plan["insert"]["shown"].as_array().unwrap().len(), 2);
    let first = &plan["insert"]["shown"][0];
    assert_eq!(first["shown"].as_array().unwrap().len(), 3);
    assert_eq!(first["name"], "P0");
    assert_eq!(first["asset_flags"]["zero_price"], 1);
    // Copy deletes nothing, and says so with an empty side rather than a missing one.
    // This caller owns nothing, so these two lines pin that the `delete` key is present
    // and empty — the panel must not have to branch on a missing key — not that copy
    // leaves the caller's rows alone. `preview_in_copy_mode_deletes_nothing` is the test
    // that pins that behaviour, against a caller who does own rows.
    assert_eq!(plan["delete"]["counts"]["portfolios"], 0);
    assert!(plan["delete"]["shown"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn replace_preview_names_what_it_will_delete() {
    // The delete side is the one that can hurt, so it is the one that has to name
    // rows: "replace will delete 1 portfolio" is not enough to catch uploading the
    // wrong file over a live account.
    let (pool, user) = db_with_user("preview-detail-delete").await;
    let pid = seed_portfolio(&pool, user, "Doomed", "EUR").await;
    let aid = seed_asset(&pool, pid, "DOOM", "Doomed One").await;
    seed_transaction(&pool, aid, "BUY", 1.0, 4.0, 0.0).await;

    let c = client(pool, user).await;
    let res = c
        .post("/api/backup/preview?mode=replace")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle_with_size(1, 1, 1))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let plan: Value = res.into_json().await.unwrap();

    assert_eq!(plan["delete"]["counts"]["portfolios"], 1);
    assert_eq!(plan["delete"]["shown"][0]["name"], "Doomed");
    assert_eq!(plan["delete"]["shown"][0]["currency"], "EUR");
    assert_eq!(plan["delete"]["shown"][0]["transactions"], 1);
}

#[tokio::test]
async fn preview_requires_a_session() {
    let (pool, _) = db_with_user("preview_requires_a_session").await;
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
async fn rows(pool: &DbPool) -> (i64, i64, i64) {
    let p: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM portfolios").fetch_one(pool).await.unwrap();
    let a: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM assets").fetch_one(pool).await.unwrap();
    let t: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM transactions").fetch_one(pool).await.unwrap();
    (p.0, a.0, t.0)
}

/// Rows in the one table a restore writes that no user owns. Separate from `rows`
/// because a refusal that writes no portfolios can still have written market data,
/// and only this count can see it.
async fn price_rows(pool: &DbPool) -> i64 {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM historical_prices")
        .fetch_one(pool)
        .await
        .unwrap();
    n
}

#[tokio::test]
async fn copy_mode_adds_alongside_and_assigns_new_ids() {
    let (pool, user) = db_with_user("copy_mode_adds_alongside_and_assigns_new_ids").await;
    let (one, _) = seed_two_portfolios(&pool, user).await;
    let before = rows(&pool).await;
    let c = client(pool.clone(), user).await;

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
    assert_ne!(new_id, one, "a bundle ref must never become a database id");
    let chain: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM transactions WHERE asset_id IN \
         (SELECT id FROM assets WHERE portfolio_id = $1)")
        .bind(new_id).fetch_one(&pool).await.unwrap();
    assert_eq!(chain.0, 1, "the transaction must hang off the new asset under the new portfolio");

    let owner: (Option<i64>,) = sqlx::query_as("SELECT owner_id FROM portfolios WHERE id = $1")
        .bind(new_id).fetch_one(&pool).await.unwrap();
    assert_eq!(owner.0, Some(user));
}

#[tokio::test]
async fn a_round_trip_through_export_and_restore_is_lossless() {
    // The test that proves the format is sufficient — what backup features most
    // often fail at.
    let (source, user) = db_with_user("a_round_trip_through_export_and_restore_is_lossless").await;
    let (one, _) = seed_two_portfolios(&source, user).await;
    let exported: String = {
        let c = Client::tracked(build_rocket(source.clone(), user)).await.unwrap();
        let path = format!("/api/backup/export?portfolio_ids={one}");
        let res = c.get(&path).header(session()).dispatch().await;
        res.into_string().await.unwrap()
    };

    let (target, target_user) = db_with_user("target_a_round_trip_through_export_and_restore").await;
    let c = Client::tracked(build_rocket(target.clone(), target_user)).await.unwrap();
    let res = c.post("/api/backup/restore?mode=copy").header(session())
        .header(rocket::http::ContentType::JSON).body(exported.clone()).dispatch().await;
    assert_eq!(res.status(), Status::Ok, "{:?}", res.into_string().await);

    // Re-export from the target and compare, ignoring the informational timestamp
    // and the ids that are target-local by construction.
    let new_id: (i64,) = sqlx::query_as("SELECT id FROM portfolios WHERE name = 'One'")
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

/// An asset row of a caller-chosen type. `common::seed_asset` hard-codes `STOCK`,
/// and every fixture in this file goes through it or through `bundle_json`, which also
/// says `STOCK` — which is exactly how the suite stayed green while restore refused the
/// two asset types the application produces.
async fn seed_asset_type(
    pool: &DbPool,
    portfolio_id: i64,
    symbol: &str,
    name: &str,
    asset_type: &str,
) -> i64 {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO assets (portfolio_id, symbol, name, asset_type, currency) \
         VALUES ($1, $2, $3, $4, 'USD') RETURNING id",
    )
    .bind(portfolio_id)
    .bind(symbol)
    .bind(name)
    .bind(asset_type)
    .fetch_one(pool)
    .await
    .unwrap();
    // The row id, undiminished: `common::seed_asset` returns `i64` too, and so
    // does every caller now.
    id
}

#[tokio::test]
async fn a_round_trip_carries_the_asset_types_the_app_actually_produces() {
    // The bug this pins: `KNOWN_ASSET_TYPES` listed `FUND`, `BOND` and `CASH` — none of
    // which anything in the application can produce — and omitted `CRYPTO` and
    // `MUTUAL_FUND`, which it produces from the create-asset dropdown
    // (`frontend/src/components/PortfolioDetail.tsx`), from the ISIN lookup
    // (`api_routes/lookup.rs`), and from its own demonstration portfolio
    // (`frontend/src/App.tsx`). Export copies `asset_type` out of the database without
    // checking it, so the failure surfaced as a `422` on **restore** — after the user
    // was already holding the file, which is the one moment a backup must not fail.
    let (source, user) = db_with_user("a_round_trip_carries_the_asset_types_the_app_actually_produces").await;
    let pid = seed_portfolio(&source, user, "Mixed", "USD").await;
    let crypto = seed_asset_type(&source, pid, "BTC-USD", "Bitcoin", "CRYPTO").await;
    let fund = seed_asset_type(&source, pid, "VWRL", "Vanguard Global Stock", "MUTUAL_FUND").await;
    seed_transaction(&source, crypto, "BUY", 0.5, 40000.0, 0.0).await;
    seed_transaction(&source, fund, "BUY", 10.0, 100.0, 1.0).await;

    let exported: String = {
        let c = Client::tracked(build_rocket(source.clone(), user)).await.unwrap();
        let path = format!("/api/backup/export?portfolio_ids={pid}");
        c.get(&path).header(session()).dispatch().await.into_string().await.unwrap()
    };
    // The bundle really does carry both types, so the restore below cannot pass by
    // having exported a bundle that dropped them.
    let parsed: Value = serde_json::from_str(&exported).unwrap();
    let exported_types: Vec<&str> = parsed["portfolios"][0]["assets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["asset_type"].as_str().unwrap())
        .collect();
    assert_eq!(exported_types, vec!["CRYPTO", "MUTUAL_FUND"], "{exported}");

    let (target, target_user) = db_with_user("target_a_round_trip_carries_asset_types").await;
    let c = Client::tracked(build_rocket(target.clone(), target_user)).await.unwrap();
    let res = c.post("/api/backup/restore?mode=copy").header(session())
        .header(rocket::http::ContentType::JSON).body(exported).dispatch().await;
    let status = res.status();
    let text = res.into_string().await.unwrap();
    assert_eq!(status, Status::Ok, "a bundle the app exported must restore: {text}");

    // And the types survived the write rather than being coerced to something safe.
    // The new portfolio's id comes from the response: a restore mints ids, and the
    // target's numbering is the target's own.
    let new_id: i64 = serde_json::from_str::<Value>(&text).unwrap()["portfolio_ids"][0]
        .as_i64()
        .expect("the outcome names the portfolio it created");
    let stored: Vec<String> = sqlx::query_as::<_, (String,)>(
        "SELECT asset_type FROM assets WHERE portfolio_id = $1 ORDER BY symbol",
    )
    .bind(new_id)
    .fetch_all(&target)
    .await
    .unwrap()
    .into_iter()
    .map(|(t,)| t)
    .collect();
    assert_eq!(stored, vec!["CRYPTO", "MUTUAL_FUND"]);
}

#[tokio::test]
async fn a_restored_portfolio_produces_the_same_tax_result() {
    // Proves the rewiring is semantically right, not just present: the tax engine
    // reaches transactions through assets through portfolios.
    let (source, user) = db_with_user("a_restored_portfolio_produces_the_same_tax_result").await;
    let (one, _) = seed_two_portfolios(&source, user).await;
    let c = Client::tracked(build_rocket(source.clone(), user)).await.unwrap();
    let path = format!("/api/portfolios/{one}/tax-summary");
    let original: Value = c.get(&path)
        .header(session()).dispatch().await.into_json().await.unwrap();

    // The bundle has to come from the source, which is the side that owns portfolio
    // `one`; the target owns nothing yet.
    let exported: String = {
        let sc = Client::tracked(build_rocket(source, user)).await.unwrap();
        let path = format!("/api/backup/export?portfolio_ids={one}");
        sc.get(&path).header(session()).dispatch().await.into_string().await.unwrap()
    };
    let (target, target_user) = db_with_user("target_a_restored_portfolio_tax_result").await;
    let c = Client::tracked(build_rocket(target, target_user)).await.unwrap();
    let res = c.post("/api/backup/restore?mode=copy").header(session())
        .header(rocket::http::ContentType::JSON).body(exported).dispatch().await;
    let body: Value = res.into_json().await.unwrap();
    let new_id = body["portfolio_ids"][0].as_i64().unwrap();

    let path = format!("/api/portfolios/{new_id}/tax-summary");
    let restored: Value = c.get(&path)
        .header(session()).dispatch().await.into_json().await.unwrap();
    assert_eq!(original, restored);
}

#[tokio::test]
async fn restore_refuses_a_split_the_ordinary_write_path_refuses() {
    // A SPLIT is a ratio, and `create_transaction` answers a SPLIT with a non-positive
    // quantity or price with a 400. A restore must not accept a row the ordinary write
    // path would reject — and the impact of *not* refusing is silent: `tax_engine::replay`
    // guards `tx_qty > 0 && denominator > 0`, so a zero-quantity SPLIT is a no-op on
    // the lots and nothing downstream would ever report it.
    let (pool, user) = db_with_user("restore_refuses_a_split_the_ordinary_write_path_refuses").await;
    seed_two_portfolios(&pool, user).await;
    let before = rows(&pool).await;
    let c = client(pool.clone(), user).await;

    let split = |qty: &str, price: &str| -> String {
        let mut v = bundle_value("Split");
        v["portfolios"][0]["assets"][0]["transactions"] = json!([{
            "type": "SPLIT", "quantity": qty, "price": price, "fee": "0",
            "date": "2024-01-15T00:00:00Z"
        }]);
        v.to_string()
    };

    for (qty, price) in [("0", "2"), ("-1", "2"), ("2", "0"), ("2", "-0.5")] {
        let res = c.post("/api/backup/restore?mode=copy").header(session())
            .header(rocket::http::ContentType::JSON).body(split(qty, price)).dispatch().await;
        let status = res.status();
        let body: Value = res.into_json().await.unwrap();
        assert_eq!(status, Status::UnprocessableEntity, "{qty} : {price} -> {body}");
        assert!(
            body["errors"].to_string().contains("greater than zero"),
            "{qty} : {price} -> {body}"
        );
    }
    assert_eq!(rows(&pool).await, before, "every refusal above wrote nothing");

    // Positive control: a real ratio restores, so the four refusals are the boundary
    // and not a blanket ban on SPLIT rows. A fractional quantity is a reverse split and
    // is legal.
    let res = c.post("/api/backup/restore?mode=copy").header(session())
        .header(rocket::http::ContentType::JSON).body(split("2", "1")).dispatch().await;
    assert_eq!(res.status(), Status::Ok);
    assert_eq!(rows(&pool).await, (before.0 + 1, before.1 + 1, before.2 + 1));
}

#[tokio::test]
async fn a_split_is_stored_with_no_fee_as_the_write_path_stores_it() {
    // `create_transaction` stores zero for a SPLIT's fee whatever the caller sent
    // (`transactions.rs:215`) and the tax engine never reads one, so a hand-authored
    // bundle must not be able to store a value the application itself cannot produce.
    // Normalised rather than refused: refusing it would reject a bundle the write path
    // accepts, which is the mirror image of the rule above.
    let (pool, user) = db_with_user("a_split_is_stored_with_no_fee_as_the_write_path_stores_it").await;
    let c = client(pool.clone(), user).await;

    let mut v = bundle_value("Split fee");
    v["portfolios"][0]["assets"][0]["transactions"] = json!([{
        "type": "SPLIT", "quantity": "2", "price": "1", "fee": "99.00",
        "date": "2024-01-15T00:00:00Z"
    }]);
    let res = c.post("/api/backup/restore?mode=copy").header(session())
        .header(rocket::http::ContentType::JSON).body(v.to_string()).dispatch().await;
    assert_eq!(res.status(), Status::Ok);
    let body: Value = res.into_json().await.unwrap();
    let new_id = body["portfolio_ids"][0].as_i64().unwrap();

    let fee: (String,) = sqlx::query_as(
        "SELECT t.fee FROM transactions t WHERE t.asset_id IN \
         (SELECT id FROM assets WHERE portfolio_id = $1)",
    )
    .bind(new_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        rust_decimal::Decimal::from_str_exact(&fee.0)
            .unwrap_or_else(|e| panic!("{fee:?} is not a decimal: {e}")),
        rust_decimal::Decimal::ZERO,
        "a SPLIT's fee is stored as the write path stores it, not as the bundle spelled it"
    );
}

#[tokio::test]
async fn a_lowercase_isin_is_stored_upper_cased_like_the_write_path_stores_it() {
    // `is_valid_isin` upper-cases internally, so a lowercase ISIN validates — and
    // `create_asset` stores `to_uppercase()` (`transactions.rs:78`) while restore used
    // to store the bundle's bytes verbatim. A hand-authored bundle could therefore
    // create a row the ordinary path would never have produced, and one that slips past
    // the ISIN uniqueness check at `transactions.rs:50-56`: two rows for one security.
    let (pool, user) = db_with_user("a_lowercase_isin_is_stored_upper_cased_like_the_write_path_stores_it").await;
    let c = client(pool.clone(), user).await;

    let mut v = bundle_value("Lowercase isin");
    v["portfolios"][0]["assets"][0]["isin"] = json!("us0378331005");
    let res = c.post("/api/backup/restore?mode=copy").header(session())
        .header(rocket::http::ContentType::JSON).body(v.to_string()).dispatch().await;
    assert_eq!(res.status(), Status::Ok, "a lowercase ISIN is a valid ISIN");
    let body: Value = res.into_json().await.unwrap();
    let new_id = body["portfolio_ids"][0].as_i64().unwrap();

    let isin: (Option<String>,) = sqlx::query_as(
        "SELECT isin FROM assets WHERE portfolio_id = $1",
    )
    .bind(new_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(isin.0.as_deref(), Some("US0378331005"));
}

#[tokio::test]
async fn a_bundle_carrying_an_isin_with_a_wrong_check_digit_is_refused() {
    // This is the **restore** path, not the write path, and it is the reason the check
    // digit reaching `is_valid_isin` needed an audit of stored data: `collect` runs it
    // over every ISIN a bundle carries, and `export::build` copies ISINs straight out
    // of the `assets` table (`export.rs:305`). A stored row with a wrong digit would
    // therefore make its own backup fail here — a backup that cannot be restored is
    // not a backup.
    //
    // It cannot happen today: production stores no ISINs at all, and after this
    // change the write path refuses one. This test is what keeps that from rotting if
    // either stops being true.
    let (pool, user) = db_with_user("a_bundle_carrying_an_isin_with_a_wrong_check_digit_is_refused").await;
    let c = client(pool, user).await;

    let mut v = bundle_value("Wrong digit");
    // Twelve shape-valid characters, Apple's body, the digit decremented. Before the
    // digit was computed this bundle validated.
    v["portfolios"][0]["assets"][0]["isin"] = json!("US0378331004");
    let res = c.post("/api/backup/preview?mode=copy").header(session())
        .header(rocket::http::ContentType::JSON).body(v.to_string()).dispatch().await;
    assert_eq!(res.status(), Status::UnprocessableEntity, "a wrong check digit is a client data error");
    let body: Value = res.into_json().await.unwrap();
    let errors = body["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1, "only the ISIN is at fault: {body}");
    assert_eq!(errors[0]["path"], "portfolios[0].assets[0].isin");
    assert!(
        errors[0]["reason"].as_str().unwrap().contains("US0378331004"),
        "the reason must name the offending value: {body}"
    );
}

#[tokio::test]
async fn the_same_bundle_with_the_right_check_digit_restores() {
    // The control. Without it the test above passes for any reason at all — shape,
    // casing, an unrelated field — rather than for the digit.
    let (pool, user) = db_with_user("the_same_bundle_with_the_right_check_digit_restores").await;
    let c = client(pool, user).await;

    let mut v = bundle_value("Right digit");
    v["portfolios"][0]["assets"][0]["isin"] = json!("US0378331005");
    let res = c.post("/api/backup/restore?mode=copy").header(session())
        .header(rocket::http::ContentType::JSON).body(v.to_string()).dispatch().await;
    assert_eq!(res.status(), Status::Ok, "the same bundle with a valid digit must apply: {}",
        res.into_string().await.unwrap_or_default());
}

#[tokio::test]
async fn a_bundle_with_no_portfolios_is_refused_by_restore() {
    // Review Focus #5, on the write path this time.
    let (pool, user) = db_with_user("a_bundle_with_no_portfolios_is_refused_by_restore").await;
    seed_two_portfolios(&pool, user).await;
    let before = rows(&pool).await;
    let c = client(pool.clone(), user).await;
    let empty = json!({ "format": "portfolio-prism-backup", "schema_version": 1,
        "created_at": "2026-11-13T00:00:00Z", "portfolios": [] }).to_string();
    let res = c.post("/api/backup/restore?mode=replace").header(session())
        .header(rocket::http::ContentType::JSON).body(empty).dispatch().await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
    assert_eq!(rows(&pool).await, before, "an empty bundle must never be able to wipe a user");
}

// ---------------------------------------------------------------------------
// Restore — replace mode.
//
// `replace` is the only part of this feature that can destroy data the bundle says
// nothing about, so the tests here are written to fail rather than to decorate.
// Three rules hold it together: the delete is scoped by `owner_id` and by nothing
// else (a restore must never be a cross-tenant wipe, whatever the file contains);
// it takes everything the caller owns, including portfolios the bundle never
// mentions; and after it finishes the totals that moved are exactly the bundle's
// counts, which is what turns "nothing else moved" into a checked claim instead of
// an assumption. `preview` is the safety mechanism the user has to look at first;
// none of these tests are that look.
// ---------------------------------------------------------------------------

/// Every row `user` owns, as one string per row, ordered, plus the `split_sync`
/// rows hanging off their assets.
///
/// The whole row rather than a count, because a count is exactly what a wrong delete
/// scope can accidentally preserve: this also moves if a foreign `owner_id` is
/// rewritten, if a row is deleted and re-inserted with a fresh id, or if a
/// `split_sync` row is stranded on a deleted asset. `split_sync` belongs in here for
/// that reason — its `asset_id` has no foreign key to `assets`, so nothing in the
/// schema would notice and nothing else in the suite would either.
async fn owned_fingerprint(pool: &DbPool, user: i64) -> Vec<String> {
    // One `$1` each, bound to the caller's id, and nothing else: the same scoping rule
    // the deletes under test claim to follow.
    //
    // The `::text` casts are not decoration. SQLite stored `transactions.date`,
    // `split_sync.last_synced_at` and `split_sync.covered_from` as text, so `||` took
    // them as-is; Postgres has them as `timestamptz`/`DATE` and has no implicit
    // concatenation from those. Casting explicitly also *pins the spelling* the
    // fingerprint compares: a date rendered `2024-01-02` on both sides of a restore is
    // the assertion, and leaving the cast out would compare two different renderings.
    let queries = [
        "SELECT 'portfolios|' || id || '|' || name || '|' || COALESCE(description, '') || \
                '|' || currency || '|' || base_currency || '|owner=' || COALESCE(owner_id, -1) \
         FROM portfolios WHERE owner_id = $1 ORDER BY id",
        "SELECT 'assets|' || id || '|' || portfolio_id || '|' || symbol || '|' || name || '|' \
                || asset_type || '|' || COALESCE(sector, '') || '|' || currency || '|' \
                || COALESCE(isin, '') \
         FROM assets WHERE portfolio_id IN (SELECT id FROM portfolios WHERE owner_id = $1) \
         ORDER BY id",
        "SELECT 'transactions|' || id || '|' || asset_id || '|' || type || '|' || quantity || \
                '|' || price || '|' || fee || '|' || date::text \
         FROM transactions WHERE asset_id IN \
             (SELECT id FROM assets WHERE portfolio_id IN \
              (SELECT id FROM portfolios WHERE owner_id = $1)) \
         ORDER BY id",
        "SELECT 'split_sync|' || asset_id || '|' || last_synced_at::text || '|' || \
                COALESCE(covered_from::text, '') \
         FROM split_sync WHERE asset_id IN \
             (SELECT id FROM assets WHERE portfolio_id IN \
              (SELECT id FROM portfolios WHERE owner_id = $1)) \
         ORDER BY asset_id",
    ];
    let mut out: Vec<String> = Vec::new();
    for sql in queries {
        let rows: Vec<(String,)> = sqlx::query_as(sql)
            .bind(user)
            .fetch_all(pool)
            .await
            .unwrap();
        out.extend(rows.into_iter().map(|r| r.0));
    }
    out
}

#[tokio::test]
async fn replace_removes_everything_the_caller_owns() {
    // Spec: replace means "make my data equal this bundle", one rule, and the
    // preview is the safety mechanism. The other two portfolios go too — the bundle
    // names "Only" and says nothing about "One" or "Two", and both are gone.
    let (pool, user) = db_with_user("replace_removes_everything_the_caller_owns").await;
    seed_two_portfolios(&pool, user).await;
    let c = client(pool.clone(), user).await;

    let res = c
        .post("/api/backup/restore?mode=replace")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle_json("Only"))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);

    let (p, a, t) = rows(&pool).await;
    assert_eq!((p, a, t), (1, 1, 1), "exactly the bundle remains");
    let name: (String,) = sqlx::query_as("SELECT name FROM portfolios")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(name.0, "Only");
}

#[tokio::test]
async fn replace_leaves_other_users_data_alone() {
    // The delete is scoped by `owner_id` and by nothing else. A restore must never be
    // a cross-tenant wipe, whatever the bundle contains — so the other user's rows
    // here are deliberately indistinguishable from the caller's by anything but
    // owner: the same portfolio name the bundle uses, an asset the caller also owns,
    // and a `split_sync` row that a delete which forgot its filter would happily
    // sweep up. A `DELETE` missing any one of its four `WHERE owner_id = ?` filters
    // fails this test.
    let (pool, user) = db_with_user("replace_leaves_other_users_data_alone").await;
    seed_two_portfolios(&pool, user).await;
    let other = common::seed_user(&pool, "someone-else").await;
    let theirs = seed_portfolio(&pool, other, "Only", "USD").await;
    let their_asset = seed_asset(&pool, theirs, "AAPL", "Apple").await;
    seed_transaction(&pool, their_asset, "BUY", 3.0, 40.0, 0.0).await;
    common::seed_split_sync(&pool, their_asset).await;

    let theirs_before = owned_fingerprint(&pool, other).await;
    assert_eq!(
        theirs_before.len(),
        4,
        "the fixture must own one portfolio, one asset, one transaction and one \
         split_sync row, or 'unchanged' proves nothing: {theirs_before:?}"
    );
    let before = rows(&pool).await;

    let c = client(pool.clone(), user).await;
    let res = c
        .post("/api/backup/restore?mode=replace")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle_json("Only"))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);

    // (a) the other user's rows, byte for byte: same ids, same columns, same owner,
    // and their `split_sync` row still attached.
    assert_eq!(
        owned_fingerprint(&pool, other).await,
        theirs_before,
        "another user's rows must survive a replace untouched"
    );
    // (b) the global totals moved by exactly the bundle's counts: the caller's own
    // 2 portfolios / 1 asset / 1 transaction left, the bundle's 1/1/1 arrived, and
    // the other user's 1/1/1 is the only other thing inside these numbers. Anything
    // else that moved in those three tables shows up here even if (a) somehow did
    // not. `rows()` counts no other table, so the foreign `split_sync` entry is (a)'s
    // job alone: a replace that deleted it would leave (b) exactly as it is.
    assert_eq!(
        rows(&pool).await,
        (before.0 - 2 + 1, before.1 - 1 + 1, before.2 - 1 + 1)
    );
}

#[tokio::test]
async fn replacing_twice_in_a_row_is_still_exactly_the_bundle() {
    // "Make my data equal this bundle" is a rule, not a one-shot: the second call
    // must delete what the first one wrote and leave the same three rows, rather
    // than doubling them or stranding a row from the first pass.
    let (pool, user) = db_with_user("replacing_twice_in_a_row_is_still_exactly_the_bundle").await;
    seed_two_portfolios(&pool, user).await;
    let c = client(pool.clone(), user).await;
    for round in 1..=2 {
        let res = c
            .post("/api/backup/restore?mode=replace")
            .header(session())
            .header(rocket::http::ContentType::JSON)
            .body(bundle_json("Only"))
            .dispatch()
            .await;
        assert_eq!(res.status(), Status::Ok, "round {round}");
    }
    assert_eq!(rows(&pool).await, (1, 1, 1));
}

#[tokio::test]
async fn a_replace_that_fails_partway_deletes_nothing_either() {
    // The worst failure mode this route has: the deletes land and the insert does
    // not, which is not a failed restore but a wipe. The one transaction is what
    // prevents it, and the copy-mode rollback test does not cover it — there the
    // worst case is a stray portfolio, here it is an empty account. So the same
    // mid-write failure is driven through `Mode::Replace`, over a caller who owns
    // real rows, and the assertion is their whole fingerprint: a rolled-back delete
    // has to give back the same ids, not merely the same count.
    let (pool, user) = db_with_user("a_replace_that_fails_partway_deletes_nothing_either").await;
    seed_two_portfolios(&pool, user).await;
    let mine_before = owned_fingerprint(&pool, user).await;
    assert_eq!(
        mine_before.len(),
        4,
        "the caller must own two portfolios, one asset and one transaction to lose: \
         {mine_before:?}"
    );

    // Valid apart from the one corrupted field, exactly as the route would have
    // received it, so the only failure available is the one after the deletes.
    let mut bundle = parse_and_validate(bundle_json("Only").as_bytes())
        .expect("the fixture is a valid bundle");
    bundle.portfolios[0].assets[0].transactions[0].date = "not-a-date".into();

    let err = backup_restore::apply(
        &pool,
        user,
        &bundle,
        Mode::Replace,
        false,
        &BackupConfig::default(),
    )
    .await
    .expect_err("a date no parser accepts stops the write");
    assert!(matches!(err, ApplyError::Db(DbFailure::Other)), "{err:?}");
    assert_eq!(
        owned_fingerprint(&pool, user).await,
        mine_before,
        "a replace that failed must not have deleted anything either"
    );

    // Positive control: the same `apply`, the same fixture, the same pool, minus the
    // corrupted field, does delete what the caller owned. Without it the assertion
    // above could be satisfied by an `apply` whose deletes never run on *any* path.
    let clean = parse_and_validate(bundle_json("Only").as_bytes())
        .expect("the fixture is a valid bundle");
    backup_restore::apply(
        &pool,
        user,
        &clean,
        Mode::Replace,
        false,
        &BackupConfig::default(),
    )
    .await
    .expect("the uncorrupted bundle applies");
    let mine_after = owned_fingerprint(&pool, user).await;
    assert!(
        mine_after.len() < mine_before.len(),
        "a replace that succeeds must leave the caller holding fewer rows than before: \
         {mine_after:?}"
    );
}

/// A plan is a statement about a moment. If rows appeared between the preview and the
/// restore, `replace` deletes what is there now — so the outcome has to report the
/// deletes it actually made, not repeat the plan's number.
#[tokio::test]
async fn restore_reports_the_rows_it_actually_deleted() {
    let (pool, user) = db_with_user("restore_reports_the_rows_it_actually_deleted").await;
    let pid = seed_portfolio(&pool, user, "Existing", "USD").await;
    let aid = seed_asset(&pool, pid, "EX", "Existing").await;
    seed_transaction(&pool, aid, "BUY", 1.0, 3.0, 0.0).await;
    seed_transaction(&pool, aid, "BUY", 1.0, 4.0, 0.0).await;
    // A second portfolio holding a second asset, so that every count below differs from
    // the bundle's 1/1/1. With one portfolio holding one asset only the transactions
    // count could tell `rows_affected` from an echo of the bundle's own numbers.
    let other = seed_portfolio(&pool, user, "Other", "USD").await;
    seed_asset(&pool, other, "OT", "Other").await;

    let c = client(pool.clone(), user).await;
    let res = c
        .post("/api/backup/restore?mode=replace")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle_with_size(1, 1, 1))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let out: Value = res.into_json().await.unwrap();

    assert_eq!(out["deleted"]["portfolios"], 2);
    assert_eq!(out["deleted"]["assets"], 2);
    assert_eq!(out["deleted"]["transactions"], 2);
    assert_eq!(out["inserted"]["portfolios"], 1);
}

#[tokio::test]
async fn copy_mode_reports_nothing_deleted() {
    let (pool, user) = db_with_user("copy_mode_reports_nothing_deleted").await;
    seed_portfolio(&pool, user, "Kept", "USD").await;

    let c = client(pool.clone(), user).await;
    let res = c
        .post("/api/backup/restore?mode=copy")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle_with_size(1, 1, 1))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let out: Value = res.into_json().await.unwrap();

    assert_eq!(out["deleted"]["portfolios"], 0);
    assert_eq!(out["deleted"]["assets"], 0);
    assert_eq!(out["deleted"]["transactions"], 0);
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
    let (pool, user) = db_with_user("every_apply_prices_on_spelling_applies_the_price_section").await;
    let c = price_restore_client(pool.clone(), user).await;

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
    let (pool, user) = db_with_user("every_apply_prices_off_spelling_and_an_absent_parameter_write_no_prices").await;
    let c = price_restore_client(pool.clone(), user).await;

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
    let (pool, user) = db_with_user("an_unrecognised_apply_prices_is_a_422_not_a_quiet_ignore").await;
    let c = price_restore_client(pool, user).await;

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
    let (pool, user) = db_with_user("a_repeated_restore_parameter_is_refused_not_half_read").await;
    let c = client(pool, user).await;

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
    let (pool, user) = db_with_user("asking_to_apply_prices_while_the_flag_is_off_is_a_422").await;
    let before = rows(&pool).await;
    let c = client(pool.clone(), user).await; // `build_rocket`'s config: the flag is off
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
    assert_eq!(
        price_rows(&pool).await,
        0,
        "and that includes the shared historical_prices rows the caller asked for"
    );

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

#[tokio::test]
async fn a_replace_that_asks_for_prices_while_the_flag_is_off_deletes_nothing() {
    // The copy-mode refusal above cannot see this: `replace` is the mode that destroys
    // data, and the flag check is the first thing `apply` does, before the transaction
    // that would delete is even opened. Move that check after the deletes and this
    // answers the same 422 over an empty portfolio list.
    let (pool, user) = db_with_user("a_replace_that_asks_for_prices_while_the_flag_is_off_deletes_nothing").await;
    seed_two_portfolios(&pool, user).await;
    let before = rows(&pool).await;
    assert_ne!(before, (0, 0, 0), "a refusal that deletes must have something to delete");
    let c = client(pool.clone(), user).await; // the flag is off

    let res = c
        .post("/api/backup/restore?mode=replace&apply_prices=1")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle_json_with_prices("Replaced"))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
    let body: Value = res.into_json().await.unwrap();
    assert!(
        body["error"].as_str().unwrap().contains("price restore disabled"),
        "{body}"
    );
    assert_eq!(rows(&pool).await, before, "a refusal must not delete");
    assert_eq!(
        price_rows(&pool).await,
        0,
        "and must not write the shared rows it refused to write"
    );
}

// ---------------------------------------------------------------------------
// What a price restore actually writes.
//
// `historical_prices` has no `owner_id` and hangs off no portfolio: it is market data
// every tenant's dated reports read. That is the entire reason `BACKUP_ALLOW_PRICE_RESTORE`
// exists and defaults off, and it is what these tests make visible rather than assume:
// the spec's rule (design.md:291-295) is `ON CONFLICT (symbol, date) DO NOTHING`, so a
// bundle fills gaps and cannot overwrite a close the server already has; the date and
// the decimal string are text and must survive the trip unchanged; and a price row that
// cannot be written costs the whole restore, because the price inserts sit inside the
// same transaction as the portfolio rows — which is what
// `a_price_row_the_database_refuses_rolls_back_the_price_rows_written_before_it` proves,
// with a trigger, rather than assuming it.
// ---------------------------------------------------------------------------

/// `common::seed_price` with the date spelled the way `prices[].date` spells it.
/// `seed_daily_prices` is the dense-range form, and a gap is the thing under test here.
async fn seed_price_on(pool: &DbPool, symbol: &str, day: &str, close: &str) {
    let day = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").expect("a YYYY-MM-DD date");
    common::seed_price(pool, symbol, day, close).await;
}

#[tokio::test]
async fn a_price_restore_fills_gaps_and_never_overwrites_an_existing_close() {
    let (pool, user) = db_with_user("a_price_restore_fills_gaps_and_never_overwrites_an_existing_close").await;
    seed_price_on(&pool, "AAPL", "2024-01-15", "149.50").await;
    let c = price_restore_client(pool.clone(), user).await;

    let mut bundle = bundle_value("Priced");
    bundle["prices"] = json!([
        // This date already exists: `DO NOTHING` means this close must not land.
        { "symbol": "AAPL", "date": "2024-01-15", "close_price": "666.00" },
        // This one is a gap, so it must.
        { "symbol": "AAPL", "date": "2024-02-01", "close_price": "180.00" },
        // The same gap twice inside one bundle: the first write wins and the second
        // conflicts with it, so a bundle cannot double-insert its own row either.
        { "symbol": "AAPL", "date": "2024-02-01", "close_price": "999.00" }
    ]);

    let res = c
        .post("/api/backup/restore?mode=copy&apply_prices=1")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle.to_string())
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(
        body["prices_inserted"],
        1,
        "one date was already stored and the third row repeated it inside the bundle: {body}"
    );

    let stored: Vec<(String, String)> = sqlx::query_as(
        "SELECT date::text, close_price FROM historical_prices ORDER BY date",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        stored,
        vec![
            ("2024-01-15".to_string(), "149.50".to_string()),
            ("2024-02-01".to_string(), "180.00".to_string()),
        ],
        "the existing close survives untouched and the gap is filled exactly once"
    );
}

#[tokio::test]
async fn a_price_row_that_cannot_be_written_rolls_the_restore_back_with_it() {
    // Over HTTP a bad `prices[].date` is refused by validation before a transaction
    // opens, so the only caller that can put a failure *after* the portfolio rows is
    // one that skipped validation — the same caller the module's other rollback tests
    // drive, for the same reason.
    let (pool, user) = db_with_user("a_price_row_that_cannot_be_written_rolls_the_restore_back_with_it").await;
    seed_two_portfolios(&pool, user).await;
    let before = rows(&pool).await;
    assert_ne!(before, (0, 0, 0), "a rollback needs something to roll back over");

    let mut bundle = parse_and_validate(bundle_priced("Priced", "AAPL").as_bytes())
        .expect("the fixture is a valid bundle");
    bundle.prices[0].date = "15-01-2024".into();

    let err = backup_restore::apply(
        &pool,
        user,
        &bundle,
        Mode::Copy,
        true,
        &BackupConfig { allow_price_restore: true },
    )
    .await
    .expect_err("a date no parser accepts stops the write");
    assert!(matches!(err, ApplyError::Db(DbFailure::Other)), "{err:?}");
    assert_eq!(
        rows(&pool).await,
        before,
        "the portfolio the price row cost must have rolled back with it"
    );
    assert_eq!(price_rows(&pool).await, 0);

    // Positive control: the same fixture minus the corrupted date writes both halves.
    // Without it these assertions would be satisfied by an `apply` that never reached
    // the price branch at all.
    let clean = parse_and_validate(bundle_priced("Priced", "AAPL").as_bytes())
        .expect("the fixture is a valid bundle");
    let out = backup_restore::apply(
        &pool,
        user,
        &clean,
        Mode::Copy,
        true,
        &BackupConfig { allow_price_restore: true },
    )
    .await
    .expect("the uncorrupted bundle applies");
    assert_eq!(out.prices_inserted, 1, "{out:?}");
    assert_eq!(rows(&pool).await, (before.0 + 1, before.1 + 1, before.2 + 1));
}

#[tokio::test]
async fn a_price_row_the_database_refuses_rolls_back_the_price_rows_written_before_it() {
    // The claim this exists to pin is narrower than the one above and much easier to
    // lose: the price inserts run on the restore's **transaction**, not on the pool.
    // The bad-date test above cannot see it, because a bad `prices[].date` is refused
    // by validation and then by the Rust parse *before any price SQL runs* — so that
    // test passes just as happily if the loop executes on the pool, which in production
    // would commit shared `historical_prices` rows and then roll back the caller's
    // bundle, leaving market data every other tenant reads from a restore that
    // reported failure. A trigger is the only way to make the database refuse a price
    // row that the Rust side already approved.
    let (pool, user) = db_with_user("a_price_row_the_database_refuses_rolls_back_the_price_rows_written_before_it").await;
    seed_two_portfolios(&pool, user).await;
    let before = rows(&pool).await;
    assert_ne!(before, (0, 0, 0), "a rollback needs something to roll back over");

    // A trigger, because a row the *database* refuses is the only failure the Rust
    // side cannot have already caught. Postgres needs a function for it; SQLite's
    // inline `BEGIN SELECT RAISE(ABORT, ...) END` has no equivalent.
    sqlx::query(
        "CREATE FUNCTION refuse_the_second_price() RETURNS trigger AS $$ \
         BEGIN \
             IF NEW.date = '2024-01-02' THEN \
                 RAISE EXCEPTION 'boom'; \
             END IF; \
             RETURN NEW; \
         END \
         $$ LANGUAGE plpgsql",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TRIGGER refuse_the_second_price BEFORE INSERT ON historical_prices \
         FOR EACH ROW EXECUTE FUNCTION refuse_the_second_price()",
    )
    .execute(&pool)
    .await
    .unwrap();

    let mut bundle = bundle_value("Two prices");
    bundle["prices"] = json!([
        // Writes cleanly, and must be undone by the row that does not.
        { "symbol": "AAPL", "date": "2024-01-01", "close_price": "150.00" },
        // Trips the trigger.
        { "symbol": "AAPL", "date": "2024-01-02", "close_price": "151.00" }
    ]);
    let bundle = parse_and_validate(bundle.to_string().as_bytes())
        .expect("the fixture is a valid bundle");

    // Bounded on purpose, as a belt to the assertion's braces. Under SQLite the
    // mutation this test exists to catch — the price loop running on the **pool**
    // instead of on `&mut *tx` — did not fail, it *hung* on the file write lock. Under
    // Postgres MVCC a second connection simply inserts and commits, so the mutation is
    // caught by `price_rows` below rather than by the timeout; the guard stays because a
    // future change that makes the two paths contend on a row lock should fail loudly
    // and quickly, not on a CI timeout nobody reads. Far longer than two rows need.
    const HANG_GUARD: std::time::Duration = std::time::Duration::from_secs(30);
    let err = tokio::time::timeout(
        HANG_GUARD,
        backup_restore::apply(
            &pool,
            user,
            &bundle,
            Mode::Copy,
            true,
            &BackupConfig { allow_price_restore: true },
        ),
    )
    .await
    .unwrap_or_else(|_| panic!(
        "apply did not return within {HANG_GUARD:?} — a price insert on the pool rather than \
         on the restore's transaction waits forever on the lock this transaction holds"
    ))
    .expect_err("a price row the database itself refuses stops the write");
    assert!(matches!(err, ApplyError::Db(DbFailure::Other)), "{err:?}");
    assert_eq!(
        price_rows(&pool).await,
        0,
        "the price row that succeeded must roll back with the one that failed"
    );
    assert_eq!(
        rows(&pool).await,
        before,
        "and the bundle's own portfolio rows with it"
    );

    // Positive control: with the trigger gone the same bundle writes both halves, so
    // the assertions above are a rollback and not a restore that never reached the
    // price section at all.
    // Postgres scopes a trigger to its table, so the drop names both.
    sqlx::query("DROP TRIGGER refuse_the_second_price ON historical_prices")
        .execute(&pool)
        .await
        .unwrap();
    let out = backup_restore::apply(
        &pool,
        user,
        &bundle,
        Mode::Copy,
        true,
        &BackupConfig { allow_price_restore: true },
    )
    .await
    .expect("the untriggered bundle applies");
    assert_eq!(out.prices_inserted, 2, "{out:?}");
    assert_eq!(price_rows(&pool).await, 2);
    assert_eq!(rows(&pool).await, (before.0 + 1, before.1 + 1, before.2 + 1));
}

#[tokio::test]
async fn a_restored_price_row_is_market_data_every_other_tenant_reads() {
    // The stakes of the flag, shown from the other side: user B never asked for
    // anything here, and user A's price restore still lands in B's export.
    let (pool, user) = db_with_user("a_restored_price_row_is_market_data_every_other_tenant_reads").await;
    seed_two_portfolios(&pool, user).await; // user A, holding AAPL
    let other = common::seed_user(&pool, "user-b").await;
    let theirs = seed_portfolio(&pool, other, "Theirs", "USD").await;
    seed_asset(&pool, theirs, "AAPL", "Apple").await; // and so does B

    let b = Client::tracked(build_rocket(pool.clone(), other)).await.unwrap();
    let export = format!("/api/backup/export?portfolio_ids={theirs}&include_prices=1");
    let res = b.get(&export).header(session()).dispatch().await;
    let before: Value = res.into_json().await.unwrap();
    assert!(before.get("prices").is_none(), "nothing is stored yet: {before}");

    let bundle = bundle_priced("Priced", "AAPL");
    let restore = "/api/backup/restore?mode=copy&apply_prices=1";

    // Flag off: refused out loud, and B's view is untouched — the refusal writes no
    // shared rows at all, not even the one the caller asked for.
    let off = client(pool.clone(), user).await;
    let res = off
        .post(restore)
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle.clone())
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
    assert_eq!(price_rows(&pool).await, 0, "a refusal writes no market data");
    let res = b.get(&export).header(session()).dispatch().await;
    let still: Value = res.into_json().await.unwrap();
    assert!(
        still.get("prices").is_none(),
        "B can see nothing of a refused restore: {still}"
    );

    // Flag on: the row lands, and it lands in B's export too, because there is no
    // owner column to keep it out of.
    let on = price_restore_client(pool.clone(), user).await;
    let res = on
        .post(restore)
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle)
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["prices_inserted"], 1, "{body}");

    let res = b.get(&export).header(session()).dispatch().await;
    let seen: Value = res.into_json().await.unwrap();
    assert_eq!(
        seen["prices"],
        json!([{ "symbol": "AAPL", "date": "2024-01-01", "close_price": "150.00" }]),
        "A's restore is exactly what B now reads"
    );
}

#[tokio::test]
async fn the_price_section_round_trips_byte_for_byte() {
    // `prices[].date` is `YYYY-MM-DD` text and `close_price` is a decimal string, and
    // both columns are TEXT, so the bundle carries them verbatim and must write them
    // back verbatim. A helpful re-scale — `149.5070` to `149.51`, `0.010000` to `0.01`
    // — is a silently different price, and a re-formatted date is a different day to
    // every reader that parses this column.
    let (pool, user) = db_with_user("the_price_section_round_trips_byte_for_byte").await;
    let (one, _) = seed_two_portfolios(&pool, user).await;
    seed_price_on(&pool, "AAPL", "2024-02-29", "149.5070").await; // a leap day
    seed_price_on(&pool, "AAPL", "2024-03-01", "0.010000").await; // trailing zeros
    let c = price_restore_client(pool.clone(), user).await;

    let path = format!("/api/backup/export?portfolio_ids={one}&include_prices=1");
    let res = c.get(&path).header(session()).dispatch().await;
    assert_eq!(res.status(), Status::Ok);
    let exported: Value = res.into_json().await.unwrap();
    assert_eq!(
        exported["prices"],
        json!([
            { "symbol": "AAPL", "date": "2024-02-29", "close_price": "149.5070" },
            { "symbol": "AAPL", "date": "2024-03-01", "close_price": "0.010000" }
        ]),
        "the export half of the round trip"
    );

    // The write half: empty the table, restore the file that just came out of the
    // export, and every character comes back.
    sqlx::query("DELETE FROM historical_prices").execute(&pool).await.unwrap();
    let res = c
        .post("/api/backup/restore?mode=copy&apply_prices=1")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(exported.to_string())
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["prices_inserted"], 2, "{body}");

    let stored: Vec<(String, String)> = sqlx::query_as(
        "SELECT date::text, close_price FROM historical_prices ORDER BY date",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        stored,
        vec![
            ("2024-02-29".to_string(), "149.5070".to_string()),
            ("2024-03-01".to_string(), "0.010000".to_string()),
        ],
        "the restore wrote back exactly what the export read"
    );
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
    let (pool, user) = db_with_user("a_bundle_whose_second_portfolio_is_invalid_writes_nothing").await;
    seed_two_portfolios(&pool, user).await;
    let before = rows(&pool).await;
    let c = client(pool.clone(), user).await;

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

/// Two hundred valid transactions and one unusable one at the end: the shape of a
/// bundle where everything up to the last row is fine, so a partial application would
/// be two hundred and one rows of a file the caller has to find by hand.
fn bundle_with_bad_last_row() -> String {
    let mut b = bundle_value("Many rows");
    let txs = b["portfolios"][0]["assets"][0]["transactions"]
        .as_array_mut()
        .unwrap();
    for i in 0..200 {
        txs.push(json!({ "type": "BUY", "quantity": "1", "price": "1", "fee": "0",
                         "date": format!("2024-01-{:02}T00:00:00Z", (i % 28) + 1) }));
    }
    // Validation catches this before the transaction opens, which is the point: the
    // write path is never reached at all.
    txs.push(json!({ "type": "BUY", "quantity": "not-a-number", "price": "1",
                     "fee": "0", "date": "2024-02-01T00:00:00Z" }));
    b.to_string()
}

#[tokio::test]
async fn a_bundle_that_fails_on_the_last_row_writes_nothing() {
    let (pool, user) = db_with_user("a_bundle_that_fails_on_the_last_row_writes_nothing").await;
    seed_two_portfolios(&pool, user).await;
    let before = rows(&pool).await;
    assert_ne!(before, (0, 0, 0), "a rollback needs something to roll back over");
    let c = client(pool.clone(), user).await;

    let res = c
        .post("/api/backup/restore?mode=copy")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle_with_bad_last_row())
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
    assert_eq!(
        rows(&pool).await,
        before,
        "byte-identical: nothing half-applied"
    );
}

#[tokio::test]
async fn a_write_that_fails_after_the_first_portfolio_rolls_everything_back() {
    // The baseline is deliberately not empty. From `(0, 0, 0)` "rolled back after
    // writing the first portfolio" and "never wrote anything" are the same
    // assertion, and only one of them is the guarantee — so the database starts with
    // rows to roll back over, and the positive control at the bottom proves this
    // exact fixture is capable of writing.
    let (pool, user) = db_with_user("a_write_that_fails_after_the_first_portfolio_rolls_everything_back").await;
    seed_two_portfolios(&pool, user).await;
    let before = rows(&pool).await;
    assert_ne!(before, (0, 0, 0), "a rollback needs something to roll back over");
    // Parsed as a real bundle first, so the only thing wrong with it is the one field
    // corrupted below: everything else is exactly what the route would have written.
    let mut bundle = parse_and_validate(two_portfolio_bundle().as_bytes())
        .expect("the fixture is a valid bundle");
    bundle.portfolios[1].assets[0].transactions[0].date = "not-a-date".into();

    let err = backup_restore::apply(
        &pool,
        user,
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

    // Positive control: the same `apply`, the same fixture, the same pool, minus the
    // corrupted field. Without it the assertion above could be satisfied by an
    // `apply` that wrote nothing on *any* path.
    let clean = parse_and_validate(two_portfolio_bundle().as_bytes())
        .expect("the fixture is a valid bundle");
    let out = backup_restore::apply(
        &pool,
        user,
        &clean,
        Mode::Copy,
        false,
        &BackupConfig::default(),
    )
    .await
    .expect("the uncorrupted bundle applies");
    assert_eq!(
        (
            out.inserted.portfolios,
            out.inserted.assets,
            out.inserted.transactions
        ),
        (2, 2, 2),
        "this fixture does write through this function: {out:?}"
    );
    assert_eq!(rows(&pool).await, (before.0 + 2, before.1 + 2, before.2 + 2));
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
    let (pool, user) = db_with_user("a_restore_writes_the_caller_and_never_another_user_s_rows").await;
    seed_two_portfolios(&pool, user).await;
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

    let c = client(pool.clone(), user).await; // runs as user, who owns none of B's rows
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
    assert_ne!(new_id, theirs, "a foreign ref must not become an id");

    // The caller owns what the caller restored...
    let owner: (Option<i64>,) = sqlx::query_as("SELECT owner_id FROM portfolios WHERE id = $1")
        .bind(new_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(owner.0, Some(user));

    // ...and B's rows are untouched, in count and in owner.
    let theirs_now: (i64, i64, i64) = sqlx::query_as(
        "SELECT \
         (SELECT COUNT(*) FROM portfolios WHERE owner_id = $1), \
         (SELECT COUNT(*) FROM assets WHERE portfolio_id IN \
          (SELECT id FROM portfolios WHERE owner_id = $2)), \
         (SELECT COUNT(*) FROM transactions WHERE asset_id IN \
          (SELECT id FROM assets WHERE portfolio_id IN \
           (SELECT id FROM portfolios WHERE owner_id = $3)))",
    )
    .bind(other)
    .bind(other)
    .bind(other)
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
    let (pool, user) = db_with_user("a_bundle_ref_never_reaches_a_row_even_when_it_is_blank").await;
    let before = rows(&pool).await;
    let mut bundle = bundle_value("Blank refs");
    bundle["portfolios"][0]["ref"] = json!("");
    bundle["portfolios"][0]["assets"][0]["ref"] = json!("   ");
    let c = client(pool.clone(), user).await;

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
         (SELECT id FROM assets WHERE portfolio_id = $1)",
    )
    .bind(new_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(chain.0, 1);
    // And no column holds a ref: the name is what the bundle's `name` said.
    let (name,): (String,) = sqlx::query_as("SELECT name FROM portfolios WHERE id = $1")
        .bind(new_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(name, "Blank refs");
    assert_eq!(rows(&pool).await, (before.0 + 1, before.1 + 1, before.2 + 1));
}

#[tokio::test]
async fn replace_leaves_no_orphan_split_sync_rows() {
    // `split_sync.asset_id` is a foreign key to `assets` in `0001_init.sql` with no
    // cascade — it was a plain INTEGER PRIMARY KEY with no foreign key in the SQLite
    // table, which is why this delete used to be merely tidy rather than required.
    // A replace that forgot it could not even delete the asset now; the row is dropped
    // first so it does not outlive the asset, and so the next asset handed that id
    // cannot inherit a stranger's split-sync throttle. The delete order is the other
    // half — no FK here cascades, so children go first.
    // `tests/foreign_keys_test.rs` pins the constraint itself.
    let (pool, user) = db_with_user("replace_leaves_no_orphan_split_sync_rows").await;
    let (one, _) = seed_two_portfolios(&pool, user).await;
    let asset = seed_asset(&pool, one, "MSFT", "Microsoft").await;
    common::seed_split_sync(&pool, asset).await;
    let (before,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM split_sync")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, 1, "the fixture must actually have a row to orphan");

    let c = client(pool.clone(), user).await;
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
    let (pool, _) = db_with_user("restore_requires_a_session").await;
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
    let (pool, user) = db_with_user("restore_refuses_an_upload_it_cannot_read_or_parse").await;
    seed_two_portfolios(&pool, user).await;
    let before = rows(&pool).await;
    let c = client(pool.clone(), user).await;

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

// ---------------------------------------------------------------------------
// The caps at their exact boundaries.
//
// `MAX_BUNDLE_BYTES` and `MAX_BUNDLE_ROWS` are the two ceilings the feature promises,
// and each is enforced twice: on the way out (an export over either is a file the same
// build's restore would refuse) and on the way in (an upload over either is refused
// before a row is written). A test that only checks "enormous is refused" would pass
// with a ceiling one row too small — which is a user who cannot back up their own
// portfolio — or one row too large. So every pair here is at the cap and one over.
// ---------------------------------------------------------------------------

/// The `bundle_value` fixture carries 3 rows (one portfolio, one asset, one
/// transaction). This pads it with `extra_transactions` more transactions and `prices`
/// price rows, so a test can sit exactly on `MAX_BUNDLE_ROWS`: the ceiling counts every
/// row a restore must write, price rows included.
fn bundle_of_rows(extra_transactions: usize, prices: usize) -> String {
    let mut b = bundle_value("Cap boundary");
    {
        let txs = b["portfolios"][0]["assets"][0]["transactions"]
            .as_array_mut()
            .unwrap();
        for i in 0..extra_transactions {
            txs.push(json!({ "type": "BUY", "quantity": "1", "price": "1", "fee": "0",
                             "date": format!("2024-01-{:02}T00:00:00Z", (i % 28) + 1) }));
        }
    }
    b["prices"] = (0..prices)
        .map(|i| {
            json!({
                "symbol": "AAPL",
                // Every pair here is a real day. Two rows sharing one is legal in a
                // bundle — the restore's `ON CONFLICT DO NOTHING` settles it.
                "date": format!("2024-{:02}-{:02}", (i % 12) + 1, (i % 28) + 1),
                "close_price": "1",
            })
        })
        .collect();
    b.to_string()
}

#[tokio::test]
async fn the_row_cap_is_exact_on_the_export_side() {
    let (pool, user) = db_with_user("the_row_cap_is_exact_on_the_export_side").await;
    let (one, _) = seed_two_portfolios(&pool, user).await;
    let asset = seed_asset(&pool, one, "MSFT", "Microsoft").await;
    // The selection is portfolio `one` alone: itself, the two assets on it, and their
    // transactions — 4 rows before this seed, so this is the cap exactly.
    seed_many_transactions(&pool, asset, MAX_BUNDLE_ROWS - 4).await;
    let c = client(pool.clone(), user).await;
    let path = format!("/api/backup/export?portfolio_ids={one}");

    let res = c.get(&path).header(session()).dispatch().await;
    assert_eq!(res.status(), Status::Ok, "exactly at the ceiling must export");
    let body: Value = res.into_json().await.unwrap();
    let exported = body["portfolios"][0]["assets"][1]["transactions"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(exported, MAX_BUNDLE_ROWS - 4, "the fixture is the ceiling, not less");

    seed_many_transactions(&pool, asset, 1).await;
    let res = c.get(&path).header(session()).dispatch().await;
    assert_eq!(res.status(), Status::UnprocessableEntity, "one row over is refused");
    let body: Value = res.into_json().await.unwrap();
    let err = body["error"].as_str().unwrap();
    assert!(err.contains(&MAX_BUNDLE_ROWS.to_string()), "{err}");
    assert!(err.contains("row limit"), "{err}");
}

#[tokio::test]
async fn the_byte_cap_is_exact_on_the_upload_side() {
    // A bundle of exactly `MAX_BUNDLE_BYTES` is read whole and answered with a plan;
    // one byte more is this module's own `413`, named by the number, before any of it
    // is parsed. It is a `413` and not a `422` because the cap is opened per route
    // rather than by raising a global Rocket limit (see
    // `no_global_body_limit_is_raised_for_the_bundle`), so an oversized body never
    // reaches validation to be answered as a bad bundle.
    let (pool, user) = db_with_user("the_byte_cap_is_exact_on_the_upload_side").await;
    seed_two_portfolios(&pool, user).await;
    let before = rows(&pool).await;
    let c = client(pool.clone(), user).await;

    // The pad is the shortfall against the cap, and `x` needs no JSON escaping, so the
    // serialised bundle is exactly `MAX_BUNDLE_BYTES` long rather than roughly it.
    let mut at_cap = bundle_value("Exactly at the cap");
    at_cap["portfolios"][0]["description"] = json!("");
    let base = serde_json::to_vec(&at_cap).unwrap().len();
    at_cap["portfolios"][0]["description"] = json!("x".repeat(MAX_BUNDLE_BYTES - base));
    let at_cap = at_cap.to_string();
    assert_eq!(at_cap.len(), MAX_BUNDLE_BYTES, "the fixture is the cap, not less");

    let res = c
        .post("/api/backup/preview?mode=copy")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(at_cap)
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok, "exactly at the ceiling must be read");

    let res = c
        .post("/api/backup/preview?mode=copy")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(body_of(MAX_BUNDLE_BYTES + 1))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::PayloadTooLarge);
    let body: Value = res.into_json().await.unwrap();
    assert!(
        body["error"].as_str().unwrap().contains(&MAX_BUNDLE_BYTES.to_string()),
        "{body}"
    );
    assert_eq!(rows(&pool).await, before, "neither answer writes anything");
}

#[tokio::test]
async fn a_bundle_over_the_row_cap_is_refused_before_any_write() {
    let (pool, user) = db_with_user("a_bundle_over_the_row_cap_is_refused_before_any_write").await;
    seed_two_portfolios(&pool, user).await;
    let before = rows(&pool).await;
    let c = client(pool.clone(), user).await;

    // Both routes that read a bundle, and in particular the one that can write: a cap
    // enforced only by `preview` would be a suggestion, and the refusal on the write route
    // is the one that has to leave the database alone.
    let over_cap = bundle_of_rows(MAX_BUNDLE_ROWS - 2, 0);
    for (route, path) in [
        ("preview", "/api/backup/preview?mode=copy"),
        ("restore", "/api/backup/restore?mode=copy"),
    ] {
        let res = c
            .post(path)
            .header(session())
            .header(rocket::http::ContentType::JSON)
            .body(over_cap.clone())
            .dispatch()
            .await;
        assert_eq!(res.status(), Status::UnprocessableEntity, "{route}");
        let body: Value = res.into_json().await.unwrap();
        assert!(
            body["errors"].to_string().contains("row limit"),
            "{route}: {body}"
        );
        assert_eq!(rows(&pool).await, before, "{route}: the refusal writes nothing");
    }
}

#[tokio::test]
async fn a_bundle_at_exactly_the_row_cap_is_restored() {
    // The other side of the same boundary, on the route that writes: a bundle the
    // ceiling allows must actually restore, or the ceiling is a quiet refusal to back
    // up a legal portfolio.
    let (pool, user) = db_with_user("a_bundle_at_exactly_the_row_cap_is_restored").await;
    let before = rows(&pool).await;
    let c = client(pool.clone(), user).await;

    let res = c
        .post("/api/backup/restore?mode=copy")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(bundle_of_rows(MAX_BUNDLE_ROWS - 3, 0))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok, "exactly at the ceiling must restore");
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(
        body["inserted"]["transactions"],
        json!((MAX_BUNDLE_ROWS - 2) as i64),
        "every row of the bundle landed: {body}"
    );
    assert_eq!(
        rows(&pool).await,
        (before.0 + 1, before.1 + 1, before.2 + (MAX_BUNDLE_ROWS - 2) as i64)
    );
}

#[tokio::test]
async fn price_rows_count_toward_the_row_cap_on_the_restore_side() {
    // The ceiling is a bound on the work one restore does inside one write transaction,
    // so it counts the rows a restore inserts into `historical_prices` too. A bundle
    // can carry tens of thousands of them while its caller's own rows are a handful,
    // and ~120k single-row INSERTs under a cap that only counted portfolios is how a
    // restore comes to hold the write lock past the 5s `busy_timeout` and answer every
    // concurrent request with a 503.
    let (pool, user) = db_with_user("price_rows_count_toward_the_row_cap_on_the_restore_side").await;
    let c = client(pool.clone(), user).await;

    let at_cap = bundle_of_rows(0, MAX_BUNDLE_ROWS - 3);
    let over_cap = bundle_of_rows(0, MAX_BUNDLE_ROWS - 2);

    let res = c
        .post("/api/backup/preview?mode=copy")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(at_cap)
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok, "the price rows fit inside the ceiling");
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["prices"], json!((MAX_BUNDLE_ROWS - 3) as i64), "{body}");

    let res = c
        .post("/api/backup/preview?mode=copy")
        .header(session())
        .header(rocket::http::ContentType::JSON)
        .body(over_cap)
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::UnprocessableEntity, "one price row over is over");
    let body: Value = res.into_json().await.unwrap();
    assert!(body["errors"].to_string().contains("row limit"), "{body}");
}
