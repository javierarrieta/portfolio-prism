#[macro_use] extern crate rocket;

// `src/test_db.rs` is compiled into the integration-test binaries as well as this
// crate, and it needs to name `db::pool_options` from both. Inside this crate
// `backend_rust::` would not resolve without this line; with it, one path works in
// every build that includes the file, which is the only reason it is here.
extern crate self as backend_rust;

pub mod db;
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

/// The same arrangement as [`test_provider`], for the database the tests run on:
/// one private, freshly migrated Postgres database per test, because Postgres has
/// no `:memory:` and the tests inside one binary run on parallel threads.
#[cfg(test)]
#[path = "test_db.rs"]
pub mod test_db;

/// The harness's own tests. A sibling file rather than a `mod tests` inside
/// `test_db.rs`, because that file is included by `tests/common/mod.rs` and would
/// otherwise re-run every one of these tests in all nineteen integration binaries.
#[cfg(test)]
#[path = "test_db_tests.rs"]
mod test_db_tests;

/// A `CurrencyService` pointed at the local provider stand-in instead of the
/// internet, for engine unit tests that must not depend on a network. A 404 rather
/// than a closed port on purpose: a refused connection is retried three times with
/// backoff, which would add seconds per call to the unit suite.
#[cfg(test)]
pub(crate) fn offline_pricing_service() -> services::currency_service::CurrencyService {
    services::currency_service::CurrencyService::with_base_url(test_provider::mock_pricing_url())
}

use rocket::{Rocket, Build};
use crate::db::DbPool;
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

/// Establish the bootstrap owner, and assign any portfolio that has no owner.
///
/// This is the identity half of what `init_db` used to do. The schema half is gone
/// — `sqlx::migrate!` owns the schema now — but these two duties are not schema,
/// they are startup policy, and three things depend on them:
///
/// 1. [`crate::auth::finalise_auth`] in `AUTH_MODE=disabled` reads the placeholder
///    row's id to become the caller. Without the row, `AUTH_MODE=disabled` — which
///    spec §10 of the Postgres design requires, and which `./start.sh` uses — has
///    nobody to be.
/// 2. `tests/common::db_with_placeholder` runs this for the tests whose subject *is*
///    the startup identity, and `tests/common::placeholder_id` reads the row's id back
///    rather than assuming it. Every test that runs as the placeholder is running as
///    this row.
/// 3. `tests/startup_test.rs` pins the orphan rule below: unowned portfolios plus no
///    configured bootstrap subject is a refusal, not a warning.
///
/// So the two duties are:
///
/// * **Placeholder upsert.** One row (`issuer='local'`, `subj='bootstrap'`) that
///   owns anything with no owner yet, created unconditionally — a fresh database
///   must be startable before any identity exists. It is handed to a real identity
///   by [`crate::auth::oidc::provision_or_update`] when the configured bootstrap
///   subject signs in.
/// * **Ownership backfill.** Portfolios with `owner_id IS NULL` are assigned to that
///   row, or startup refuses. Silently leaving one unowned would make it invisible
///   to every user while still consuming the one ISIN lookup quota per symbol.
///
/// Must run **after** `sqlx::migrate!`: both statements name tables the migrator
/// creates. It is idempotent, and on a freshly migrated Postgres database it does
/// nothing at all — no orphans, and the row already exists after the first run — so
/// it stays quiet in the normal production path.
///
/// Returns `Err` rather than exiting, so "startup refuses" is testable; `main`
/// turns an error into `exit(1)`.
pub async fn ensure_bootstrap_owner(pool: &DbPool, cfg: &StartupConfig) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO users (issuer, subj, email, name, created_at) \
         VALUES ($1, $2, NULL, \'Placeholder owner\', $3) \
         ON CONFLICT (issuer, subj) DO NOTHING",
    )
    .bind(PLACEHOLDER_ISSUER)
    .bind(PLACEHOLDER_SUBJ)
    .bind(chrono::Utc::now())
    .execute(pool)
    .await?;

    backfill_ownership(pool, cfg).await
}

/// Assign portfolios that have no owner, or refuse to start.
///
/// The rule is the spec's: an unowned portfolio is either handed to the
/// placeholder (development, or OIDC with a designated bootstrap subject) or it
/// stops the process. See [`ensure_bootstrap_owner`] for why this is a startup step
/// rather than a migration.
async fn backfill_ownership(pool: &DbPool, cfg: &StartupConfig) -> anyhow::Result<()> {
    let (orphans,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM portfolios WHERE owner_id IS NULL")
            .fetch_one(pool)
            .await?;

    if orphans == 0 {
        return Ok(());
    }

    let placeholder: (i64,) = sqlx::query_as(
        "SELECT id FROM users WHERE issuer = $1 AND subj = $2",
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

    sqlx::query("UPDATE portfolios SET owner_id = $1 WHERE owner_id IS NULL")
        .bind(placeholder.0)
        .execute(pool)
        .await?;
    eprintln!(
        "WARN: assigned {orphans} pre-existing portfolio(s) to the placeholder owner \
         (user {}); they transfer on bootstrap handover", placeholder.0
    );
    Ok(())
}

pub fn build_rocket(
    pool: DbPool,
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
