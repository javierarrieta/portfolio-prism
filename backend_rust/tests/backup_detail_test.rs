//! `owned_detail` — the live half of the preview, and the readback after a restore.
//!
//! Runtime `sqlx::query` here, not `query!`: the compile-time macro lives in `src/`
//! only, so the cache does not have to know about test SQL.

mod common;

use backend_rust::backup::detail::{flag, owned_detail, MAX_ASSETS_SHOWN};
use chrono::NaiveDate;
use common::{db_with_user, seed_asset, seed_asset_currency, seed_portfolio, seed_transaction, seed_user};

/// A function, not a `const` — `Option::unwrap` is not a `const fn` (see Task 1).
fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
}

#[tokio::test]
async fn owned_detail_describes_what_the_caller_owns() {
    let (pool, uid) = db_with_user("owned-detail-basic").await;
    let pid = seed_portfolio(&pool, uid, "Alpha", "EUR").await;
    // `seed_asset` hardcodes the asset currency to `'USD'`, so reaching for it here
    // would make **both** assets differ from a EUR portfolio and the mismatch count
    // would be 2, not the 1 asserted below. Seeding AAA in the portfolio's own currency
    // is what makes "BBB is the one whose currency differs" true — and it is the
    // stronger assertion, because a matching asset must not be counted.
    let a1 = seed_asset_currency(&pool, pid, "AAA", "Alpha One", "EUR").await;
    let a2 = seed_asset_currency(&pool, pid, "BBB", "Beta", "USD").await;
    seed_transaction(&pool, a1, "BUY", 1.0, 10.0, 0.0).await;
    seed_transaction(&pool, a1, "BUY", 2.0, 0.0, 0.0).await;
    seed_transaction(&pool, a2, "SELL", 1.0, 5.0, 0.0).await;

    let side = owned_detail(&pool, uid, today()).await.unwrap();

    assert_eq!(side.counts.portfolios, 1);
    assert_eq!(side.counts.assets, 2);
    assert_eq!(side.counts.transactions, 3);
    assert_eq!(side.shown[0].name, "Alpha");
    assert_eq!(side.shown[0].currency, "EUR");
    assert_eq!(side.shown[0].transactions, 3);
    // AAA holds the zero-price row; BBB is the one whose currency differs.
    assert_eq!(side.shown[0].asset_flags.get(flag::ZERO_PRICE), Some(&1));
    assert_eq!(side.shown[0].asset_flags.get(flag::CURRENCY_MISMATCH), Some(&1));
    assert_eq!(side.shown[0].shown[0].zero_price, 1);
}

#[tokio::test]
async fn an_empty_portfolio_is_flagged_not_missing() {
    let (pool, uid) = db_with_user("owned-detail-empty").await;
    seed_portfolio(&pool, uid, "Hollow", "USD").await;

    let side = owned_detail(&pool, uid, today()).await.unwrap();
    assert_eq!(side.counts.assets, 0);
    assert_eq!(side.shown[0].flags, vec![flag::NO_ASSETS.to_string()]);
}

/// The only thing between two tenants is the `owner_id` filter, and a filter that is
/// missing still answers 200.
///
/// **Both users live in one database.** `test_pool` names each database
/// `prism_t_<pid>_<caller>`, so a second `db_with_user` would be a second *database*
/// and this test would assert nothing about isolation at all. One pool, two users.
#[tokio::test]
async fn owned_detail_returns_only_the_callers_rows() {
    let (pool, mine) = db_with_user("owned-detail-isolation").await;
    let theirs = seed_user(&pool, "owned-detail-theirs").await;

    let their_pid = seed_portfolio(&pool, theirs, "Theirs", "USD").await;
    let their_asset = seed_asset(&pool, their_pid, "ZZZ", "Theirs One").await;
    seed_transaction(&pool, their_asset, "BUY", 1.0, 1.0, 0.0).await;
    seed_portfolio(&pool, mine, "Mine", "USD").await;

    let side = owned_detail(&pool, mine, today()).await.unwrap();
    assert_eq!(side.counts.portfolios, 1);
    assert_eq!(side.counts.transactions, 0);
    assert_eq!(side.shown[0].name, "Mine");
    // The other rows are there — same pool, same tables — and none of them are here.
    assert!(their_asset > 0 && theirs != mine);
}

/// One unparseable price must not turn every preview and every readback for that user
/// into a 500. The zero-price test is a text match (`^-?0+(\.0+)?$`) rather than
/// `price::numeric = 0` for exactly this reason: Postgres does not promise to evaluate
/// a guard before a cast, so a cast could reach the bad row anyway. This test is what
/// stops someone "simplifying" the regex back into a cast.
#[tokio::test]
async fn a_price_that_is_not_a_number_does_not_break_the_read() {
    let (pool, uid) = db_with_user("owned-detail-bad-price").await;
    let pid = seed_portfolio(&pool, uid, "Odd", "USD").await;
    let aid = seed_asset(&pool, pid, "ODD", "Odd One").await;
    // Not reachable through the app — validation owns that — but a legacy row or a
    // hand-edited database is not the app's problem to survive.
    sqlx::query(
        "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) \
         VALUES ($1, 'BUY', '1', 'not-a-number', '0', now())",
    )
    .bind(aid)
    .execute(&pool)
    .await
    .unwrap();

    let side = owned_detail(&pool, uid, today()).await;
    assert!(side.is_ok(), "one bad price row must not fail the whole read");
    assert_eq!(side.unwrap().counts.transactions, 1);
}

/// Same invariant as the bundle side, arrived at by SQL: the aggregate must cover
/// assets the detail query never returns.
#[tokio::test]
async fn asset_flags_cover_assets_past_the_cap() {
    let (pool, uid) = db_with_user("owned-detail-cap").await;
    let pid = seed_portfolio(&pool, uid, "Wide", "USD").await;
    for i in 0..51 {
        let aid = seed_asset(&pool, pid, &format!("S{i}"), "one").await;
        // The 51st asset holds the only zero-price row.
        let price = if i == 50 { 0.0 } else { 10.0 };
        seed_transaction(&pool, aid, "BUY", 1.0, price, 0.0).await;
    }

    let side = owned_detail(&pool, uid, today()).await.unwrap();
    let p = &side.shown[0];
    assert_eq!(p.assets, 51);
    assert_eq!(p.shown.len(), MAX_ASSETS_SHOWN);
    assert_eq!(p.more_assets, 1);
    assert_eq!(p.asset_flags.get(flag::ZERO_PRICE), Some(&1));
}
