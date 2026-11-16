//! The detail behind the preview's counts.
//!
//! Two producers, one shape: [`detail_from_bundle`] walks a parsed bundle and
//! [`owned_detail`](crate::backup::detail::owned_detail) reads the caller's live rows.
//! Both return [`PlanSide`], so the panel renders them with one component and a
//! mismatch between the two is a bug rather than a difference in wording.
//!
//! The rule the whole module exists to hold: **caps bound display, never counting.**
//! `counts` and `asset_flags` are computed over every portfolio and asset; `shown`
//! lists at most [`MAX_PORTFOLIOS_SHOWN`] and [`MAX_ASSETS_SHOWN`]. A header built
//! from `shown` would under-report a flag sitting past the cap, and a preview that
//! under-reports is worse than one that says nothing.

use super::restore::Counts;
use super::{Bundle, BundleAsset, BundlePortfolio};
use chrono::NaiveDate;
use std::collections::BTreeMap;

/// Portfolios listed in one side. Chosen so a bundle at the cap is a scannable list
/// rather than a wall; it is a display constant, not a contract.
pub const MAX_PORTFOLIOS_SHOWN: usize = 25;
/// Assets listed in one portfolio, for the same reason and with the same status.
pub const MAX_ASSETS_SHOWN: usize = 50;

