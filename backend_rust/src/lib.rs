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

pub fn get_cors_options(origins: Vec<String>) -> Cors {
    let origins_static: Vec<&'static str> = origins.into_iter()
        .map(|s| Box::leak(s.into_boxed_str()) as &'static str)
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
    }.to_cors().expect("Failed to configure CORS")
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
