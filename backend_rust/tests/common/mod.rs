//! The shared toolbox every integration-test binary includes with `mod common;`.
//!
//! It is one file serving twenty-odd binaries, so almost every binary leaves some of
//! it unused; `dead_code` is allowed for the whole module rather than function by
//! function, which is the only way a helper can be added without hunting down which
//! of the binaries will not call it.
#![allow(dead_code)]

use backend_rust::auth::{AuthMode, OidcConfig, StartupConfig};
use backend_rust::backup::BackupConfig;
use backend_rust::db::DbPool;
use backend_rust::services::currency_service::CurrencyService;
use backend_rust::{build_rocket as lib_build_rocket, ensure_bootstrap_owner, get_cors_options};
use chrono::{DateTime, NaiveDate, Utc};
use rocket::{Build, Rocket};

// The mock lives in `src/test_provider.rs` so the library's own unit tests can use
// it too; this include compiles the same file into the integration-test binary.
#[path = "../../src/test_provider.rs"]
mod test_provider;

// Likewise `src/test_db.rs`: one implementation of "a database this test owns alone",
// shared by the unit suite and the integration suite rather than copied into each.
#[path = "../../src/test_db.rs"]
mod test_db;

// Each integration-test binary uses a subset of these; the ones it does not use are
// not dead code in the file, only in that binary.
#[allow(unused_imports)]
pub use test_provider::{MockYahoo, mock_pricing_url};

/// The user every existing test runs as: the placeholder row
/// [`ensure_bootstrap_owner`] upserts unconditionally, which is `id = 1` on a freshly
/// migrated database — the identity restarts at 1 and the placeholder is the first
/// row any fixture path creates.
///
/// `i64` because `users.id` is `BIGINT`: every seed helper returns a row id, and a
/// fixture that narrowed one to `i32` would be a fixture that could not express the
/// ids the database produces.
pub const DEFAULT_USER: i64 = 1;

/// A `StartupConfig` asking for `disabled` mode, built through the injectable
/// environment source rather than by mutating the process environment (which is
/// `unsafe` in edition 2024 and racy across parallel tests).
pub fn disabled_config() -> StartupConfig {
    StartupConfig::from_env_with(&|key| (key == "AUTH_MODE").then(|| "disabled".to_string()))
        .expect("AUTH_MODE=disabled parses in a debug build")
}

/// An `OidcConfig` that parses but is never contacted: provider discovery is lazy,
/// so tests that only assert "no session means 401" need no provider.
pub fn oidc_cfg() -> OidcConfig {
    OidcConfig {
        issuer: "https://auth.example.invalid/application/o/pp".to_string(),
        client_id: "test-client".to_string(),
        client_secret: "test-secret".to_string(),
        redirect_uri: "https://pp.example.invalid/auth/callback".to_string(),
        post_logout_redirect_uri: "https://pp.example.invalid".to_string(),
        bootstrap_subj: None,
    }
}

/// A config that asks for OIDC, for the tests that must exercise the authenticated
/// path rather than the development shortcut.
#[allow(dead_code)] // not every integration-test binary needs the OIDC path
pub fn oidc_config() -> StartupConfig {
    StartupConfig {
        requested: backend_rust::auth::AuthRequest::Oidc,
        oidc: Some(oidc_cfg()),
        bootstrap_subj: None,
    }
}

/// Build the app running as `as_user` (development mode). Tests that need the
/// cookie path construct `AuthMode::Oidc(oidc_cfg())` themselves.
pub fn build_rocket(pool: DbPool, as_user: i64) -> Rocket<Build> {
    rocket_with_service(pool, offline_pricing_service(), as_user)
}

/// `build_rocket` with pricing pointed at a specific provider URL — normally a
/// [`MockYahoo::serving`] with canned data, rather than the plain 404 the default
/// mock gives. `build_rocket` is already offline; this is for tests that need the
/// provider to answer *something*.
#[allow(dead_code)]
pub fn build_rocket_pricing(pool: DbPool, as_user: i64, base_url: &str) -> Rocket<Build> {
    rocket_with_service(pool, CurrencyService::with_base_url(base_url), as_user)
}

#[allow(dead_code)]
/// A `CurrencyService` whose provider is the local 404 mock. Every test rocket uses
/// this; `scripts/check_pricing_seam.sh` fails the build if one does not.
pub fn offline_pricing_service() -> CurrencyService {
    CurrencyService::with_base_url(mock_pricing_url())
}

