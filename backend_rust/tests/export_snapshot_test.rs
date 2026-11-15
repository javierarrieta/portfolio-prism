//! The one thing `export::build`'s transaction exists to provide: a single snapshot.
//!
//! Spec §5.4 and the module docs make the bundle a point-in-time copy, and the failure
//! that buys against is a bundle that straddles two database states — the portfolio row
//! from one moment and its transactions from a later one. Under the server default
//! (READ COMMITTED) Postgres takes a **fresh snapshot per statement**, so wrapping the
//! reads in a transaction does not prevent that on its own. `REPEATABLE READ` does, and
//! takes no row locks, so the export still cannot block a writer.
//!
//! Pinned the way `tests/statement_timeout_test.rs` pins the timeout: by asking the
//! server what it is actually doing, not by trusting the SQL in `export.rs`. The third
//! test pins the other half of the same statement — that the transaction is `READ ONLY`
//! as far as the server is concerned, so a write attempted inside it is refused rather
//! than quietly performed.

mod common;

use backend_rust::backup::export::begin_read_tx;
use common::{db_with_user, pool_options, seed_portfolio, test_pool_url};

#[tokio::test]
async fn the_export_transaction_runs_at_repeatable_read() {
    let url = test_pool_url("export_isolation").await;
    let pool = pool_options().connect(&url).await.unwrap();

    // Inside: the transaction `build` opens, on a pooled connection, really is at
    // REPEATABLE READ. The test calls `begin_read_tx` — the same call `build` makes —
    // and asserts the server's answer rather than the spelling of the statement in
    // `export.rs`. What it structurally cannot see is `build` bypassing the helper and
    // calling `pool.begin()` itself, which would leave this test green; Rule 3 of
    // `scripts/check_backup_portability.sh` is what catches that, and
    // `tests/backup_portability_test.rs::a_module_that_bypasses_its_own_transaction_helper_fails`
    // is the mutation proving the gate bites.
    let mut tx = begin_read_tx(&pool).await.unwrap();
    let (inside,): (String,) = sqlx::query_as("SHOW transaction_isolation")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(
        inside, "repeatable read",
        "`export::begin_read_tx` must actually open its transaction at REPEATABLE READ"
    );
    tx.commit().await.unwrap();

    // Outside: a pooled connection with no transaction around it is still at the server
    // default. Without this the assertion above could be passing because someone set
    // `default_transaction_isolation` on the database or the role, which is not what
    // `export::build` claims and would silently change every other route.
    let (outside,): (String,) = sqlx::query_as("SHOW transaction_isolation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        outside, "read committed",
        "only the export transaction raises the isolation level; every other statement \
         runs at the server default"
    );

    pool.close().await;
}

#[tokio::test]
async fn a_commit_between_two_reads_is_invisible_inside_the_export_transaction() {
    // The behaviour, not the setting: this is the straddle the transaction exists to
    // prevent, and the same read outside a transaction shows what the default would have
    // allowed.
    let (pool, owner) = db_with_user("export_snapshot").await;
    let pid = seed_portfolio(&pool, owner, "before", "USD").await;

    let mut tx = begin_read_tx(&pool).await.unwrap();
    let first = name(&mut tx, pid).await;
    assert_eq!(first, "before");

    // A write from another connection, committed while the export transaction is open —
    // exactly what another request does to a bundle being built.
    sqlx::query("UPDATE portfolios SET name = 'after' WHERE id = $1")
        .bind(pid)
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(
        name(&mut tx, pid).await,
        "before",
        "the second read must see the snapshot the first read used, not the commit that \
         landed between them; a bundle that mixes the two is the bug"
    );
    tx.commit().await.unwrap();

    // And the default really would have shown it, so the assertion above is about the
    // isolation level and not about a write that never landed.
    assert_eq!(
        sqlx::query_as::<_, (String,)>("SELECT name FROM portfolios WHERE id = $1")
            .bind(pid)
            .fetch_one(&pool)
            .await
            .unwrap()
            .0,
        "after",
        "outside the transaction the committed write is visible, which is what READ \
         COMMITTED would have given every read inside `build` as well"
    );

    pool.close().await;
}

async fn name(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, id: i64) -> String {
    sqlx::query_as::<_, (String,)>("SELECT name FROM portfolios WHERE id = $1")
        .bind(id)
        .fetch_one(&mut **tx)
        .await
        .unwrap()
        .0
}

#[tokio::test]
async fn the_export_transaction_is_read_only_as_far_as_the_server() {
    // The other half of `EXPORT_ISOLATION`, read back from the server for the same
    // reason the isolation level is: what matters is what Postgres is doing, not the
    // string in `export.rs`. `READ ONLY` is what turns "`build_in` contains only reads"
    // from a property of the Rust code into one the server enforces.
    let url = test_pool_url("export_read_only").await;
    let pool = pool_options().connect(&url).await.unwrap();

    let mut tx = begin_read_tx(&pool).await.unwrap();
    let (read_only,): (String,) = sqlx::query_as("SHOW transaction_read_only")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(
        read_only, "on",
        "`export::begin_read_tx` must open a transaction the server itself forbids writing in"
    );

    // The consequence rather than the setting: a write attempted inside the export is
    // refused. Before `READ ONLY` this statement succeeded, and an export that writes is
    // not an export — which is the drift the setting exists to make impossible. The
    // statement is refused whether or not the row exists, so nothing needs seeding.
    let err = sqlx::query("UPDATE portfolios SET name = 'mutated' WHERE id = 1")
        .execute(&mut *tx)
        .await
        .expect_err("a write inside the export transaction must not be allowed to succeed");
    let code = match &err {
        sqlx::Error::Database(db) => db.code().map(|c| c.to_string()),
        other => panic!("expected a database error, got {other:?}"),
    };
    assert_eq!(
        code.as_deref(),
        Some("25006"),
        "must be read_only_sql_transaction, and therefore DbFailure::Other and a 500 if \
         it ever reached classify: a refused write is a bug in the route, not a condition \
         a client can fix. Got: {err}"
    );

    // The refused statement aborts the transaction, so it rolls back rather than commits.
    // Nothing was written either way, which is the whole point.
    tx.rollback().await.unwrap();

    // Control: outside the export transaction a pooled connection can still write, so the
    // assertion inside is about this transaction and not about the role or the session.
    let (outside,): (String,) = sqlx::query_as("SHOW transaction_read_only")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        outside, "off",
        "only the export transaction is read-only; every other statement runs at the \
         server default, or the assertion inside would be passing for the wrong reason"
    );

    pool.close().await;
}
