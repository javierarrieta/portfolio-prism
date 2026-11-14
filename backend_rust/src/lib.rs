#[macro_use] extern crate rocket;

pub mod db_types;
pub mod models;
pub mod schemas;
pub mod services;
pub mod engines;
pub mod openapi;
pub mod access;
pub mod report_csv;
pub mod auth;
pub mod backup;
pub mod api_routes {
    pub mod portfolios;
    pub mod transactions;
    pub mod analytics;
    pub mod lookup;
    pub mod backup;
}

/// Test-only stand-in for the pricing provider, compiled into the library's own
/// test build. The same file is included by `tests/common/mod.rs` for integration
/// tests; it lives in `src/` so both can reach it and in no build that ships.
#[cfg(test)]
#[path = "test_provider.rs"]
pub mod test_provider;

/// A `CurrencyService` pointed at the local provider stand-in instead of the
/// internet, for engine unit tests that must not depend on a network. A 404 rather
/// than a closed port on purpose: a refused connection is retried three times with
/// backoff, which would add seconds per call to the unit suite.
#[cfg(test)]
pub(crate) fn offline_pricing_service() -> services::currency_service::CurrencyService {
    services::currency_service::CurrencyService::with_base_url(test_provider::mock_pricing_url())
}

use rocket::{Rocket, Build};
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnection;
use crate::services::currency_service::CurrencyService;
use crate::auth::{AuthMode, StartupConfig};
use crate::auth::fairing::AuthFairing;
use rocket_cors::{CorsOptions, AllowedOrigins, AllowedHeaders, Cors};

#[get("/")]
pub fn index() -> &'static str {
    "Welcome to the Portfolio Prism API (Rust)"
}

#[get("/openapi.json")]
pub fn openapi_json() -> (rocket::http::ContentType, String) {
    // Same serialiser as `--print-openapi`, so the served document and the
    // committed file are byte-identical.
    (rocket::http::ContentType::JSON, crate::openapi::document_json())
}

/// Origins allowed when `ALLOWED_ORIGINS` is unset or holds no usable entry.
pub const DEFAULT_ALLOWED_ORIGINS: &[&str] =
    &["http://localhost:5173", "http://127.0.0.1:5173"];

/// Parse the `ALLOWED_ORIGINS` env var into exact CORS origins.
///
/// Accepts a comma-separated list (`https://a.com,https://b.com`) and also
/// tolerates the JSON-array spelling (`["https://a.com", "https://b.com"]`)
/// that JSON-shaped config sources emit.
///
/// Empty entries are dropped on purpose: `rocket_cors` parses every exact
/// origin as a URL, so an empty string makes `CorsOptions::to_cors()` fail and
/// `get_cors_options` aborts startup. When nothing usable survives, the defaults
/// are returned instead of an empty allowlist, which would reject every browser
/// request with 403 while the server looks healthy.
///
/// Entries that are non-empty but not a valid origin are left alone so
/// `get_cors_options` still fails loudly on a typo instead of silently
/// under-allowing.
pub fn parse_allowed_origins(raw: Option<&str>) -> Vec<String> {
    let origins: Vec<String> = raw
        .unwrap_or_default()
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|s| s.trim().trim_matches('"').trim_matches('\'').trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    if origins.is_empty() {
        DEFAULT_ALLOWED_ORIGINS.iter().map(|s| s.to_string()).collect()
    } else {
        origins
    }
}