#[allow(dead_code)]
fn rocket_with_service(
    pool: DbPool,
    currency_service: CurrencyService,
    as_user: i64,
) -> Rocket<Build> {
    let cors = get_cors_options(vec!["http://localhost:5173".to_string()]);
    #[cfg(debug_assertions)]
    let mode = AuthMode::Disabled { as_user };
    lib_build_rocket(pool, currency_service, cors, mode, BackupConfig::default())
}

/// `build_rocket` with a specific `BackupConfig`. There is no other way to set the
/// flag in a test: `std::env::set_var` is `unsafe` in edition 2024 and racy across
/// parallel tests, and Rocket refuses a second `.manage` of a type it already
/// holds — so the value has to enter through `build_rocket`, which is exactly what
/// this exercises.
#[allow(dead_code)]
pub fn build_rocket_with_backup_config(
    pool: DbPool,
    as_user: i64,
    backup_cfg: BackupConfig,
) -> Rocket<Build> {
    let cors = get_cors_options(vec!["http://localhost:5173".to_string()]);
    #[cfg(debug_assertions)]
    let mode = AuthMode::Disabled { as_user };
    lib_build_rocket(pool, offline_pricing_service(), cors, mode, backup_cfg)
}

/// Build the app in OIDC mode — the mode production runs in, and the only one in
/// which unauthenticated requests are actually denied.
pub fn build_rocket_oidc(pool: DbPool) -> Rocket<Build> {
    let currency_service = offline_pricing_service();
    let cors = get_cors_options(vec!["http://localhost:5173".to_string()]);
    lib_build_rocket(
        pool,
        currency_service,
        cors,
        AuthMode::Oidc(oidc_cfg()),
        BackupConfig::default(),
    )
}

/// Build in OIDC mode with an explicit origin allowlist, for the tests that care
/// about how CORS and authentication are ordered.
#[allow(dead_code)]
pub fn build_rocket_oidc_with_cors(pool: DbPool, origins: Vec<String>) -> Rocket<Build> {
    let currency_service = offline_pricing_service();
    let cors = get_cors_options(origins);
    lib_build_rocket(
        pool,
        currency_service,
        cors,
        AuthMode::Oidc(oidc_cfg()),
        BackupConfig::default(),
    )
}

/// A database for one test, migrated to the same head `main` migrates to, and given
/// the same placeholder owner `main` guarantees. Isolation is per call — see
/// `src/test_db.rs` — so a test's rows are its own and nothing truncates them.
///
/// `cfg` decides only whether the placeholder owner is *claimable*, exactly as at
/// startup: under `disabled` the placeholder is usable, under OIDC it is usable only
/// if a bootstrap subject was configured. Tests that pass [`disabled_config`] get the
/// row they have always assumed; a test passing [`oidc_config`] gets an empty `users`
/// table, which is what production would have.
pub async fn setup_db(cfg: &StartupConfig) -> DbPool {
    // Every existing caller wants the placeholder owner and has no name of its own to
    // pass, so the name is counted rather than chosen. Task 5 replaces the call sites
    // with `test_pool("<test name>")`, which names the database after the test that
    // owns it.
    let pool = test_db::test_database(&test_db::next_caller("setup_db")).await;
    ensure_bootstrap_owner(&pool, cfg)
        .await
        .expect("bootstrap owner");
    pool
}

/// A private, freshly migrated database named after `caller`. See `src/test_db.rs`.
pub async fn test_pool(caller: &str) -> DbPool {
    test_db::test_database(caller).await
}

/// The URL of a private database, for a test that must open a **second** pool against
/// the same one — a lock holder and the connection it locks out cannot share a pool.
pub async fn test_pool_url(caller: &str) -> String {
    test_db::test_database_url(caller).await
}

/// The harness pool options: `main`'s, plus `idle_timeout(None)`. See
/// [`crate::test_db::pool_options`] for why the one extra setting is not optional —
/// this exists so a test that needs a *second* pool against the same database
/// (`test_pool_url`) builds it the same way the harness would build the first.
pub fn pool_options() -> sqlx::postgres::PgPoolOptions {
    test_db::pool_options()
}

/// A `caller` unique within this test binary, for a fixture several tests in one
/// binary share and that has no test name of its own to borrow. Prefer passing the
/// test's own name to [`test_pool`]; this is the fallback when the fixture cannot see
/// it.
pub fn unique_caller(prefix: &str) -> String {
    test_db::next_caller(prefix)
}

/// A session cookie pair for `LocalRequest::private_cookie`.
pub fn session_cookie(uid: i64, tv: i64) -> (&'static str, String) {
    (
        backend_rust::auth::session::SESSION_COOKIE,
        backend_rust::auth::session::value(uid, tv, backend_rust::auth::session::expiry_now()),
    )
}

