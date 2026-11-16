//! `owned_detail` — the live half of the preview, and the readback after a restore.
//!
//! The tests below the divider go over HTTP, because what they pin is the route that
//! hands this function to the panel: that it answers the caller and only the caller, that
//! it needs a session, that after a restore it agrees with what the preview promised, that
//! it reads the clock for itself rather than being handed `today`, and that it tells every
//! cache to keep nothing. The round-trip test is the one half-exception: it restores over
//! HTTP but reads back through `owned_detail` directly, because that is the call `preview`
//! makes.
//!
//! Runtime `sqlx::query` here, not `query!`: the compile-time macro lives in `src/`
//! only, so the cache does not have to know about test SQL.

mod common;

use backend_rust::backup::detail::{
    detail_from_bundle, flag, owned_detail, PlanPortfolio, MAX_ASSETS_SHOWN,
    MAX_PORTFOLIOS_SHOWN,
};
use backend_rust::backup::Bundle;
use chrono::NaiveDate;
use common::{
    build_rocket, build_rocket_oidc, db_with_user, seed_asset, seed_asset_currency,
    seed_portfolio, seed_transaction, seed_transaction_at, seed_user,
};
use rocket::http::Status;
use rocket::local::asynchronous::Client;
use serde_json::{Value, json};
use std::collections::BTreeMap;

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
    // Counted directly rather than asserted through the fixture's own return values:
    // `their_asset > 0 && theirs != mine` could not fail, since the asset id comes from
    // a sequence and two `seed_user` calls cannot hand back the same id.
    let theirs_rows = sqlx::query_as::<_, (i64, i64, i64)>(
        "SELECT (SELECT COUNT(*) FROM portfolios WHERE owner_id = $1),
                (SELECT COUNT(*) FROM assets a WHERE a.portfolio_id IN
                    (SELECT id FROM portfolios WHERE owner_id = $1)),
                (SELECT COUNT(*) FROM transactions t WHERE t.asset_id IN
                    (SELECT a2.id FROM assets a2 WHERE a2.portfolio_id IN
                        (SELECT id FROM portfolios WHERE owner_id = $1)))",
    )
    .bind(theirs)
    .fetch_one(&pool)
    .await
    .unwrap();
    // Exactly what the fixture seeded: one portfolio, one asset, one transaction.
    assert_eq!(theirs_rows, (1, 1, 1));
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
        // Zero-padded, because the detail query orders by symbol and `S0..S50` sorts
        // `S50` to position 47 of 51 — *inside* the 50 shown, leaving `S9` as the
        // excluded asset. The flag assertion below would then pass for an aggregation
        // folded over `shown`, which is the exact regression this module exists to
        // prevent. Padded, lexicographic order equals insertion order and the flagged
        // asset is the one past the cap.
        let aid = seed_asset(&pool, pid, &format!("S{i:03}"), "one").await;
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
    // The half that makes the line above mean "past the cap": none of the 50 listed
    // assets holds a zero-price row, so the count can only have come from the one that
    // is not listed. Mirrors the bundle-side test of the same name.
    assert!(p.shown.iter().all(|a| a.zero_price == 0));
}

/// `future_dated` on the SQL side, including its boundary: the comparison is `>` today,
/// not `>=`, so a transaction dated exactly today must not be flagged. The second row is
/// 23:59:59Z rather than midnight so the assertion also covers *truncating* to the day —
/// a comparison against a timestamp would call that row future-dated.
#[tokio::test]
async fn a_transaction_after_today_is_flagged_and_one_on_today_is_not() {
    let (pool, uid) = db_with_user("owned-detail-future").await;
    let pid = seed_portfolio(&pool, uid, "P", "USD").await;
    let later = seed_asset(&pool, pid, "LATER", "Later").await;
    let same_day = seed_asset(&pool, pid, "TODAY", "Today").await;
    seed_transaction_at(&pool, later, "BUY", 1.0, 1.0, 0.0, "2026-06-02T00:00:00Z").await;
    seed_transaction_at(&pool, same_day, "BUY", 1.0, 1.0, 0.0, "2026-06-01T23:59:59Z").await;

    let side = owned_detail(&pool, uid, today()).await.unwrap();
    let p = &side.shown[0];
    assert_eq!(p.asset_flags.get(flag::FUTURE_DATED), Some(&1));
    // Named by symbol, not by index: which of the two sorts first is the query's order,
    // not something this test should have to restate.
    let later_row = p.shown.iter().find(|a| a.symbol == "LATER").unwrap();
    let same_day_row = p.shown.iter().find(|a| a.symbol == "TODAY").unwrap();
    assert!(later_row.flags.contains(&flag::FUTURE_DATED.to_string()));
    assert!(!same_day_row.flags.contains(&flag::FUTURE_DATED.to_string()));
}

