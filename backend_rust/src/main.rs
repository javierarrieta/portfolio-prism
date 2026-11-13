use sqlx::sqlite::SqliteConnectOptions;
use sqlx::SqlitePool;
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

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite:../portfolio.db?mode=rwc".to_string());

    // `busy_timeout` only; the journal mode is left alone. An earlier version of this
    // comment justified that by calling the production volume NFS. That was wrong — it
    // read the StorageClass *name* (`nfs-client`) without checking its provisioner, and
    // the manifests it cited are not what is deployed. Measured against the live
    // cluster: the database is on `local-path-retain`, a node filesystem, RWO, with
    // `replicas: 1` and `Recreate`. WAL would be safe on it.
    //
    // WAL is still not enabled, for a different reason (#61): the PVC is opted into
    // daily restic backups, and the backup Job copies the mounted volume with a plain
    // `restic backup /volume` — no `sqlite3 .backup`, no checkpoint, nothing that
    // understands SQLite. On rollback-journal mode a snapshot taken mid-transaction can
    // restore corrupt, which is at least detectable with `PRAGMA integrity_check`. With
    // WAL, committed data can sit in `-wal` uncheckpointed, so copying `portfolio.db`
    // alone restores an older but internally consistent database — silently stale, and
    // it passes `integrity_check`. Enabling WAL before the backup is SQLite-aware would
    // turn a detectable failure into an invisible one.
    //
    // What is unchanged by all of this: correctness here does not rest on locking or on
    // journal mode at all. It rests on the unique index `enforce_split_uniqueness`
    // installs. `busy_timeout` just decides how long a writer waits for a lock instead
    // of erroring.
    let opts = match database_url.parse::<SqliteConnectOptions>() {
        Ok(o) => o.busy_timeout(std::time::Duration::from_secs(5)),
        Err(e) => refuse(anyhow::anyhow!("invalid DATABASE_URL: {e}")),
    };

    let pool = SqlitePool::connect_with(opts)
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

    let backup_cfg = backend_rust::backup::BackupConfig::from_env();
    if backup_cfg.allow_price_restore {
        eprintln!("WARN: BACKUP_ALLOW_PRICE_RESTORE is on; a restore may write shared historical_prices rows");
    } else if let Some(raw) = std::env::var("BACKUP_ALLOW_PRICE_RESTORE").ok()
        && !backend_rust::backup::BackupConfig::is_on(&raw)
    {
        // An operator who wrote `on` or `enabled` believes price restore is enabled.
        // Saying nothing would leave them believing it while restores quietly refuse.
        eprintln!("WARN: BACKUP_ALLOW_PRICE_RESTORE={raw:?} is not recognised; price restore stays off (use 1, true, or yes)");
    }

    build_rocket(pool, currency_service, cors, auth, backup_cfg)
        .launch()
        .await?;
    Ok(())
}
