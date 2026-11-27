//! `staging::apply` — the only function in this module that writes portfolio rows.
//!
//! The claims under test are the ones the approval gate is *for*, and each one is stated as
//! the guarantee rather than as branch coverage:
//!
//! - nothing writes until a session has approved;
//! - a batch applies exactly once, and a second call writes nothing;
//! - a batch that fails halfway leaves **nothing**, and is still `approved` afterwards — a
//!   claim on a batch is not a burn of it, and a batch consumed by a failed apply is a batch
//!   nobody can recover;
//! - duplicate matching is `Decimal` equality against stored rows, so `1.5` and `1.50` are
//!   one row and not two;
//! - a handle or a target portfolio belonging to someone else is refused, and refused
//!   without side effects.

use backend_rust::agent::protocol::{Proposal, ProposalAsset, ProposalTx, Target};
use backend_rust::agent::staging;
use rust_decimal::Decimal;
use serde_json::json;

mod common;
use common::db_with_user;

fn tx(ty: &str, date: &str, qty: &str, price: &str, fee: &str) -> ProposalTx {
    serde_json::from_value(json!({
        "type": ty, "date": date, "quantity": qty, "price": price, "fee": fee
    }))
    .unwrap()
}

fn asset(symbol: &str, txns: Vec<ProposalTx>) -> ProposalAsset {
    serde_json::from_value(json!({
        "symbol": symbol, "asset_type": "ETF", "transactions": txns
    }))
    .unwrap()
}

fn new_portfolio(assets: Vec<ProposalAsset>) -> Proposal {
    Proposal {
        target: Target::New {
            name: "Depot".into(),
            base_currency: "EUR".into(),
        },
        assets,
    }
}

/// A portfolio to merge into. Duplicate matching is per-asset, and `Target::New` creates a
/// fresh portfolio on every apply — so a test about deduplication that stages `New` twice
/// is a test about two unrelated portfolios. Every dedupe test merges.
async fn owned_portfolio(pool: &sqlx::PgPool, user: i64) -> i64 {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO portfolios (name, currency, base_currency, owner_id)
         VALUES ('Target', 'EUR', 'EUR', $1) RETURNING id",
    )
    .bind(user)
    .fetch_one(pool)
    .await
    .unwrap();
    id
}

async fn stage_and_approve(pool: &sqlx::PgPool, user: i64, p: &Proposal) -> String {
    let s = staging::stage(pool, user, p, 60).await.unwrap();
    assert!(staging::approve(pool, user, &s.handle).await.unwrap());
    s.handle
}

async fn counts(pool: &sqlx::PgPool) -> (i64, i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM portfolios),
                (SELECT count(*) FROM assets),
                (SELECT count(*) FROM transactions)",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

// --- the gate ----------------------------------------------------------------------

#[tokio::test]
async fn a_batch_nobody_approved_writes_nothing() {
    let (pool, user) = db_with_user("apply_not_approved").await;
    let s = staging::stage(&pool, user, &new_portfolio(vec![asset(
        "IWDA",
        vec![tx("BUY", "2024-03-01", "1.5", "82.34", "0.00")],
    )]), 60)
    .await
    .unwrap();

    let err = staging::apply(&pool, user, &s.handle).await.unwrap_err();
    assert_eq!(err.code(), "not_approved");
    assert_eq!(counts(&pool).await, (0, 0, 0), "the gate is not decorative");

    // And the batch is untouched: a refused apply must not consume anything.
    assert_eq!(
        staging::preview_of(&pool, user, &s.handle).await.unwrap().unwrap().0,
        "staged"
    );
}

#[tokio::test]
async fn a_batch_applies_exactly_once() {
    let (pool, user) = db_with_user("apply_once").await;
    let handle = stage_and_approve(
        &pool,
        user,
        &new_portfolio(vec![asset("IWDA", vec![tx("BUY", "2024-03-01", "1.5", "82.34", "0.00")])]),
    )
    .await;

    let first = staging::apply(&pool, user, &handle).await.unwrap();
    assert_eq!(first.transaction_count, 1);
    let after_first = counts(&pool).await;

    let err = staging::apply(&pool, user, &handle).await.unwrap_err();
    assert_eq!(err.code(), "already_applied");
    assert_ne!(err.code(), "not_approved", "the agent must be able to tell these apart");
    assert_eq!(
        counts(&pool).await,
        after_first,
        "the second apply wrote nothing"
    );
}

