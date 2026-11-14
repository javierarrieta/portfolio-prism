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
//! server what it is actually doing, not by trusting the SQL in `export.rs`.

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
