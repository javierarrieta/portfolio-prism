//! The cross-user matrix: for every mounted route, user B must not be able to
//! reach user A's data — and user A must still be able to reach their own.
//!
//! Every denial case is paired with a success case for the owner. Without the
//! pair, a guard that simply broke everything would pass this file.
//!
//! Runs under `disabled` mode with two seeded users, because that mode lets a test
//! choose which identity a request runs as. The unauthenticated path is covered
//! by `auth_test.rs`.

mod common;

use common::{build_rocket, disabled_config, seed_asset, seed_portfolio, seed_transaction, setup_db};
use rocket::http::{Header, Status};
use rocket::local::asynchronous::Client;
use sqlx::SqlitePool;

struct Users {
    a: i32,
    b: i32,
    a_portfolio: i32,
    a_asset: i32,
    a_transaction: i32,
    b_portfolio: i32,
}

async fn seed_two_users(pool: &SqlitePool) -> Users {
    let a = common::seed_user(pool, "user-a").await;
    let b = common::seed_user(pool, "user-b").await;
    let a_portfolio = seed_portfolio(pool, a, "A portfolio", "USD").await;
    let b_portfolio = seed_portfolio(pool, b, "B portfolio", "USD").await;
    let a_asset = seed_asset(pool, a_portfolio, "AAA", "Asset A").await;
    let a_transaction = seed_transaction(pool, a_asset, "BUY", 1.0, 100.0, 1.0).await;
    Users { a, b, a_portfolio, a_asset, a_transaction, b_portfolio }
}

/// A client whose every request runs as `as_user`.
///
/// Bound to a variable at every call site: a `LocalRequest` borrows the `Client`,
/// so chaining straight off a temporary does not compile.
async fn client_as(pool: SqlitePool, as_user: i32) -> Client {
    Client::tracked(build_rocket(pool, as_user)).await.expect("valid rocket")
}

fn json() -> Header<'static> {
    Header::new("Content-Type", "application/json")
}

#[tokio::test]
async fn b_cannot_read_a_s_portfolio_but_a_can() {
    let pool = setup_db(&disabled_config()).await;
    let u = seed_two_users(&pool).await;
    let path = format!("/api/portfolios/{}", u.a_portfolio);

    let b = client_as(pool.clone(), u.b).await;
    assert_eq!(b.get(&path).dispatch().await.status(), Status::NotFound);

    let a = client_as(pool.clone(), u.a).await;
    assert_eq!(a.get(&path).dispatch().await.status(), Status::Ok);
}

#[tokio::test]
async fn b_cannot_update_or_delete_a_s_portfolio_but_a_can() {
    let pool = setup_db(&disabled_config()).await;
    let u = seed_two_users(&pool).await;
    let path = format!("/api/portfolios/{}", u.a_portfolio);
    let body = r#"{"currency":"EUR"}"#;

    let b = client_as(pool.clone(), u.b).await;
    assert_eq!(b.patch(&path).header(json()).body(body).dispatch().await.status(), Status::NotFound);
    assert_eq!(b.delete(&path).dispatch().await.status(), Status::NotFound);

    let a = client_as(pool.clone(), u.a).await;
    assert_eq!(a.patch(&path).header(json()).body(body).dispatch().await.status(), Status::Ok);
    // A's own portfolio holds an asset, so A is blocked by content, not by access.
    assert_eq!(a.delete(&path).dispatch().await.status(), Status::Conflict);
}

#[tokio::test]
async fn delete_of_a_foreign_empty_portfolio_is_404_not_409() {
    // The existence oracle the ordering inside `delete_portfolio` removes: a
    // non-owner must not be able to tell "exists and is empty" (409) from "does not
    // exist" (404).
    let pool = setup_db(&disabled_config()).await;
    let a = common::seed_user(&pool, "user-a").await;
    let b = common::seed_user(&pool, "user-b").await;
    let empty = seed_portfolio(&pool, a, "empty but A's", "USD").await;

    let client = client_as(pool, b).await;
    assert_eq!(client.delete(format!("/api/portfolios/{empty}")).dispatch().await.status(), Status::NotFound);
}

