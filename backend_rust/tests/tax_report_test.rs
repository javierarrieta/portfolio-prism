//! The dated tax report: the payload's typing, the three reconciliation equalities and
//! the failure contract.
//!
//! Hermetic by construction, like `fx_pricing_test.rs`: every price and rate a test here
//! needs is seeded into `historical_prices`, and the pricing layer reads the database
//! before the provider. The two payload tests below need no price and no rate at all —
//! they never open a database, so `mod common` is deliberately not declared until the
//! first test that seeds something lands (Task 4).

use backend_rust::models::{Asset, Transaction};
use backend_rust::schemas::{DatedLot, LotMatch, PriceSource, ReportRow, RowKind, SellBreakdown};
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use std::str::FromStr;

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
