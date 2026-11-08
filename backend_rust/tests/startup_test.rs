//! Startup behaviour: schema reconciliation, the ownership backfill, the
//! bootstrap handover, and the configuration refusals.
//!
//! Everything here asserts on a `Result`. `init_db` returns one instead of
//! calling `process::exit` precisely so that "startup refuses" is testable.

mod common;

use backend_rust::auth::{oidc::provision_or_update, StartupConfig};
use backend_rust::{init_db, PLACEHOLDER_ISSUER, PLACEHOLDER_SUBJ};
use common::{disabled_config, oidc_cfg, setup_db};
use sqlx::SqlitePool;

/// A config asking for OIDC with a designated bootstrap subject.
fn oidc_with_bootstrap(subj: &str) -> StartupConfig {
    let mut cfg = common::oidc_config();
    cfg.bootstrap_subj = Some(subj.to_string());
    cfg.oidc.get_or_insert_with(oidc_cfg).bootstrap_subj = Some(subj.to_string());
    cfg
}

async fn orphan_portfolio(pool: &SqlitePool) -> i64 {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO portfolios (name, description, currency, base_currency) \
         VALUES ('orphan', NULL, 'USD', 'USD') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    id
}

async fn orphan_count(pool: &SqlitePool) -> i64 {
    sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM portfolios WHERE owner_id IS NULL")
        .fetch_one(pool)
        .await
        .unwrap()
        .0
}

async fn placeholder_id(pool: &SqlitePool) -> Option<i64> {
    sqlx::query_as::<_, (i64,)>("SELECT id FROM users WHERE issuer = ? AND subj = ?")
        .bind(PLACEHOLDER_ISSUER)
        .bind(PLACEHOLDER_SUBJ)
        .fetch_optional(pool)
        .await
        .unwrap()
        .map(|r| r.0)
}

#[tokio::test]
async fn orphans_are_assigned_in_disabled_mode() {
    let pool = SqlitePool::connect(":memory:").await.unwrap();
    init_db(&pool, &disabled_config()).await.unwrap();
    orphan_portfolio(&pool).await;

    // Re-run init_db the way a restart would, with the orphan present.
    init_db(&pool, &disabled_config()).await.unwrap();
    assert_eq!(orphan_count(&pool).await, 0);
}

#[tokio::test]
async fn orphans_are_assigned_in_oidc_mode_with_a_bootstrap_subject() {
    let pool = SqlitePool::connect(":memory:").await.unwrap();
    init_db(&pool, &oidc_with_bootstrap("authlib:7")).await.unwrap();
    orphan_portfolio(&pool).await;

    init_db(&pool, &oidc_with_bootstrap("authlib:7")).await.unwrap();
    assert_eq!(orphan_count(&pool).await, 0);
}

#[tokio::test]
async fn orphans_with_no_bootstrap_subject_refuse_startup() {
    let pool = SqlitePool::connect(":memory:").await.unwrap();
    init_db(&pool, &common::oidc_config()).await.unwrap();
    orphan_portfolio(&pool).await;

    let err = init_db(&pool, &common::oidc_config()).await.unwrap_err().to_string();
    assert!(err.contains("BOOTSTRAP_OWNER_SUBJ"), "{err}");
    assert!(err.contains('1'), "the message must name the count: {err}");
    assert_eq!(orphan_count(&pool).await, 1, "refusing must not half-assign");
}

#[tokio::test]
async fn a_blank_bootstrap_subject_counts_as_unset_and_refuses() {
    for raw in ["", "   "] {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        init_db(&pool, &common::oidc_config()).await.unwrap();
        orphan_portfolio(&pool).await;

        let mut cfg = common::oidc_config();
        cfg.bootstrap_subj = Some(raw.to_string());
        let err = init_db(&pool, &cfg).await.unwrap_err().to_string();
        assert!(err.contains("BOOTSTRAP_OWNER_SUBJ"), "raw={raw:?}: {err}");
    }
}

#[tokio::test]
async fn a_fresh_database_gets_the_placeholder_row_and_starts() {
    // D-12: this is what keeps ./start.sh alive on an empty database.
    let pool = setup_db(&disabled_config()).await;
    assert!(placeholder_id(&pool).await.is_some(), "the placeholder must exist with no orphans");
    assert_eq!(orphan_count(&pool).await, 0);
}

