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
        // A runaway query must not pin a pool connection forever: `30s` per *statement*,
        // not per transaction — the limit restarts for every statement, so a transaction
        // of twenty two-second statements is never cancelled while one statement that
        // never returns is. That is the right shape for a pool, and it bounds a single
        // statement inside the long export read and the long restore transaction, never
        // either as a whole. Nothing here bounds those as a whole, and it is worth saying
        // so rather than implying a guarantee: the export's hold lasts as long as the
        // tenant's row volume makes it, because the bundle caps are checked on the
        // *built* bundle (`export::cap_error`), so an over-cap selection does every read
        // and only then gets a 422. For restore the caps do bound the work up front —
        // `parse_and_validate` checks both before `apply` opens its transaction.
        // Cancelling is `57014 query_canceled`, which `backup::export::classify` maps to
        // the retryable `503`.
        //
        // Set per connection, not with `ALTER DATABASE` or in `postgresql.conf`, so the
        // limit travels with the pool that needs it — including the test pool, which
        // builds through this function. The literal is the SQL the server sees, so
        // `SET statement_timeout` is greppable here, and
        // `tests/statement_timeout_test.rs` reads it back and shows which scope it bounds.
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                // macro-exempt: `SET` is a utility statement. Postgres `PREPARE` accepts
                // only SELECT/WITH/INSERT/UPDATE/DELETE/MERGE/VALUES, so no sqlx macro can
                // describe it — and there are no parameters to check anyway.
                sqlx::query("SET statement_timeout = '30s'")
                    .execute(&mut *conn)
                    .await
                    .map(|_| ())
            })
        })
}

/// Build the pool the way `main` does. See [`pool_options`].
pub async fn connect(url: &str) -> Result<DbPool, sqlx::Error> {
    pool_options().connect(url).await
}