/// A session cookie that is already expired, with a matching token version.
#[allow(dead_code)]
pub fn expired_session_cookie(uid: i64, tv: i64) -> (&'static str, String) {
    (
        backend_rust::auth::session::SESSION_COOKIE,
        backend_rust::auth::session::value(uid, tv, 1_000_000),
    )
}

/// Create a user row and return its id.
#[allow(dead_code)]
pub async fn seed_user(pool: &DbPool, subj: &str) -> i64 {
    let res = sqlx::query_as::<_, (i64,)>(
        "INSERT INTO users (issuer, subj, email, name, created_at) \
         VALUES ('https://auth.example.invalid', $1, 'user@example.com', $2, now()) \
         RETURNING id",
    )
    .bind(subj)
    .bind(format!("User {subj}"))
    .fetch_one(pool)
    .await
    .unwrap();
    res.0
}

/// Create a portfolio owned by `owner_id`.
#[allow(dead_code)]
pub async fn seed_portfolio(pool: &DbPool, owner_id: i64, name: &str, currency: &str) -> i64 {
    let res = sqlx::query_as::<_, (i64,)>(
        "INSERT INTO portfolios (name, description, currency, base_currency, owner_id) VALUES ($1, $2, $3, $4, $5) RETURNING id"
    )
    .bind(name)
    .bind(None::<String>)
    .bind(currency)
    .bind(currency)
    .bind(owner_id)
    .fetch_one(pool)
    .await
    .unwrap();
    res.0
}

#[allow(dead_code)]
pub async fn seed_asset(pool: &DbPool, portfolio_id: i64, symbol: &str, name: &str) -> i64 {
    let res = sqlx::query_as::<_, (i64,)>(
        "INSERT INTO assets (portfolio_id, symbol, name, asset_type, currency) VALUES ($1, $2, $3, $4, 'USD') RETURNING id"
    )
    .bind(portfolio_id)
    .bind(symbol)
    .bind(name)
    .bind("STOCK")
    .fetch_one(pool)
    .await
    .unwrap();
    res.0
}

/// `seed_asset` hardcodes `'USD'`, which is why no fixture in the suite could tell a
/// correct dated-FX implementation from the broken one: the asset currency and the
/// portfolio base currency were always the same string, so every conversion was a
/// no-op. Multi-currency tests use this instead.
#[allow(dead_code)]
pub async fn seed_asset_currency(
    pool: &DbPool,
    portfolio_id: i64,
    symbol: &str,
    name: &str,
    currency: &str,
) -> i64 {
    let res = sqlx::query_as::<_, (i64,)>(
        "INSERT INTO assets (portfolio_id, symbol, name, asset_type, currency) VALUES ($1, $2, $3, $4, $5) RETURNING id"
    )
    .bind(portfolio_id)
    .bind(symbol)
    .bind(name)
    .bind("STOCK")
    .bind(currency)
    .fetch_one(pool)
    .await
    .unwrap();
    res.0
}

/// The text a fixture stores for a float, computed here rather than by the database.
///
/// `quantity`, `price` and `fee` are `text` columns holding a decimal as a string —
/// that is the whole storage policy (spec §3), and it is why a float has no business
/// being bound to them. Binding one anyway worked on SQLite only because SQLite applies
/// the column's TEXT affinity to the incoming REAL, and its float-to-text rule always
/// keeps a decimal point: `100.0` stored as `"100.0"`. Postgres sends the parameter as
/// `float8` and takes the implicit assignment cast, whose rule drops it: `100.0`
/// becomes `"100"`. Same number, different string, and the string is what the API
/// echoes back — so a fixture that let the database do the conversion changed the
/// response body while changing engines.
///
/// `{:?}` is Rust's shortest round-trip float formatting, which keeps a decimal point on
/// integral values exactly as SQLite's affinity did, so the stored text — and therefore
/// the JSON — is unchanged by the switch. `Display` (`{}`) would not: it prints `100`.
fn dec(value: f64) -> String {
    format!("{value:?}")
}