/// An asset with no transactions is a row the restore will create and the panel must
/// show as empty, so the flag has to come from the SQL side too — and its date span must
/// be absent rather than an empty string or a zero date.
#[tokio::test]
async fn an_asset_with_no_transactions_is_flagged_and_carries_no_dates() {
    let (pool, uid) = db_with_user("owned-detail-no-transactions").await;
    let pid = seed_portfolio(&pool, uid, "P", "USD").await;
    let empty = seed_asset(&pool, pid, "EMPTY", "Empty").await;
    let full = seed_asset(&pool, pid, "FULL", "Full").await;
    seed_transaction(&pool, full, "BUY", 1.0, 1.0, 0.0).await;

    let side = owned_detail(&pool, uid, today()).await.unwrap();
    let p = &side.shown[0];
    assert_eq!(p.asset_flags.get(flag::NO_TRANSACTIONS), Some(&1));
    assert_eq!(side.counts.transactions, 1);

    let empty_row = p.shown.iter().find(|a| a.symbol == "EMPTY").unwrap();
    assert_eq!(empty_row.transactions, 0);
    assert_eq!(empty_row.first_date, None);
    assert_eq!(empty_row.last_date, None);
    assert!(empty_row.flags.contains(&flag::NO_TRANSACTIONS.to_string()));
    // The matching half: an asset that does have rows must not carry the flag.
    let full_row = p.shown.iter().find(|a| a.symbol == "FULL").unwrap();
    assert!(!full_row.flags.contains(&flag::NO_TRANSACTIONS.to_string()));
    // And the fixture really left that asset empty, so the flag above is answering the
    // preview's own count rather than a seed that silently inserted a row.
    let rows =
        sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM transactions WHERE asset_id = $1")
            .bind(empty)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(rows.0, 0);
}

/// Three copies of one symbol set the flag **once**, and it is a portfolio-level flag —
/// the same shape the bundle side produces, and the thing Task 5's round-trip compares.
#[tokio::test]
async fn a_duplicated_symbol_sets_the_flag_once_and_never_in_asset_flags() {
    let (pool, uid) = db_with_user("owned-detail-duplicate").await;
    let pid = seed_portfolio(&pool, uid, "P", "USD").await;
    for name in ["one", "two", "three"] {
        let aid = seed_asset(&pool, pid, "DUP", name).await;
        seed_transaction(&pool, aid, "BUY", 1.0, 1.0, 0.0).await;
    }
    let unique = seed_asset(&pool, pid, "UNIQUE", "Unique").await;
    seed_transaction(&pool, unique, "BUY", 1.0, 1.0, 0.0).await;

    let side = owned_detail(&pool, uid, today()).await.unwrap();
    let p = &side.shown[0];
    assert_eq!(p.assets, 4);
    // `==` not `contains`: a flag pushed once per duplicate would produce three entries
    // and pass a `contains` assertion.
    assert_eq!(p.flags, vec![flag::DUPLICATE_SYMBOL.to_string()]);
    assert!(!p.asset_flags.contains_key(flag::DUPLICATE_SYMBOL));
}

/// Presence, not count: two symbols each duplicated twice reads exactly as one duplicated
/// symbol does, so the producers cannot disagree about a number neither of them
/// surfaces.
#[tokio::test]
async fn two_distinct_duplicated_symbols_still_report_the_flag_once() {
    let (pool, uid) = db_with_user("owned-detail-duplicate-two").await;
    let pid = seed_portfolio(&pool, uid, "P", "USD").await;
    for symbol in ["AAA", "BBB"] {
        for name in ["x", "y"] {
            let aid = seed_asset(&pool, pid, symbol, name).await;
            seed_transaction(&pool, aid, "BUY", 1.0, 1.0, 0.0).await;
        }
    }

    let side = owned_detail(&pool, uid, today()).await.unwrap();
    let p = &side.shown[0];
    assert_eq!(p.assets, 4);
    assert_eq!(p.flags, vec![flag::DUPLICATE_SYMBOL.to_string()]);
}

