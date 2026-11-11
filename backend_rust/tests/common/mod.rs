use backend_rust::auth::{AuthMode, OidcConfig, StartupConfig};
use backend_rust::services::currency_service::CurrencyService;
use backend_rust::{build_rocket as lib_build_rocket, get_cors_options, init_db};
use chrono::{DateTime, NaiveDate, Utc};
use rocket::{Build, Rocket};
use sqlx::SqlitePool;

/// The user every existing test runs as: the placeholder row `init_db` creates
/// unconditionally, which is `id = 1` on a fresh in-memory database.
pub const DEFAULT_USER: i32 = 1;

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
pub fn oidc_config() -> StartupConfig {
    StartupConfig {
        requested: backend_rust::auth::AuthRequest::Oidc,
        oidc: Some(oidc_cfg()),
        bootstrap_subj: None,
    }
}

/// Build the app running as `as_user` (development mode). Tests that need the
/// cookie path construct `AuthMode::Oidc(oidc_cfg())` themselves.
pub fn build_rocket(pool: SqlitePool, as_user: i32) -> Rocket<Build> {
    let currency_service = CurrencyService::new();
    let cors = get_cors_options(vec!["http://localhost:5173".to_string()]);
    #[cfg(debug_assertions)]
    let mode = AuthMode::Disabled { as_user };
    lib_build_rocket(pool, currency_service, cors, mode)
}

/// Build the app in OIDC mode — the mode production runs in, and the only one in
/// which unauthenticated requests are actually denied.
pub fn build_rocket_oidc(pool: SqlitePool) -> Rocket<Build> {
    let currency_service = CurrencyService::new();
    let cors = get_cors_options(vec!["http://localhost:5173".to_string()]);
    lib_build_rocket(pool, currency_service, cors, AuthMode::Oidc(oidc_cfg()))
}

/// Build in OIDC mode with an explicit origin allowlist, for the tests that care
/// about how CORS and authentication are ordered.
#[allow(dead_code)]
pub fn build_rocket_oidc_with_cors(pool: SqlitePool, origins: Vec<String>) -> Rocket<Build> {
    let currency_service = CurrencyService::new();
    let cors = get_cors_options(origins);
    lib_build_rocket(pool, currency_service, cors, AuthMode::Oidc(oidc_cfg()))
}

/// Migrate a fresh in-memory database under `cfg`, exactly as `main` does.
pub async fn setup_db(cfg: &StartupConfig) -> SqlitePool {
    let pool = SqlitePool::connect(":memory:").await.unwrap();
    init_db(&pool, cfg).await.unwrap();
    pool
}

/// A session cookie pair for `LocalRequest::private_cookie`.
pub fn session_cookie(uid: i64, tv: i64) -> (&'static str, String) {
    (
        backend_rust::auth::session::SESSION_COOKIE,
        backend_rust::auth::session::value(uid, tv, backend_rust::auth::session::expiry_now()),
    )
}

/// A session cookie that is already expired, with a matching token version.
pub fn expired_session_cookie(uid: i64, tv: i64) -> (&'static str, String) {
    (
        backend_rust::auth::session::SESSION_COOKIE,
        backend_rust::auth::session::value(uid, tv, 1_000_000),
    )
}

/// Create a user row and return its id.
pub async fn seed_user(pool: &SqlitePool, subj: &str) -> i32 {
    let res = sqlx::query_as::<_, (i64,)>(
        "INSERT INTO users (issuer, subj, email, name, created_at) \
         VALUES ('https://auth.example.invalid', ?, 'user@example.com', ?, datetime('now')) \
         RETURNING id",
    )
    .bind(subj)
    .bind(format!("User {subj}"))
    .fetch_one(pool)
    .await
    .unwrap();
    res.0 as i32
}

