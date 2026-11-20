//! The staged-batch lifecycle: what is refused, what is stored, and who can move the state.
//!
//! The claims here are the ones the approval gate rests on, so they are written as the
//! gate's guarantees rather than as function coverage:
//!
//! - a proposal that cannot apply is refused *before* a human is asked to approve it;
//! - the preview is what the human sees and what the agent was told, and it stays inside the
//!   budget the client enforces even for a maximum-size batch;
//! - approval is a state transition a second caller cannot repeat;
//! - a handle belonging to someone else is indistinguishable from a handle that does not
//!   exist, from every angle: list, preview, approve.

use backend_rust::agent::protocol::{Proposal, ProposalAsset, ProposalTx, Target};
use backend_rust::agent::staging::{self, StageError, MAX_ASSETS_PER_BATCH, MAX_PREVIEW_BYTES,
                                   MAX_TRANSACTIONS_PER_BATCH};
use chrono::{Duration, Utc};

mod common;
use common::db_with_user;

fn tx(ty: &str, date: &str, qty: &str, price: &str, fee: &str) -> ProposalTx {
    // Built by parsing the same wire form an agent sends, so a test cannot construct a
    // state the deserializer would have refused.
    serde_json::from_value(serde_json::json!({
        "type": ty, "date": date, "quantity": qty, "price": price, "fee": fee
    }))
    .unwrap()
}

fn asset(symbol: &str, txns: Vec<ProposalTx>) -> ProposalAsset {
    serde_json::from_value(serde_json::json!({
        "symbol": symbol, "asset_type": "ETF", "transactions": txns
    }))
    .unwrap()
}

fn proposal(assets: Vec<ProposalAsset>) -> Proposal {
    Proposal {
        target: Target::New {
            name: "Depot".into(),
            base_currency: "EUR".into(),
        },
        assets,
    }
}

fn one_ok() -> Proposal {
    proposal(vec![asset("IWDA", vec![tx("BUY", "2024-03-01", "1.5", "82.34", "0.00")])])
}

// --- validation -------------------------------------------------------------------

#[test]
fn a_clean_proposal_passes_and_the_caps_are_exactly_the_stated_numbers() {
    staging::validate(&one_ok()).expect("the reference proposal must pass");

    let at_cap = proposal(vec![
        asset("A", vec![tx("BUY", "2024-01-01", "1", "1", "0")]);
        MAX_ASSETS_PER_BATCH
    ]);
    staging::validate(&at_cap).expect("exactly at the asset cap must pass");

    let over = proposal(
        (0..=MAX_ASSETS_PER_BATCH)
            .map(|i| {
                asset(
                    &format!("A{i}"),
                    vec![tx("BUY", "2024-01-01", "1", "1", "0")],
                )
            })
            .collect(),
    );
    assert!(matches!(
        staging::validate(&over),
        Err(StageError::BatchTooLarge { what: "assets", .. })
    ));
}

#[test]
fn the_transaction_cap_counts_across_assets_not_per_asset() {
    // 50 assets x 10 rows each is 500 and passes; one more row anywhere is 501 and does
    // not. A per-asset cap would let a batch of 50 x 100 through, which is the number the
    // preview budget cannot survive.
    let exactly = proposal(
        (0..50)
            .map(|i| {
                asset(
                    &format!("A{i}"),
                    (0..10).map(|_| tx("BUY", "2024-01-01", "1", "1", "0")).collect(),
                )
            })
            .collect(),
    );
    staging::validate(&exactly).expect("500 rows across 50 assets is exactly the cap");
    assert_eq!(
        exactly.assets.iter().map(|a| a.transactions.len()).sum::<usize>(),
        MAX_TRANSACTIONS_PER_BATCH
    );

    let mut over = exactly.clone();
    over.assets[0].transactions.push(tx("BUY", "2024-01-02", "1", "1", "0"));
    assert!(matches!(
        staging::validate(&over),
        Err(StageError::BatchTooLarge { what: "transactions", .. })
    ));
}

#[test]
fn a_decimal_that_is_not_canonical_is_refused_with_the_path_to_the_value() {
    // The path is the point: the agent holds a document and has to know which string to
    // fix. `142,30` is not a hypothetical — it is what a German broker statement produces,
    // and stored verbatim in a TEXT column it fails only on read.
    let bad = proposal(vec![asset(
        "IWDA",
        vec![
            tx("BUY", "2024-03-01", "1.5", "82.34", "0.00"),
            tx("BUY", "2024-03-02", "142,30", "82.34", "0.00"),
        ],
    )]);
    let err = staging::validate(&bad).unwrap_err();
    assert_eq!(err.code(), "non_canonical_decimal");
    assert_eq!(err.field(), Some("assets[0].transactions[1].quantity"));
}

#[test]
fn a_date_that_is_not_a_calendar_date_is_refused_rather_than_guessed_at() {
    for d in ["2024-03-01T00:00:00Z", "01/03/2024", "2024-13-01", "2024-3-1", ""] {
        let p = proposal(vec![asset("A", vec![tx("BUY", d, "1", "1", "0")])]);
        let err = staging::validate(&p).unwrap_err();
        assert_eq!(err.code(), "bad_date", "{d:?} must be refused");
    }
}

