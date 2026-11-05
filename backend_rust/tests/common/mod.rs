use chrono::{DateTime, Utc};
use rocket::{Rocket, Build};
use sqlx::SqlitePool;
use backend_rust::{build_rocket as lib_build_rocket, init_db, get_cors_options};
use backend_rust::services::currency_service::CurrencyService;

pub fn build_rocket(pool: SqlitePool) -> Rocket<Build> {
    let currency_service = CurrencyService::new();
    let cors = get_cors_options(vec!["http://localhost:5173".to_string()]);
    lib_build_rocket(pool, currency_service, cors)
}

pub async fn setup_db() -> SqlitePool {
    let pool = SqlitePool::connect(":memory:").await.unwrap();
    
    init_db(&pool).await.unwrap();
    
    pool
}

pub async fn seed_portfolio(pool: &SqlitePool, name: &str, currency: &str) -> i32 {
    let res = sqlx::query_as::<_, (i32,)>(
        "INSERT INTO portfolios (name, description, currency, base_currency) VALUES (?, ?, ?, ?) RETURNING id"
    )
    .bind(name)
    .bind(None::<String>)
    .bind(currency)
    .bind(currency)
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

#[allow(dead_code)]
pub async fn seed_transaction(
    pool: &SqlitePool,
    asset_id: i32,
    tx_type: &str,
    qty: f64,
    price: f64,
    fee: f64,
) -> i32 {
    let date = DateTime::parse_from_rfc3339("2024-06-15T00:00:00Z").unwrap().with_timezone(&Utc);
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
    let date = DateTime::parse_from_rfc3339(date).unwrap().with_timezone(&Utc);
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