#[tokio::test]
async fn an_expired_approval_is_refused_rather_than_honoured() {
    let (pool, user) = db_with_user("apply_expired").await;
    let handle = stage_and_approve(
        &pool,
        user,
        &new_portfolio(vec![asset("IWDA", vec![tx("BUY", "2024-03-01", "1.5", "82.34", "0.00")])]),
    )
    .await;

    sqlx::query("UPDATE agent_staged_batch SET expires_at = now() - interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();

    let err = staging::apply(&pool, user, &handle).await.unwrap_err();
    assert_eq!(err.code(), "expired");
    assert_eq!(counts(&pool).await, (0, 0, 0));
}

#[tokio::test]
async fn another_users_handle_is_refused_and_reads_as_no_handle() {
    let (pool, alice) = db_with_user("apply_iso").await;
    let handle = stage_and_approve(
        &pool,
        alice,
        &new_portfolio(vec![asset("IWDA", vec![tx("BUY", "2024-03-01", "1.5", "82.34", "0.00")])]),
    )
    .await;
    let bob = common::seed_user(&pool, "bob").await;

    let err = staging::apply(&pool, bob, &handle).await.unwrap_err();
    assert_eq!(err.code(), "stale_handle", "indistinguishable from a handle that never existed");
    assert_eq!(counts(&pool).await, (0, 0, 0));
}

// --- the happy path ----------------------------------------------------------------

#[tokio::test]
async fn an_approved_batch_lands_as_one_portfolio_and_named_counts() {
    let (pool, user) = db_with_user("apply_ok").await;
    let handle = stage_and_approve(
        &pool,
        user,
        &new_portfolio(vec![
            asset(
                "iwda",
                vec![
                    tx("BUY", "2024-03-01", "1.5", "82.34", "0.00"),
                    tx("BUY", "2024-04-01", "0.5", "84.10", "1.00"),
                ],
            ),
            asset("VWCE", vec![tx("BUY", "2024-03-05", "2", "103.00", "0.00")]),
        ]),
    )
    .await;

    let out = staging::apply(&pool, user, &handle).await.unwrap();
    assert_eq!(out.transaction_count, 3);
    assert_eq!(out.skipped_duplicates, 0);
    assert_eq!(out.asset_ids.len(), 2);

    let (portfolios, assets, transactions) = counts(&pool).await;
    assert_eq!((portfolios, assets, transactions), (1, 2, 3));

    // The portfolio is the caller's, and the symbol is stored uppercased by the same seam
    // the HTTP route uses — the agent sent "iwda".
    let (owner, symbol): (Option<i64>, String) = sqlx::query_as(
        "SELECT p.owner_id, a.symbol FROM portfolios p JOIN assets a ON a.portfolio_id = p.id
         WHERE a.symbol = 'IWDA'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(owner, Some(user));
    assert_eq!(symbol, "IWDA");
    assert!(out.portfolio_id > 0);
}

// --- all or nothing ----------------------------------------------------------------

#[tokio::test]
async fn a_batch_that_fails_halfway_leaves_nothing_and_is_still_approvable() {
    // The second asset carries an ISIN already used by a *differently named* asset in the
    // target portfolio, which is the collision `services::assets::insert` refuses and which
    // symbol-based merge cannot absorb. So the first asset's rows are written and then the
    // second asset's are refused.
    let (pool, user) = db_with_user("apply_atomic").await;
    let (portfolio,): (i64,) = sqlx::query_as(
        "INSERT INTO portfolios (name, currency, base_currency, owner_id)
         VALUES ('Target', 'EUR', 'EUR', $1) RETURNING id",
    )
    .bind(user)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO assets (portfolio_id, symbol, name, asset_type, currency, isin)
         VALUES ($1, 'HOLDER', 'Holder', 'ETF', 'EUR', 'US0378331005')",
    )
    .bind(portfolio)
    .execute(&pool)
    .await
    .unwrap();

    let mut second = asset("VWCE", vec![tx("BUY", "2024-03-05", "2", "103.00", "0.00")]);
    second.isin = Some("US0378331005".into()); // taken, under another symbol
    let handle = stage_and_approve(
        &pool,
        user,
        &Proposal {
            target: Target::Existing { portfolio_id: portfolio },
            assets: vec![
                asset("IWDA", vec![tx("BUY", "2024-03-01", "1.5", "82.34", "0.00")]),
                second,
            ],
        },
    )
    .await;

    let err = staging::apply(&pool, user, &handle).await.unwrap_err();
    assert_eq!(err.code(), "write_refused");
    // One portfolio and one asset: the two this test created by hand. The batch's own
    // portfolio row and asset row must not have survived its second asset's failure.
    assert_eq!(
        counts(&pool).await,
        (1, 1, 0),
        "the first asset's rows must not survive the second asset's failure"
    );
    assert_eq!(
        staging::preview_of(&pool, user, &handle).await.unwrap().unwrap().0,
        "approved",
        "a failed apply rolls its own claim back, so the batch can be retried"
    );
}

// --- duplicate matching ------------------------------------------------------------

#[tokio::test]
async fn a_duplicate_is_matched_on_value_and_reported_not_silent() {
    let (pool, user) = db_with_user("apply_dupes").await;
    let portfolio = owned_portfolio(&pool, user).await;
    let proposal = Proposal {
        target: Target::Existing { portfolio_id: portfolio },
        assets: vec![asset("IWDA", vec![tx("BUY", "2024-03-01", "1.5", "82.34", "0.00")])],
    };
    staging::apply(&pool, user, &stage_and_approve(&pool, user, &proposal).await)
        .await
        .unwrap();

    // The same statement staged a second time: every row is already there.
    let out = staging::apply(&pool, user, &stage_and_approve(&pool, user, &proposal).await)
        .await
        .unwrap();
    assert_eq!(out.transaction_count, 0, "nothing new was written");
    assert_eq!(
        out.skipped_duplicates, 1,
        "and the agent is told a row was skipped, rather than concluding rows were lost"
    );
    let (_, _, transactions) = counts(&pool).await;
    assert_eq!(transactions, 1);
}

#[tokio::test]
async fn duplicate_matching_is_decimal_equality_not_string_equality() {
    // `1.50` and `1.5` are the same cost basis. A string comparison would import both and
    // quietly double a tax lot, which is the specific corruption `backup::restore` refused
    // to guess about.
    let (pool, user) = db_with_user("apply_decimal_eq").await;
    let portfolio = owned_portfolio(&pool, user).await;
    let first = Proposal {
        target: Target::Existing { portfolio_id: portfolio },
        assets: vec![asset("IWDA", vec![tx("BUY", "2024-03-01", "1.5", "82.34", "0.00")])],
    };
    staging::apply(&pool, user, &stage_and_approve(&pool, user, &first).await)
        .await
        .unwrap();

    let out = staging::apply(
        &pool,
        user,
        &stage_and_approve(
            &pool,
            user,
            &Proposal {
                target: Target::Existing { portfolio_id: portfolio },
                assets: vec![asset(
                    "IWDA",
                    vec![tx("BUY", "2024-03-01", "1.50", "82.340", "0.0000")],
                )],
            },
        )
        .await,
    )
    .await
    .unwrap();
    assert_eq!(out.transaction_count, 0, "1.50/82.340/0.0000 is the same row");
    assert_eq!(out.skipped_duplicates, 1);

    // And a genuinely different amount is not skipped.
    let out = staging::apply(
        &pool,
        user,
        &stage_and_approve(
            &pool,
            user,
            &Proposal {
                target: Target::Existing { portfolio_id: portfolio },
                assets: vec![asset(
                    "IWDA",
                    vec![tx("BUY", "2024-03-01", "1.6", "82.34", "0.00")],
                )],
            },
        )
        .await,
    )
    .await
    .unwrap();
    assert_eq!(out.transaction_count, 1, "a different quantity is a different row");
}

#[tokio::test]
async fn a_split_is_matched_on_the_fee_that_was_stored_not_the_fee_that_was_sent() {
    // `services::transactions::insert` discards a split's fee, so a staged split carrying
    // `fee: 1.00` stores `0`. Keying the duplicate check on the sent value would make every
    // re-staged split look new and re-import it.
    let (pool, user) = db_with_user("apply_split_fee").await;
    let portfolio = owned_portfolio(&pool, user).await;
    let split = Proposal {
        target: Target::Existing { portfolio_id: portfolio },
        assets: vec![asset("VWCE", vec![tx("SPLIT", "2024-06-01", "2", "1.00", "1.00")])],
    };
    stage_and_approve(&pool, user, &split).await;
    staging::apply(&pool, user, &stage_and_approve(&pool, user, &split).await)
        .await
        .unwrap();

    let (fee,): (String,) = sqlx::query_as("SELECT fee FROM transactions LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(Decimal::from_str_exact(&fee).unwrap(), Decimal::ZERO, "the fee was discarded");

    let out = staging::apply(&pool, user, &stage_and_approve(&pool, user, &split).await)
        .await
        .unwrap();
    assert_eq!(out.skipped_duplicates, 1, "and the re-staged split is recognised as the same row");
}

// --- the target portfolio ----------------------------------------------------------

#[tokio::test]
async fn merging_into_an_existing_portfolio_reuses_the_asset_and_refuses_a_strangers_portfolio() {
    let (pool, user) = db_with_user("apply_merge").await;
    let (portfolio,): (i64,) = sqlx::query_as(
        "INSERT INTO portfolios (name, currency, base_currency, owner_id)
         VALUES ('Mine', 'EUR', 'EUR', $1) RETURNING id",
    )
    .bind(user)
    .fetch_one(&pool)
    .await
    .unwrap();

    let merge = Proposal {
        target: Target::Existing { portfolio_id: portfolio },
        assets: vec![asset("IWDA", vec![tx("BUY", "2024-03-01", "1.5", "82.34", "0.00")])],
    };
    staging::apply(&pool, user, &stage_and_approve(&pool, user, &merge).await)
        .await
        .unwrap();

    // Second batch, same target, same symbol: one asset, not a symbol collision.
    let out = staging::apply(&pool, user, &stage_and_approve(&pool, user, &merge).await)
        .await
        .unwrap();
    assert_eq!(out.portfolio_id, portfolio, "no new portfolio was created");
    assert_eq!(out.skipped_duplicates, 1);
    let (portfolios, assets, _) = counts(&pool).await;
    assert_eq!((portfolios, assets), (1, 1));

    // A portfolio that is somebody else's is not a target, and the preview that named it
    // does not make it one.
    let bob = common::seed_user(&pool, "bob").await;
    let stranger = Proposal {
        target: Target::Existing { portfolio_id: portfolio },
        assets: vec![asset("NOPE", vec![tx("BUY", "2024-03-01", "1", "1", "0")])],
    };
    let err = staging::apply(&pool, bob, &stage_and_approve(&pool, bob, &stranger).await)
        .await
        .unwrap_err();
    assert_eq!(err.code(), "portfolio_not_found");
    let (portfolios_after, assets_after, _) = counts(&pool).await;
    assert_eq!(
        (portfolios_after, assets_after),
        (portfolios, assets),
        "and the refusal wrote nothing into it"
    );
}

#[tokio::test]
async fn a_target_portfolio_deleted_after_approval_is_refused_not_guessed_at() {
    let (pool, user) = db_with_user("apply_gone").await;
    let (portfolio,): (i64,) = sqlx::query_as(
        "INSERT INTO portfolios (name, currency, base_currency, owner_id)
         VALUES ('Doomed', 'EUR', 'EUR', $1) RETURNING id",
    )
    .bind(user)
    .fetch_one(&pool)
    .await
    .unwrap();

    let handle = stage_and_approve(
        &pool,
        user,
        &Proposal {
            target: Target::Existing { portfolio_id: portfolio },
            assets: vec![asset("IWDA", vec![tx("BUY", "2024-03-01", "1.5", "82.34", "0.00")])],
        },
    )
    .await;

    sqlx::query("DELETE FROM portfolios WHERE id = $1")
        .bind(portfolio)
        .execute(&pool)
        .await
        .unwrap();

    let err = staging::apply(&pool, user, &handle).await.unwrap_err();
    assert_eq!(err.code(), "portfolio_not_found");
    assert_eq!(counts(&pool).await, (0, 0, 0));
}

// --- the currency an asset is created with ----------------------------------------

/// The currency stored for the first asset an apply creates.
async fn asset_currency(pool: &sqlx::PgPool, asset_id: i64) -> String {
    sqlx::query_scalar("SELECT currency FROM assets WHERE id = $1")
        .bind(asset_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn an_asset_without_a_currency_takes_the_portfolios_base_currency() {
    // `render_preview` warns, in as many words, that "one or more assets carry no currency
    // and will take the portfolio's base currency", and a human approves on the strength of
    // that sentence. Defaulting to a literal "USD" — which this path did — makes the preview
    // a lie for every non-USD portfolio, and the engines then price a EUR holding in dollars.
    // The existing EUR fixtures here never looked at the created asset's currency, which is
    // how the contradiction survived a green suite.
    let (pool, user) = db_with_user("apply_currency_new").await;
    let handle = stage_and_approve(
        &pool,
        user,
        &new_portfolio(vec![asset("IWDA", vec![tx("BUY", "2024-03-01", "1.5", "82.34", "0")])]),
    )
    .await;
    let out = staging::apply(&pool, user, &handle).await.unwrap();
    assert_eq!(
        asset_currency(&pool, out.asset_ids[0]).await,
        "EUR",
        "the new portfolio's base currency, not a literal"
    );
}

#[tokio::test]
async fn merging_into_an_existing_portfolio_defaults_to_its_base_currency_not_usd() {
    // The stronger case: the proposal says nothing about currency and the portfolio it merges
    // into is EUR for a reason. `Target::New` could have guessed from the proposal; this can
    // only be answered by reading the row.
    let (pool, user) = db_with_user("apply_currency_existing").await;
    let (portfolio,): (i64,) = sqlx::query_as(
        "INSERT INTO portfolios (name, currency, base_currency, owner_id)
         VALUES ('Depot EUR', 'EUR', 'EUR', $1) RETURNING id",
    )
    .bind(user)
    .fetch_one(&pool)
    .await
    .unwrap();

    let out = staging::apply(
        &pool,
        user,
        &stage_and_approve(
            &pool,
            user,
            &Proposal {
                target: Target::Existing { portfolio_id: portfolio },
                assets: vec![asset("IWDA", vec![tx("BUY", "2024-03-01", "1.5", "82.34", "0")])],
            },
        )
        .await,
    )
    .await
    .unwrap();

    assert_eq!(asset_currency(&pool, out.asset_ids[0]).await, "EUR");
}

#[tokio::test]
async fn an_explicit_currency_survives_a_portfolio_with_a_different_base() {
    // The default is a fallback, not an override: a USD line in a EUR portfolio is a USD line.
    let (pool, user) = db_with_user("apply_currency_explicit").await;
    let mut apple = asset("AAPL", vec![tx("BUY", "2024-03-01", "1", "100", "0")]);
    apple.currency = Some("USD".into());

    let out = staging::apply(
        &pool,
        user,
        &stage_and_approve(
            &pool,
            user,
            &Proposal {
                target: Target::New {
                    name: "Depot".into(),
                    base_currency: "EUR".into(),
                },
                assets: vec![apple],
            },
        )
        .await,
    )
    .await
    .unwrap();

    assert_eq!(asset_currency(&pool, out.asset_ids[0]).await, "USD");
}

#[tokio::test]
async fn apply_refuses_to_create_a_portfolio_past_the_cap_and_writes_nothing() {
    // `stage` refuses early, but the count can move between staging and apply — a portfolio
    // created by hand, or by another batch that applied first. The check inside the
    // transaction is the one that decides, and a refusal must leave the batch untouched.
    let (pool, user) = db_with_user("apply_portfolio_cap").await;
    let handle = stage_and_approve(
        &pool,
        user,
        &new_portfolio(vec![asset("IWDA", vec![tx("BUY", "2024-03-01", "1.5", "82.34", "0")])]),
    )
    .await;

    for i in 0..staging::MAX_PORTFOLIOS_PER_USER {
        sqlx::query(
            "INSERT INTO portfolios (name, currency, base_currency, owner_id)
             VALUES ($1, 'EUR', 'EUR', $2)",
        )
        .bind(format!("P{i}"))
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    }

    let err = staging::apply(&pool, user, &handle).await.unwrap_err();
    assert_eq!(err.code(), "too_many_portfolios");
    assert!(
        err.message().contains("merge into an existing portfolio"),
        "the agent is told what to do instead: {}",
        err.message()
    );
    assert_eq!(
        counts(&pool).await,
        (staging::MAX_PORTFOLIOS_PER_USER, 0, 0),
        "and the refusal wrote nothing"
    );

    // The batch is still approved and still spendable — the claim rolled back with the
    // failure, exactly as it does for any other failed write. Discarding it would be the
    // wrong advice; the user has one slot to free up.
    assert_eq!(
        staging::preview_of(&pool, user, &handle).await.unwrap().unwrap().0,
        "approved"
    );
}

// --- the locking, proven by contention ---------------------------------------------
//
// The module's two load-bearing claims are about concurrency: that a batch applies once,
// and that the portfolio cap cannot be walked past. Both were asserted only by sequential
// tests, which pass whether or not the locking exists.
//
// A `tokio::join!` race was tried first and rejected: on a current-thread runtime the two
// futures interleave only where they choose to, and the cap race passed with the advisory
// lock deleted — a test that cannot fail is not a test. These force the contention from
// the outside instead, by holding the very lock the code is supposed to take. Blocking on
// an advisory lock or a row lock is not a timing window; it is guaranteed. So each test
// fails the moment the lock it names is removed.

/// Hold `pg_advisory_lock(user_id)` on a session of our own, run `f`, and report whether it
/// finished while the lock was held. The connection is returned so the caller can release
/// it by dropping.
async fn held_user_lock<R, F: std::future::Future<Output = R>>(
    pool: &sqlx::PgPool,
    user: i64,
    f: F,
) -> (Result<R, ()>, sqlx::pool::PoolConnection<sqlx::Postgres>) {
    let mut conn = pool.acquire().await.unwrap();
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(user)
        .execute(&mut *conn)
        .await
        .unwrap();
    let outcome = tokio::time::timeout(std::time::Duration::from_millis(400), f)
        .await
        .map_err(|_| ());
    (outcome, conn)
}

#[tokio::test]
async fn apply_takes_the_user_lock_so_the_portfolio_cap_cannot_be_walked_past() {
    // The claim: `apply` serialises against anything else holding the user's advisory lock,
    // which is what makes the in-transaction portfolio count mean something. Two approved
    // `Target::New` batches are two rows, so the batch row's `FOR UPDATE` cannot do this
    // job — and the comment in `apply` used to claim it did.
    let (pool, user) = db_with_user("apply_user_lock").await;
    let handle = stage_and_approve(
        &pool,
        user,
        &new_portfolio(vec![asset("IWDA", vec![tx("BUY", "2024-03-01", "1.5", "82.34", "0")])]),
    )
    .await;

    let (outcome, conn) = held_user_lock(&pool, user, staging::apply(&pool, user, &handle)).await;
    assert!(
        outcome.is_err(),
        "apply ran while another session held the user's advisory lock, so the portfolio \
         cap it checks inside that lock is not serialised with anything"
    );

    // Release, and the same apply goes through — so the wait above was the lock and not
    // some other refusal.
    drop(conn);
    staging::apply(&pool, user, &handle)
        .await
        .expect("apply proceeds once the user lock is free");
    assert_eq!(counts(&pool).await.0, 1);
}

#[tokio::test]
async fn apply_serialises_on_the_batch_row_for_its_whole_transaction() {
    // The other half: two callers cannot both read `approved`. Forcing it from outside —
    // lock the batch row ourselves and show `apply` waits.
    //
    // What this pins, precisely: that `apply` cannot proceed at all while another session
    // holds the batch row. It does **not** distinguish the `FOR UPDATE` at the top from the
    // flip's `UPDATE` further down — both take the row lock, so removing either still
    // blocks here, and a test that claimed otherwise would be overclaiming. The property
    // that matters is the one asserted: no second writer interleaves with an apply.
    let (pool, user) = db_with_user("apply_row_lock").await;
    let handle = stage_and_approve(
        &pool,
        user,
        &new_portfolio(vec![asset("IWDA", vec![tx("BUY", "2024-03-01", "1.5", "82.34", "0")])]),
    )
    .await;
    let (id,): (i64,) =
        sqlx::query_as("SELECT id FROM agent_staged_batch WHERE handle = $1")
            .bind(&handle)
            .fetch_one(&pool)
            .await
            .unwrap();

    let mut conn = pool.acquire().await.unwrap();
    // `FOR UPDATE` outside a transaction is a statement that commits itself and releases
    // the lock immediately, so the transaction has to be opened by hand here.
    sqlx::query("BEGIN").execute(&mut *conn).await.unwrap();
    sqlx::query("SELECT id FROM agent_staged_batch WHERE id = $1 FOR UPDATE")
        .bind(id)
        .execute(&mut *conn)
        .await
        .unwrap();

    let outcome = tokio::time::timeout(
        std::time::Duration::from_millis(400),
        staging::apply(&pool, user, &handle),
    )
    .await
    .map_err(|_| ());
    assert!(
        outcome.is_err(),
        "apply did not wait for the batch row, so two callers can interleave with an apply"
    );

    drop(conn);
    staging::apply(&pool, user, &handle)
        .await
        .expect("apply proceeds once the row is released");
    assert_eq!(counts(&pool).await, (1, 1, 1));
}