#[allow(dead_code)]
pub async fn seed_transaction(
    pool: &DbPool,
    asset_id: i64,
    tx_type: &str,
    qty: f64,
    price: f64,
    fee: f64,
) -> i64 {
    let date = DateTime::parse_from_rfc3339("2024-06-15T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let res = sqlx::query_as::<_, (i64,)>(
        "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"
    )
    .bind(asset_id)
    .bind(tx_type)
    .bind(dec(qty))
    .bind(dec(price))
    .bind(dec(fee))
    .bind(date)
    .fetch_one(pool)
    .await
    .unwrap();
    res.0
}
#[allow(dead_code)]
pub async fn seed_transaction_at(
    pool: &DbPool,
    asset_id: i64,
    tx_type: &str,
    qty: f64,
    price: f64,
    fee: f64,
    date: &str,
) -> i64 {
    let date = DateTime::parse_from_rfc3339(date)
        .unwrap()
        .with_timezone(&Utc);
    let res = sqlx::query_as::<_, (i64,)>(
        "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"
    )
    .bind(asset_id)
    .bind(tx_type)
    .bind(dec(qty))
    .bind(dec(price))
    .bind(dec(fee))
    .bind(date)
    .fetch_one(pool)
    .await
    .unwrap();
    res.0
}

/// One stored close. The pricing layer reads `historical_prices` before the provider, so
/// a seeded row is what makes a test hermetic. Which row is needed depends on the path:
///
/// * `get_price` (`currency_service.rs:511-535`) answers only from a row whose `date`
///   equals the date asked for, and today's tax-summary route asks for
///   `Utc::now().date_naive()`. Seed **exactly today** or the provider is called.
/// * The dated series path is governed by `stored_window_is_fresh`
///   (`currency_service.rs:65`), which accepts any row within
///   `PRICE_WINDOW_FRESH_DAYS` of the date asked for.
///
/// Do not assume the window rule covers a `get_price` call: a row two days old satisfies
/// the window and still reaches Yahoo.
#[allow(dead_code)]
pub async fn seed_price(pool: &DbPool, symbol: &str, date: NaiveDate, close: &str) {
    sqlx::query(
        "INSERT INTO historical_prices (symbol, date, close_price) VALUES ($1, $2, $3)
             ON CONFLICT(symbol, date) DO UPDATE SET close_price = excluded.close_price",
    )
    .bind(symbol)
    .bind(date)
    .bind(close)
    .execute(pool)
    .await
    .unwrap();
}

/// A close for `symbol` on every calendar day in `[start, end]`. Dense on purpose: the
/// pricing layer answers from the database only when the newest stored row is within
/// `PRICE_WINDOW_FRESH_DAYS` of the date asked for, so a sparse seed reaches the
/// provider and the test is no longer hermetic. `analytics_test.rs`'s own fixture skips
/// weekends for the opposite reason — to exercise the gap — and that is why this one is
/// shared rather than copied.
#[allow(dead_code)]
pub async fn seed_daily_prices(pool: &DbPool, symbol: &str, start: &str, end: &str, close: &str) {
    let mut day = NaiveDate::parse_from_str(start, "%Y-%m-%d").unwrap();
    let last = NaiveDate::parse_from_str(end, "%Y-%m-%d").unwrap();
    while day <= last {
        seed_price(pool, symbol, day, close).await;
        day = day.succ_opt().expect("a date has a successor");
    }
}

/// Pin split auto-detection for `asset_id` so `sync_splits_for_asset` returns on its
/// throttle instead of fetching.
///
/// Keyed on the **asset**, not the symbol (#38): a symbol-level row let the first
/// asset holding a symbol suppress every other asset holding it for 30 days, while
/// which splits get inserted is decided by the calling asset's own history.
///
/// `covered_from` is set to [`FULLY_COVERED`] rather than left NULL, because NULL now
/// means "synced while the asset had no transactions" — which does not suppress a
/// sync for an asset that has since gained one.
///
/// The bound value must be a *bare* date: `sync_splits_for_asset` parses this column
/// with `NaiveDate::parse_from_str(.., "%Y-%m-%d")`, which rejects trailing input, so
/// `datetime('now')` — a datetime string — fails the parse, falls through the throttle
/// and fetches from the provider. Do not "simplify" the bind back to a SQL datetime
/// function.
#[allow(dead_code)]
pub async fn seed_split_sync(pool: &DbPool, asset_id: i64) {
    sqlx::query(
        "INSERT INTO split_sync (asset_id, last_synced_at, covered_from) VALUES ($1, $2, $3) \
         ON CONFLICT (asset_id) DO UPDATE SET last_synced_at = EXCLUDED.last_synced_at, \
                                              covered_from   = EXCLUDED.covered_from",
    )
    .bind(asset_id)
    .bind(Utc::now().date_naive())
    .bind(FULLY_COVERED)
    .execute(pool)
    .await
    .unwrap();
}

/// Far enough back that no fixture transaction can predate it, so the coverage half
/// of the throttle condition holds and the seeded row actually suppresses a sync.
pub const FULLY_COVERED: NaiveDate = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