/// The date span on the SQL side, with one of the three dates bound as a `+02:00` instant
/// so the UTC day is literally the thing under test. `2026-06-02T01:00:00+02:00` is
/// `2026-06-01T23:00:00Z` once stored — a `TIMESTAMPTZ` keeps no offset — so the day the
/// database can name is 2026-06-01, and the bundle side now converts to the same day
/// rather than slicing the string as written. Asserting the flag too, because the same
/// day feeds `future_dated`.
#[tokio::test]
async fn the_date_span_names_the_utc_day_of_each_instant() {
    let (pool, uid) = db_with_user("owned-detail-date-span").await;
    let pid = seed_portfolio(&pool, uid, "P", "USD").await;
    let aid = seed_asset(&pool, pid, "SPAN", "Span").await;
    seed_transaction_at(&pool, aid, "BUY", 1.0, 1.0, 0.0, "2026-02-05T00:00:00Z").await;
    seed_transaction_at(&pool, aid, "BUY", 1.0, 1.0, 0.0, "2025-11-01T00:00:00Z").await;
    seed_transaction_at(&pool, aid, "BUY", 1.0, 1.0, 0.0, "2026-06-02T01:00:00+02:00").await;

    let side = owned_detail(&pool, uid, today()).await.unwrap();
    let a = &side.shown[0].shown[0];
    assert_eq!(a.transactions, 3);
    assert_eq!(a.first_date.as_deref(), Some("2025-11-01"));
    assert_eq!(a.last_date.as_deref(), Some("2026-06-01"));
    assert!(!a.flags.contains(&flag::FUTURE_DATED.to_string()));
    assert!(!side.shown[0].asset_flags.contains_key(flag::FUTURE_DATED));
}

/// The portfolio cap on this side, which is the same invariant as the asset cap arrived at
/// by a different query: 25 portfolios are listed, the 26th is counted and reported as
/// `more_portfolios`, and `transactions_in_shown` is the total held by the 25 listed —
/// not by all 26. The 26th holds three transactions while the others hold one each, so a
/// sum over every portfolio gives 28 where the answer is 25.
#[tokio::test]
async fn the_portfolio_cap_bounds_the_list_and_not_the_count() {
    let (pool, uid) = db_with_user("owned-detail-portfolio-cap").await;
    let mut held_by_shown = 0;
    for i in 0..26 {
        let pid = seed_portfolio(&pool, uid, &format!("P{i:02}"), "USD").await;
        let aid = seed_asset(&pool, pid, &format!("S{i:02}"), "one").await;
        let txs = if i == 25 { 3 } else { 1 };
        for _ in 0..txs {
            seed_transaction(&pool, aid, "BUY", 1.0, 1.0, 0.0).await;
        }
        // Portfolios are listed in id order and seeded in order, so the first 25 are the
        // ones the response must list.
        if i < MAX_PORTFOLIOS_SHOWN {
            held_by_shown += txs;
        }
    }

    let side = owned_detail(&pool, uid, today()).await.unwrap();
    assert_eq!(side.counts.portfolios, 26);
    assert_eq!(side.counts.assets, 26);
    assert_eq!(side.counts.transactions, 28);
    assert_eq!(side.shown.len(), MAX_PORTFOLIOS_SHOWN);
    assert_eq!(side.more_portfolios, 1);
    assert_eq!(side.transactions_in_shown, held_by_shown);
    assert_eq!(held_by_shown, 25);
}

// ---------------------------------------------------------------------------
// `GET /api/backup/owned` — the route that hands `owned_detail` to the panel.
// ---------------------------------------------------------------------------

/// The route takes no id, so the only thing scoping it is `owner_id` inside
/// `owned_detail`. Two users in one database — see the note on the isolation test in
/// Task 2 for why a second `db_with_user` would not do.
#[tokio::test]
async fn owned_route_returns_the_callers_side() {
    let (pool, uid) = db_with_user("owned-route-isolation").await;
    let theirs = seed_user(&pool, "owned-route-theirs").await;
    seed_portfolio(&pool, theirs, "Theirs", "USD").await;
    seed_portfolio(&pool, uid, "Mine", "USD").await;

    let client = Client::tracked(build_rocket(pool, uid)).await.unwrap();
    let res = client
        .get("/api/backup/owned")
        .header(common::session_header())
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let side: Value = res.into_json().await.unwrap();
    assert_eq!(side["counts"]["portfolios"], 1);
    assert_eq!(side["shown"][0]["name"], "Mine");
    // The leak this pins is a name in the body, so the assertion is on the name. The
    // other tenant's portfolio id is a free number off a sequence — asserting it is
    // positive could not fail, and would prove nothing about isolation. Drop the
    // `owner_id` filter from `owned_detail`'s portfolio aggregate and "Theirs" appears
    // in this body, which is the mutation this test exists to catch.
    let body = side.to_string();
    assert!(
        body.contains("Mine"),
        "the caller's own portfolio is missing from the readback: {body}"
    );
    assert!(
        !body.contains("Theirs"),
        "another tenant's portfolio leaked into the readback: {body}"
    );
}

