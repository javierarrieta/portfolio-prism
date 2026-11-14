//! The harness's one promise: two tests, two databases.
//!
//! This lives in its own file rather than in `tests/common/mod.rs`, where the plan
//! put it, because `mod common;` is included by every integration binary — a test
//! there runs twenty-three times and adds twenty-three to the suite count for one
//! property. Here it runs once, which is what a property of the harness needs.
//!
//! Every assertion is written so that *doing nothing* fails it. A test that seeds
//! into one pool and asserts the other is empty is satisfied just as well by a
//! fixture that silently wrote nothing, so each direction asserts both halves: the
//! row is there, and the other pool cannot see it.

mod common;

use backend_rust::db::DbPool;
use common::{seed_asset, seed_portfolio, seed_user, test_pool};

/// Which database a pool is actually talking to, read from the server.
async fn current_database(pool: &DbPool) -> String {
    sqlx::query_as::<_, (String,)>("SELECT current_database()")
        .fetch_one(pool)
        .await
        .unwrap()
        .0
}

async fn count(pool: &DbPool, table: &str) -> i64 {
    sqlx::query_as::<_, (i64,)>(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(pool)
        .await
        .unwrap()
        .0
}

#[tokio::test]
async fn two_pools_in_one_test_do_not_see_each_others_rows() {
    let a = test_pool("isolation_pool_a").await;
    let b = test_pool("isolation_pool_b").await;

    let user_a = seed_user(&a, "only-in-a").await;
    let portfolio = seed_portfolio(&a, user_a, "only in a", "USD").await;
    seed_asset(&a, portfolio, "ISOA", "Only in a").await;

    // The positive half. Without these the counts below would pass on two empty
    // databases, which is the vacuous version of this test and the one that hides a
    // broken fixture.
    assert_eq!(
        count(&a, "users").await,
        1,
        "the fixture wrote no user, so nothing below proves anything"
    );
    assert_eq!(
        count(&a, "portfolios").await,
        1,
        "and the same for the portfolio"
    );
    assert_eq!(count(&a, "assets").await, 1, "and the same for the asset");

    // The negative half, in the other direction too: a fresh pool sees none of it.
    assert_eq!(
        count(&b, "users").await,
        0,
        "test databases are not isolated: b sees a's users"
    );
    assert_eq!(
        count(&b, "portfolios").await,
        0,
        "test databases are not isolated: b sees a's portfolios"
    );
    assert_eq!(
        count(&b, "assets").await,
        0,
        "test databases are not isolated: b sees a's assets"
    );

    // The same, written the other way, so a one-directional leak cannot pass.
    let user_b = seed_user(&b, "only-in-b").await;
    seed_portfolio(&b, user_b, "only in b", "USD").await;
    assert_eq!(
        count(&a, "users").await,
        1,
        "test databases are not isolated: a sees b's users"
    );
    assert_eq!(
        count(&a, "portfolios").await,
        1,
        "test databases are not isolated: a sees b's portfolios"
    );

    // And the direct statement, from the server rather than inferred from row counts:
    // the two pools are connected to two different databases.
    let name_a = current_database(&a).await;
    let name_b = current_database(&b).await;
    assert_ne!(
        name_a, name_b,
        "both pools are connected to {name_a}; the harness handed one test one database twice"
    );
    assert!(
        name_a.starts_with("prism_t_") && name_b.starts_with("prism_t_"),
        "a test pool is not connected to a harness database: {name_a}, {name_b}"
    );
}
