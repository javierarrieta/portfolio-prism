#[macro_use] extern crate rocket;

pub mod db_types;
pub mod models;
pub mod schemas;
pub mod services;
pub mod engines;
pub mod openapi;
pub mod api_routes {
    pub mod portfolios;
    pub mod transactions;
    pub mod analytics;
    pub mod lookup;
}

use rocket::{Rocket, Build};
use rocket::serde::json::Json;
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnection;
use crate::services::currency_service::CurrencyService;
use crate::openapi::ApiDoc;
use utoipa::OpenApi;
use rocket_cors::{CorsOptions, AllowedOrigins, AllowedHeaders, Cors};

#[get("/")]
pub fn index() -> &'static str {
    "Welcome to the Portfolio Prism API (Rust)"
}

#[get("/openapi.json")]
pub fn openapi_json() -> Json<serde_json::Value> {
    Json(serde_json::to_value(ApiDoc::openapi()).unwrap())
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

pub async fn init_db(pool: &SqlitePool) -> Result<(), sqlx::Error> {
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

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_historical_prices_symbol_date ON historical_prices(symbol, date)")
        .execute(pool)
        .await?;

    sqlx::query("CREATE TABLE IF NOT EXISTS split_sync (
        symbol TEXT PRIMARY KEY,
        last_synced_at DATE NOT NULL
    )").execute(pool).await?;

    migrate_decimal_columns(pool).await?;

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

pub fn build_rocket(pool: SqlitePool, currency_service: CurrencyService, cors: Cors) -> Rocket<Build> {
    rocket::build()
        .attach(cors)
        .manage(pool)
        .manage(currency_service)
        .mount("/", routes![index, openapi_json])
        .mount("/api/portfolios", routes![
            api_routes::portfolios::create_portfolio,
            api_routes::portfolios::list_portfolios,
            api_routes::portfolios::get_portfolio,
            api_routes::portfolios::delete_portfolio,
            api_routes::portfolios::update_portfolio,
            api_routes::analytics::get_portfolio_tax_summary,
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
        ])
        .mount("/api-docs", routes![openapi_json])
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