#[tokio::test]
async fn owned_route_needs_a_session() {
    let (pool, _uid) = db_with_user("owned-route-anon").await;
    let client = Client::tracked(build_rocket_oidc(pool)).await.unwrap();
    let res = client.get("/api/backup/owned").dispatch().await;
    assert_eq!(res.status(), Status::Unauthorized);
}

/// A bundle built for the round-trip test alone, because the shared `bundle_with_size`
/// cannot produce a single flag: every asset is USD in a USD portfolio, every price is
/// `10.00`, every date is in 2024 and every symbol is unique. Restoring it and comparing
/// the two producers' `asset_flags` compared two empty maps, so `bundle_with_size` stays
/// untouched — `tests/restore_concurrency_test.rs` restores it and wants no flags either
/// — and this stands beside it.
///
/// Every asset-level flag code the module has appears in `Flags`, each on its own asset,
/// with `duplicate_symbol` at the portfolio level, so `1` is the only right count for any
/// of them and a producer that flags two assets where the other flags one cannot pass.
/// `Plain` carries none, which is what makes the counts in `Flags` per-portfolio rather
/// than global.
///
/// `PLUSZERO` is the deliberate oddity. `+0` is a price a bundle may really carry —
/// validation accepts it and `restore.rs` writes the string verbatim — and the rule both
/// producers share is a *text* match (`^-?0+(\.0+)?$` in SQL, `is_zero` on the bundle
/// side) that does **not** accept it. So this row is the exact drift the comparison
/// exists to catch: widen the SQL regex to `\+?-?0+` and the database side counts two
/// zero-price assets where the bundle side still counts one.
fn flagged_bundle() -> String {
    let tx = |price: &str, date: &str| {
        json!({ "type": "BUY", "quantity": "1.0", "price": price, "fee": "0.00",
                "date": date })
    };
    let asset = |r: &str, symbol: &str, name: &str, currency: &str, txs: Vec<Value>| {
        json!({ "ref": r, "symbol": symbol, "name": name, "asset_type": "STOCK",
                "sector": null, "currency": currency, "isin": null, "transactions": txs })
    };
    json!({
        "format": "portfolio-prism-backup",
        "schema_version": 1,
        "created_at": "2026-06-01T00:00:00Z",
        "portfolios": [
            {
                "ref": "p-flags", "name": "Flags", "description": null,
                "currency": "EUR", "base_currency": "EUR",
                "assets": [
                    // zero_price: the one spelling of zero the text rule accepts.
                    asset("a-zero", "ZERO", "Zero", "EUR",
                        vec![tx("0.00", "2024-01-15T00:00:00Z")]),
                    // Flagged by neither side, and here to prove it stays that way.
                    asset("a-plus", "PLUSZERO", "Plus zero", "EUR",
                        vec![tx("+0", "2024-01-15T00:00:00Z")]),
                    // duplicate_symbol: two assets, one symbol, one portfolio.
                    asset("a-dup-1", "DUP", "Dup one", "EUR",
                        vec![tx("2.00", "2024-01-16T00:00:00Z")]),
                    asset("a-dup-2", "DUP", "Dup two", "EUR",
                        vec![tx("3.00", "2024-01-17T00:00:00Z")]),
                    // currency_mismatch: this portfolio is EUR.
                    asset("a-fx", "FX", "Fx", "USD",
                        vec![tx("4.00", "2024-01-18T00:00:00Z")]),
                    // no_transactions.
                    asset("a-none", "NONE", "None", "EUR", vec![]),
                    // future_dated: one day past the `today()` both producers are given.
                    asset("a-soon", "SOON", "Soon", "EUR",
                        vec![tx("5.00", "2026-06-02T00:00:00Z")]),
                ]
            },
            {
                "ref": "p-plain", "name": "Plain", "description": null,
                "currency": "USD", "base_currency": "USD",
                "assets": [
                    asset("a-calm", "CALM", "Calm", "USD",
                        vec![tx("6.00", "2024-01-19T00:00:00Z")]),
                ]
            }
        ]
    })
    .to_string()
}