pub fn get_cors_options(origins: Vec<String>) -> Cors {
    let origins_static: Vec<&'static str> = origins.iter()
        .map(|s| Box::leak(s.clone().into_boxed_str()) as &'static str)
        .collect();

    CorsOptions {
        allowed_origins: AllowedOrigins::some_exact(&origins_static),
        allowed_methods: vec![
            rocket::http::Method::Get,
            rocket::http::Method::Post,
            rocket::http::Method::Put,
            rocket::http::Method::Patch,
            rocket::http::Method::Delete,
            rocket::http::Method::Options,
        ]
            .into_iter()
            .map(|m| m.into())
            .collect(),
        allowed_headers: AllowedHeaders::some(&["Content-Type", "Authorization"]),
        allow_credentials: true,
        ..Default::default()
    }
    .to_cors()
    .unwrap_or_else(|e| panic!(
        "Failed to configure CORS for origins {origins:?}: {e}. \
         Each entry must be a full origin such as https://example.com"
    ))
}

/// Placeholder identity: the row that owns anything with no owner yet.
///
/// `subj` is a marker, not a subject any provider issues. It is handed to a real
/// identity by [`crate::auth::oidc::provision_or_update`] when the configured
/// bootstrap subject signs in, and that handover can happen only once, because
/// the handover updates this row in place and so removes the condition for it.
pub const PLACEHOLDER_ISSUER: &str = "local";
pub const PLACEHOLDER_SUBJ: &str = "bootstrap";

/// Create/upgrade the schema and reconcile ownership.
///
/// Returns `Err` rather than exiting, so "startup refuses" is testable; `main`
/// turns an error into `exit(1)`.
pub async fn init_db(pool: &SqlitePool, cfg: &StartupConfig) -> anyhow::Result<()> {
    sqlx::query("CREATE TABLE IF NOT EXISTS portfolios (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        name TEXT NOT NULL,
        description TEXT,
        currency TEXT NOT NULL DEFAULT 'USD',
        base_currency TEXT NOT NULL DEFAULT 'USD'
    )").execute(pool).await?;

    sqlx::query("CREATE TABLE IF NOT EXISTS assets (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        portfolio_id INTEGER NOT NULL,
        symbol TEXT NOT NULL,
        name TEXT NOT NULL,
        asset_type TEXT NOT NULL,
        sector TEXT,
        currency TEXT NOT NULL DEFAULT 'USD',
        isin TEXT,
        FOREIGN KEY (portfolio_id) REFERENCES portfolios(id)
    )").execute(pool).await?;

    let column_exists: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM pragma_table_info('assets') WHERE name = 'isin'"
    )
    .fetch_one(pool)
    .await
    .unwrap_or((0,));

    if column_exists.0 == 0 {
        if let Err(e) = sqlx::query("ALTER TABLE assets ADD COLUMN isin TEXT")
            .execute(pool)
            .await
        {
            eprintln!("WARN: Failed to add isin column to assets table: {}", e);
        }
    }

    let _ = sqlx::query("DROP INDEX IF EXISTS idx_assets_isin")
        .execute(pool)
        .await;

    sqlx::query("CREATE TABLE IF NOT EXISTS transactions (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        asset_id INTEGER NOT NULL,
        type TEXT NOT NULL,
        quantity TEXT NOT NULL,
        price TEXT NOT NULL,
        fee TEXT NOT NULL,
        date TEXT NOT NULL,
        FOREIGN KEY (asset_id) REFERENCES assets(id)
    )").execute(pool).await?;

    sqlx::query("CREATE TABLE IF NOT EXISTS historical_prices (
        symbol TEXT NOT NULL,
        date DATE NOT NULL,
        close_price TEXT NOT NULL
    )").execute(pool).await?;

    migrate_split_sync_key(pool).await?;

    migrate_decimal_columns(pool).await?;

    // After the decimal migration: it renames close_price on this same table, and a
    // dedupe that runs before it operates on a schema about to change.
    enforce_price_uniqueness(pool).await?;

    enforce_split_uniqueness(pool).await?;

    // --- identity -------------------------------------------------------------
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            issuer TEXT NOT NULL,
            subj TEXT NOT NULL,
            email TEXT,
            name TEXT,
            is_active INTEGER NOT NULL DEFAULT 1,
            token_version INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            last_login_at TEXT
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS uq_users_issuer_subj ON users(issuer, subj)")
        .execute(pool)
        .await?;

    add_owner_id_column(pool).await?;

    // The placeholder row is created **unconditionally**, not only when orphans
    // exist: `finalise_auth` must be able to name a real row in `disabled` mode on
    // a fresh database, or `./start.sh` dies on the empty database it just created.
    sqlx::query(
        "INSERT INTO users (issuer, subj, email, name, created_at) \
         VALUES (?1, ?2, NULL, 'Placeholder owner', ?3) \
         ON CONFLICT (issuer, subj) DO NOTHING",
    )
    .bind(PLACEHOLDER_ISSUER)
    .bind(PLACEHOLDER_SUBJ)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await?;

    backfill_ownership(pool, cfg).await?;

    Ok(())
}