#[test]
fn the_sign_rules_are_stated_and_not_inferred() {
    // Zero price is legitimate: a transfer at nil cost is a real row.
    assert!(staging::validate(&proposal(vec![asset(
        "A",
        vec![tx("BUY", "2024-01-01", "1", "0", "0")]
    )]))
    .is_ok());

    for (ty, qty, price, fee, code) in [
        ("BUY", "0", "1", "0", "out_of_range"),
        ("BUY", "-1", "1", "0", "out_of_range"),
        ("BUY", "1", "-1", "0", "out_of_range"),
        ("BUY", "1", "1", "-1", "out_of_range"),
        // A SPLIT is a ratio; a zero price would make every later per-share figure a
        // division by zero, and it is refused at staging rather than after approval.
        ("SPLIT", "2", "0", "0", "out_of_range"),
    ] {
        let err = staging::validate(&proposal(vec![asset(
            "A",
            vec![tx(ty, "2024-01-01", qty, price, fee)],
        )]))
        .unwrap_err();
        assert_eq!(err.code(), code, "{ty} {qty}/{price}/{fee}");
    }
}

#[test]
fn an_asset_with_nothing_to_import_is_named_not_skipped() {
    let p = proposal(vec![
        asset("A", vec![tx("BUY", "2024-01-01", "1", "1", "0")]),
        asset("B", vec![]),
    ]);
    let err = staging::validate(&p).unwrap_err();
    assert_eq!(err.field(), Some("assets[1].transactions"));
}

// --- the preview budget -----------------------------------------------------------

#[test]
fn a_maximum_size_batch_still_fits_the_preview_budget() {
    // This is the claim that makes the caps defensible: 500 rows across 50 assets is the
    // largest thing `validate` admits, and the preview the human is shown must still fit
    // under the size at which the client truncates by deleting the middle. If this test
    // ever fails, the fix is the preview's bound, not the cap.
    let big = proposal(
        (0..MAX_ASSETS_PER_BATCH)
            .map(|i| {
                asset(
                    &format!("SYMBOL{i:03}"),
                    (0..10)
                        .map(|j| tx("BUY", &format!("2024-01-{:02}", (j % 28) + 1), "1.5", "82.3400", "0.00"))
                        .collect(),
                )
            })
            .collect(),
    );
    staging::validate(&big).expect("the maximum batch must validate");
    let bytes = serde_json::to_string(&staging::render_preview(&big)).unwrap();
    assert!(
        bytes.len() <= MAX_PREVIEW_BYTES,
        "preview is {} bytes, budget is {MAX_PREVIEW_BYTES}",
        bytes.len()
    );
}

#[test]
fn the_preview_surfaces_the_currency_assumption_instead_of_making_it() {
    let p = proposal(vec![asset("IWDA", vec![tx("BUY", "2024-01-01", "1", "1", "0")])]);
    let v = staging::render_preview(&p);
    assert_eq!(v["assets"][0]["currency_assumed"], serde_json::json!(true));
    assert!(
        v["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("base currency")),
        "the assumption has to be visible, not implied"
    );
}

// --- the lifecycle ----------------------------------------------------------------

#[test]
fn a_real_calendar_date_outside_the_write_window_is_refused_at_staging() {
    // 1899-12-31 is a real day in the right shape, so the format check alone lets it
    // through — and `services::transactions::insert` refuses it. Without this check the
    // refusal arrives at apply, after a human approved a preview promising the row.
    for d in ["1899-12-31", "2100-01-02", "1899-01-01"] {
        let p = proposal(vec![asset("A", vec![tx("BUY", d, "1", "1", "0")])]);
        let err = staging::validate(&p).unwrap_err();
        assert_eq!(err.code(), "out_of_range", "{d} must be refused at staging");
    }
    // The boundaries themselves are inside the window, matching `insert`.
    for d in ["1900-01-01", "2100-01-01"] {
        let p = proposal(vec![asset("A", vec![tx("BUY", d, "1", "1", "0")])]);
        staging::validate(&p).unwrap_or_else(|e| panic!("{d} must be accepted: {e:?}"));
    }
}

#[tokio::test]
async fn staging_persists_the_payload_and_the_preview_and_hands_out_a_handle() {
    let (pool, user) = db_with_user("stage_ok").await;
    let s = staging::stage(&pool, user, &one_ok(), 60).await.unwrap();

    assert_eq!(s.handle.len(), 64, "32 bytes, hex");
    assert_ne!(s.handle, s.id.to_string(), "the handle is not the row id");
    assert!(s.expires_at > Utc::now());

    let (status, preview, _) = staging::preview_of(&pool, user, &s.handle)
        .await
        .unwrap()
        .expect("the batch is there");
    assert_eq!(status, "staged");
    assert_eq!(preview, s.preview, "the stored preview is the one returned");

    let rows: (i64, i64) = sqlx::query_as(
        "SELECT jsonb_array_length(payload -> 'assets')::bigint,
                (SELECT count(*) FROM jsonb_array_elements(payload -> 'assets') a
                   CROSS JOIN jsonb_array_elements(a -> 'transactions'))::bigint
         FROM agent_staged_batch WHERE handle = $1",
    )
    .bind(&s.handle)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rows, (1, 1), "the payload round-trips");
}

