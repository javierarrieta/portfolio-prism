//! The single place the engine is named. Everything else takes `DbPool`.
//!
//! Postgres only: there is no engine flag, no dialect abstraction and no `sqlite`
//! feature in `Cargo.toml`. A later task that changes the engine changes this file
//! and nothing else.
use sqlx::postgres::PgPoolOptions;

/// The pool type the whole backend passes around.
pub type DbPool = sqlx::PgPool;

/// The pool options production uses, in one place so a test can ask for the same ones.
///
/// [`crate::test_db::pool_options`] builds on this rather than repeating it, and
/// `tests/backup_test.rs::an_exhausted_pool_is_a_lock_not_an_outage` only proves what
/// production does while the two agree on `max_connections` and `acquire_timeout`. A
/// hand-typed copy drifts silently, and the test keeps passing while it measures a pool
/// production no longer has.
pub fn pool_options() -> PgPoolOptions {
    PgPoolOptions::new()
        .max_connections(5)
        .min_connections(0)
        // What a saturated pool turns into: a 503 with Retry-After, not a hung
        // request (spec §5.4, §10).
        .acquire_timeout(std::time::Duration::from_secs(5))
        .max_lifetime(std::time::Duration::from_secs(4 * 60 * 60))
}

/// Build the pool the way `main` does. See [`pool_options`].
pub async fn connect(url: &str) -> Result<DbPool, sqlx::Error> {
    pool_options().connect(url).await
}