#[tokio::test]
async fn b_cannot_delete_a_s_asset_but_a_can() {
    let pool = setup_db(&disabled_config()).await;
    let u = seed_two_users(&pool).await;
    // A transaction-free asset: `delete_asset` issues a bare `DELETE FROM assets`,
    // which a foreign key rejects when transactions reference the asset. That is a
    // pre-existing bug, unrelated to access control, and pinned by its own test below
    // rather than hidden inside this one.
    let clean = seed_asset(&pool, u.a_portfolio, "CLEAN", "No transactions").await;
    let path = format!("/api/assets/{clean}");

    let b = client_as(pool.clone(), u.b).await;
    assert_eq!(b.delete(&path).dispatch().await.status(), Status::NotFound);

    let a = client_as(pool.clone(), u.a).await;
    assert_eq!(a.delete(&path).dispatch().await.status(), Status::NoContent);
}

#[tokio::test]
async fn deleting_an_asset_that_has_transactions_fails_today() {
    // Pre-existing, and deliberately left as-is: `delete_asset` issues a bare
    // `DELETE FROM assets` while sqlx enables `PRAGMA foreign_keys` by default, so
    // the delete is rejected and the handler reports 500. Recorded here so the
    // behaviour is visible rather than mistaken for an access-control regression.
    let pool = setup_db(&disabled_config()).await;
    let u = seed_two_users(&pool).await;
    let path = format!("/api/assets/{}", u.a_asset);

    let a = client_as(pool, u.a).await;
    assert_eq!(a.delete(&path).dispatch().await.status(), Status::InternalServerError);
}

#[tokio::test]
async fn b_cannot_write_into_a_s_asset_through_b_s_own_path() {
    // The cross-portfolio nested write: B owns `b_portfolio`, A owns `a_asset`
    // inside `a_portfolio`, and B addresses the asset through B's own path.
    let pool = setup_db(&disabled_config()).await;
    let u = seed_two_users(&pool).await;
    let b_path = format!("/api/portfolios/{}/assets/{}", u.b_portfolio, u.a_asset);
    let a_path = format!("/api/portfolios/{}/assets/{}", u.a_portfolio, u.a_asset);
    let body = r#"{"currency":"EUR"}"#;

    let b = client_as(pool.clone(), u.b).await;
    assert_eq!(b.patch(&b_path).header(json()).body(body).dispatch().await.status(), Status::NotFound);

    let a = client_as(pool.clone(), u.a).await;
    assert_eq!(a.patch(&a_path).header(json()).body(body).dispatch().await.status(), Status::Ok);
}

#[tokio::test]
async fn b_cannot_add_an_asset_to_a_s_portfolio() {
    let pool = setup_db(&disabled_config()).await;
    let u = seed_two_users(&pool).await;
    let path = format!("/api/portfolios/{}/assets", u.a_portfolio);
    let body = r#"{"symbol":"ZZZ","name":"Z","asset_type":"STOCK","sector":"","currency":"USD","isin":""}"#;

    let b = client_as(pool.clone(), u.b).await;
    assert_eq!(b.post(&path).header(json()).body(body).dispatch().await.status(), Status::NotFound);

    let a = client_as(pool.clone(), u.a).await;
    // 200, not the 201 the handler's own `#[utoipa::path]` documents — a
    // pre-existing mismatch between the annotation and the return type.
    assert_eq!(a.post(&path).header(json()).body(body).dispatch().await.status(), Status::Ok);
}

#[tokio::test]
async fn b_cannot_write_a_transaction_into_a_s_asset() {
    let pool = setup_db(&disabled_config()).await;
    let u = seed_two_users(&pool).await;
    let body = r#"{"quantity":1.0,"price":10.0,"fee":0.0,"date":"2024-01-01T00:00:00Z","type":"BUY"}"#;
    let b_path = format!("/api/portfolios/{}/assets/{}/transactions", u.b_portfolio, u.a_asset);
    let a_path = format!("/api/portfolios/{}/assets/{}/transactions", u.a_portfolio, u.a_asset);

    let b = client_as(pool.clone(), u.b).await;
    assert_eq!(b.post(&b_path).header(json()).body(body).dispatch().await.status(), Status::NotFound);

    let a = client_as(pool.clone(), u.a).await;
    assert_eq!(a.post(&a_path).header(json()).body(body).dispatch().await.status(), Status::Ok);
}

