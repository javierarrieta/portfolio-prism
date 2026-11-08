//! Public behaviour of the access chokepoint.
//!
//! The private `resolve` cases (owner / another user / `NULL` owner) live in
//! `src/access.rs`, where they can reach the private item. This file covers what a
//! handler actually calls.

mod common;

use backend_rust::access::{require, require_asset, require_asset_in, require_transaction, Access};
use common::{DEFAULT_USER, disabled_config, setup_db};
use rocket::http::Status;

#[tokio::test]
async fn viewer_and_editor_and_owner_all_admit_the_owner() {
    // Pins that the level argument is threaded even though membership is absent:
    // if `min` were ignored these would all pass for the wrong reason, so the
    // ordering test below is the other half of the pair.
    let pool = setup_db(&disabled_config()).await;
    let pid = common::seed_portfolio(&pool, DEFAULT_USER, "p", "USD").await;
    for min in [Access::Viewer, Access::Editor, Access::Owner] {
        assert!(require(&pool, DEFAULT_USER, pid, min).await.is_ok(), "{min:?}");
    }
}

#[tokio::test]
async fn a_non_owner_is_not_found_at_every_level() {
    let pool = setup_db(&disabled_config()).await;
    let pid = common::seed_portfolio(&pool, DEFAULT_USER, "p", "USD").await;
    let stranger = common::seed_user(&pool, "stranger");
    let stranger = stranger.await;
    for min in [Access::Viewer, Access::Editor, Access::Owner] {
        assert_eq!(
            require(&pool, stranger, pid, min).await.unwrap_err(),
            Status::NotFound,
            "{min:?}"
        );
    }
}

#[tokio::test]
async fn access_orders_viewer_below_editor_below_owner() {
    assert!(Access::Viewer < Access::Editor);
    assert!(Access::Editor < Access::Owner);
    assert!(Access::Viewer < Access::Owner);
}

#[tokio::test]
async fn require_asset_in_rejects_a_path_that_disagrees_with_the_row() {
    let pool = setup_db(&disabled_config()).await;
    let mine = common::seed_portfolio(&pool, DEFAULT_USER, "mine", "USD").await;
    let other = common::seed_portfolio(&pool, DEFAULT_USER, "other", "USD").await;
    let asset = common::seed_asset(&pool, mine, "AAA", "AAA").await;

    // Right asset, wrong portfolio in the path.
    assert_eq!(
        require_asset_in(&pool, DEFAULT_USER, other, asset, Access::Editor)
            .await
            .unwrap_err(),
        Status::NotFound
    );
    // Right asset, right portfolio.
    assert!(require_asset_in(&pool, DEFAULT_USER, mine, asset, Access::Editor).await.is_ok());
    // The same asset with no path to disagree with is accepted.
    assert!(require_asset(&pool, DEFAULT_USER, asset, Access::Editor).await.is_ok());
}

#[tokio::test]
async fn a_transaction_that_does_not_exist_is_not_found() {
    let pool = setup_db(&disabled_config()).await;
    assert_eq!(
        require_transaction(&pool, DEFAULT_USER, 4242, Access::Viewer).await.unwrap_err(),
        Status::NotFound
    );
}

#[tokio::test]
async fn foreign_keys_are_enforced_so_an_orphan_transaction_cannot_be_written() {
    // The spec assumed foreign keys are off. They are not: sqlx enables
    // `PRAGMA foreign_keys` by default, which is why `require_transaction`'s
    // "asset is gone" branch is defensive rather than reachable. This test pins
    // that assumption; if it ever fails, the dangling-asset path becomes live and
    // needs its own end-to-end coverage (the branch itself is unit-tested in
    // `src/access.rs`, where the schema is built without constraints).
    let pool = setup_db(&disabled_config()).await;
    let err = sqlx::query(
        "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) \
         VALUES (999999, 'BUY', '1.0', '1.0', '0.0', '2024-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap_err()
    .to_string();
    assert!(err.to_lowercase().contains("foreign key"), "{err}");
}

#[tokio::test]
async fn a_database_failure_is_a_server_error_not_a_denial() {
    // Dropping the table is the cheapest way to make the query itself fail. A
    // collapsed error would report 404 and hide an outage behind a denial.
    let pool = setup_db(&disabled_config()).await;
    let pid = common::seed_portfolio(&pool, DEFAULT_USER, "p", "USD").await;
    assert!(require(&pool, DEFAULT_USER, pid, Access::Viewer).await.is_ok());

    // Rows must exist for the lookup to reach the table that is about to fail;
    // otherwise the honest answer is "no such row", which is a 404, not an outage.
    let asset = common::seed_asset(&pool, pid, "AAA", "AAA").await;
    let tx = common::seed_transaction(&pool, asset, "BUY", 1.0, 100.0, 0.0).await;

    // Drop in foreign-key-safe order so SQLite allows each statement, leaving all
    // three helpers facing "no such table".
    for table in ["transactions", "assets", "portfolios"] {
        sqlx::query(&format!("DROP TABLE {table}")).execute(&pool).await.unwrap();
    }

    assert_eq!(
        require(&pool, DEFAULT_USER, pid, Access::Viewer).await.unwrap_err(),
        Status::InternalServerError
    );
    assert_eq!(
        require_asset(&pool, DEFAULT_USER, asset, Access::Viewer).await.unwrap_err(),
        Status::InternalServerError,
        "the asset row resolves to a portfolio lookup that then fails"
    );
    assert_eq!(
        require_transaction(&pool, DEFAULT_USER, tx, Access::Viewer).await.unwrap_err(),
        Status::InternalServerError
    );
}
