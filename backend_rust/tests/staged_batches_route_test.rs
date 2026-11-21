//! The three session routes, exercised over HTTP rather than through `staging::`.
//!
//! `tests/staging_test.rs` proves the lifecycle. What only a route test can prove is the
//! shape a caller actually sees: the status code, the body, and that a handle belonging to
//! someone else is answered exactly like a handle that does not exist — including in the
//! JSON, which is where a careless `format!("{:?}", status)` would leak the difference.
//!
//! Two rockets over one pool, because `build_rocket` pins the caller at build time
//! (`AuthMode::Disabled { as_user }`) and isolation is the thing under test. A fresh
//! database per test would make the isolation assertions vacuous.

use backend_rust::agent::protocol::{Proposal, ProposalAsset, Target};
use backend_rust::agent::staging;
use rocket::http::{Method, Status};
use rocket::local::asynchronous::{Client, LocalResponse};
use serde_json::{json, Value};

mod common;
use common::{build_rocket, seed_user, test_pool};

async fn two_users(caller: &str) -> (sqlx::PgPool, i64, i64, Client, Client) {
    // One database, two rockets. `db_with_user` would hand back a database of its own per
    // call, and then "bob cannot see alice's batch" would be a statement about two
    // databases rather than about the query.
    let pool = test_pool(caller).await;
    let alice = seed_user(&pool, "alice").await;
    let bob = seed_user(&pool, "bob").await;

    let as_alice = Client::tracked(build_rocket(pool.clone(), alice)).await.unwrap();
    let as_bob = Client::tracked(build_rocket(pool.clone(), bob)).await.unwrap();
    (pool, alice, bob, as_alice, as_bob)
}

fn proposal() -> Proposal {
    Proposal {
        target: Target::New {
            name: "Depot".into(),
            base_currency: "EUR".into(),
        },
        assets: vec![serde_json::from_value::<ProposalAsset>(json!({
            "symbol": "IWDA",
            "asset_type": "ETF",
            "transactions": [
                {"type": "BUY", "date": "2024-03-01", "quantity": "1.5", "price": "82.34", "fee": "0.00"}
            ]
        }))
        .unwrap()],
    }
}

async fn stage(pool: &sqlx::PgPool, user: i64) -> String {
    staging::stage(pool, user, &proposal(), 60)
        .await
        .unwrap()
        .handle
}

async fn req<'c>(client: &'c Client, method: Method, path: &'c str) -> LocalResponse<'c> {
    client.req(method, path).dispatch().await
}

#[tokio::test]
async fn the_list_is_the_callers_and_nobody_elses() {
    let (pool, alice, _, as_alice, as_bob) = two_users("list_is_mine").await;
    let handle = stage(&pool, alice).await;

    let resp = req(&as_alice, Method::Get, "/api/staged-batches").await;
    assert_eq!(resp.status(), Status::Ok);
    let body: Value = resp.into_json().await.unwrap();
    assert_eq!(body.as_array().unwrap().len(), 1);
    let row = &body[0];
    assert_eq!(row["handle"], json!(handle));
    assert_eq!(row["status"], json!("staged"));
    assert_eq!(row["asset_count"], json!(1));
    assert_eq!(row["transaction_count"], json!(1));
    assert_eq!(row["expired"], json!(false));
    // The list is a menu, so it must not carry the payload: that is the difference between
    // a panel that loads quickly and one that ships every staged row to the browser.
    assert!(row.get("payload").is_none(), "the list leaked the payload");
    assert!(row.get("preview").is_none(), "the list leaked the preview");

    let resp = req(&as_bob, Method::Get, "/api/staged-batches").await;
    assert_eq!(resp.status(), Status::Ok);
    assert_eq!(
        resp.into_json::<Value>().await.unwrap().as_array().unwrap().len(),
        0,
        "a second user's list is empty, not somebody else's batches"
    );
}

#[tokio::test]
async fn the_detail_route_answers_the_stored_preview_and_labels_it() {
    let (pool, alice, _, as_alice, _) = two_users("detail").await;
    let handle = stage(&pool, alice).await;

    let path = format!("/api/staged-batches/{handle}");
    let resp = req(&as_alice, Method::Get, &path).await;
    assert_eq!(resp.status(), Status::Ok);
    let body: Value = resp.into_json().await.unwrap();

    // The preview is rendered from what was stored, so the counts and the target come from
    // the same bytes the agent was shown.
    assert_eq!(body["assets"][0]["symbol"], json!("IWDA"));
    assert_eq!(body["status"], json!("staged"));
    assert_eq!(body["expired"], json!(false));
    assert!(body["expires_at"].is_string());
}

#[tokio::test]
async fn another_users_handle_is_answered_identically_to_no_handle() {
    let (pool, alice, _, as_alice, as_bob) = two_users("isolated").await;
    let handle = stage(&pool, alice).await;
    let ghost = "f".repeat(64);

    for (label, path) in [
        ("real-but-not-mine", format!("/api/staged-batches/{handle}")),
        ("nonexistent", format!("/api/staged-batches/{ghost}")),
    ] {
        let resp = req(&as_bob, Method::Get, &path).await;
        assert_eq!(resp.status(), Status::NotFound, "{label}");
        let body: Value = resp.into_json().await.unwrap();
        assert_eq!(body["error"], json!("not_found"), "{label}");
    }

    // Approve too, and then prove the refusal was a no-op rather than a side effect.
    let path = format!("/api/staged-batches/{handle}/approve");
    let resp = req(&as_bob, Method::Post, &path).await;
    assert_eq!(resp.status(), Status::NotFound);
    assert_eq!(
        staging::preview_of(&pool, alice, &handle)
            .await
            .unwrap()
            .unwrap()
            .0,
        "staged",
        "a denied approval must not move the state"
    );

    // And alice can still approve it herself.
    let resp = req(&as_alice, Method::Post, &path).await;
    assert_eq!(resp.status(), Status::Ok);
}

#[tokio::test]
async fn approval_is_a_one_shot_over_http_too() {
    let (pool, alice, _, as_alice, _) = two_users("approve_once").await;
    let handle = stage(&pool, alice).await;
    let path = format!("/api/staged-batches/{handle}/approve");

    let resp = req(&as_alice, Method::Post, &path).await;
    assert_eq!(resp.status(), Status::Ok);
    assert_eq!(resp.into_json::<Value>().await.unwrap()["status"], json!("approved"));

    let resp = req(&as_alice, Method::Post, &path).await;
    assert_eq!(resp.status(), Status::Conflict, "the second approve must not pass");
    let body: Value = resp.into_json().await.unwrap();
    assert_eq!(body["error"], json!("not_approvable"));
    assert_eq!(
        body["current_status"],
        json!("approved"),
        "the body says why, so the UI does not have to guess between 'already approved' and 'gone'"
    );
}

#[tokio::test]
async fn an_expired_batch_is_reported_expired_rather_than_approvable() {
    let (pool, alice, _, as_alice, _) = two_users("expired").await;
    let handle = stage(&pool, alice).await;
    sqlx::query("UPDATE agent_staged_batch SET expires_at = now() - interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();

    let path = format!("/api/staged-batches/{handle}");
    let resp = req(&as_alice, Method::Get, &path).await;
    let body: Value = resp.into_json().await.unwrap();
    assert_eq!(body["expired"], json!(true));

    let resp = req(&as_alice, Method::Get, "/api/staged-batches").await;
    assert_eq!(resp.into_json::<Value>().await.unwrap()[0]["expired"], json!(true));
}