#[tokio::test]
async fn the_placeholder_is_not_duplicated_by_a_second_run() {
    let pool = setup_db(&disabled_config()).await;
    init_db(&pool, &disabled_config()).await.unwrap();
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users WHERE issuer = ? AND subj = ?")
        .bind(PLACEHOLDER_ISSUER)
        .bind(PLACEHOLDER_SUBJ)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

// ---------------------------------------------------------------------------
// bootstrap handover, exercised through the primitives (D-14)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_bootstrap_subject_takes_over_the_placeholder_row_and_keeps_its_id() {
    let pool = setup_db(&common::oidc_config()).await;
    let placeholder = placeholder_id(&pool).await.unwrap();
    // Give the placeholder something to own, the way the backfill would have.
    let portfolio = sqlx::query_as::<_, (i64,)>(
        "INSERT INTO portfolios (name, description, currency, base_currency) VALUES ('legacy', NULL, 'USD', 'USD') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .0;
    sqlx::query("UPDATE portfolios SET owner_id = ? WHERE id = ?")
        .bind(placeholder)
        .bind(portfolio)
        .execute(&pool)
        .await
        .unwrap();

    let user = provision_or_update(&pool, "https://auth/x", "authlib:7", Some("a@b.c"), Some("A"), Some("authlib:7"))
        .await
        .unwrap();

    assert_eq!(user.id as i64, placeholder, "handover preserves the id, which is the point");
    assert!(placeholder_id(&pool).await.is_none(), "the placeholder row became the real one");
    let owner: (i64,) = sqlx::query_as("SELECT owner_id FROM portfolios WHERE id = ?")
        .bind(portfolio)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(owner.0, placeholder, "the legacy portfolio came with the row");
}

#[tokio::test]
async fn a_non_matching_subject_provisions_a_new_row_and_leaves_the_placeholder() {
    let pool = setup_db(&common::oidc_config()).await;
    let placeholder = placeholder_id(&pool).await.unwrap();

    let user = provision_or_update(&pool, "https://auth/x", "someone-else", None, None, Some("authlib:7"))
        .await
        .unwrap();

    assert_ne!(user.id as i64, placeholder);
    assert_eq!(placeholder_id(&pool).await, Some(placeholder));
}

#[tokio::test]
async fn handover_cannot_happen_twice() {
    // The rule: handover fires only when the subject matches *and* the placeholder
    // row exists. The first handover removes the row, so it is not repeatable.
    let pool = setup_db(&common::oidc_config()).await;
    let first = provision_or_update(&pool, "https://auth/x", "authlib:7", None, None, Some("authlib:7"))
        .await
        .unwrap();

    let second = provision_or_update(&pool, "https://auth/x", "authlib:7", None, None, Some("authlib:7"))
        .await
        .unwrap();
    assert_eq!(first.id, second.id, "the second call must reuse the row, not create one");

    // The handover *renames* the placeholder row rather than adding beside it, so a
    // fully bootstrapped database holds exactly one user.
    let (rows,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users").fetch_one(&pool).await.unwrap();
    assert_eq!(rows, 1, "one user: the placeholder, taken over");
}

#[tokio::test]
async fn a_restart_recreates_an_empty_placeholder_and_a_new_subject_can_take_it() {
    // The restart case named explicitly: after a handover the placeholder is gone;
    // init_db recreates one that owns nothing, and whatever BOOTSTRAP_OWNER_SUBJ is
    // set to next is what takes it. That is what the variable means.
    let pool = setup_db(&common::oidc_config()).await;
    provision_or_update(&pool, "https://auth/x", "authlib:7", None, None, Some("authlib:7"))
        .await
        .unwrap();
    assert!(placeholder_id(&pool).await.is_none());

    init_db(&pool, &common::oidc_config()).await.unwrap();
    let recreated = placeholder_id(&pool).await.unwrap();
    let (owned,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM portfolios WHERE owner_id = ?")
        .bind(recreated)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(owned, 0, "the recreated placeholder owns nothing");

    let next = provision_or_update(&pool, "https://auth/x", "authlib:99", None, None, Some("authlib:99"))
        .await
        .unwrap();
    assert_eq!(next.id as i64, recreated, "the newly designated subject takes the new placeholder");
}

#[tokio::test]
async fn a_known_identity_is_updated_not_duplicated() {
    let pool = setup_db(&common::oidc_config()).await;
    let first = provision_or_update(&pool, "https://auth/x", "sub-1", Some("old@x.y"), Some("Old"), None)
        .await
        .unwrap();
    let second = provision_or_update(&pool, "https://auth/x", "sub-1", Some("new@x.y"), Some("New"), None)
        .await
        .unwrap();

    assert_eq!(first.id, second.id);
    let (email,): (Option<String>,) =
        sqlx::query_as("SELECT email FROM users WHERE id = ?").bind(first.id as i64).fetch_one(&pool).await.unwrap();
    assert_eq!(email.as_deref(), Some("new@x.y"), "email/name refresh on every login");
}

#[tokio::test]
async fn concurrent_first_logins_for_one_subject_do_not_fail() {
    // ON CONFLICT DO NOTHING + re-select: two racing inserts must both resolve to
    // the same row instead of one login dying on the unique index.
    let pool = setup_db(&common::oidc_config()).await;
    let (a, b) = tokio::join!(
        provision_or_update(&pool, "https://auth/x", "racer", None, None, None),
        provision_or_update(&pool, "https://auth/x", "racer", None, None, None),
    );
    assert_eq!(a.unwrap().id, b.unwrap().id);
}
