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
use backend_rust::agent::staging::{self, ApplyError, StageError, ValidationError, MAX_ASSETS_PER_BATCH,
                                   MAX_PORTFOLIOS_PER_USER, MAX_PREVIEW_BYTES,
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
        Err(ValidationError::BatchTooLarge { what: "assets", .. })
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
        Err(ValidationError::BatchTooLarge { what: "transactions", .. })
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

#[test]
fn an_isin_that_will_not_validate_is_refused_at_staging_and_not_at_apply() {
    // The same shape as the date-window test below it, and the same trap: `resolve_asset`
    // hands the string to `services::assets::insert`, which answers `IsinInvalid`. That is
    // a `write_refused` *after* a human approved the batch; the transaction rolls back, the
    // batch stays `approved`, and every retry fails identically until it expires. The
    // preview shows the ISIN and says nothing about it being wrong, so the approver had no
    // way to see it coming.
    let with_isin = |isin: &str| -> Proposal {
        serde_json::from_value(serde_json::json!({
            "target": {"kind": "new", "name": "Depot", "base_currency": "EUR"},
            "assets": [{
                "symbol": "A", "asset_type": "ETF", "isin": isin,
                "transactions": [tx("BUY", "2024-01-01", "1", "1", "0")]
            }]
        }))
        .unwrap()
    };

    // Real instruments, from the vectors in `api_routes::lookup`'s own tests.
    for good in ["IE00B3RBWM25", "CH0038863350", "de0007164600"] {
        staging::validate(&with_isin(good))
            .unwrap_or_else(|e| panic!("{good} is a real ISIN and must pass: {e:?}"));
    }

    // Absent and blank both mean "no ISIN", which is what `insert` treats them as too.
    staging::validate(&with_isin("")).unwrap_or_else(|e| panic!("an empty isin is no ISIN: {e:?}"));

    for bad in [
        "IE00B3RBWM24",   // right shape, check digit decremented
        "X",              // nonsense
        "IE00B3RBWM25X",  // thirteen characters
        "1E00B3RBWM25",   // a digit where the country code's letters belong
        " IE00B3RBWM25",  // padded: `insert` upper-cases but does NOT trim, so neither may this
    ] {
        let err = staging::validate(&with_isin(bad)).unwrap_err();
        assert_eq!(err.code(), "out_of_range", "{bad} must be refused at staging");
        assert_eq!(err.field(), Some("assets[0].isin"), "{bad} must name its path");
    }
}

#[test]
fn an_asset_name_over_the_cap_is_refused_the_way_a_portfolio_name_is() {
    // `MAX_NAME_LEN` bounds `target.name` and nothing else, while `assets[i].name` goes
    // into the preview verbatim and into the row. Same limit, same wire code, same promise.
    let with_name = |name: &str| -> Proposal {
        serde_json::from_value(serde_json::json!({
            "target": {"kind": "new", "name": "Depot", "base_currency": "EUR"},
            "assets": [{
                "symbol": "A", "asset_type": "ETF", "name": name,
                "transactions": [tx("BUY", "2024-01-01", "1", "1", "0")]
            }]
        }))
        .unwrap()
    };

    staging::validate(&with_name(&"x".repeat(120))).expect("exactly at the cap must pass");
    let err = staging::validate(&with_name(&"x".repeat(121))).unwrap_err();
    assert_eq!(err.code(), "out_of_range");
    assert_eq!(err.field(), Some("assets[0].name"));
}

#[test]
fn the_preview_never_promises_a_view_that_does_not_exist() {
    // With more than `PREVIEW_ASSETS_SHOWN` assets the preview lists some and counts the
    // rest — and there is no other view: `get_batch` and `get_staged` both serve this stored
    // preview and nothing else. The warning used to send the reader to a "full view" for the
    // remainder, which is a promise the approval cannot keep.
    let many = proposal(
        (0..30)
            .map(|i| asset(&format!("A{i}"), vec![tx("BUY", "2024-01-01", "1", "1", "0")]))
            .collect(),
    );
    let preview = staging::render_preview(&many);
    let warnings = preview["warnings"].as_array().expect("warnings");
    let text = warnings
        .iter()
        .map(|w| w.as_str().unwrap_or_default().to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(text.contains("not shown"), "{warnings:?}");
    for forbidden in ["full view", "listed by symbol only"] {
        assert!(!text.contains(forbidden), "the preview promises {forbidden:?}: {text}");
    }
}

#[tokio::test]
async fn a_batch_that_could_never_apply_is_refused_before_a_human_sees_it() {
    // End to end on the staging side of the same rule: `stage` must not hand out a handle
    // for a proposal that `apply` could never write, because the handle is what invites the
    // approval.
    let (pool, user) = db_with_user("stage_bad_isin").await;
    let mut p = one_ok();
    p.assets[0].isin = Some("IE00B3RBWM24".into());

    let err = staging::stage(&pool, user, &p, 60).await.unwrap_err();
    assert_eq!(err.code(), "out_of_range");
    assert_eq!(err.field(), Some("assets[0].isin"));
    // And the message says something an agent can act on, not a Debug dump.
    assert!(
        err.message().contains("assets[0].isin"),
        "the message must name the field: {}",
        err.message()
    );

    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_staged_batch")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0, "a refused proposal leaves no row behind");
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

// --- the target and the identity fields -------------------------------------------

#[tokio::test]
async fn a_target_that_cannot_become_a_portfolio_is_refused_at_staging() {
    // `Target::New` becomes a `portfolios` row verbatim, so an empty name or an empty base
    // currency is not cosmetic: a base currency of "" is a portfolio whose every pricing
    // lookup fails, and the failure arrives after a human approved the batch.
    for (name, base, what) in [
        ("", "EUR", "target.name"),
        ("   ", "EUR", "target.name"),
        ("Depot", "", "target.base_currency"),
    ] {
        let p = Proposal {
            target: Target::New {
                name: name.into(),
                base_currency: base.into(),
            },
            assets: vec![asset("IWDA", vec![tx("BUY", "2024-03-01", "1", "1", "0")])],
        };
        let err = staging::validate(&p).unwrap_err();
        assert_eq!(err.code(), "empty_asset", "{what} blank: {err:?}");
        assert_eq!(err.field(), Some(what), "the agent must be told which field");
    }
}

#[tokio::test]
async fn an_identifier_over_its_cap_is_refused_and_names_the_limit() {
    // The limit travels as data on the error rather than being baked into a sentence, so the
    // constant is stated once.
    let long = "x".repeat(41);
    let p = proposal(vec![asset(&long, vec![tx("BUY", "2024-03-01", "1", "1", "0")])]);
    let err = staging::validate(&p).unwrap_err();
    assert_eq!(err.code(), "out_of_range");
    assert_eq!(err.field(), Some("assets[0].symbol"));
    assert!(format!("{err:?}").contains("max: 40"), "the cap is in the answer: {err:?}");

    // Trailing whitespace does not buy extra length: the trim happens before the count.
    let at_cap = format!("{}   ", "x".repeat(MAX_IDENTIFIER_LEN_FOR_TEST));
    staging::validate(&proposal(vec![asset(
        &at_cap,
        vec![tx("BUY", "2024-03-01", "1", "1", "0")],
    )]))
    .expect("40 characters plus padding is at the cap, not over it");
}

/// Mirrors `staging`'s private `MAX_IDENTIFIER_LEN`; the over-cap case above is the one that
/// would fail loudly if the two ever disagree.
const MAX_IDENTIFIER_LEN_FOR_TEST: usize = 40;

#[tokio::test]
async fn a_blank_symbol_or_asset_type_is_refused_before_any_asset_row_exists() {
    // `services::assets::insert` does not refuse a blank symbol, so an asset nothing can list
    // or merge into would be created at apply — after the approval.
    for (symbol, asset_type, field) in [
        ("", "ETF", "assets[0].symbol"),
        ("IWDA", "  ", "assets[0].asset_type"),
    ] {
        let p = Proposal {
            target: Target::New {
                name: "Depot".into(),
                base_currency: "EUR".into(),
            },
            assets: vec![ProposalAsset {
                symbol: symbol.into(),
                isin: None,
                name: None,
                asset_type: asset_type.into(),
                currency: None,
                transactions: vec![tx("BUY", "2024-03-01", "1", "1", "0")],
            }],
        };
        let err = staging::validate(&p).unwrap_err();
        assert_eq!(err.code(), "empty_asset");
        assert_eq!(err.field(), Some(field));
    }
}

// --- approval and expiry -----------------------------------------------------------

#[tokio::test]
async fn an_expired_batch_cannot_be_approved() {
    // `apply` refuses an expired batch, so approving one is a promise that cannot be kept:
    // the human is told "approved, an agent may now apply this batch", the agent is told
    // `expired`, and nothing reconciles the two except a timestamp in a table. The expiry
    // belongs in the same conditional UPDATE as the status.
    let (pool, user) = db_with_user("approve_expired").await;
    let s = staging::stage(&pool, user, &one_ok(), 60).await.unwrap();
    sqlx::query("UPDATE agent_staged_batch SET expires_at = now() - interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();

    assert!(
        !staging::approve(&pool, user, &s.handle).await.unwrap(),
        "an approval nobody can spend is not an approval"
    );

    let (status, approved_at): (String, Option<chrono::DateTime<Utc>>) = sqlx::query_as(
        "SELECT status, approved_at FROM agent_staged_batch WHERE handle = $1",
    )
    .bind(&s.handle)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "staged", "and the row is left for the sweep, not marked");
    assert!(approved_at.is_none(), "no human act is recorded for a refused approval");
}

// --- the portfolio backstop --------------------------------------------------------

/// Give `user` exactly `n` portfolios, so a limit test can start one row below the line.
async fn seed_portfolios(pool: &sqlx::PgPool, user: i64, n: i64) {
    for i in 0..n {
        sqlx::query(
            "INSERT INTO portfolios (name, currency, base_currency, owner_id)
             VALUES ($1, 'EUR', 'EUR', $2)",
        )
        .bind(format!("P{i}"))
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn a_new_portfolio_is_refused_once_the_user_is_at_the_cap() {
    // Without this an `import` token could mint one portfolio per apply, forever, and every
    // one of them is a row in every portfolio listing from then on. The number itself is
    // arbitrary — see `MAX_PORTFOLIOS_PER_USER` — but the shape of the refusal is not: it
    // arrives at staging, before a human is asked to approve a batch that cannot land.
    let (pool, user) = db_with_user("portfolio_cap").await;
    seed_portfolios(&pool, user, MAX_PORTFOLIOS_PER_USER - 1).await;

    // One room left: the ninth stages, and so does the tenth after it is applied.
    staging::stage(&pool, user, &one_ok(), 60)
        .await
        .expect("one slot left means one batch stages");

    seed_portfolios(&pool, user, 1).await;
    let err = staging::stage(&pool, user, &one_ok(), 60).await.unwrap_err();
    assert_eq!(err.code(), "too_many_portfolios");
    assert!(
        format!("{err:?}").contains(&format!("max: {MAX_PORTFOLIOS_PER_USER}")),
        "the limit is in the answer: {err:?}"
    );
}

#[tokio::test]
async fn the_portfolio_cap_leaves_merging_into_an_existing_portfolio_alone() {
    // The cap is on creating, not on importing. A user at the limit can still import into a
    // portfolio they already own — that is the whole point of `Target::Existing`.
    let (pool, user) = db_with_user("portfolio_cap_merge").await;
    seed_portfolios(&pool, user, MAX_PORTFOLIOS_PER_USER).await;
    let (target,): (i64,) = sqlx::query_as(
        "SELECT id FROM portfolios WHERE owner_id = $1 LIMIT 1",
    )
    .bind(user)
    .fetch_one(&pool)
    .await
    .unwrap();

    let merge = Proposal {
        target: Target::Existing { portfolio_id: target },
        assets: vec![asset("IWDA", vec![tx("BUY", "2024-03-01", "1", "1", "0")])],
    };
    staging::stage(&pool, user, &merge, 60)
        .await
        .expect("an existing portfolio is not a new one");
}

#[test]
fn revalidation_can_only_ever_report_a_document_error() {
    // The guarantee that made `ApplyError::Rejected(StageError)` an impossible state, now
    // asserted rather than trusted. `Rejected` takes `ValidationError`, so the codes it can
    // surface are exactly the ones a document can cause. It used to take the whole
    // `StageError` and therefore claimed it could answer `too_many_pending`,
    // `too_many_portfolios` or `database` from re-reading a payload — arms that could never
    // run, because those are facts about the moment of staging. Widen `Rejected` back and
    // this fails.
    let cases = [
        ValidationError::BatchTooLarge { what: "assets", given: 1, max: 0 },
        ValidationError::EmptyAsset { field: "f".into() },
        ValidationError::NonCanonicalDecimal { field: "f".into(), given: "1e3".into() },
        ValidationError::BadDate { field: "f".into(), given: "2024-3-1".into() },
        ValidationError::OutOfRange { field: "f".into(), rule: "r", given: "1".into() },
        ValidationError::TooLong { field: "f".into(), given: "x".into(), max: 0 },
    ];
    let staging_only = ["too_many_pending", "too_many_portfolios", "database"];
    for case in cases {
        let code = ApplyError::Rejected(case.clone()).code();
        assert!(
            !staging_only.contains(&code),
            "{case:?} must not make apply answer a staging-only code, got {code}"
        );
        // And every one of them still has to be a code an agent can act on.
        assert!(
            StageError::Rejected(case.clone()).code() == code,
            "stage and apply must agree on the code for {case:?}"
        );
    }
}