/// Add `portfolios.owner_id`.
///
/// Runs on a dedicated connection with `legacy_alter_table = ON`, the same
/// mitigation `migrate_decimal_columns` uses: the bundled SQLite's stale
/// per-connection schema makes an `ALTER` that follows a `DROP COLUMN` + `RENAME`
/// fail on file-backed databases, and a fresh `ALTER` on a pooled connection is
/// the same class of hazard. `migrate_one_column` is deliberately untouched.
async fn add_owner_id_column(pool: &SqlitePool) -> anyhow::Result<()> {
    let mut conn = pool.acquire().await?;
    sqlx::query("PRAGMA legacy_alter_table = ON").execute(&mut *conn).await?;
    let result = async {
        if !column_exists(&mut conn, "portfolios", "owner_id").await? {
            sqlx::query("ALTER TABLE portfolios ADD COLUMN owner_id INTEGER")
                .execute(&mut *conn)
                .await?;
        }
        Ok::<(), sqlx::Error>(())
    }
    .await;
    sqlx::query("PRAGMA legacy_alter_table = OFF").execute(&mut *conn).await?;
    Ok(result?)
}

/// Assign portfolios that have no owner, or refuse to start.
///
/// The rule is the spec's: an unowned portfolio is either handed to the
/// placeholder (development, or OIDC with a designated bootstrap subject) or it
/// stops the process. Silently leaving it unowned would make it invisible to every
/// user while still consuming the one ISIN lookup quota per symbol.
async fn backfill_ownership(pool: &SqlitePool, cfg: &StartupConfig) -> anyhow::Result<()> {
    let (orphans,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM portfolios WHERE owner_id IS NULL")
            .fetch_one(pool)
            .await?;

    if orphans == 0 {
        return Ok(());
    }

    let placeholder: (i64,) = sqlx::query_as(
        "SELECT id FROM users WHERE issuer = ?1 AND subj = ?2",
    )
    .bind(PLACEHOLDER_ISSUER)
    .bind(PLACEHOLDER_SUBJ)
    .fetch_one(pool)
    .await?;

    // A blank `BOOTSTRAP_OWNER_SUBJ` counts as unset here too, exactly as it does in
    // the handover rule. If it did not, `BOOTSTRAP_OWNER_SUBJ=" "` would silently
    // satisfy the refusal condition and hand pre-existing data to the placeholder.
    let bootstrap = cfg.bootstrap_subj.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let assign = match cfg.requested {
        crate::auth::AuthRequest::Disabled => true,
        crate::auth::AuthRequest::Oidc => bootstrap.is_some(),
    };

    if !assign {
        anyhow::bail!(
            "{orphans} portfolio(s) have no owner and no bootstrap owner is configured. \
             Set BOOTSTRAP_OWNER_SUBJ to the Authentik subject that should take ownership \
             of pre-existing data, then restart."
        );
    }

    sqlx::query("UPDATE portfolios SET owner_id = ? WHERE owner_id IS NULL")
        .bind(placeholder.0)
        .execute(pool)
        .await?;
    eprintln!(
        "WARN: assigned {orphans} pre-existing portfolio(s) to the placeholder owner \
         (user {}); they transfer on bootstrap handover", placeholder.0
    );
    Ok(())
}

async fn column_type(
    conn: &mut SqliteConnection,
    table: &str,
    column: &str,
) -> Result<Option<String>, sqlx::Error> {
    let rows: Vec<(i64, String, String, i64, Option<String>, i64)> =
        sqlx::query_as(&format!("PRAGMA table_info({table})")).fetch_all(&mut *conn).await?;
    Ok(rows.iter().find(|r| r.1 == column).map(|r| r.2.clone()))
}

async fn column_exists(
    conn: &mut SqliteConnection,
    table: &str,
    column: &str,
) -> Result<bool, sqlx::Error> {
    Ok(column_type(conn, table, column).await?.is_some())
}

async fn migrate_one_column(
    conn: &mut SqliteConnection,
    table: &str,
    column: &str,
) -> Result<(), sqlx::Error> {
    let old = format!("{column}_old");
    let current = column_type(conn, table, column).await?;
    let old_exists = column_exists(conn, table, &old).await?;

    match (current.as_deref(), old_exists) {
        (Some("TEXT"), true) => {
            // add happened, drop-leftover crash happened after ADD: re-copy from
            // `_old` (idempotent; harmless if the UPDATE already ran, and it
            // recovers the real values if the crash landed before the UPDATE),
            // then finish the drop.
            sqlx::query(&format!("UPDATE {table} SET {column} = CAST({old} AS TEXT)"))
                .execute(&mut *conn)
                .await?;
            sqlx::query(&format!("ALTER TABLE {table} DROP COLUMN {old}"))
                .execute(&mut *conn).await?;
            Ok(())
        }
        (Some("TEXT"), false) => Ok(()),
        (None, true) => {
            // RENAME happened, ADD never did: recreate and repopulate from _old.
            sqlx::query(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} TEXT NOT NULL DEFAULT '0'"
            ))
            .execute(&mut *conn)
            .await?;
            sqlx::query(&format!("UPDATE {table} SET {column} = CAST({old} AS TEXT)"))
                .execute(&mut *conn)
                .await?;
            sqlx::query(&format!("ALTER TABLE {table} DROP COLUMN {old}"))
                .execute(&mut *conn).await?;
            Ok(())
        }
        (None, false) => Ok(()),
        (Some(_), true) => {
            // non-TEXT column plus a stray _old leftover: drop the leftover, then migrate.
            sqlx::query(&format!("ALTER TABLE {table} DROP COLUMN {old}"))
                .execute(&mut *conn).await?;
            sqlx::query(&format!("ALTER TABLE {table} RENAME COLUMN {column} TO {old}"))
                .execute(&mut *conn)
                .await?;
            sqlx::query(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} TEXT NOT NULL DEFAULT '0'"
            ))
            .execute(&mut *conn)
            .await?;
            sqlx::query(&format!("UPDATE {table} SET {column} = CAST({old} AS TEXT)"))
                .execute(&mut *conn)
                .await?;
            sqlx::query(&format!("ALTER TABLE {table} DROP COLUMN {old}"))
                .execute(&mut *conn).await?;
            Ok(())
        }
        (Some(_), false) => {
            // standard migration of a non-TEXT column.
            sqlx::query(&format!("ALTER TABLE {table} RENAME COLUMN {column} TO {old}"))
                .execute(&mut *conn)
                .await?;
            sqlx::query(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} TEXT NOT NULL DEFAULT '0'"
            ))
            .execute(&mut *conn)
            .await?;
            sqlx::query(&format!("UPDATE {table} SET {column} = CAST({old} AS TEXT)"))
                .execute(&mut *conn)
                .await?;
            sqlx::query(&format!("ALTER TABLE {table} DROP COLUMN {old}"))
                .execute(&mut *conn).await?;
            Ok(())
        }
    }
}

