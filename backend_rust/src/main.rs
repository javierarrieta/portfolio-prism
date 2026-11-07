#[macro_use] extern crate rocket;

use sqlx::sqlite::SqlitePool;
use backend_rust::services::currency_service::CurrencyService;
use backend_rust::{get_cors_options, parse_allowed_origins};
use backend_rust::build_rocket;
use backend_rust::init_db;

#[launch]
async fn rocket() -> _ {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite:../portfolio.db?mode=rwc".to_string());
    
    let pool = SqlitePool::connect(&database_url)
        .await
        .expect("Failed to connect to database");
    
    let currency_service = CurrencyService::new();

    let origins = parse_allowed_origins(std::env::var("ALLOWED_ORIGINS").ok().as_deref());
    eprintln!("ALLOWED_ORIGINS -> CORS origins: {:?}", origins);

    let cors = get_cors_options(origins);

    init_db(&pool).await.expect("Failed to initialize database");

    build_rocket(pool, currency_service, cors)
}
