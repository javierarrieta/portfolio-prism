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

/// `true` when a decimal-as-string price is zero.
///
/// Deliberately textual, not parsed. The database side of the preview (Task 2) runs the
/// same test in SQL as `price ~ '^-?0+(\.0+)?$'` over a TEXT column, and the binding
/// rule for the whole feature is that one flag code means exactly one thing on both
/// sides. Parsing to a number here would be a *superset* of that rule: `+0`, `0.` and
/// `0e0` all parse to `0.0` in Rust but fail the text rule, and a restored `+0` is
/// reachable — bundle validation accepts it (`Decimal::from_str_exact` takes a leading
/// `+`) and `restore.rs:399` writes the bundle's price string verbatim into the column.
/// So the two producers would disagree about the same row.
///
/// The accepted shape is exactly the regex's: an optional `-`, one or more `0`, then
/// optionally a `.` and one or more `0`, and nothing else. No trimming: the SQL side
/// anchors with `^` and `$` and would not accept padding either, so padding makes a
/// price not-zero here exactly as it does there. A price that is not a number is therefore **not** zero — it is
/// unparseable, which is a different fact, and the zero test must not swallow it.
fn is_zero(price: &str) -> bool {
    let digits = price.strip_prefix('-').unwrap_or(price);
    match digits.split_once('.') {
        None => !digits.is_empty() && digits.bytes().all(|b| b == b'0'),
        Some((int, frac)) => {
            !int.is_empty()
                && int.bytes().all(|b| b == b'0')
                && !frac.is_empty()
                && frac.bytes().all(|b| b == b'0')
        }
    }
}

/// The `YYYY-MM-DD` prefix of an RFC 3339 timestamp. Validation guarantees the shape;
/// anything shorter contributes no date rather than panicking in a preview. Sliced
/// with `get`, not indexing: `len() >= 10` is a byte count, so indexing byte 10 would
/// panic on a multi-byte character straddling it.
///
/// This is the *fallback*, not the rule — see [`utc_day`].
fn date_part(rfc3339: &str) -> Option<String> {
    rfc3339.get(..10).map(str::to_string)
}

/// The calendar day of an RFC 3339 timestamp, **in UTC**.
///
/// The offset a bundle writes does not survive the restore: `restore.rs:379` parses the
/// same string and converts it to UTC before binding it into a `TIMESTAMPTZ`, which keeps
/// no offset, so [`owned_detail`] can only ever report the UTC day. Taking the first ten
/// characters as written would therefore name one day before the restore and another one
/// after it — `2026-06-02T01:00:00+02:00` is `2026-06-01` in UTC — and `first_date`,
/// `last_date` and `future_dated` all read this value, so one row would give the two
/// producers two different answers. Parsing and converting is what makes them agree.
///
/// A string that does not parse falls back to the textual prefix rather than
/// contributing nothing: a bare `YYYY-MM-DD` has no offset to convert, and the
/// no-panic-on-multibyte behaviour [`date_part`] pins stays reachable.
fn utc_day(rfc3339: &str) -> Option<String> {
    match chrono::DateTime::parse_from_rfc3339(rfc3339) {
        Ok(dt) => Some(dt.with_timezone(&chrono::Utc).format("%Y-%m-%d").to_string()),
        Err(_) => date_part(rfc3339),
    }
}