/// Stable flag codes. The frontend renders an unrecognised code rather than throwing,
/// so adding one here is a backend-only change.
pub mod flag {
    pub const NO_ASSETS: &str = "no_assets";
    pub const NO_TRANSACTIONS: &str = "no_transactions";
    pub const ZERO_PRICE: &str = "zero_price";
    pub const FUTURE_DATED: &str = "future_dated";
    pub const CURRENCY_MISMATCH: &str = "currency_mismatch";
    pub const DUPLICATE_SYMBOL: &str = "duplicate_symbol";
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PlanAsset {
    pub symbol: String,
    pub name: String,
    pub currency: String,
    pub transactions: usize,
    /// `YYYY-MM-DD`, `None` when there are no transactions to span.
    pub first_date: Option<String>,
    pub last_date: Option<String>,
    /// A count, not a bool: one zero-price row and thirty are different facts.
    pub zero_price: usize,
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PlanPortfolio {
    pub name: String,
    pub currency: String,
    /// Exact, over every asset — not `shown.len()`.
    pub assets: usize,
    pub transactions: usize,
    pub shown: Vec<PlanAsset>,
    pub more_assets: usize,
    /// Flag counts over **every** asset, so a collapsed header cannot hide one.
    /// A `BTreeMap` so the JSON is in a stable order and diffable.
    pub asset_flags: BTreeMap<String, usize>,
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PlanSide {
    pub counts: Counts,
    pub shown: Vec<PlanPortfolio>,
    pub more_portfolios: usize,
    /// Transactions held by the portfolios in `shown` — the denominator the panel
    /// quotes when it says how much of the total is on screen.
    pub transactions_in_shown: usize,
}

impl PlanSide {
    pub fn empty() -> Self {
        Self {
            counts: Counts::default(),
            shown: Vec::new(),
            more_portfolios: 0,
            transactions_in_shown: 0,
        }
    }
}

/// `true` when a decimal-as-string price is zero. Parsed, never string-compared:
/// `0`, `0.0` and `0.0000` are the same price and a `== "0.0"` would call two of
/// them clean. A price that does not parse is not counted — validation owns whether
/// a bundle gets this far at all.
fn is_zero(price: &str) -> bool {
    price.trim().parse::<f64>().is_ok_and(|v| v == 0.0)
}

/// The `YYYY-MM-DD` prefix of an RFC 3339 timestamp. Validation guarantees the shape;
/// anything shorter contributes no date rather than panicking in a preview.
fn date_part(rfc3339: &str) -> Option<String> {
    (rfc3339.len() >= 10).then(|| rfc3339[..10].to_string())
}

fn asset_detail(a: &BundleAsset, portfolio_currency: &str, today: NaiveDate) -> PlanAsset {
    let mut flags = Vec::new();
    let zero_price = a.transactions.iter().filter(|t| is_zero(&t.price)).count();
    let dates: Vec<String> = a
        .transactions
        .iter()
        .filter_map(|t| date_part(&t.date))
        .filter(|d| d.len() == 10)
        .collect();
    let first_date = dates.iter().min().cloned();
    let last_date = dates.iter().max().cloned();
    let today_s = today.format("%Y-%m-%d").to_string();

    if a.transactions.is_empty() {
        flags.push(flag::NO_TRANSACTIONS.to_string());
    }
    if zero_price > 0 {
        flags.push(flag::ZERO_PRICE.to_string());
    }
    if dates.iter().any(|d| *d > today_s) {
        flags.push(flag::FUTURE_DATED.to_string());
    }
    if a.currency != portfolio_currency {
        flags.push(flag::CURRENCY_MISMATCH.to_string());
    }

    PlanAsset {
        symbol: a.symbol.clone(),
        name: a.name.clone(),
        currency: a.currency.clone(),
        transactions: a.transactions.len(),
        first_date,
        last_date,
        zero_price,
        flags,
    }
}

fn portfolio_detail(p: &BundlePortfolio, today: NaiveDate) -> PlanPortfolio {
    let mut asset_flags: BTreeMap<String, usize> = BTreeMap::new();
    let mut all: Vec<PlanAsset> = Vec::with_capacity(p.assets.len());

    // Every asset is detailed and counted; only the first MAX_ASSETS_SHOWN are kept.
    for a in &p.assets {
        let d = asset_detail(a, &p.currency, today);
        for f in &d.flags {
            *asset_flags.entry(f.clone()).or_insert(0) += 1;
        }
        all.push(d);
    }

    let mut flags = Vec::new();
    if p.assets.is_empty() {
        flags.push(flag::NO_ASSETS.to_string());
    }
    // A portfolio fact, not an asset fact: it lands in `flags`, not `asset_flags`, and
    // counts *how many symbols are duplicated* — the same thing the SQL side counts.
    // Two producers disagreeing about what one code means is exactly the drift the
    // round-trip test in Task 5 is there to catch.
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut dupes: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for a in &p.assets {
        if !seen.insert(a.symbol.as_str()) {
            dupes.insert(a.symbol.as_str());
        }
    }
    if !dupes.is_empty() {
        flags.push(flag::DUPLICATE_SYMBOL.to_string());
    }

    let transactions = p.assets.iter().map(|a| a.transactions.len()).sum();
    let more_assets = all.len().saturating_sub(MAX_ASSETS_SHOWN);
    let shown = all.into_iter().take(MAX_ASSETS_SHOWN).collect();

    PlanPortfolio {
        name: p.name.clone(),
        currency: p.currency.clone(),
        assets: p.assets.len(),
        transactions,
        shown,
        more_assets,
        asset_flags,
        flags,
    }
}

/// The insert side: everything a bundle contains, read off the bundle. No database,
/// no clock beyond the `today` the caller passes so the future-dated flag is testable
/// without waiting.
pub fn detail_from_bundle(bundle: &Bundle, today: NaiveDate) -> PlanSide {
    let mut side = PlanSide::empty();
    for p in &bundle.portfolios {
        let d = portfolio_detail(p, today);
        side.counts.portfolios += 1;
        side.counts.assets += d.assets;
        side.counts.transactions += d.transactions;
        if side.shown.len() < MAX_PORTFOLIOS_SHOWN {
            side.transactions_in_shown += d.transactions;
            side.shown.push(d);
        } else {
            side.more_portfolios += 1;
        }
    }
    side
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::{Bundle, BundleAsset, BundlePortfolio, BundlePrice, BundleTransaction};
    use chrono::NaiveDate;

    /// A function, not a `const`: `Option::unwrap` is not a `const fn`, so a constant
    /// built with `.unwrap()` is E0015 and the module does not compile at all.
    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
    }

    fn tx(price: &str, date: &str) -> BundleTransaction {
        BundleTransaction {
            r#type: "BUY".into(),
            quantity: "1".into(),
            price: price.into(),
            fee: "0".into(),
            date: format!("{date}T00:00:00+00:00"),
        }
    }

    fn asset(symbol: &str, currency: &str, txs: Vec<BundleTransaction>) -> BundleAsset {
        BundleAsset {
            bundle_ref: format!("a{symbol}"),
            symbol: symbol.into(),
            name: symbol.into(),
            asset_type: "STOCK".into(),
            sector: None,
            currency: currency.into(),
            isin: None,
            transactions: txs,
        }
    }

    fn portfolio(name: &str, currency: &str, assets: Vec<BundleAsset>) -> BundlePortfolio {
        BundlePortfolio {
            bundle_ref: format!("p{name}"),
            name: name.into(),
            description: None,
            currency: currency.into(),
            base_currency: currency.into(),
            assets,
        }
    }

    fn bundle(portfolios: Vec<BundlePortfolio>) -> Bundle {
        Bundle {
            format: "portfolio-prism-backup".into(),
            schema_version: 1,
            created_at: "2026-06-01T00:00:00+00:00".into(),
            portfolios,
            prices: Vec::<BundlePrice>::new(),
        }
    }

    /// The invariant the whole design rests on: the caps bound what is listed, never
    /// what is counted. A refactor that moves the counting inside the display loop
    /// produces a preview that lies, and nothing else in the suite would notice.
    #[test]
    fn counts_stay_exact_when_the_display_is_truncated() {
        let assets = (0..51)
            .map(|i| asset(&format!("S{i}"), "USD", vec![tx("10", "2026-01-05"), tx("11", "2026-02-05")]))
            .collect::<Vec<_>>();
        let portfolios = (0..26)
            .map(|i| portfolio(&format!("P{i}"), "USD", assets.clone()))
            .collect::<Vec<_>>();
        let side = detail_from_bundle(&bundle(portfolios), today());

        assert_eq!(side.counts.portfolios, 26);
        assert_eq!(side.counts.assets, 26 * 51);
        assert_eq!(side.counts.transactions, 26 * 51 * 2);
        assert_eq!(side.shown.len(), MAX_PORTFOLIOS_SHOWN);
        assert_eq!(side.more_portfolios, 1);
        assert_eq!(side.transactions_in_shown, 25 * 51 * 2);

        let p = &side.shown[0];
        assert_eq!(p.assets, 51);
        assert_eq!(p.transactions, 102);
        assert_eq!(p.shown.len(), MAX_ASSETS_SHOWN);
        assert_eq!(p.more_assets, 1);
    }

    /// A flag on asset 51 must reach the header of a portfolio showing 50. The header
    /// is what a collapsed portfolio shows, so an aggregation over `shown` alone
    /// would report no problem where there is one.
    #[test]
    fn asset_flags_are_counted_over_every_asset_not_just_the_shown_ones() {
        let mut assets = (0..50)
            .map(|i| asset(&format!("S{i}"), "USD", vec![tx("10", "2026-01-05")]))
            .collect::<Vec<_>>();
        assets.push(asset("LAST", "USD", vec![tx("0.0", "2026-01-05")]));
        let side = detail_from_bundle(&bundle(vec![portfolio("P", "USD", assets)]), today());

        let p = &side.shown[0];
        assert_eq!(p.shown.len(), 50);
        assert_eq!(p.asset_flags.get(flag::ZERO_PRICE), Some(&1));
        assert!(p.shown.iter().all(|a| a.zero_price == 0));
    }

    #[test]
    fn a_portfolio_with_no_assets_is_flagged_and_still_counted() {
        let side = detail_from_bundle(&bundle(vec![portfolio("Empty", "USD", vec![])]), today());
        let p = &side.shown[0];
        assert_eq!(p.assets, 0);
        assert_eq!(p.flags, vec![flag::NO_ASSETS.to_string()]);
        assert_eq!(side.counts.assets, 0);
    }

    #[test]
    fn an_asset_with_no_transactions_is_flagged_and_carries_no_dates() {
        let side = detail_from_bundle(
            &bundle(vec![portfolio("P", "USD", vec![asset("AAA", "USD", vec![])])]),
            today(),
        );
        let a = &side.shown[0].shown[0];
        assert_eq!(a.transactions, 0);
        assert_eq!(a.first_date, None);
        assert_eq!(a.last_date, None);
        assert_eq!(a.flags, vec![flag::NO_TRANSACTIONS.to_string()]);
    }

    /// `0`, `0.0` and `0.0000` are one price. A `price == "0.0"` string compare catches
    /// one of the three and reports the other two as clean.
    #[test]
    fn zero_price_counts_every_spelling_of_zero() {
        let side = detail_from_bundle(
            &bundle(vec![portfolio("P", "USD", vec![asset(
                "AAA",
                "USD",
                vec![tx("0", "2026-01-05"), tx("0.0", "2026-01-06"), tx("0.0000", "2026-01-07"), tx("12.5", "2026-01-08")],
            )])]),
            today(),
        );
        let a = &side.shown[0].shown[0];
        assert_eq!(a.zero_price, 3);
        assert!(a.flags.contains(&flag::ZERO_PRICE.to_string()));
    }

    #[test]
    fn a_transaction_after_today_is_flagged_and_one_on_today_is_not() {
        let side = detail_from_bundle(
            &bundle(vec![
                portfolio("F", "USD", vec![asset("AAA", "USD", vec![tx("1", "2026-06-02")])]),
                portfolio("N", "USD", vec![asset("BBB", "USD", vec![tx("1", "2026-06-01")])]),
            ]),
            today(),
        );
        assert!(side.shown[0].shown[0].flags.contains(&flag::FUTURE_DATED.to_string()));
        assert!(!side.shown[1].shown[0].flags.contains(&flag::FUTURE_DATED.to_string()));
    }

    #[test]
    fn an_asset_whose_currency_differs_from_its_portfolio_is_flagged() {
        let side = detail_from_bundle(
            &bundle(vec![portfolio("P", "EUR", vec![asset("AAA", "USD", vec![tx("1", "2026-01-05")])])]),
            today(),
        );
        assert!(side.shown[0].shown[0].flags.contains(&flag::CURRENCY_MISMATCH.to_string()));
    }

    #[test]
    fn the_same_symbol_twice_in_one_portfolio_is_flagged_once() {
        let side = detail_from_bundle(
            &bundle(vec![portfolio("P", "USD", vec![
                asset("AAA", "USD", vec![tx("1", "2026-01-05")]),
                asset("AAA", "USD", vec![tx("2", "2026-02-05")]),
                asset("AAA", "USD", vec![tx("3", "2026-03-05")]),
            ])]),
            today(),
        );
        let p = &side.shown[0];
        // Portfolio-level, and present/absent rather than a count: three copies of one
        // symbol is one duplicated symbol, which is what the SQL side reports too.
        assert!(p.flags.contains(&flag::DUPLICATE_SYMBOL.to_string()));
        assert!(!p.asset_flags.contains_key(flag::DUPLICATE_SYMBOL));
    }

    #[test]
    fn a_date_span_names_the_first_and_last_transaction_date() {
        let side = detail_from_bundle(
            &bundle(vec![portfolio("P", "USD", vec![asset(
                "AAA", "USD", vec![tx("1", "2026-02-05"), tx("2", "2025-11-01"), tx("3", "2026-01-09")],
            )])]),
            today(),
        );
        let a = &side.shown[0].shown[0];
        assert_eq!(a.first_date.as_deref(), Some("2025-11-01"));
        assert_eq!(a.last_date.as_deref(), Some("2026-02-05"));
    }
}