/// Create a portfolio owned by `owner_id`.
pub async fn seed_portfolio(pool: &SqlitePool, owner_id: i32, name: &str, currency: &str) -> i32 {
    let res = sqlx::query_as::<_, (i32,)>(
        "INSERT INTO portfolios (name, description, currency, base_currency, owner_id) VALUES (?, ?, ?, ?, ?) RETURNING id"
    )
    .bind(name)
    .bind(None::<String>)
    .bind(currency)
    .bind(currency)
    .bind(owner_id as i64)
    .fetch_one(pool)
    .await
    .unwrap();
    res.0
}

#[allow(dead_code)]
pub async fn seed_asset(pool: &SqlitePool, portfolio_id: i32, symbol: &str, name: &str) -> i32 {
    let res = sqlx::query_as::<_, (i32,)>(
        "INSERT INTO assets (portfolio_id, symbol, name, asset_type, currency) VALUES (?, ?, ?, ?, 'USD') RETURNING id"
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
    pool: &SqlitePool,
    portfolio_id: i32,
    symbol: &str,
    name: &str,
    currency: &str,
) -> i32 {
    let res = sqlx::query_as::<_, (i32,)>(
        "INSERT INTO assets (portfolio_id, symbol, name, asset_type, currency) VALUES (?, ?, ?, ?, ?) RETURNING id"
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

#[allow(dead_code)]
pub async fn seed_transaction(
    pool: &SqlitePool,
    asset_id: i32,
    tx_type: &str,
    qty: f64,
    price: f64,
    fee: f64,
) -> i32 {
    let date = DateTime::parse_from_rfc3339("2024-06-15T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let res = sqlx::query_as::<_, (i32,)>(
        "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) VALUES (?, ?, ?, ?, ?, ?) RETURNING id"
    )
    .bind(asset_id)
    .bind(tx_type)
    .bind(qty)
    .bind(price)
    .bind(fee)
    .bind(date)
    .fetch_one(pool)
    .await
    .unwrap();
    res.0
}
#[allow(dead_code)]
pub async fn seed_transaction_at(
    pool: &SqlitePool,
    asset_id: i32,
    tx_type: &str,
    qty: f64,
    price: f64,
    fee: f64,
    date: &str,
) -> i32 {
    let date = DateTime::parse_from_rfc3339(date)
        .unwrap()
        .with_timezone(&Utc);
    let res = sqlx::query_as::<_, (i32,)>(
        "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) VALUES (?, ?, ?, ?, ?, ?) RETURNING id"
    )
    .bind(asset_id)
    .bind(tx_type)
    .bind(qty)
    .bind(price)
    .bind(fee)
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
pub async fn seed_price(pool: &SqlitePool, symbol: &str, date: NaiveDate, close: &str) {
    sqlx::query(
        "INSERT INTO historical_prices (symbol, date, close_price) VALUES (?, ?, ?)
             ON CONFLICT(symbol, date) DO UPDATE SET close_price = excluded.close_price",
    )
    .bind(symbol)
    .bind(date)
    .bind(close)
    .execute(pool)
    .await
    .unwrap();
}

/// Pin split auto-detection for `symbol` so `sync_splits_for_asset` returns on its
/// 30-day cache check instead of fetching. Same row `analytics_test.rs:218` writes.
///
/// The bound value must be a *bare* date: `sync_splits_for_asset`
/// (`currency_service.rs:870`) parses this column with `NaiveDate::parse_from_str(..,
/// "%Y-%m-%d")`, which rejects trailing input, so `datetime('now')` — a datetime string
/// — fails the parse, falls through the cache check and fetches from Yahoo. Do not
/// "simplify" the bind back to a SQL datetime function.
#[allow(dead_code)]
pub async fn seed_split_sync(pool: &SqlitePool, symbol: &str) {
    sqlx::query("INSERT OR REPLACE INTO split_sync (symbol, last_synced_at) VALUES (?, ?)")
        .bind(symbol)
        .bind(Utc::now().date_naive())
        .execute(pool)
        .await
        .unwrap();
}
