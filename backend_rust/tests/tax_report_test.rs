//! The dated tax report: the payload's typing, the three reconciliation equalities and
//! the failure contract.
//!
//! Hermetic by construction, like `fx_pricing_test.rs`: every price and rate a test here
//! needs is seeded into `historical_prices`, and the pricing layer reads the database
//! before the provider. The two payload tests below need no price and no rate at all —
//! they never open a database; `mod common` arrived with the first test that seeds one
//! (Task 4), which is also what retires the dead-code warnings on its helpers.

mod common;

use backend_rust::api_routes::analytics::{
    build_tax_report, pricing_failure_status, MissingItems, ReportError, ReportWindow,
};
use backend_rust::models::{Asset, Portfolio, Transaction};
use backend_rust::schemas::{DatedLot, LotMatch, PriceSource, ReportRow, RowKind, SellBreakdown};
use backend_rust::services::currency_service::{CurrencyService, PricingError};
use chrono::{DateTime, NaiveDate, Utc};
use common::{build_rocket, disabled_config, seed_split_sync, setup_db, DEFAULT_USER};
use rocket::http::Status;
use rocket::local::asynchronous::Client;
use rust_decimal::Decimal;
use sqlx::sqlite::SqlitePool;
use std::str::FromStr;
use std::time::Duration;

