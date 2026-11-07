use sqlx::sqlite::SqlitePool;
use backend_rust::services::currency_service::CurrencyService;
use backend_rust::{get_cors_options, parse_allowed_origins};
use backend_rust::auth::{finalise_auth, StartupConfig};
use backend_rust::build_rocket;
use backend_rust::init_db;

/// Print the OpenAPI document and exit, before touching configuration, the
/// database, or the network.
///
/// This is checked **first** on purpose: CI runs it with no `OIDC_*` variables and
/// no secret key, so if the configuration refusal ran before it, generating the
/// document would exit 1 in exactly the environment that needs it most.
fn print_openapi_requested() -> bool {
    std::env::args().any(|arg| arg == "--print-openapi")
}

/// Refuse to start, naming the reason.
///
/// `exit(1)` rather than `return Err`: `main`'s error type is `rocket::Error`,
/// which has no variant for "the operator configured this wrong", and routing a
/// configuration refusal through it buries the message in a launch failure.
fn refuse(err: anyhow::Error) -> ! {
    eprintln!("startup refused: {err}");
    std::process::exit(1);
}

#[rocket::main]
async fn main() -> Result<(), rocket::Error> {
    if print_openapi_requested() {
        println!("{}", backend_rust::openapi::document_json());
        return Ok(());
    }

    let cfg = match StartupConfig::from_env() {
        Ok(cfg) => cfg,
        Err(e) => refuse(e),
    };

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite:../portfolio.db?mode=rwc".to_string());

    let pool = SqlitePool::connect(&database_url)
        .await
        .expect("Failed to connect to database");

    if let Err(e) = init_db(&pool, &cfg).await {
        refuse(e);
    }

    let auth = match finalise_auth(&pool, &cfg).await {
        Ok(auth) => auth,
        Err(e) => refuse(e),
    };

    let currency_service = CurrencyService::new();

    let origins = parse_allowed_origins(std::env::var("ALLOWED_ORIGINS").ok().as_deref());
    eprintln!("ALLOWED_ORIGINS -> CORS origins: {:?}", origins);

    let cors = get_cors_options(origins);

    build_rocket(pool, currency_service, cors, auth).launch().await?;
    Ok(())
}