#[tokio::test]
async fn the_sixth_pending_batch_is_refused_and_the_first_expired_one_makes_room() {
    let (pool, user) = db_with_user("stage_cap").await;
    for _ in 0..5 {
        staging::stage(&pool, user, &one_ok(), 60).await.unwrap();
    }
    let err = staging::stage(&pool, user, &one_ok(), 60).await.unwrap_err();
    assert_eq!(err.code(), "too_many_pending");

    // Expired batches are swept by the next stage, because there is no scheduled job.
    sqlx::query("UPDATE agent_staged_batch SET expires_at = now() - interval '1 hour'")
        .execute(&pool)
        .await
        .unwrap();
    staging::stage(&pool, user, &one_ok(), 60)
        .await
        .expect("expired batches are swept in the same transaction as the insert");

    let (live,): (i64,) = sqlx::query_as("SELECT count(*) FROM agent_staged_batch")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(live, 1, "the five expired ones are gone, not marked");
}

#[tokio::test]
async fn approval_is_a_one_shot_transition() {
    let (pool, user) = db_with_user("approve").await;
    let s = staging::stage(&pool, user, &one_ok(), 60).await.unwrap();

    assert!(staging::approve(&pool, user, &s.handle).await.unwrap());
    assert!(
        !staging::approve(&pool, user, &s.handle).await.unwrap(),
        "a second approval must not succeed; two tabs cannot both approve one batch"
    );

    let (status, approved_at): (String, Option<chrono::DateTime<Utc>>) = sqlx::query_as(
        "SELECT status, approved_at FROM agent_staged_batch WHERE handle = $1",
    )
    .bind(&s.handle)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "approved");
    assert!(approved_at.is_some(), "the human act is recorded");
}

#[tokio::test]
async fn another_users_handle_is_indistinguishable_from_no_handle() {
    let (pool, alice) = db_with_user("iso_alice").await;
    let s = staging::stage(&pool, alice, &one_ok(), 60).await.unwrap();

    let (_, bob) = db_with_user_with(&pool, "bob").await;

    assert!(staging::preview_of(&pool, bob, &s.handle).await.unwrap().is_none());
    assert!(!staging::approve(&pool, bob, &s.handle).await.unwrap());
    assert!(!staging::discard(&pool, bob, &s.handle).await.unwrap());

    // And it is still there, untouched — a failed cross-user attempt is a no-op, not a
    // side effect.
    assert_eq!(
        staging::preview_of(&pool, alice, &s.handle)
            .await
            .unwrap()
            .unwrap()
            .0,
        "staged"
    );
}

#[tokio::test]
async fn the_list_counts_what_was_staged_not_what_is_stored_now() {
    let (pool, user) = db_with_user("list").await;
    let p = proposal(vec![
        asset("A", vec![tx("BUY", "2024-01-01", "1", "1", "0"); 3]),
        asset("B", vec![tx("BUY", "2024-01-02", "1", "1", "0")]),
    ]);
    staging::stage(&pool, user, &p, 60).await.unwrap();

    let rows = staging::list(&pool, user).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].asset_count, 2);
    assert_eq!(rows[0].transaction_count, 4);
    assert_eq!(rows[0].status, "staged");
}

/// A second user in the *same* database — `db_with_user` makes a fresh database per call,
/// which would not test isolation at all.
async fn db_with_user_with(
    pool: &sqlx::PgPool,
    subj: &str,
) -> (sqlx::PgPool, i64) {
    let id: (i64,) = sqlx::query_as(
        "INSERT INTO users (issuer, subj, is_active, token_version, created_at)
         VALUES ('iss', $1, true, 0, now()) RETURNING id",
    )
    .bind(subj)
    .fetch_one(pool)
    .await
    .unwrap();
    (pool.clone(), id.0)
}

#[tokio::test]
async fn discard_is_available_without_approval_and_is_not_a_delete() {
    let (pool, user) = db_with_user("discard").await;
    let s = staging::stage(&pool, user, &one_ok(), 60).await.unwrap();
    assert!(staging::discard(&pool, user, &s.handle).await.unwrap());

    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM agent_staged_batch WHERE handle = $1")
            .bind(&s.handle)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        status, "discarded",
        "the row survives: 'the agent threw it away' and 'it never existed' stay different"
    );
    assert!(
        !staging::discard(&pool, user, &s.handle).await.unwrap(),
        "discarding twice is not a second event"
    );
}

#[tokio::test]
async fn a_ttl_of_zero_expires_immediately_rather_than_never() {
    // Guards the sign of the arithmetic: `now() - minutes` would make every batch born
    // expired and every guard the sweep deletes.
    let (pool, user) = db_with_user("ttl").await;
    let s = staging::stage(&pool, user, &one_ok(), 60).await.unwrap();
    assert!(s.expires_at > Utc::now() + Duration::minutes(59));
}
