use backend_rust::services::currency_service::CurrencyService;
use backend_rust::{get_cors_options, parse_allowed_origins};
use backend_rust::auth::{finalise_auth, StartupConfig};
use backend_rust::build_rocket;
use backend_rust::db;

/// Print the OpenAPI document and exit, before touching configuration, the
/// database, or the network.
///
/// This is checked **first** on purpose: CI runs it with no `OIDC_*` variables and
/// no secret key, so if the configuration refusal ran before it, generating the
/// document would exit 1 in exactly the environment that needs it most.
fn print_openapi_requested() -> bool {
    std::env::args().any(|arg| arg == "--print-openapi")
}

/// Print the backup bundle schema and exit, before touching configuration, the
/// database, or the network — the same contract `--print-openapi` holds, and for
/// the same reason: CI runs it with no `OIDC_*` variables and no secret key.
fn print_backup_schema_requested() -> bool {
    std::env::args().any(|arg| arg == "--print-backup-schema")
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

    if print_backup_schema_requested() {
        println!("{}", backend_rust::backup::schema::document_json());
        return Ok(());
    }

    let cfg = match StartupConfig::from_env() {
        Ok(cfg) => cfg,
        Err(e) => refuse(e),
    };

    // Postgres only: there is no default and no SQLite fallback. A default that
    // silently opened a file database is the opposite of this configuration's purpose,
    // so a missing value names what is missing and stops.
    let Some(database_url) = std::env::var("DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
    else {
        refuse(anyhow::anyhow!(
            "DATABASE_URL is required. Postgres-only: there is no SQLite fallback. \
             Local: nix-shell -p postgresql_18 --run 'bash scripts/dev-db.sh', then \
             export DATABASE_URL=postgres://postgres@127.0.0.1:55432/prism"
        ));
    };

    let pool = match db::connect(&database_url).await {
        Ok(pool) => pool,
        Err(e) => refuse(anyhow::anyhow!("cannot connect to DATABASE_URL: {e}")),
    };

    // Migrations run before the pool serves anything else. This replaces `init_db`,
    // whose `PRAGMA table_info` guards were a hand-rolled migrator, and whose
    // `CREATE TABLE IF NOT EXISTS` could never alter a table it had already made —
    // which is why `migrate_split_sync_key` had to detect shape by column instead.
    if let Err(e) = sqlx::migrate!("./migrations").run(&pool).await {
        refuse(anyhow::anyhow!("migrations failed: {e}"));
    }

    // After the migrator, because both of its statements name tables the migrator
    // creates. See the function's doc comment for what depends on it.
    if let Err(e) = backend_rust::ensure_bootstrap_owner(&pool, &cfg).await {
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

    let backup_cfg = backend_rust::backup::BackupConfig::from_env();
    if backup_cfg.allow_price_restore {
        eprintln!("WARN: BACKUP_ALLOW_PRICE_RESTORE is on; a restore may write shared historical_prices rows");
    } else if let Some(raw) = std::env::var("BACKUP_ALLOW_PRICE_RESTORE").ok()
        && !backend_rust::backup::BackupConfig::is_recognised(&raw)
    {
        // An operator who wrote `on` or `enabled` believes price restore is enabled.
        // Saying nothing would leave them believing it while restores quietly refuse.
        // A value in either accepted set is a decision and stays quiet: warning about
        // `0` would train operators to ignore the warning above this one.
        eprintln!("WARN: BACKUP_ALLOW_PRICE_RESTORE={raw:?} is not recognised; price restore stays off (1/true/yes to enable, 0/false/no to disable)");
    }

    build_rocket(pool, currency_service, cors, auth, backup_cfg)
        .launch()
        .await?;
    Ok(())
}
