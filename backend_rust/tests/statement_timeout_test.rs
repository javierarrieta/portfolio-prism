//! What `SET statement_timeout` on the pool actually does, and what it produces.
//!
//! Spec §5.4 makes a statement timeout one of the three conditions that mean "try
//! later", so two things are measured here rather than asserted from documentation:
//! that a pooled connection really runs under the limit `db::pool_options` sets, and
//! that the error it produces is `57014 query_canceled` — the SQLSTATE the plan's own
//! snippet left out, which is why "statement timeout → 503" was unreachable as written.
//!
//! The third test is the one that says what the limit *bounds*. It is per statement, not
//! per transaction: export holds one connection for its whole read and restore runs one
//! long transaction, and neither is cancelled by a transaction that merely takes a while.

mod common;

use backend_rust::backup::export::{DbFailure, classify};
use common::{pool_options, test_pool_url};
use rocket::http::Status;
use sqlx::Connection as _;

/// What the server reports for `setting` on a connection opened directly — the value
/// the server itself would give, with nothing from `db::pool_options` involved.
async fn show_on_a_raw_connection(url: &str, setting: &str) -> String {
    let mut conn = sqlx::postgres::PgConnection::connect(url).await.unwrap();
    let (value,): (String,) = sqlx::query_as(&format!("SHOW {setting}"))
        .fetch_one(&mut conn)
        .await
        .unwrap();
    value
}

#[tokio::test]
async fn a_pooled_connection_runs_under_the_statement_timeout() {
    let url = test_pool_url("pooled_statement_timeout").await;
    let pool = pool_options().connect(&url).await.unwrap();
    let (pooled,): (String,) = sqlx::query_as("SHOW statement_timeout")
        .fetch_one(&pool)
        .await
        .unwrap();
    // The server canonicalises an int-with-unit GUC to whole seconds when it can, so
    // `30000` and `'30s'` both read back as `30s`: this pins the duration, not the
    // spelling. Change the literal in `db::pool_options` and this names both values.
    assert_eq!(
        pooled, "30s",
        "`db::pool_options` sets `SET statement_timeout = '30s'` on every connection it opens"
    );

    // The other half: the same database on a connection the pool did not open has no
    // timeout at all. Without this the assertion above could be passing because someone
    // ran `ALTER DATABASE … SET statement_timeout`, which is not what this code claims.
    assert_eq!(
        show_on_a_raw_connection(&url, "statement_timeout").await,
        "0",
        "the limit comes from the pool's `after_connect`, not from the server"
    );

    pool.close().await;
}

#[tokio::test]
async fn a_statement_timeout_is_a_retryable_lock() {
    // Driven through the driver, not a fake: this is the proof that the code `classify`
    // matches is the code Postgres actually sends, which is the half a hand-written
    // `DatabaseError` cannot prove.
    let url = test_pool_url("statement_timeout_fires").await;
    let mut conn = sqlx::postgres::PgConnection::connect(&url).await.unwrap();
    sqlx::query("SET statement_timeout = '50ms'")
        .execute(&mut conn)
        .await
        .unwrap();

    let err = sqlx::query("SELECT pg_sleep(5)")
        .execute(&mut conn)
        .await
        .expect_err("a 5 s sleep is over a 50 ms limit");
    let sqlx::Error::Database(db) = &err else {
        panic!("a cancelled statement must arrive as a database error, got {err:?}");
    };
    assert_eq!(
        db.code().as_deref(),
        Some("57014"),
        "statement_timeout raises query_canceled; the message is {err}"
    );

    assert!(matches!(classify(err), DbFailure::Lock));
    assert_eq!(DbFailure::Lock.status(), Status::ServiceUnavailable);
}

#[tokio::test]
async fn the_timeout_bounds_one_statement_and_not_the_transaction() {
    // Three statements of 90 ms each inside one transaction: 270 ms of work under a
    // 200 ms limit, and none of them is over it. If the limit were a transaction limit
    // — `idle_in_transaction_session_timeout`, or a `SET LOCAL` in a long transaction —
    // the third statement would fail and this test would say so.
    let url = test_pool_url("statement_timeout_is_per_statement").await;
    let mut conn = sqlx::postgres::PgConnection::connect(&url).await.unwrap();
    sqlx::query("SET statement_timeout = '200ms'")
        .execute(&mut conn)
        .await
        .unwrap();
    sqlx::query("BEGIN").execute(&mut conn).await.unwrap();
    for _ in 0..3 {
        sqlx::query("SELECT pg_sleep(0.09)")
            .execute(&mut conn)
            .await
            .expect("90 ms is under a 200 ms statement limit, however many there are");
    }
    sqlx::query("COMMIT").execute(&mut conn).await.unwrap();

    // And the same connection does cancel one statement that is over on its own, so the
    // test above passed because of the per-statement rule and not because the limit is
    // doing nothing.
    let err = sqlx::query("SELECT pg_sleep(0.5)")
        .execute(&mut conn)
        .await
        .expect_err("500 ms is over a 200 ms limit");
    let sqlx::Error::Database(db) = &err else {
        panic!("a cancelled statement must arrive as a database error, got {err:?}");
    };
    assert_eq!(
        db.code().as_deref(),
        Some("57014"),
        "the limit is live on this connection: {err}"
    );

    // The control that makes this a scope proof rather than a duration proof: the same
    // 270 ms of sleep, in one statement instead of three, is cancelled. Same work, same
    // connection, same limit, opposite outcome — the only difference is where the
    // statement began. If `SET statement_timeout` ever meant per-transaction, the loop
    // above would fail here instead.
    let err = sqlx::query("SELECT pg_sleep(0.09), pg_sleep(0.09), pg_sleep(0.09)")
        .execute(&mut conn)
        .await
        .expect_err("270 ms in one statement is over a 200 ms limit");
    let sqlx::Error::Database(db) = &err else {
        panic!("a cancelled statement must arrive as a database error, got {err:?}");
    };
    assert_eq!(
        db.code().as_deref(),
        Some("57014"),
        "the same work in one statement is cancelled: {err}"
    );
}