fn day(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn ts(s: &str) -> DateTime<Utc> {
    day(s).and_hms_opt(0, 0, 0).unwrap().and_utc()
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn fixture_asset() -> Asset {
    Asset {
        id: 1,
        portfolio_id: 1,
        symbol: "AAPL".to_string(),
        name: "Apple".to_string(),
        asset_type: "STOCK".to_string(),
        sector: None,
        currency: "USD".to_string(),
        isin: None,
    }
}

fn fixture_sell(id: i32) -> Transaction {
    Transaction {
        id,
        asset_id: 1,
        r#type: "SELL".to_string(),
        quantity: "100.0".to_string(),
        price: "130.0".to_string(),
        fee: "0.0".to_string(),
        date: ts("2024-12-01"),
    }
}

/// A sell of 100 units against TWO buy lots (60 matched + 20 matched) with 20 units
/// unmatched. Two matches is the point: with a single match, `LotMatch.pnl`,
/// `.cost_basis` and `.proceeds` are numerically identical to the `SellBreakdown`'s
/// aggregates, so wiring `lot_pnl` from the breakdown instead of the match compiles
/// clean and passes every assertion — while over-reporting on any real multi-lot sell,
/// because every lot row would carry the whole sell's P&L. `lot_pnl` is the column the
/// CSV sums. The two sources now differ (1800 vs 2200) and a swap fails.
fn fixture_breakdown(sell_id: i32) -> SellBreakdown {
    SellBreakdown {
        sell_transaction_id: sell_id,
        sell_date: ts("2024-12-01"),
        sold_qty: dec("100.0"),
        matched_qty: dec("80.0"),
        unmatched_qty: dec("20.0"),
        proceeds: dec("10200.0"),
        cost_basis: dec("8000.0"),
        realized_pnl: dec("2200.0"),
        matches: vec![
            LotMatch {
                buy_transaction_id: 1,
                buy_date: ts("2024-01-01"),
                buy_price: dec("100.0"),
                unit_cost: dec("100.0"),
                matched_qty: dec("60.0"),
                lot_qty_before_match: dec("60.0"),
                cost_basis: dec("6000.0"),
                proceeds: dec("7800.0"),
                pnl: dec("1800.0"),
                holding_days: 335,
                term: None,
            },
            LotMatch {
                buy_transaction_id: 2,
                buy_date: ts("2024-02-01"),
                buy_price: dec("100.0"),
                unit_cost: dec("100.0"),
                matched_qty: dec("20.0"),
                lot_qty_before_match: dec("40.0"),
                cost_basis: dec("2000.0"),
                proceeds: dec("2400.0"),
                pnl: dec("400.0"),
                holding_days: 304,
                term: None,
            },
        ],
    }
}

#[test]
fn an_unmatched_row_serialises_its_money_cells_as_null_not_zero() {
    // The rule the whole payload obeys: a consumer has to be able to tell "not
    // applicable" from "zero". `UNMATCHED` is where every money cell is inapplicable,
    // which makes it the row kind where a default would be easiest and least visible.
    let asset = fixture_asset();
    let sell = fixture_sell(7);
    let breakdown = fixture_breakdown(7);
    let row = ReportRow::unmatched(&asset, None, None, Some(PriceSource::Database), &sell, &breakdown);
    let v = serde_json::to_value(&row).unwrap();

    assert_eq!(v["row_type"], RowKind::Unmatched.as_str());
    assert_eq!(v["sell_transaction_id"], 7);
    assert_eq!(v["unmatched_qty"], "20.0");
    assert!(v["lot_pnl"].is_null(), "money that does not exist is null, not \"0\"");
    assert!(v["realized_pnl"].is_null());
    assert!(v["proceeds"].is_null());
    assert!(v["cost_basis"].is_null(), "the sell's matched-only allocations stay null");
    assert!(v["lot_cost_basis"].is_null());
    assert!(v["lot_proceeds"].is_null());
    assert!(v["buy_transaction_id"].is_null());
    assert_eq!(v["price_source"], "database");
    // The as-entered sell facts are NOT money allocations: they are what the user typed,
    // and dropping them would make the row unlabelable.
    assert_eq!(v["sell_price"], "130.0");
    assert_eq!(v["sell_fee"], "0.0");
    // Asset identity is plumbed through `of` and was asserted by nothing.
    assert_eq!(v["asset_id"], 1);
    assert_eq!(v["symbol"], "AAPL");
    assert_eq!(v["asset_type"], "STOCK");
    assert_eq!(v["asset_currency"], "USD");
}

#[test]
fn a_sold_lot_row_carries_money_as_decimal_strings() {
    let asset = fixture_asset();
    let sell = fixture_sell(3);
    let breakdown = fixture_breakdown(3);
    let matched = &breakdown.matches[0];
    let row = ReportRow::sold_lot(
        &asset, Some(day("2024-06-30")), None, Some(PriceSource::Database),
        &sell, &breakdown, matched,
    );
    let v = serde_json::to_value(&row).unwrap();

    assert_eq!(v["row_type"], RowKind::SoldLot.as_str());
    // The two P&L figures come from DIFFERENT sources and the fixture makes them differ:
    // `lot_pnl` is this match's `LotMatch.pnl` (1800), `realized_pnl` is the whole sell's
    // matched total from `SellBreakdown` (2200). Wiring either from the other compiles
    // clean and would make every lot row of a multi-lot sell carry the entire sell's P&L.
    assert_eq!(v["lot_pnl"], "1800.0", "this match's pnl, not the sell's total");
    assert_eq!(v["realized_pnl"], "2200.0", "the sell's matched total, not this match's");
    assert_eq!(v["lot_cost_basis"], "6000.0");
    assert_eq!(v["lot_proceeds"], "7800.0");
    assert_eq!(v["cost_basis"], "8000.0", "sell-level matched-only figure");
    assert_eq!(v["proceeds"], "10200.0", "sell-level matched-only figure");
    assert_eq!(v["sell_price"], "130.0", "as entered, in the asset's currency");
    assert_eq!(v["unmatched_qty"], "20.0", "the sell's context repeats on every lot row");
    assert_eq!(v["sell_transaction_id"], 3, "from the sell, not the breakdown");
    assert!(v["market_value"].is_null(), "an open-lot cell on a sold-lot row");
    assert_eq!(v["price_date_used"], "2024-06-30");
    assert_eq!(v["symbol"], "AAPL");
    assert_eq!(v["asset_currency"], "USD");
}

/// `open_lot` is the one `ReportRow` constructor that reads a type with no `Serialize`
/// (`DatedLot`), so until Task 4 builds a real replay nothing else would notice a wrong
/// field mapping. `market_value` taken from the wrong source, or the two quantity fields
/// swapped, compiles clean and silently misreports an open position on a tax file.
/// Asserted by value, not by wire form: this is a mapping test, not a contract test.
#[test]
fn an_open_lot_row_maps_every_dated_lot_field_and_leaves_the_sell_side_null() {
    let lot = DatedLot {
        buy_transaction_id: 7,
        buy_date: ts("2024-01-01"),
        buy_price: dec("100.0"),
        original_qty: dec("120.0"),
        remaining_qty: dec("80.0"),
        unit_cost: dec("50.0"),
        market_value: dec("12500.0"),
        unrealized_gain_loss: dec("2500.0"),
        unrealized_roi: Some(dec("0.25")),
    };

    let row = ReportRow::open_lot(
        &fixture_asset(),
        Some(day("2024-06-30")),
        Some(day("2024-06-29")),
        Some(PriceSource::Database),
        &lot,
    );

    assert_eq!(row.row_type, RowKind::OpenLot);
    assert_eq!(row.buy_transaction_id, Some(7));
    assert_eq!(row.buy_date, Some(ts("2024-01-01")));
    assert_eq!(row.buy_price, Some(dec("100.0")));
    assert_eq!(row.unit_cost, Some(dec("50.0")));
    assert_eq!(row.original_qty, Some(dec("120.0")));
    assert_eq!(
        row.remaining_qty,
        Some(dec("80.0")),
        "the two quantity fields must not land in each other's column"
    );
    assert_eq!(row.market_value, Some(dec("12500.0")));
    assert_eq!(row.unrealized_gain_loss, Some(dec("2500.0")));
    assert_eq!(row.unrealized_roi, Some(dec("0.25")));
    assert_eq!(row.price_date_used, Some(day("2024-06-30")));
    assert_eq!(row.fx_rate_date_used, Some(day("2024-06-29")));

    // An open lot has no sell. Every sell-side cell is null, not zero — a zero would read
    // as a realized figure of nothing rather than the absence of a sale.
    assert_eq!(row.sell_transaction_id, None);
    assert_eq!(row.sell_date, None);
    assert_eq!(row.sold_qty, None);
    assert_eq!(row.sell_price, None);
    assert_eq!(row.sell_fee, None);
    assert_eq!(row.proceeds, None);
    assert_eq!(row.cost_basis, None);
    assert_eq!(row.realized_pnl, None);
    // An open lot has no holding period and no HYBRID term bucket claimed for it.
    assert_eq!(row.holding_days, None);
    assert_eq!(row.term, None);
}

// --------------------------------------------------------------------------------------
// The dated route: the window, the reconciliation equalities and the failure contract.
// --------------------------------------------------------------------------------------

/// One portfolio, two assets and four transactions, with a close seeded for each asset
/// up to `through`. The window end a test uses must be no later than `through`, or the
/// pricing layer stops answering from the database.
///
/// SOLDIT: buys 60 @ 100 (Jan) and 40 @ 130 (Mar), sells 100 @ 130 (May) and 10 @ 140
/// (late June) — so the May sell drains both lots (60 * 30 + 40 * 0 = 1800 realized) and
/// the 10 June units have no lot left to match, which is the row kind that carries nulls.
/// HELDIT: one buy of 50 @ 200 (Feb), which is the open lot.
///
/// *** DEVIATION FROM THE BRIEF, RECORDED: the brief's buy was 40 @ 120. *** With 120 the
/// fixture is 100 bought against 90 sold, so the June sell is fully matched (10 of lot2's
/// remaining 20), nothing is unmatched, `realized_in_window` is 200 and
/// `realized_cumulative` is 2200 — every one of the money assertions in
/// `the_window_selects_the_sells_but_not_the_lots_behind_them` is then false while the
/// fixture's own comment ("10 units are unmatched") is also false. The assertions encode
/// the test's point (a sell outside the window still counts in the cumulative total, and
/// an unmatched tail realizes nothing and is a row of its own), so the *fixture* is what
/// moved: 40 @ 130 puts the May sell's realized at exactly 1800 and leaves the June sell
/// with nothing to match. See the task-4 report, defects §1.
async fn fixture(pool: &SqlitePool, through: &str) -> (i32, i32, i32) {
    let port_id = common::seed_portfolio(pool, DEFAULT_USER, "Dated", "USD").await;
    let sold = common::seed_asset(pool, port_id, "SOLDIT", "Sold it").await;
    let held = common::seed_asset(pool, port_id, "HELDIT", "Held it").await;

    common::seed_transaction_at(pool, sold, "BUY", 60.0, 100.0, 0.0, "2024-01-01T00:00:00Z").await;
    common::seed_transaction_at(pool, sold, "BUY", 40.0, 130.0, 0.0, "2024-03-01T00:00:00Z").await;
    common::seed_transaction_at(pool, sold, "SELL", 100.0, 130.0, 0.0, "2024-05-01T00:00:00Z").await;
    common::seed_transaction_at(pool, sold, "SELL", 10.0, 140.0, 0.0, "2024-06-25T00:00:00Z").await;
    common::seed_transaction_at(pool, held, "BUY", 50.0, 200.0, 0.0, "2024-02-01T00:00:00Z").await;

    common::seed_daily_prices(pool, "SOLDIT", "2024-01-01", through, "110.0").await;
    common::seed_daily_prices(pool, "HELDIT", "2024-02-01", through, "220.0").await;
    // No split is seeded, so this only has to keep `sync_splits_for_asset` on its cache
    // path for the undated call.
    seed_split_sync(pool, "SOLDIT").await;
    seed_split_sync(pool, "HELDIT").await;

    (port_id, sold, held)
}

fn dec_at(v: &serde_json::Value, path: &str) -> Decimal {
    Decimal::from_str(v.pointer(path).and_then(|p| p.as_str()).unwrap_or_else(|| panic!("{path} missing"))).unwrap()
}

/// The loaded rows `build_tax_report` takes, read back through SQLx: two three-line
/// helpers, because that function takes the rows rather than the ids.
async fn load_fixture_portfolio(pool: &SqlitePool, id: i32) -> Portfolio {
    sqlx::query_as::<_, Portfolio>("SELECT * FROM portfolios WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn load_fixture_assets(pool: &SqlitePool, portfolio_id: i32) -> Vec<Asset> {
    sqlx::query_as::<_, Asset>("SELECT * FROM assets WHERE portfolio_id = ?")
        .bind(portfolio_id)
        .fetch_all(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_dated_report_sums_its_own_rows_and_agrees_with_the_undated_summary() {
    // Goal 1, all three equalities, under all three strategies:
    // (a) the sold-lot rows' `lot_pnl` sum to the asset's `realized_in_window`;
    // (b) with `from` at the portfolio's first transaction, that sum is also the asset's
    //     `realized_pnl` on the undated endpoint, which is fully driven here because the
    //     window is the whole history and both calls price from the same seeded rows;
    // (c) the open-lot rows' `unrealized_gain_loss` sums to `unrealized_at_end`.
    let pool = setup_db(&disabled_config()).await;
    let through = Utc::now().date_naive().to_string();
    let (port_id, _, _) = fixture(&pool, &through).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();

    for strategy in ["FIFO", "LIFO", "HYBRID"] {
        let dated = client
            .get(format!(
                "/api/portfolios/{port_id}/tax-summary?strategy={strategy}&threshold_days=30&from=2024-01-01&to={through}"
            ))
            .dispatch()
            .await;
        assert_eq!(dated.status(), Status::Ok, "{strategy}");
        let report: serde_json::Value = serde_json::from_str(&dated.into_string().await.unwrap()).unwrap();

        for totals in report["asset_totals"].as_array().unwrap() {
            let symbol = totals["symbol"].as_str().unwrap();
            let rows: Vec<&serde_json::Value> = report["rows"].as_array().unwrap().iter()
                .filter(|r| r["symbol"] == symbol)
                .collect();

            let sold: Decimal = rows.iter()
                .filter(|r| r["row_type"] == "SOLD_LOT")
                .map(|r| dec_at(r, "/lot_pnl"))
                .sum();
            assert_eq!(sold, dec_at(totals, "/realized_in_window"), "{strategy}/{symbol}: rows must sum to the server total");

            let open: Decimal = rows.iter()
                .filter(|r| r["row_type"] == "OPEN_LOT")
                .map(|r| dec_at(r, "/unrealized_gain_loss"))
                .sum();
            assert_eq!(open, dec_at(totals, "/unrealized_at_end"), "{strategy}/{symbol}");
        }

        // The rollup is the server's, and the rows must not be the only source of it.
        let rollup: Decimal = report["asset_totals"].as_array().unwrap().iter()
            .map(|t| dec_at(t, "/realized_in_window"))
            .sum();
        assert_eq!(rollup, dec_at(&report, "/totals/realized_in_window"), "{strategy}");

        let undated = client
            .get(format!("/api/portfolios/{port_id}/tax-summary?strategy={strategy}&threshold_days=30"))
            .dispatch()
            .await;
        let summary: serde_json::Value = serde_json::from_str(&undated.into_string().await.unwrap()).unwrap();
        for asset in summary["assets"].as_array().unwrap() {
            let symbol = asset["symbol"].as_str().unwrap();
            let dated_asset = report["asset_totals"].as_array().unwrap().iter()
                .find(|t| t["symbol"] == symbol)
                .unwrap();
            assert_eq!(
                dec_at(dated_asset, "/realized_cumulative"),
                dec_at(asset, "/realized_pnl"),
                "{strategy}/{symbol}: cumulative realized must equal what the Holdings tab shows",
            );
        }
    }
}

#[tokio::test]
async fn an_undated_call_still_returns_the_shape_the_statement_reads() {
    // Review Focus 5. The printable statement and `PortfolioDetail` read `assets[].tax_lots`,
    // `current_shares` and `total_portfolio_value`, none of which the typed report carries.
    // If this test fails because `context` appears and `assets` does not, the typed payload
    // has replaced the legacy response and both shipped screens are broken.
    let pool = setup_db(&disabled_config()).await;
    let through = Utc::now().date_naive().to_string();
    let (port_id, _, _) = fixture(&pool, &through).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();

    let resp = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?strategy=FIFO&threshold_days=30"))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Ok);
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();

    assert!(body["assets"].is_array());
    assert!(body["assets"][0]["tax_lots"].is_array());
    assert!(body["assets"][0]["current_shares"].is_string());
    assert!(body["total_portfolio_value"].is_string());
    assert_eq!(body["strategy"], "FIFO");
    assert!(body.get("rows").is_none(), "the legacy shape has no rows");
    assert!(body.get("context").is_none(), "and no run context");
}

/// The undated path's response *shape* is unchanged, but its `strategy` **value** is not,
/// and this is the one place Task 4 silently altered live behaviour. Before Task 4 the route
/// did `strategy.unwrap_or_else(|| "FIFO")` and passed the string through verbatim, so
/// `?strategy=lifo` reached the engine's `_ =>` arm (`tax_engine.rs:722`), computed **FIFO**,
/// and echoed back `"lifo"` — a number labelled with a method that produced it. Task 4 routes
/// the undated path through `normalize_strategy`, so the same request now computes LIFO and
/// echoes `"LIFO"`. That is the truthful reading and it is what the dated path already did,
/// so it is kept — but it is a behaviour change, not a preservation, and the shipped UI only
/// ever sends the three uppercase tokens (`PortfolioDetail.tsx:179-181`), which is why no
/// other test noticed. Pinned here so the next person does not rediscover it.
#[tokio::test]
async fn an_undated_call_normalizes_the_strategy_it_echoes() {
    let pool = setup_db(&disabled_config()).await;
    let through = Utc::now().date_naive().to_string();
    let (port_id, _, _) = fixture(&pool, &through).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();

    let resp = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?strategy=lifo"))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Ok);
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();

    assert_eq!(
        body["strategy"], "LIFO",
        "lowercase is normalized, not echoed verbatim as it was before Task 4"
    );
    // An unrecognised token still collapses to FIFO rather than erroring: permissive is the
    // dated path's opposite, and this is decision D6.
    let resp = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?strategy=nonsense"))
        .dispatch()
        .await;
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
    assert_eq!(body["strategy"], "FIFO");
}

#[tokio::test]
async fn the_undated_path_still_names_the_strategy_the_engine_used() {
    // The spec's file 6 fix: the response must name the matching method the engine actually
    // used. It does — by normalizing, not by echoing. `?strategy=average_cost` falls through to
    // FIFO in the engine (`tax_engine.rs:722`) AND now reports `FIFO`, because the undated path
    // goes through `normalize_strategy` (`analytics.rs:678-684`). Before Task 4 it echoed
    // `average_cost` verbatim while the engine ran FIFO, which is the bug this test exists to
    // keep fixed. See `an_undated_call_normalizes_the_strategy_it_echoes` for the lowercase case.
    let pool = setup_db(&disabled_config()).await;
    let through = Utc::now().date_naive().to_string();
    let (port_id, _, _) = fixture(&pool, &through).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();

    let resp = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?strategy=average_cost&threshold_days=30"))
        .dispatch()
        .await;
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
    assert_eq!(body["strategy"], "FIFO", "the response must not name a method the engine did not use");
}

#[tokio::test]
async fn a_window_end_on_a_non_trading_day_reports_the_close_it_used() {
    // 2024-06-30 is a Sunday and the fixture stops on the Friday before it (2 days, inside
    // `PRICE_WINDOW_FRESH_DAYS = 4`), so the database answers and the file states which
    // close priced the position. Silent substitution is the failure Goal 5 names.
    let pool = setup_db(&disabled_config()).await;
    let (port_id, _, _) = fixture(&pool, "2024-06-28").await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();

    let resp = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?strategy=FIFO&threshold_days=30&to=2024-06-30"))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Ok);
    let report: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();

    assert_eq!(report["context"]["requested_to"], "2024-06-30");
    assert_eq!(
        report["context"]["requested_from"], "2024-01-01",
        "`to` alone is cumulative from the portfolio's first transaction",
    );
    for totals in report["asset_totals"].as_array().unwrap() {
        assert_eq!(totals["price_date_used"], "2024-06-28", "{}", totals["symbol"]);
        assert_eq!(totals["price_source"], "database");
    }
    for row in report["rows"].as_array().unwrap() {
        assert_eq!(row["price_date_used"], "2024-06-28", "one close per asset, repeated on its rows");
    }
}

#[tokio::test]
async fn the_window_selects_the_sells_but_not_the_lots_behind_them() {
    // A sell dated before `from` is out of the window's rows and out of
    // `realized_in_window`, and still part of `realized_cumulative` — and the buy lots it
    // consumed still show up as open lots (or not) at `end`. Getting this wrong changes a
    // filed quarter.
    let pool = setup_db(&disabled_config()).await;
    let (port_id, _, _) = fixture(&pool, "2024-06-28").await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();

    let resp = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?strategy=FIFO&threshold_days=30&from=2024-06-01&to=2024-06-30"))
        .dispatch()
        .await;
    let report: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();

    assert!(report["rows"].as_array().unwrap().iter().all(|r| r["sell_date"].is_null() || r["sell_date"].as_str().unwrap() >= "2024-06-01"));
    let soldit = report["asset_totals"].as_array().unwrap().iter()
        .find(|t| t["symbol"] == "SOLDIT").unwrap();
    // Only the 10 June units are in the window, and all 10 of them are unmatched: FIFO
    // takes the oldest lot first, and the May sell already consumed both lots.
    assert_eq!(dec_at(soldit, "/realized_in_window"), dec("0.0"), "an unmatched tail realizes nothing");
    assert_eq!(dec_at(soldit, "/realized_cumulative"), dec("1800.0") + dec("-0.0") + dec("0.0"));
    let unmatched = report["rows"].as_array().unwrap().iter()
        .filter(|r| r["row_type"] == "UNMATCHED")
        .collect::<Vec<_>>();
    assert_eq!(unmatched.len(), 1, "the June sell's unmatched tail is a row of its own");
    assert!(unmatched[0]["realized_pnl"].is_null());
    assert_eq!(unmatched[0]["unmatched_qty"], "10.0");
}

/// The validation failures, each of which names the parameter and carries two empty
/// lists. They are all 422 with `retryable: false`: the client typed something the
/// server cannot act on, and retrying changes nothing.
#[tokio::test]
async fn invalid_windows_are_422s_that_name_the_parameter() {
    let pool = setup_db(&disabled_config()).await;
    let (port_id, _, _) = fixture(&pool, Utc::now().date_naive().to_string().as_str()).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();

    let today = Utc::now().date_naive();
    let tomorrow = (today + chrono::Duration::days(1)).to_string();
    let cases: Vec<(&str, String, &str)> = vec![
        ("from after to", "from=2024-06-30&to=2024-06-01".to_string(), "from"),
        ("future to", format!("to={tomorrow}"), "to"),
        ("future from", format!("from={tomorrow}"), "from"),
        ("unparseable to", "to=31-06-2024".to_string(), "to"),
        ("unparseable from", "from=not-a-date".to_string(), "from"),
        ("unknown strategy", "to=2024-06-30&strategy=average_cost".to_string(), "strategy"),
        ("negative threshold", "to=2024-06-30&threshold_days=-1".to_string(), "threshold_days"),
    ];

    for (label, query, param) in cases {
        let resp = client
            .get(format!("/api/portfolios/{port_id}/tax-summary?{query}"))
            .dispatch()
            .await;
        assert_eq!(resp.status(), Status::UnprocessableEntity, "{label}");
        let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
        assert!(
            body["error"].as_str().unwrap().contains(param),
            "{label}: the body must name the offending parameter, got {}",
            body["error"],
        );
        assert_eq!(body["missing_prices"], serde_json::json!([]), "{label}");
        assert_eq!(body["missing_rates"], serde_json::json!([]), "{label}");
        assert_eq!(body["retryable"], false, "{label}");
    }
}

/// `?from=<tomorrow>` with no `to` must blame `from` and say *why*. The original ordering tested
/// `from > to` first, and an absent `to` defaults to today, so that fired first and answered
/// "must not be after `to`" — naming a parameter the client never sent and sending it looking
/// for one it did not supply. The status was 422 either way, which is why the table above cannot
/// catch it; only the detail can.
#[tokio::test]
async fn a_future_from_alone_blames_from_and_says_future() {
    let pool = setup_db(&disabled_config()).await;
    let (port_id, _, _) = fixture(&pool, Utc::now().date_naive().to_string().as_str()).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();
    let tomorrow = (Utc::now().date_naive() + chrono::Duration::days(1)).to_string();

    let resp = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?from={tomorrow}"))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::UnprocessableEntity);
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
    let err = body["error"].as_str().unwrap();
    assert!(
        err.contains("future"),
        "must say the date is in the future, not invent a `to` the client never sent: {err}"
    );
    assert!(
        !err.contains("after"),
        "must not blame a comparison against a defaulted `to`: {err}"
    );
}

/// `from` alone means "through today" — a settled value. Every other valid dated test passes
/// `to`, so the one-sided branch was only ever exercised in its invalid form.
#[tokio::test]
async fn a_from_alone_runs_through_today() {
    let pool = setup_db(&disabled_config()).await;
    let through = Utc::now().date_naive().to_string();
    let (port_id, _, _) = fixture(&pool, &through).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();

    let resp = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?from=2024-01-01"))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Ok);
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();

    assert_eq!(body["context"]["requested_from"], "2024-01-01");
    assert_eq!(
        body["context"]["requested_to"], through,
        "an absent `to` is today, not an error and not the first transaction date"
    );
    assert!(body["rows"].as_array().unwrap().iter().count() > 0);
}

#[tokio::test]
async fn a_missing_portfolio_is_404_and_an_unknown_strategy_on_the_undated_path_is_not_an_error() {
    let pool = setup_db(&disabled_config()).await;
    // Today, not a past date: the undated call below prices through `get_price`, which
    // answers only from a row dated exactly today and otherwise asks the provider. A
    // `through` of 2024-06-28 would make this test reach the network.
    let (port_id, _, _) = fixture(&pool, Utc::now().date_naive().to_string().as_str()).await;
    let client = Client::tracked(build_rocket(pool, DEFAULT_USER)).await.unwrap();

    let missing = client.get("/api/portfolios/9999/tax-summary?strategy=FIFO").dispatch().await;
    assert_eq!(missing.status(), Status::NotFound);

    // Dated-only strictness (decision D6): the same unknown value that is a 422 with a
    // window is a 200 without one.
    let undated = client
        .get(format!("/api/portfolios/{port_id}/tax-summary?strategy=average_cost"))
        .dispatch()
        .await;
    assert_eq!(undated.status(), Status::Ok);
}

#[test]
fn the_missing_item_lists_are_capped_and_the_totals_say_so() {
    // The cap is testable without a provider because it is applied when the collected
    // failures render, not when they are found. See Step 9 for why the collection itself
    // has no hermetic test.
    let mut missing = MissingItems::new();
    for i in 0..21 {
        missing.add_price(&format!("SYM{i}"));
    }
    for i in 0..11 {
        missing.add_rate(&format!("C{i}"), "USD");
    }
    // Duplicates are not items: the list names symbols and pairs, once each.
    missing.add_price("SYM0");
    missing.add_rate("C0", "USD");

    let body = missing.into_error_body("no price or rate exists at or before the requested date");
    assert_eq!(body.missing_prices.len(), 20);
    assert_eq!(body.missing_prices_total, 21);
    assert_eq!(body.missing_rates.len(), 10);
    assert_eq!(body.missing_rates_total, 11);
    assert!(!body.retryable);
}

#[test]
fn a_missing_rate_is_named_as_a_pair_not_a_symbol() {
    let mut missing = MissingItems::new();
    missing.add_rate("EUR", "USD");
    let body = missing.into_error_body("no price or rate exists at or before the requested date");
    assert_eq!(body.missing_rates[0].from, "EUR");
    assert_eq!(body.missing_rates[0].to, "USD");
    assert!(body.missing_prices.is_empty());
}

#[test]
fn the_two_failure_classes_carry_the_right_status_and_retryable_flag() {
    // The contract, pinned on the constructors rather than through a provider.
    let no_data = ReportError::no_data(MissingItems::new());
    assert_eq!(no_data.status, Status::UnprocessableEntity);
    assert!(!no_data.body.retryable);

    let provider = ReportError::provider("timeout".to_string());
    assert_eq!(provider.status, Status::ServiceUnavailable);
    assert!(provider.body.retryable);

    let deadline = ReportError::deadline();
    assert_eq!(deadline.status, Status::ServiceUnavailable);
    assert!(deadline.body.retryable);

    // `PricingError`'s two variants mean different user actions, and the report's mapping
    // is the place that difference becomes a status code.
    assert_eq!(pricing_failure_status(&PricingError::NoData), Status::UnprocessableEntity);
    assert_eq!(
        pricing_failure_status(&PricingError::ProviderUnavailable("timeout".to_string())),
        Status::ServiceUnavailable,
    );
}

#[tokio::test]
async fn the_deadline_fails_closed_instead_of_returning_a_partial_report() {
    // The one hermetic route to a 503: the check runs before the first pricing step, so
    // `Duration::ZERO` trips it with no outbound call at all. A test that waited 20 s
    // would not be worth having.
    let pool = setup_db(&disabled_config()).await;
    let (port_id, _, _) = fixture(&pool, "2024-06-28").await;
    let portfolio = load_fixture_portfolio(&pool, port_id).await;
    let assets = load_fixture_assets(&pool, port_id).await;
    let svc = CurrencyService::new();

    let err = build_tax_report(
        &pool, &svc, &portfolio, &assets, "FIFO", 30,
        &ReportWindow { from: None, to: day("2024-06-30") },
        Duration::ZERO,
    )
    .await
    .expect_err("a zero deadline must not produce a report");

    assert_eq!(err.status, Status::ServiceUnavailable);
    assert!(err.body.retryable);
    assert!(err.body.missing_prices.is_empty(), "no partial payload, and no missing items either");
}

// --------------------------------------------------------------------------------------
// What cannot be tested here, and why (Task 4, Step 9)
// --------------------------------------------------------------------------------------
//
// Two cases the spec asks for have no hermetic test, and this file does not pretend
// otherwise. Both are the same missing seam; the compensating tests are above.
//
// 1. "A missing price returns 422 naming every missing symbol." A seeded database cannot
//    produce `Err(NoData)`: `get_historical_prices` answers from `historical_prices` when
//    the window's newest row is within `PRICE_WINDOW_FRESH_DAYS` of the requested date,
//    and otherwise asks the provider (`currency_service.rs:713-717`). There is no third
//    path, so "no data" is only ever the provider's answer — and the provider is a
//    hardcoded Yahoo URL (`CurrencyService::new()` takes no arguments), so a test that
//    wants it reaches the network. Pinned instead: the *shape* of the 422
//    (`missing_prices`, `missing_rates`, both totals, `retryable: false`) through the
//    validation failures, which share the body; the caps and the pair-not-symbol rule
//    through `MissingItems`; and `pricing_failure_status` for the mapping itself.
// 2. "An unreachable provider returns 503 with `retryable: true`." Same seam, and the
//    deadline test is the hermetic path to that body:
//    `the_deadline_fails_closed_instead_of_returning_a_partial_report` asserts the
//    status, `retryable`, and that no partial payload comes back.
//
// If a future change adds a provider seam, both become writable and should be added;
// until then a test asserting them would either hit the network or pass for the wrong
// reason.