#[tokio::test]
async fn b_cannot_list_or_delete_a_s_transactions() {
    let pool = setup_db(&disabled_config()).await;
    let u = seed_two_users(&pool).await;
    let list = format!("/api/portfolios/{}/transactions", u.a_portfolio);
    let del = format!("/api/transactions/{}", u.a_transaction);

    let b = client_as(pool.clone(), u.b).await;
    assert_eq!(b.get(&list).dispatch().await.status(), Status::NotFound);
    assert_eq!(b.delete(&del).dispatch().await.status(), Status::NotFound);

    let a = client_as(pool.clone(), u.a).await;
    assert_eq!(a.get(&list).dispatch().await.status(), Status::Ok);
    assert_eq!(a.delete(&del).dispatch().await.status(), Status::NoContent);
}

#[tokio::test]
async fn b_cannot_read_a_s_tax_summary_performance_or_sell_pnl() {
    let pool = setup_db(&disabled_config()).await;
    let u = seed_two_users(&pool).await;
    let q = "strategy=FIFO&threshold_days=365";
    let paths = vec![
        format!("/api/portfolios/{}/tax-summary?{q}", u.a_portfolio),
        format!("/api/portfolios/{}/performance", u.a_portfolio),
    ];

    let b = client_as(pool.clone(), u.b).await;
    for path in &paths {
        assert_eq!(b.get(path).dispatch().await.status(), Status::NotFound, "{path}");
    }

    let a = client_as(pool.clone(), u.a).await;
    for path in &paths {
        assert_eq!(a.get(path).dispatch().await.status(), Status::Ok, "{path}");
    }

    // The sell-pnl route: the 15th operation, absent from the spec's inventory.
    let sell = seed_transaction(&pool, u.a_asset, "SELL", 1.0, 120.0, 1.0).await;
    let pnl = format!("/api/transactions/{sell}/pnl?{q}");
    assert_eq!(b.get(&pnl).dispatch().await.status(), Status::NotFound);
    assert_eq!(a.get(&pnl).dispatch().await.status(), Status::Ok);

    // The dated variant, which the grep-based access gate cannot see: it is a
    // parameter-dependent path on an existing route, so this matrix is the real gate
    // (Access level, spec). `require` runs before the window is even parsed, so B's
    // dated request is a 404 and not a 422.
    let dated_b = format!("/api/portfolios/{}/tax-summary?strategy=FIFO&threshold_days=365&from=2024-01-01&to=2024-12-31", u.a_portfolio);
    assert_eq!(b.get(&dated_b).dispatch().await.status(), Status::NotFound);

    // A's own dated request gets past the access check and stops at the window: the
    // fixture has one transaction and no seeded close, so a window that could be answered
    // would reach the provider. `from > to` proves the request arrived instead.
    let invalid_a = format!("/api/portfolios/{}/tax-summary?from=2024-12-31&to=2024-01-01", u.a_portfolio);
    assert_eq!(a.get(&invalid_a).dispatch().await.status(), Status::UnprocessableEntity);
}

#[tokio::test]
async fn the_list_endpoint_returns_no_foreign_rows() {
    // D-5: filtering only the portfolios query would still return B's asset rows,
    // because the assets query had no filter at all and the handler groups by
    // portfolio_id afterwards.
    let pool = setup_db(&disabled_config()).await;
    let u = seed_two_users(&pool).await;
    seed_asset(&pool, u.b_portfolio, "BBB", "Asset B").await;

    let a = client_as(pool, u.a).await;
    let resp = a.get("/api/portfolios/").dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body: serde_json::Value = resp.into_json().await.unwrap();
    let portfolios = body.as_array().unwrap();
    assert_eq!(portfolios.len(), 1, "A must see exactly their own portfolio");
    let serialised = serde_json::to_string(&portfolios).unwrap();
    assert!(serialised.contains("AAA"));
    assert!(!serialised.contains("BBB"), "A's response leaked B's asset rows");
}