/// Collapse duplicate `historical_prices` rows and make `(symbol, date)` unique.
///
/// The table was created with no primary key and only a non-unique index (the
/// `CREATE TABLE IF NOT EXISTS historical_prices` in `init_db`), so `INSERT OR
/// REPLACE` always inserted and every fetch
/// appended. Reads have no `ORDER BY`, which made "the close on date X" an
/// arbitrary row among N.
///
/// The tie-break is `MAX(rowid)`: the table has no write timestamp, so the most
/// recently written row is the only deterministic survivor available. That is a
/// deliberate choice about Yahoo's revisions around corporate actions, and it is
/// what makes an as-of price reproducible from here on.
///
/// Must run AFTER `migrate_decimal_columns`, which renames this table's
/// `close_price` column on a dedicated connection.
pub async fn enforce_price_uniqueness(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        "DELETE FROM historical_prices WHERE rowid NOT IN (
             SELECT MAX(rowid) FROM historical_prices GROUP BY symbol, date
         )",
    )
    .execute(pool)
    .await?;

    // Superseded by the unique index below; leaving it costs a write for no read.
    sqlx::query("DROP INDEX IF EXISTS idx_historical_prices_symbol_date")
        .execute(pool)
        .await?;

    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS uq_historical_prices_symbol_date
             ON historical_prices(symbol, date)",
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Make a duplicated split row impossible rather than unlikely (#50).
///
/// `apply_split_events` used a `SELECT COUNT(*)` and then an `INSERT`, with no
/// transaction and no constraint behind it. Two concurrent callers could both see
/// zero and both insert, doubling a split adjustment and silently corrupting cost
/// basis for that asset. A check-then-write cannot be made safe by tightening the
/// check; it is safe when a second writer physically cannot land. So the guard is a
/// partial unique index, and the count check becomes an optimisation.
///
/// The index is on `date(date)` rather than `date` because `transactions.date` is
/// stored as a full timestamp (`2024-06-01T00:00:00Z`) while the dedupe has always
/// compared calendar days. The expression has to match the one the queries use or
/// SQLite will not use the index.
///
/// Existing duplicates are removed first, keeping the lowest `id`, because a unique
/// index cannot be created over data that already violates it. That runs
/// unconditionally rather than assuming any given database is clean.
pub async fn enforce_split_uniqueness(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM transactions
         WHERE type = 'SPLIT'
           AND id NOT IN (
               SELECT MIN(id) FROM transactions
               WHERE type = 'SPLIT'
               GROUP BY asset_id, date(date)
           )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS uq_split_asset_date
             ON transactions(asset_id, date(date)) WHERE type = 'SPLIT'",
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Re-key the split-sync throttle from symbol to asset (#38).
///
/// Keyed on the symbol, the throttle let the first asset holding a symbol suppress
/// every other asset holding it for 30 days — while the set of splits actually
/// inserted is decided by *the calling asset's* earliest transaction date. So the
/// first asset to sync could succeed, insert nothing relevant, and record the
/// throttle anyway, leaving a second portfolio priced pre-split on a tax screen.
///
/// The table is a throttle cache: every row is re-derivable by fetching again, and a
/// row written under the old rule means something the new rule cannot use — a
/// symbol-level timestamp says nothing about whose history was evaluated. So this
/// drops and recreates rather than transforming. The whole cost of losing the rows is
/// one extra provider call per asset, once. Rollback is the same shape swap in the
/// other direction and equally lossless.
///
/// Detection is by column rather than a version number, because `CREATE TABLE IF NOT
/// EXISTS` will not alter an existing table: an old-shaped table left in place would
/// keep the bug while the code above it looked fixed. `PRAGMA table_info` returns no
/// rows when the table does not exist, which is the fresh-database case and falls
/// straight through to the create.
pub async fn migrate_split_sync_key(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    use sqlx::Row;

    let columns = sqlx::query("PRAGMA table_info(split_sync)")
        .fetch_all(pool)
        .await?;
    if columns
        .iter()
        .any(|row| matches!(row.try_get::<String, _>("name").as_deref(), Ok("symbol")))
    {
        sqlx::query("DROP TABLE split_sync").execute(pool).await?;
    }

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS split_sync (
            asset_id INTEGER PRIMARY KEY,
            last_synced_at DATE NOT NULL,
            covered_from DATE
        )",
    )
    .execute(pool)
    .await
    .map(|_| ())
}

pub async fn migrate_decimal_columns(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    // Run all ALTER statements on a single dedicated connection with
    // `legacy_alter_table` enabled. Bundled SQLite 3.46 has a stale per-connection
    // schema bug that makes `DROP COLUMN` followed by `RENAME COLUMN` + `ADD COLUMN`
    // fail on file-backed databases (the brief's original pool-based approach only
    // works on `:memory:` connections); legacy mode avoids the buggy code path.
    // The setting is reset before the connection is returned to the pool so it
    // cannot leak modern `ALTER` semantics into unrelated pooled connections.
    let mut conn = pool.acquire().await?;
    sqlx::query("PRAGMA legacy_alter_table = ON").execute(&mut *conn).await?;
    let result = (async {
        for column in ["quantity", "price", "fee"] {
            migrate_one_column(&mut conn, "transactions", column).await?;
        }
        migrate_one_column(&mut conn, "historical_prices", "close_price").await
    })
    .await;
    sqlx::query("PRAGMA legacy_alter_table = OFF").execute(&mut *conn).await?;
    result
}

pub fn build_rocket(
    pool: SqlitePool,
    currency_service: CurrencyService,
    cors: Cors,
    auth: AuthMode,
    // Threaded through rather than managed here: Rocket 0.5.1's `manage` panics on a
    // type it already holds, so a default managed inside this function would both
    // crash `main`'s `.manage(...)` and win over the real value.
    backup_cfg: crate::backup::BackupConfig,
) -> Rocket<Build> {
    // The OIDC-only routes are mounted only when there is a provider to talk to;
    // `/auth/me` exists in both modes so the frontend can always ask who it is.
    let oidc_routes = matches!(auth, AuthMode::Oidc(_));

    // No `Limits` here on purpose. The bundle upload is capped per route — a
    // handler opens its own `Data` stream under `backup::MAX_BUNDLE_BYTES` — so no
    // global body limit is raised to serve one endpoint. See
    // `api_routes::backup`'s module comment and the probes in `tests/backup_test.rs`.
    let rocket = rocket::build()
        .attach(cors)
        .attach(AuthFairing)
        .manage(pool)
        .manage(currency_service)
        .manage(auth)
        .manage(backup_cfg)
        .mount("/", routes![index])
        // The sentinel the fairing rewrites denied requests to. Outside `/api`, so
        // it is neither covered by the prefix rule nor part of the API surface, and
        // mounted for every method so no method can answer 405 where 401 is required.
        .mount("/_auth", routes![
            auth::routes::denied_get,
            auth::routes::denied_post,
            auth::routes::denied_put,
            auth::routes::denied_delete,
            auth::routes::denied_patch,
            auth::routes::denied_head,
            auth::routes::denied_options,
        ])
        // The outage sentinel: same seven methods, so no method can answer 405
        // where 503 is the truth.
        .mount("/_auth", routes![
            auth::routes::unavailable_get,
            auth::routes::unavailable_post,
            auth::routes::unavailable_put,
            auth::routes::unavailable_delete,
            auth::routes::unavailable_patch,
            auth::routes::unavailable_head,
            auth::routes::unavailable_options,
        ])
        .mount("/auth", routes![auth::routes::me])
        .mount("/api/portfolios", routes![
            api_routes::portfolios::create_portfolio,
            api_routes::portfolios::list_portfolios,
            api_routes::portfolios::get_portfolio,
            api_routes::portfolios::delete_portfolio,
            api_routes::portfolios::update_portfolio,
            api_routes::analytics::get_portfolio_tax_summary,
            // Mounted with the portfolio routes because its path is portfolio-scoped.
            // It is the only write there, and the only split-sync entry point (#50).
            api_routes::analytics::sync_portfolio_splits,
            api_routes::analytics::get_portfolio_performance,
        ])
        .mount("/api", routes![
            api_routes::lookup::lookup_isin,
            api_routes::transactions::create_asset,
            api_routes::transactions::update_asset,
            api_routes::transactions::delete_asset,
            api_routes::transactions::create_transaction,
            api_routes::transactions::list_portfolio_transactions,
            api_routes::transactions::delete_transaction,
            api_routes::analytics::get_transaction_sell_pnl,
            api_routes::backup::capabilities,
            api_routes::backup::export,
            api_routes::backup::preview,
            api_routes::backup::restore,
        ])
        // `openapi_json` is mounted here only. The `#[get("/openapi.json")]` attribute
        // stays on the handler because the attribute *is* the route path and the same
        // handler serves both mounts; the duplicate `/openapi.json` mount is gone.
        .mount("/api-docs", routes![openapi_json])
        // Every `401` answers with the same body, whichever path produced it. Rocket
        // prefers a registered catcher over a guard's error responder, so without
        // this a guard rejection would render Rocket's HTML error page.
        .register("/", auth::routes::catchers());

    if oidc_routes {
        rocket.mount("/auth", routes![
            auth::routes::login,
            auth::routes::callback,
            auth::routes::logout,
            auth::routes::logout_all,
        ])
    } else {
        rocket
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic;
    use std::sync::Mutex;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// Run `f` with the default panic hook silenced, so the expected panics in
    /// this module do not bury the test summary in stack traces.
    fn assert_panics<F: FnOnce() + panic::UnwindSafe>(f: F) {
        static LOCK: Mutex<()> = Mutex::new(());
        // Serialised because the panic hook is process-wide.
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let hook = panic::take_hook();
        panic::set_hook(Box::new(|_| {}));
        let result = panic::catch_unwind(f);
        panic::set_hook(hook);
        assert!(result.is_err(), "expected a panic, got a clean return");
    }

    #[test]
    fn splits_and_trims_a_comma_separated_list() {
        assert_eq!(
            parse_allowed_origins(Some(" http://localhost:5173 , https://app.example.com ")),
            strings(&["http://localhost:5173", "https://app.example.com"])
        );
    }

    #[test]
    fn trailing_comma_produces_no_empty_origin() {
        // Regression: the empty entry used to reach rocket_cors, whose
        // Url::parse("") fails, so get_cors_options aborted startup.
        assert_eq!(
            parse_allowed_origins(Some("https://app.example.com,")),
            strings(&["https://app.example.com"])
        );
        assert_eq!(
            parse_allowed_origins(Some("https://a.com,,https://b.com")),
            strings(&["https://a.com", "https://b.com"])
        );
    }

    #[test]
    fn json_array_spelling_is_accepted() {
        assert_eq!(
            parse_allowed_origins(Some(r#"["https://a.com", "https://b.com"]"#)),
            strings(&["https://a.com", "https://b.com"])
        );
        assert_eq!(
            parse_allowed_origins(Some("['https://a.com']")),
            strings(&["https://a.com"])
        );
    }

    #[test]
    fn unusable_values_fall_back_to_the_defaults() {
        for raw in [None, Some(""), Some("   "), Some(","), Some(" , , "), Some("[]"), Some("[ ]")] {
            assert_eq!(
                parse_allowed_origins(raw),
                strings(DEFAULT_ALLOWED_ORIGINS),
                "raw={raw:?}"
            );
        }
    }

    #[test]
    fn defaults_are_accepted_by_cors() {
        let _cors = get_cors_options(parse_allowed_origins(None));
    }

    #[test]
    fn an_empty_origin_really_does_abort_startup() {
        // Pins down why the filter matters: rocket_cors parses every exact
        // origin as a URL, "" is not one, so to_cors() errors and the .expect
        // in get_cors_options panics.
        assert_panics(|| { get_cors_options(strings(&["https://a.com", ""])); });
    }

    #[test]
    fn a_malformed_origin_still_aborts_startup() {
        // A typo must fail loudly rather than silently under-allow origins.
        assert_panics(|| { get_cors_options(strings(&["not a url"])); });
    }
}