fn asset_detail(a: &BundleAsset, portfolio_currency: &str, today: NaiveDate) -> PlanAsset {
    let mut flags = Vec::new();
    let zero_price = a.transactions.iter().filter(|t| is_zero(&t.price)).count();
    let dates: Vec<String> = a
        .transactions
        .iter()
        .filter_map(|t| utc_day(&t.date))
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

    // Push order is contractual, not incidental: `flags` is a `Vec<String>` and the
    // round-trip test compares vectors, not sets, so both sides push `no_assets` first.
    // (This used to read "`no_assets` first, then `duplicate_symbol`"; the second half is
    // gone from the database side — see the note in `owned_detail`.)
    let mut flags = Vec::new();
    if p.assets.is_empty() {
        flags.push(flag::NO_ASSETS.to_string());
    }
    // A portfolio fact, not an asset fact: it lands in `flags`, not `asset_flags`. It
    // records *that* a symbol is duplicated, once per duplicated symbol — three copies
    // of one symbol set the flag once. Nothing here counts how many symbols are
    // duplicated, and the flag is not a count.
    //
    // Read this before trusting it, because the honest version is narrower than the useful
    // one. The database side of this flag was deleted when migration `0006` made it
    // unreachable. What is left is **not reachable through the preview route either**:
    // `api_routes::backup::preview` runs `parse_and_validate` before it builds a plan, and
    // `validate` refuses a case-insensitive duplicate symbol per portfolio — so a bundle
    // that would set this flag is refused with a 422 before any plan exists, and no client
    // ever sees it. It is reachable only for a caller that plans without validating, which
    // today means the unit tests.
    //
    // So this is a tripwire, not a message to a human: if it ever fires in a real request,
    // the ordering in `preview` changed and the guarantee that nothing unvalidatable is
    // planned has gone. It is kept rather than deleted because the code is published in the
    // preview flag vocabulary and rendered by `BackupPlanDetail.tsx`, so removing it is a
    // schema decision, not a cleanup — and because the comparison below is now the same
    // rule `validate` enforces (upper-cased, matching the `upper(symbol)` index), which
    // makes it a correct tripwire even though it was not one before.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut dupes: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for a in &p.assets {
        // Upper-cased on purpose, and it used to be a bug that it was not: `validate`'s rule
        // is `to_uppercase()` because the index is `upper(symbol)`, so a bundle carrying
        // `MSFT` beside `msft` was refused by the validator and would not have been flagged
        // here. A tripwire that misses the case the rule it pre-announces catches is worse
        // than no tripwire.
        if !seen.insert(a.symbol.to_uppercase()) {
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

/// The live side: everything `user_id` owns, read from the database.
///
/// Three queries, and the split is the point. The portfolio aggregate and the flag
/// aggregate cover **every** owned row, so every exact number comes from a full
/// scan of the caller's data; only the asset detail query is bounded, and only by
/// the display caps — and the asset cap is applied in Rust, from the same rows
/// `more_assets` is computed from, so a plain SQL `LIMIT` cannot under-report it. A
/// single joined query could not promise that without returning every asset to count
/// it. Bounding the rows exactly *is* possible — a windowed `ROW_NUMBER()` partitioned
/// by portfolio and ordered the same way the cap is applied would return precisely the
/// listed assets, and it would make the read cheaper, because the detail query runs five
/// correlated subqueries for every row it returns — and it is deliberately not done: the
/// cap is a display constant and the window would copy it into SQL, while the row count
/// is already bounded by the portfolio cap the query is handed (`ANY($1)`).
///
/// **All three reads share one snapshot.** They run on one connection inside the
/// `READ ONLY REPEATABLE READ` transaction [`super::export::begin_read_tx`] opens, and
/// the transaction ends as soon as the third read returns. This is not decoration: at
/// the server default Postgres takes a fresh snapshot per *statement*, so an ordinary
/// write committing between query 1 and query 3 would be counted by query 1 and absent
/// from query 3 — `counts.assets = 51` next to 50 rows and `more_assets = 0`, a
/// truncated list presented as complete, which is the exact failure this feature exists
/// to prevent. Wrapping the three reads in a plain transaction would not fix it, only
/// make it rarer; `REPEATABLE READ` is what takes one snapshot for all three.
///
/// Stated plainly, because it is not testable: **no test pins the snapshot property.**
/// There is no way to force a write to land between two statements on demand, so the
/// guarantee rests on the isolation level plus the single connection — the same basis
/// `export::build` documents for itself, and the same one Rule 3 of
/// `scripts/check_backup_portability.sh` guards (a bare `.begin(` here would run at the
/// server default and every test would stay green).
///
/// `today` is a parameter rather than a clock read so the future-dated flag is
/// testable; `plan_for` passes the current UTC date.
pub async fn owned_detail(
    pool: &crate::db::DbPool,
    user_id: i64,
    today: chrono::NaiveDate,
) -> Result<PlanSide, crate::backup::export::DbFailure> {
    use crate::backup::export::classify;

    // One snapshot for all three reads, on one connection, opened through the export's
    // helper so there is one spelling of the isolation level in the module. The scope is
    // deliberately the three queries and nothing else: the aggregation below is pure
    // Rust over rows already fetched, and widening the transaction to cover it would hold
    // the snapshot open for no reason.
    let mut tx = super::export::begin_read_tx(pool).await?;

    // One row per owned portfolio, with its exact asset and transaction counts.
    let portfolios = sqlx::query!(
        r#"SELECT p.id AS "id!", p.name AS "name!", p.currency AS "currency!",
                 (SELECT COUNT(*) FROM assets a WHERE a.portfolio_id = p.id) AS "assets!",
                 (SELECT COUNT(*) FROM transactions t
                    JOIN assets a ON a.id = t.asset_id
                   WHERE a.portfolio_id = p.id) AS "transactions!"
           FROM portfolios p WHERE p.owner_id = $1 ORDER BY p.id"#,
        user_id
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(classify)?;

    // Flag counts per portfolio, over every asset the caller owns. `AT TIME ZONE
    // 'UTC'` pins the day: `date::date` would use the session zone and a portfolio
    // would be future-dated in one timezone and not in another.
    let flags = sqlx::query!(
        r#"SELECT p.id AS "portfolio_id!",
                  COUNT(*) FILTER (WHERE (SELECT COUNT(*) FROM transactions t
                                           WHERE t.asset_id = a.id) = 0) AS "no_transactions!",
                  COUNT(*) FILTER (WHERE EXISTS (SELECT 1 FROM transactions t
                                                  WHERE t.asset_id = a.id
                                                    AND t.price ~ '^-?0+(\.0+)?$')) AS "zero_price!",
                  COUNT(*) FILTER (WHERE EXISTS (SELECT 1 FROM transactions t
                                                  WHERE t.asset_id = a.id
                                                    AND (t.date AT TIME ZONE 'UTC')::date > $2)) AS "future_dated!",
                  COUNT(*) FILTER (WHERE a.currency <> p.currency) AS "currency_mismatch!"
            FROM portfolios p JOIN assets a ON a.portfolio_id = p.id
           WHERE p.owner_id = $1 GROUP BY p.id"#,
        user_id,
        today
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(classify)?;

    // Detail for the portfolios that will actually be listed: at most
    // MAX_PORTFOLIOS_SHOWN portfolios, ordered so the cap is deterministic. The asset
    // cap is applied in Rust below, not here, because the same rows are what
    // `more_assets` is computed from — pushing a plain SQL LIMIT would under-report it.
    // A ROW_NUMBER() OVER (PARTITION BY portfolio_id ORDER BY symbol, id) window in a
    // subquery would bound it exactly instead, and would cut work: this query runs five
    // correlated subqueries per row returned, so 25x50 rows is less than the unbounded
    // read. Not done here because the row count is already bounded by the portfolio cap
    // the query is handed, and the window would copy a display constant into SQL.
    let listed: Vec<i64> = portfolios.iter().take(MAX_PORTFOLIOS_SHOWN).map(|p| p.id).collect();
    let assets = sqlx::query!(
        r#"SELECT a.portfolio_id AS "portfolio_id!", a.symbol AS "symbol!",
                 a.name AS "name!", a.currency AS "currency!",
                 (SELECT COUNT(*) FROM transactions t WHERE t.asset_id = a.id) AS "transactions!",
                 (SELECT MIN(t.date) FROM transactions t WHERE t.asset_id = a.id) AS "first_date",
                 (SELECT MAX(t.date) FROM transactions t WHERE t.asset_id = a.id) AS "last_date",
                 (SELECT COUNT(*) FROM transactions t
                   WHERE t.asset_id = a.id AND t.price ~ '^-?0+(\.0+)?$') AS "zero_price!",
                 (SELECT COUNT(*) FROM transactions t
                   WHERE t.asset_id = a.id
                     AND (t.date AT TIME ZONE 'UTC')::date > $2) AS "future_dated!"
           FROM assets a WHERE a.portfolio_id = ANY($1)
          ORDER BY a.portfolio_id, a.symbol, a.id"#,
        &listed,
        today
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(classify)?;

    // The snapshot is no longer needed; ending the transaction releases it. A read-only
    // commit writes nothing — see `export::build`, which does the same for the same
    // reason. A `?` above drops `tx` instead, which rolls the read-only transaction back.
    tx.commit().await.map_err(classify)?;

    let mut side = PlanSide::empty();
    for p in &portfolios {
        side.counts.portfolios += 1;
        side.counts.assets += p.assets as usize;
        side.counts.transactions += p.transactions as usize;
    }

    for p in portfolios.iter().take(MAX_PORTFOLIOS_SHOWN) {
        let rows: Vec<&_> = assets.iter().filter(|a| a.portfolio_id == p.id).collect();
        let shown: Vec<PlanAsset> = rows
            .iter()
            .take(MAX_ASSETS_SHOWN)
            .map(|a| PlanAsset {
                symbol: a.symbol.clone(),
                name: a.name.clone(),
                currency: a.currency.clone(),
                transactions: a.transactions as usize,
                first_date: a.first_date.map(|d| d.date_naive().format("%Y-%m-%d").to_string()),
                last_date: a.last_date.map(|d| d.date_naive().format("%Y-%m-%d").to_string()),
                zero_price: a.zero_price as usize,
                flags: asset_flags_from(
                    a.transactions as usize,
                    a.zero_price as usize,
                    a.future_dated as usize,
                    &a.currency,
                    &p.currency,
                ),
            })
            .collect();

        let mut asset_flags: BTreeMap<String, usize> = BTreeMap::new();
        // Push order is contractual, not incidental: `flags` is a `Vec<String>` and the
        // round-trip test compares vectors, not sets, so this side pushes in the same
        // order as `portfolio_detail` on the bundle side.
        //
        // `duplicate_symbol` used to be pushed here, from a `GROUP BY symbol HAVING COUNT(*)
        // > 1` subquery over `assets`. Migration `0006_asset_symbol_unique.sql` makes that
        // subquery unreachable, and "unreachable" is the provable claim rather than a hunch:
        // the migration refuses to apply to a database that already holds a duplicated
        // symbol, and the unique index it creates on `(portfolio_id, upper(symbol))` prevents
        // any new one. There is no sequence of writes that leaves the database in a state the
        // detector could see. It is deleted rather than left dormant because a detector that
        // cannot fire is not a safety net — it is a reader's false premise, and the round-trip
        // test would keep comparing a flag one producer can emit and the other never can.
        //
        // The flag itself still exists on the bundle side (`detail_from_bundle`), but do
        // not read that as a live path: the preview route validates before it plans, and
        // `validate` refuses a duplicated symbol, so the bundle side is a tripwire too —
        // see the note there. Neither producer can set this code through an HTTP preview
        // today, which is the fact the round-trip test below now pins by asserting absence
        // on both sides.
        let mut flags_vec = Vec::new();
        if p.assets == 0 {
            flags_vec.push(flag::NO_ASSETS.to_string());
        }
        if let Some(f) = flags.iter().find(|f| f.portfolio_id == p.id) {
            // Asset-level facts, counted over every asset the caller owns.
            for (code, n) in [
                (flag::NO_TRANSACTIONS, f.no_transactions),
                (flag::ZERO_PRICE, f.zero_price),
                (flag::FUTURE_DATED, f.future_dated),
                (flag::CURRENCY_MISMATCH, f.currency_mismatch),
            ] {
                if n > 0 {
                    asset_flags.insert(code.to_string(), n as usize);
                }
            }
        }

        side.transactions_in_shown += p.transactions as usize;
        side.shown.push(PlanPortfolio {
            name: p.name.clone(),
            currency: p.currency.clone(),
            assets: p.assets as usize,
            transactions: p.transactions as usize,
            more_assets: rows.len().saturating_sub(MAX_ASSETS_SHOWN),
            shown,
            asset_flags,
            flags: flags_vec,
        });
    }
    side.more_portfolios = portfolios.len().saturating_sub(MAX_PORTFOLIOS_SHOWN);
    Ok(side)
}

/// Per-asset flags from the numbers the detail query already returned, so the asset
/// rows and the portfolio's `asset_flags` cannot disagree about the same asset. The
/// same four codes the bundle side sets, or the two producers would render differently
/// depending on which one filled the panel.
fn asset_flags_from(
    transactions: usize,
    zero_price: usize,
    future_dated: usize,
    asset_currency: &str,
    portfolio_currency: &str,
) -> Vec<String> {
    let mut flags = Vec::new();
    if transactions == 0 {
        flags.push(flag::NO_TRANSACTIONS.to_string());
    }
    if zero_price > 0 {
        flags.push(flag::ZERO_PRICE.to_string());
    }
    if future_dated > 0 {
        flags.push(flag::FUTURE_DATED.to_string());
    }
    if asset_currency != portfolio_currency {
        flags.push(flag::CURRENCY_MISMATCH.to_string());
    }
    flags
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

    /// A transaction whose date string is used verbatim. `tx` appends a `+00:00` time,
    /// so it cannot express the one thing the offset tests are about.
    fn tx_at(price: &str, rfc3339: &str) -> BundleTransaction {
        BundleTransaction {
            date: rfc3339.into(),
            ..tx(price, "2026-01-01")
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
        // The "still counted" half of the name: an empty portfolio is a portfolio the
        // restore will create, so the total says one even though it holds nothing.
        assert_eq!(side.counts.portfolios, 1);
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
    /// one of the three and reports the other two as clean. A price that is not a
    /// number at all is not one of those spellings of zero and must not be counted —
    /// the row is still counted as a transaction, which is why both numbers are asserted.
    #[test]
    fn zero_price_counts_every_spelling_of_zero() {
        let side = detail_from_bundle(
            &bundle(vec![portfolio("P", "USD", vec![asset(
                "AAA",
                "USD",
                vec![tx("0", "2026-01-05"), tx("0.0", "2026-01-06"), tx("0.0000", "2026-01-07"), tx("12.5", "2026-01-08"), tx("n/a", "2026-01-09")],
            )])]),
            today(),
        );
        let a = &side.shown[0].shown[0];
        assert_eq!(a.zero_price, 3);
        assert_eq!(a.transactions, 5);
        assert!(a.flags.contains(&flag::ZERO_PRICE.to_string()));
    }

    /// The rule itself, pinned on the function that implements it, because the drift it
    /// prevents is invisible from the outside: Task 2's SQL producer matches the same
    /// TEXT column against `^-?0+(\.0+)?$`, so a price string the two producers judge
    /// differently is one flag meaning two things. Everything in the second list parses
    /// to `0.0` as a float — `+0` reaches this code for real, since bundle validation
    /// accepts it and `restore.rs:399` writes the string verbatim — and `not-a-number`
    /// is the row the text match exists to leave alone.
    #[test]
    fn zero_price_is_the_text_rule_not_a_float_parse() {
        for zero in ["0", "-0", "0.0", "-0.00"] {
            assert!(is_zero(zero), "{zero} must read as zero");
        }
        for not_zero in ["+0", "0.", "0e0", "0.0.0", "not-a-number", "1e-400", " 0", "0 "] {
            assert!(!is_zero(not_zero), "{not_zero} must not read as zero");
        }
    }

    /// The date prefix is taken by byte count, so a multi-byte character landing on the
    /// boundary is the case that panics on a plain slice. Unreachable over HTTP —
    /// validation requires an RFC 3339 date — but `date_part` is reachable from anything
    /// that hands it a string, and "no date" beats a panic in a preview.
    #[test]
    fn a_multibyte_character_on_the_date_boundary_yields_no_date() {
        assert_eq!(date_part("2026-01-0\u{e9}T00:00:00+00:00"), None);
        assert_eq!(date_part("2026-01-05\u{e9}T00:00:00+00:00").as_deref(), Some("2026-01-05"));
        assert_eq!(date_part("2026-01-0\u{e9}"), None);
        assert_eq!(date_part("short"), None);
        assert_eq!(date_part("2026-01-05T00:00:00+00:00").as_deref(), Some("2026-01-05"));
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

    /// The two producers must be able to name the same day for the same row, so the day
    /// is UTC on both. `2026-06-02T01:00:00+02:00` restores as the instant
    /// `2026-06-01T23:00:00Z` — a `TIMESTAMPTZ` keeps no offset — so the readback says
    /// 2026-06-01 and raises no flag. Slicing the string as written said 2026-06-02 and
    /// flagged the asset future-dated, which is one row giving the panel two answers
    /// depending on which producer filled it. All three fields the day feeds are asserted
    /// because all three read it: `first_date`, `last_date` and `future_dated`.
    #[test]
    fn a_non_utc_offset_is_converted_to_the_utc_day() {
        let side = detail_from_bundle(
            &bundle(vec![portfolio("P", "USD", vec![asset(
                "AAA",
                "USD",
                vec![tx_at("1", "2026-06-02T01:00:00+02:00")],
            )])]),
            today(),
        );
        let a = &side.shown[0].shown[0];
        assert_eq!(a.first_date.as_deref(), Some("2026-06-01"));
        assert_eq!(a.last_date.as_deref(), Some("2026-06-01"));
        assert!(!a.flags.contains(&flag::FUTURE_DATED.to_string()));
    }

    /// The mirror direction of the test above: a negative offset moves the written date
    /// the other way. `2026-05-31T23:00:00-02:00` is `2026-06-01T01:00:00Z`, so the UTC
    /// day is today and the asset is not future-dated, while slicing the string would have
    /// named 2026-05-31 — a mismatch only this direction can show.
    #[test]
    fn a_negative_offset_moves_the_day_the_other_way() {
        let side = detail_from_bundle(
            &bundle(vec![portfolio("P", "USD", vec![asset(
                "AAA",
                "USD",
                vec![tx_at("1", "2026-05-31T23:00:00-02:00")],
            )])]),
            today(),
        );
        let a = &side.shown[0].shown[0];
        assert_eq!(a.first_date.as_deref(), Some("2026-06-01"));
        assert_eq!(a.last_date.as_deref(), Some("2026-06-01"));
        assert!(!a.flags.contains(&flag::FUTURE_DATED.to_string()));
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
        // Exactly one flag entry for three copies of one symbol — `contains` could not
        // tell that apart from the flag being pushed once per duplicate, which is the
        // thing this test's name claims. This is now the flag's only producer: the
        // `owned_detail` side computed it from a `GROUP BY symbol` over `assets` until
        // migration `0006_asset_symbol_unique.sql` made that unreachable. See the note in
        // `owned_detail` before assuming a second producer exists to agree with.
        assert_eq!(p.flags, vec![flag::DUPLICATE_SYMBOL.to_string()]);
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