/// A portfolio's assets lined up by what identifies them here — symbol and name — with
/// the two numbers that come from the per-asset flag logic: how many of the asset's rows
/// read as a zero price, and the flags that conclusion produced.
///
/// The key is symbol **and** name because the SQL side orders assets by symbol while the
/// bundle side keeps the order they were written in, and `DUP` is one symbol held by two
/// different assets. Comparing `shown` positionally would compare `ZERO` against `DUP`.
fn per_asset(p: &PlanPortfolio) -> BTreeMap<(&str, &str), (usize, Vec<&str>)> {
    p.shown
        .iter()
        .map(|a| {
            (
                (a.symbol.as_str(), a.name.as_str()),
                (a.zero_price, a.flags.iter().map(String::as_str).collect()),
            )
        })
        .collect()
}

/// The claim the readback rests on: what the preview promised to insert is what the
/// database holds afterwards. If the two producers drift, this is where it shows.
#[tokio::test]
async fn after_a_restore_the_readback_matches_what_the_preview_promised() {
    let (pool, uid) = db_with_user("owned-route-roundtrip").await;
    let bundle = flagged_bundle();
    let parsed: Bundle = serde_json::from_str(&bundle).unwrap();
    let promised = detail_from_bundle(&parsed, today());

    let client = Client::tracked(build_rocket(pool.clone(), uid)).await.unwrap();
    let res = client
        .post("/api/backup/restore?mode=replace")
        .header(common::session_header())
        .header(rocket::http::ContentType::JSON)
        .body(bundle.clone())
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);

    let after = owned_detail(&pool, uid, today()).await.unwrap();
    assert_eq!(after.counts.portfolios, promised.counts.portfolios);
    assert_eq!(after.counts.assets, promised.counts.assets);
    assert_eq!(after.counts.transactions, promised.counts.transactions);
    assert_eq!(after.shown.len(), promised.shown.len());
    assert_eq!(after.shown[0].assets, promised.shown[0].assets);

    // The flags too, not just the numbers: two producers that agree on counts and
    // disagree on what a code means would pass every assertion above. Portfolios are
    // lined up by name, not index: the SQL side orders assets by symbol and the bundle
    // side keeps the order they were written in, so the names are the only safe key.
    for want in &promised.shown {
        let got = after
            .shown
            .iter()
            .find(|p| p.name == want.name)
            .unwrap_or_else(|| panic!("{} is missing from the readback", want.name));
        assert_eq!(
            got.asset_flags, want.asset_flags,
            "asset_flags differ for {}",
            want.name
        );
        assert_eq!(got.flags, want.flags, "flags differ for {}", want.name);
        // The per-asset flags and zero counts too. The map above is built from the flag
        // aggregate's `FILTER` clauses; the list each asset carries is built by
        // `asset_flags_from` from the *detail* query's counts, and the bundle side has
        // its own `is_zero` and its own pushes. That is four pieces of logic, not two, and
        // dropping the currency comparison from `asset_flags_from` moves nothing in the
        // map above — so this is the only place that can catch it.
        assert_eq!(
            per_asset(got),
            per_asset(want),
            "per-asset flags differ for {}",
            want.name
        );
    }

    // The equality above is only worth having if there is something in the maps to agree
    // about. Two empty maps are equal, and for as long as this test ran on
    // `bundle_with_size` — every asset USD in a USD portfolio, every price `10.00`, every
    // date in 2024, every symbol unique — that is all it checked: the producers "agreed"
    // by agreeing that nothing happened. Each code is named with its count so a producer
    // that stops flagging fails here, rather than in the equality above where the failure
    // message is two `{}`s.
    let want_flags = promised.shown.iter().find(|p| p.name == "Flags").unwrap();
    let got_flags = after.shown.iter().find(|p| p.name == "Flags").unwrap();
    assert!(
        !want_flags.asset_flags.is_empty() && !got_flags.asset_flags.is_empty(),
        "the fixture produced no flags on at least one side: bundle {:?}, database {:?}",
        want_flags.asset_flags,
        got_flags.asset_flags
    );
    for (code, n) in [
        (flag::ZERO_PRICE, 1usize),
        (flag::CURRENCY_MISMATCH, 1),
        (flag::NO_TRANSACTIONS, 1),
        (flag::FUTURE_DATED, 1),
    ] {
        assert_eq!(
            want_flags.asset_flags.get(code),
            Some(&n),
            "the bundle side never set {code}"
        );
        assert_eq!(
            got_flags.asset_flags.get(code),
            Some(&n),
            "the database side never set {code}"
        );
    }
    // `duplicate_symbol` is a portfolio fact, and the one code the loop above would
    // least like to lose: the SQL side counts *duplicated symbols*, not duplicated
    // assets, and this fixture has two assets sharing one symbol — a producer that
    // counted assets would report 2, and the flag is presence either way.
    assert_eq!(want_flags.flags, vec![flag::DUPLICATE_SYMBOL.to_string()]);
    assert_eq!(got_flags.flags, vec![flag::DUPLICATE_SYMBOL.to_string()]);
    // The clean half. Without a portfolio that has nothing to flag, the four codes above
    // could have come from a count taken across the whole caller rather than per
    // portfolio, and both producers could be wrong in the same direction.
    let want_plain = promised.shown.iter().find(|p| p.name == "Plain").unwrap();
    let got_plain = after.shown.iter().find(|p| p.name == "Plain").unwrap();
    assert!(want_plain.asset_flags.is_empty() && want_plain.flags.is_empty());
    assert!(
        got_plain.asset_flags.is_empty() && got_plain.flags.is_empty(),
        "the unflagged portfolio came back flagged: {:?} / {:?}",
        got_plain.asset_flags,
        got_plain.flags
    );
}