#[tokio::test]
async fn a_nonexistent_portfolio_is_404_not_201_when_creating_an_asset() {
    // D-6: the insert used to accept any portfolio_id and create a dangling asset.
    let pool = setup_db(&disabled_config()).await;
    let a = common::seed_user(&pool, "user-a").await;
    let client = client_as(pool, a).await;
    let resp = client
        .post("/api/portfolios/4242/assets")
        .header(json())
        .body(r#"{"symbol":"ZZZ","name":"Z","asset_type":"STOCK","sector":"","currency":"USD","isin":""}"#)
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::NotFound);
}

#[tokio::test]
async fn two_users_sharing_an_email_cannot_reach_each_other_s_rows() {
    // What makes "email never decides ownership" enforceable rather than a comment:
    // with self-registration enabled, an email claim is claimable by anyone.
    let pool = setup_db(&disabled_config()).await;
    let a = common::seed_user(&pool, "user-a").await;
    let b = common::seed_user(&pool, "user-b").await;
    for id in [a, b] {
        sqlx::query("UPDATE users SET email = 'shared@example.com' WHERE id = ?")
            .bind(id as i64)
            .execute(&pool)
            .await
            .unwrap();
    }
    let a_portfolio = seed_portfolio(&pool, a, "A's", "USD").await;

    let client = client_as(pool, b).await;
    let resp = client.get(format!("/api/portfolios/{a_portfolio}")).dispatch().await;
    assert_eq!(resp.status(), Status::NotFound, "a shared email must grant nothing");
}

// Routes with no id to attack.
//
// `POST /api/portfolios/` and `GET /api/assets/lookup` authenticate but check no
// resource id, so there is no cross-user case to invent here. The first is covered
// by the ownership it asserts (every other test depends on that column) and by the
// F1 sweep in `auth_test.rs`; the second touches none of the three tables.

// ---------------------------------------------------------------------------
// Export.
//
// The static access gate cannot see these: the ids arrive in a query string and
// the rows are reached by `WHERE portfolio_id = ?`, neither of which Rule B
// matches. This file is the gate for the export route.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn b_cannot_export_a_s_portfolio_but_a_can() {
    // The export handler loops `require(.., Viewer)` over every id before it reads
    // anything. This is what proves that loop exists; the static gate cannot.
    let pool = setup_db(&disabled_config()).await;
    let u = seed_two_users(&pool).await;
    let path = format!("/api/backup/export?portfolio_ids={}", u.a_portfolio);

    let b = client_as(pool.clone(), u.b).await;
    assert_eq!(b.get(&path).dispatch().await.status(), Status::NotFound);

    let a = client_as(pool.clone(), u.a).await;
    assert_eq!(a.get(&path).dispatch().await.status(), Status::Ok);
}

#[tokio::test]
async fn one_foreign_id_denies_the_whole_export() {
    // A request for `[mine, theirs]` must not return mine and quietly drop theirs.
    // A partial success would look like a bug in the user's own selection.
    let pool = setup_db(&disabled_config()).await;
    let u = seed_two_users(&pool).await;
    let path = format!("/api/backup/export?portfolio_ids={},{}", u.a_portfolio, u.b_portfolio);

    let a = client_as(pool, u.a).await;
    let res = a.get(&path).dispatch().await;
    assert_eq!(res.status(), Status::NotFound);
    // And the denial carries no rows at all: a partial bundle would be the leak.
    let body = res.into_string().await.unwrap();
    assert!(!body.contains("A portfolio"), "a denied export must not return any rows: {body}");
}

#[tokio::test]
async fn an_export_never_contains_another_user_s_rows() {
    // Positive-side proof: A's bundle of A's portfolio holds A's asset and none of
    // B's. `seed_two_users` gives B an empty portfolio, so B's asset is seeded
    // here — and A's own asset is asserted present, or an empty bundle would pass
    // the negative half for the wrong reason.
    let pool = setup_db(&disabled_config()).await;
    let u = seed_two_users(&pool).await;
    seed_asset(&pool, u.b_portfolio, "BBB", "Asset B").await;

    let a = client_as(pool, u.a).await;
    let res = a
        .get(format!("/api/backup/export?portfolio_ids={}", u.a_portfolio))
        .dispatch()
        .await;
    let body: serde_json::Value = res.into_json().await.unwrap();
    let text = body.to_string();
    assert!(text.contains("Asset A"), "A's own bundle must contain A's asset: {text}");
    assert!(!text.contains("Asset B"), "A's bundle must not contain B's asset: {text}");
}
