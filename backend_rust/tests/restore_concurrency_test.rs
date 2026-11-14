//! Two restores at once — the test the SQLite engine never allowed.
//!
//! Under SQLite the second restore waited on the file lock and got `SQLITE_BUSY`.
//! Postgres has no file lock, so the two halves of the rule are tested separately here:
//!
//! * `replace` reads the caller's rows in order to delete them, so two restores for one
//!   caller genuinely conflict. `restore::apply` opens a `SERIALIZABLE` transaction, so
//!   the loser aborts `40001` and the caller is told to come back. At the server default
//!   — READ COMMITTED — the same race has two outcomes and neither is a `503`: the loser
//!   either finishes anyway, so the caller silently owns both restores' rows, or its
//!   `DELETE FROM assets` hits rows the winner already replaced and fails `23503` on
//!   `transactions_asset_id_fkey`, which is a `500`. Which was measured where, because
//!   they did not all reproduce the same way: over HTTP only the silent both-`200`
//!   outcome reproduced (6/6 runs of the mutation in the Task 7 report); the `23503`
//!   form was measured directly against the server with two concurrent READ COMMITTED
//!   transactions in `psql`, and the same concurrent-delete conflict shows up over HTTP
//!   as a `500` if `40001` is removed from `classify`. `a_concurrent_replace_restore_is_a_503_with_retry_after`
//!   is what keeps all of it from coming back.
//! * `copy` writes disjoint rows, so there is no read-write dependency for the
//!   serialiser to break and **both** restores commit. That is the contract, not a gap
//!   in the test, and `two_concurrent_copy_restores_both_commit` pins it in that
//!   direction so a future change that makes copy abort is noticed rather than
//!   celebrated.
//!
//! The client contract in both directions is `503` with `Retry-After`, never a `500`.
//! `tests/backup_test.rs` explains why the *pool-timeout* half of the same `503` cannot
//! be driven through this route at all.

mod common;

use common::{
    build_rocket, bundle_json, bundle_with_size, db_with_user, post_restore, seed_portfolio,
};
use rocket::http::Status;
use rocket::local::asynchronous::Client;

use backend_rust::db::DbPool;

/// Portfolios in the database, which is what a restore either wrote or did not.
async fn portfolios(pool: &DbPool) -> i64 {
    sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM portfolios")
        .fetch_one(pool)
        .await
        .unwrap()
        .0
}

/// A client for the caller's own database.
async fn client(pool: DbPool, user: i64) -> Client {
    Client::tracked(build_rocket(pool, user)).await.unwrap()
}

#[tokio::test]
async fn a_concurrent_replace_restore_is_a_503_with_retry_after() {
    // A bundle big enough that the winner's transaction outlasts the moment the loser
    // notices it lost: 2 portfolios, 25 assets, 200 transactions is ~230 statements,
    // and the loser aborts on its first delete — microseconds into a transaction that
    // takes tens of milliseconds. A one-row bundle leaves the two requests little
    // overlap to work with, and a race test that depends on luck is worse than none.
    let bundle = bundle_with_size(2, 25, 8);
    let (pool, user) = db_with_user("concurrent_replace_restore").await;
    // Something to delete. With an empty account the two restores have no rows to
    // conflict over and the test would pass by doing nothing.
    seed_portfolio(&pool, user, "Already here", "USD").await;
    let before = portfolios(&pool).await;
    assert_eq!(before, 1, "the fixture is what replace deletes");

    let c = client(pool.clone(), user).await;
    let (a, b) = tokio::join!(
        post_restore(&c, "replace", &bundle),
        post_restore(&c, "replace", &bundle),
    );

    let mut codes = [a.0, b.0];
    codes.sort();
    // Exactly one success and exactly one refusal. If both are 2xx the restore is not
    // SERIALIZABLE, and that is a finding about the engine switch, not a test to relax;
    // if both are 5xx the loser won, which is the same bug seen from the other side.
    // One caveat, because this is a real race: `[Ok, Ok]` is both the regression
    // signature above and what a badly-scheduled run looks like when the two requests
    // barely overlap. Re-run it before reading a single `[Ok, Ok]` as an SSI regression —
    // the mutation in the Task 7 report produced it 6 runs in 6, so a genuine
    // regression is persistent, not occasional.
    assert_eq!(
        codes,
        [Status::Ok, Status::ServiceUnavailable],
        "one restore must succeed and the other must be refused: {codes:?}"
    );
    let (loser, winner) = if a.0 == Status::Ok { (b, a) } else { (a, b) };
    assert_eq!(
        loser.0,
        Status::ServiceUnavailable,
        "the loser must be 503, got {:?} — a 500 here means the mapping in \
         `backup::export::classify` missed the sqlx error variant Postgres produced \
         (40001 for a serialisation failure), or that `apply` is no longer SERIALIZABLE \
         and the loser is back to losing with a 23503 constraint violation",
        loser.0
    );

    // The header is half of the contract #64 shipped, and a remapping must not silently
    // drop it. `lock_response` is the only way to build a `503` in that module, so this
    // is the real route proving the real header.
    let retry = loser
        .1
        .get_one("Retry-After")
        .expect("a 503 must say when to retry");
    assert!(
        retry.parse::<u32>().is_ok(),
        "Retry-After must be a number of seconds: {retry:?}"
    );
    assert_eq!(
        winner.1.get_one("Retry-After"),
        None,
        "the winner is a 200 and must not be told to retry"
    );

    // And the loser wrote *nothing*: it is not a partial restore sitting beside the
    // winner's. One transaction means the aborted one left the database exactly as it
    // found it, so what is here is the winner's bundle and nothing else.
    assert_eq!(
        portfolios(&pool).await,
        2,
        "the winner's two portfolios and nothing the loser wrote"
    );

    pool.close().await;
}

#[tokio::test]
async fn two_concurrent_copy_restores_both_commit() {
    // Copy mode is the opposite case, and it is pinned on purpose rather than left to
    // chance. Two copies insert disjoint rows: neither reads what the other writes, so
    // there is no read-write dependency for the serialiser to order, and a cycle it
    // could break never forms. Both committing is therefore a structural fact about
    // `SERIALIZABLE`, not a timing accident — which is exactly why it is worth a test:
    // if a future change gives copy a shared row, a lock, or a unique index to fight
    // over, the contract moves and this is the test that says which way.
    let (pool, user) = db_with_user("concurrent_copy_restores").await;
    let bundle = bundle_json("Copied");
    let c = client(pool.clone(), user).await;

    let (a, b) = tokio::join!(
        post_restore(&c, "copy", &bundle),
        post_restore(&c, "copy", &bundle),
    );

    assert_eq!(
        (a.0, b.0),
        (Status::Ok, Status::Ok),
        "copy does not conflict"
    );
    assert_eq!(
        portfolios(&pool).await,
        2,
        "both restores wrote their portfolio"
    );

    pool.close().await;
}