/// The handler reads the clock itself — `Utc::now().date_naive()` in
/// `api_routes/backup.rs` — while every other test in this file passes `today` in as a
/// parameter. Nothing else pins that reading, and a `today` one day off in the handler
/// would mis-flag `future_dated` for every user in production with this whole file
/// green. Over HTTP for that reason: the thing under test is the call the handler makes,
/// not the function it calls.
#[tokio::test]
async fn owned_route_reads_the_clock_for_itself() {
    let (pool, uid) = db_with_user("owned-route-clock").await;
    let pid = seed_portfolio(&pool, uid, "P", "USD").await;
    let tomorrow = seed_asset(&pool, pid, "TOMORROW", "Tomorrow").await;
    let same_day = seed_asset(&pool, pid, "TODAY", "Today").await;
    let now = chrono::Utc::now().date_naive();
    // Midday UTC on both rows, so the hour cannot move either one to another day. The
    // window this leaves is the one no test can close: if UTC midnight falls between this
    // line and the handler's own reading, TOMORROW becomes "today" and the first
    // assertion below fails. It is microseconds wide.
    seed_transaction_at(
        &pool,
        tomorrow,
        "BUY",
        1.0,
        1.0,
        0.0,
        &format!("{}T12:00:00Z", now.succ_opt().unwrap()),
    )
    .await;
    seed_transaction_at(
        &pool,
        same_day,
        "BUY",
        1.0,
        1.0,
        0.0,
        &format!("{now}T12:00:00Z"),
    )
    .await;

    let client = Client::tracked(build_rocket(pool, uid)).await.unwrap();
    let res = client
        .get("/api/backup/owned")
        .header(common::session_header())
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let side: Value = res.into_json().await.unwrap();

    // The portfolio header first, because that is what the panel renders: exactly one
    // future-dated asset, counted over every asset the caller owns.
    assert_eq!(side["shown"][0]["asset_flags"][flag::FUTURE_DATED], 1);
    // And which asset it was. TOMORROW is the half that catches a `today` that is too
    // late; TODAY not being in the list is the half that catches one that is too early.
    let flagged: Vec<&str> = side["shown"][0]["shown"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| {
            a["flags"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f.as_str() == Some(flag::FUTURE_DATED))
        })
        .map(|a| a["symbol"].as_str().unwrap())
        .collect();
    assert_eq!(flagged, vec!["TOMORROW"]);
}

/// The readback is one caller's live holdings, so a cache that kept it would answer the
/// next caller with someone else's portfolio. `export` sets the same header for the same
/// reason, and `tests/backup_test.rs::export_is_a_download_and_is_never_cached` pins it
/// there; this is the same claim on this route.
#[tokio::test]
async fn owned_route_says_the_response_is_not_cacheable() {
    let (pool, uid) = db_with_user("owned-route-no-store").await;
    let client = Client::tracked(build_rocket(pool, uid)).await.unwrap();
    let res = client
        .get("/api/backup/owned")
        .header(common::session_header())
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    assert_eq!(res.headers().get_one("Cache-Control"), Some("no-store"));
}
