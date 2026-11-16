# Import Preview Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the restore preview name the portfolios and securities a bundle contains and the ones it will delete, surface anomalies without blocking, and show what the database holds after a restore.

**Architecture:** One `PlanSide` shape, produced two ways — a pure walk of the parsed bundle for the insert side, and three aggregate queries scoped by `owner_id` for the delete side and the new readback route. One React component renders that shape at three call sites. Exact totals always come from a full walk or a grouped aggregate; only `shown` lists are capped.

**Tech Stack:** Rust (Rocket, SQLx/Postgres), React + TypeScript + Vite, Vitest + Testing Library.

**Spec:** `docs/superpowers/specs/2026-11-16-import-preview-design.md` — read it before starting; this plan argues from it, and where the two disagree the spec wins.

## Global Constraints

- Postgres only. No SQLite, no dual-dialect SQL, no `COALESCE`/`IFNULL` portability hedging.
- `sqlx::query!` (compile-time checked) in `src/`; runtime `sqlx::query`/`query_as` in `tests/`.
- After any change to a `sqlx::query!` in `src/`, run `cargo sqlx prepare -- --all-targets` from `backend_rust/` and commit the refreshed `.sqlx/` cache. `scripts/check_sqlx_cache.sh` fails CI otherwise. The `--all-targets` flag is not optional — without it the cache is written from `src/` alone and the gate still fails.
- Caps are 25 portfolios and 50 assets per portfolio. They bound display only. `counts` and `asset_flags` are computed over every portfolio and asset, always.
- Flags never block a restore and never gate a checkbox. The typed `REPLACE` confirmation is the only gate on `replace`.
- The bundle format is unchanged: no `schema_version` bump, no edit to `docs/backup-schema/v1.json`, no new bundle field.
- Flag codes are stable strings: `no_assets`, `no_transactions`, `zero_price`, `future_dated`, `currency_mismatch`, `duplicate_symbol`. The frontend renders an unrecognised code rather than throwing.
- No personal data in any file this plan writes or commits — no real symbols, real counts, real dates, real hostnames, in code, tests, docs, commit messages or the PR body. Every example below is synthetic.
- Copy rule: the panel never says price history "will be left alone". It is an operator flow, described as one.
- Local Postgres for the test database: `nix-shell -p postgresql_18 --run 'bash scripts/dev-db.sh'` from `backend_rust/`, then `sqlx migrate run --no-dotenv`. Doctests fail without the migrations applied.

## Review Focus

The spec implies these inputs and no task's happy path exercises them. Each line gets its test in the task named.

- **A `200` whose body is valid JSON but not a plan** — a proxy envelope, a truncated gzipped body, a renamed field. The panel must refuse and say the response was unexpected, never render half a plan: a throw during render is a blank page at the moment someone is deciding whether to delete their data. (Task 7)
- **A decimal zero written as `0`, `0.0` or `0.0000`** — all three are the same price and all three must count as `zero_price`. String comparison would catch one of the three and quietly miss the others. (Task 1 for the bundle side, Task 2 for the database side)
- **A flag on an asset past the display cap** — a portfolio with 51 assets where the 51st holds the only zero-price transaction must still report `zero_price: 1` on the collapsed header. A header computed from `shown` under-reports, and an under-reporting header is worse than no header. (Task 1 and Task 2)
- **Two users in one database** — `GET /api/backup/owned` returns only the caller's rows. The route takes no id, so the only thing standing between tenants is the `owner_id` filter, and a filter that is missing still returns 200. (Task 2)
- **A price that is not a number at all.** The zero-price test runs over every row a user owns; if it were written as a `::numeric` cast, one unparseable row would make every preview and every readback for that user a 500. It is a text match instead, and a test seeds a bad row to keep it that way. (Task 2)
- **A restore that deletes rows added after the preview** — the plan is a statement about a moment. `deleted` must report what the `DELETE` actually removed, not repeat the plan's number. (Task 4)

## File Structure

| File | Responsibility |
| --- | --- |
| Create `backend_rust/src/backup/detail.rs` | The `PlanSide`/`PlanPortfolio`/`PlanAsset` shapes, the flag codes and their computation, `detail_from_bundle` (pure) and `owned_detail` (database). Nothing else — no routes, no plan assembly. |
| Modify `backend_rust/src/backup/restore.rs` | `Plan.delete`/`Plan.insert` become `PlanSide`; `plan_for` delegates to `detail`; `RestoreOutcome` gains `deleted`. |
| Modify `backend_rust/src/api_routes/backup.rs` | New `GET /api/backup/owned`; preview response description updated. |
| Modify `backend_rust/src/lib.rs`, `backend_rust/src/openapi.rs` | Mount and document the new route. |
| Modify `backend_rust/tests/route_coverage_test.rs` | Add the new route to `PROTECTED_ROUTES`. |
| Create `backend_rust/tests/backup_detail_test.rs` | Integration tests for `owned_detail` and the readback route. |
| Modify `backend_rust/tests/backup_test.rs` | Preview-shape tests, `deleted` tests. |
| Create `frontend/src/components/BackupPlanDetail.tsx` | `PlanSideView`, the side/portfolio/asset types, `countList`, flag labels, the collapse rule. |
| Modify `frontend/src/components/BackupPanel.tsx` | Stricter `asPlan`, three `PlanSideView` call sites, readback, new prices copy. |
| Create `frontend/src/test/components/BackupPlanDetail.test.tsx` | Component tests. |
| Modify `frontend/src/test/components/BackupPanel.test.tsx` | Plan guard, readback, copy. |
| Regenerate `docs/openapi/openapi.json` | Published document. |

---

### Task 1: Detail types and the bundle-side producer

**Files:**
- Create: `backend_rust/src/backup/detail.rs`
- Modify: `backend_rust/src/backup/mod.rs` (add `pub mod detail;` beside `pub mod restore;`)
- Modify: `backend_rust/tests/backup_portability_test.rs:89` — the portability tripwire counts
  the files under `src/backup/`; a fifth module makes it 5 and the assertion moves with it. That
  is the tripwire doing its job. INSERT counts stay at 4: `detail.rs` adds no INSERT statements.
- Test: unit tests in `backend_rust/src/backup/detail.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `crate::backup::{Bundle, BundlePortfolio, BundleAsset, BundleTransaction}`, `crate::backup::restore::Counts`.
- Produces:
  - `pub const MAX_PORTFOLIOS_SHOWN: usize` / `pub const MAX_ASSETS_SHOWN: usize`
  - `pub struct PlanAsset { symbol: String, name: String, currency: String, transactions: usize, first_date: Option<String>, last_date: Option<String>, zero_price: usize, flags: Vec<String> }`
  - `pub struct PlanPortfolio { name: String, currency: String, assets: usize, transactions: usize, shown: Vec<PlanAsset>, more_assets: usize, asset_flags: BTreeMap<String, usize>, flags: Vec<String> }`
  - `pub struct PlanSide { counts: Counts, shown: Vec<PlanPortfolio>, more_portfolios: usize, transactions_in_shown: usize }`
  - `pub fn detail_from_bundle(bundle: &Bundle, today: NaiveDate) -> PlanSide`
  - `pub mod flag { pub const NO_ASSETS: &str; pub const NO_TRANSACTIONS: &str; pub const ZERO_PRICE: &str; pub const FUTURE_DATED: &str; pub const CURRENCY_MISMATCH: &str; pub const DUPLICATE_SYMBOL: &str; }`

- [ ] **Step 1: Write the failing tests**

Create `backend_rust/src/backup/detail.rs` with the types, the `flag` module, the two constants, and at the bottom:

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib backup::detail`
Expected: compile failure — `detail_from_bundle` not found.

- [ ] **Step 3: Implement**

In `backend_rust/src/backup/detail.rs`, above the test module:

```rust
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib backup::detail`
Expected: PASS, 9 tests.

- [ ] **Step 5: Commit**

```bash
git add backend_rust/src/backup/detail.rs backend_rust/src/backup/mod.rs
git commit -m "feat(backup): the detail behind a preview's counts, with caps that bound display only"
```

---

### Task 2: The database-side producer

**Files:**
- Modify: `backend_rust/src/backup/detail.rs` (add `owned_detail`)
- Create: `backend_rust/tests/backup_detail_test.rs`
- Modify: `backend_rust/.sqlx/` (regenerated)

**Interfaces:**
- Consumes: `crate::db::DbPool`, `crate::backup::export::{classify, DbFailure}` (both are `pub` there — `restore.rs` imports them rather than defining them), the Task 1 types.
- Produces: `pub async fn owned_detail(pool: &DbPool, user_id: i64, today: NaiveDate) -> Result<PlanSide, DbFailure>`

- [ ] **Step 1: Write the failing test**

Create `backend_rust/tests/backup_detail_test.rs`:

```rust
//! `owned_detail` — the live half of the preview, and the readback after a restore.
//!
//! Runtime `sqlx::query` here, not `query!`: the compile-time macro lives in `src/`
//! only, so the cache does not have to know about test SQL.

mod common;

use backend_rust::backup::detail::{flag, owned_detail, MAX_ASSETS_SHOWN};
use chrono::NaiveDate;
use common::{db_with_user, seed_asset, seed_asset_currency, seed_portfolio, seed_transaction, seed_user};

/// A function, not a `const` — `Option::unwrap` is not a `const fn` (see Task 1).
fn today() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
}

#[tokio::test]
async fn owned_detail_describes_what_the_caller_owns() {
    let (pool, uid) = db_with_user("owned-detail-basic").await;
    let pid = seed_portfolio(&pool, uid, "Alpha", "EUR").await;
    let a1 = seed_asset(&pool, pid, "AAA", "Alpha One").await;
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
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test backup_detail_test`
Expected: compile failure — `owned_detail` not found.

- [ ] **Step 3: Implement**

Append to `backend_rust/src/backup/detail.rs`:

```rust
/// The live side: everything `user_id` owns, read from the database.
///
/// Three queries, and the split is the point. The portfolio aggregate and the flag
/// aggregate cover **every** owned row, so every exact number comes from a full
/// scan of the caller's data; only the asset detail query is bounded, and only by
/// the display cap. A single joined query could not promise that without returning
/// every asset to count it.
///
/// `today` is a parameter rather than a clock read so the future-dated flag is
/// testable; `plan_for` passes the current UTC date.
pub async fn owned_detail(
    pool: &crate::db::DbPool,
    user_id: i64,
    today: chrono::NaiveDate,
) -> Result<PlanSide, crate::backup::export::DbFailure> {
    use crate::backup::export::classify;

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
    .fetch_all(pool)
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
                  COUNT(*) FILTER (WHERE a.currency <> p.currency) AS "currency_mismatch!",
                  (SELECT COUNT(*) FROM (SELECT a2.symbol FROM assets a2
                                          WHERE a2.portfolio_id = p.id
                                          GROUP BY a2.symbol HAVING COUNT(*) > 1) d) AS "duplicates!"
            FROM portfolios p JOIN assets a ON a.portfolio_id = p.id
           WHERE p.owner_id = $1 GROUP BY p.id"#,
        user_id,
        today
    )
    .fetch_all(pool)
    .await
    .map_err(classify)?;

    // Detail for the portfolios that will actually be listed, ordered so the cap is
    // deterministic. There is deliberately no LIMIT on assets: more_assets is computed from
    // these rows, and a LIMIT would under-report it. The cap bounds the response, not the query.
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
    .fetch_all(pool)
    .await
    .map_err(classify)?;

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
        let mut flags_vec = Vec::new();
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
            // Portfolio-level, and meaning the same thing it means on the bundle side:
            // how many symbols are duplicated, expressed as present or absent.
            if f.duplicates > 0 {
                flags_vec.push(flag::DUPLICATE_SYMBOL.to_string());
            }
        }
        if p.assets == 0 {
            flags_vec.push(flag::NO_ASSETS.to_string());
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
```

The `sqlx::query!` output struct field names come from the `AS "name!"` aliases in each
statement, and `first_date` / `last_date` arrive as `Option<DateTime<Utc>>` because
`transactions.date` is `TIMESTAMPTZ` — hence `.date_naive().format("%Y-%m-%d")`. The
bundle side reaches the same `YYYY-MM-DD` shape by taking the first ten characters of
the RFC 3339 string, so both sides print the same way.

- [ ] **Step 4: Refresh the SQLx cache**

Run: `cargo sqlx prepare -- --all-targets`
Expected: `.sqlx/` gains three `query-*.json` entries. Without `--all-targets` the next gate fails.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --test backup_detail_test`
Expected: PASS, 5 tests.

- [ ] **Step 6: Commit**

```bash
git add backend_rust/src/backup/detail.rs backend_rust/tests/backup_detail_test.rs backend_rust/.sqlx
git commit -m "feat(backup): read the live side of a preview from grouped aggregates, not the sample"
```

---

### Task 3: The plan carries both sides

**Files:**
- Modify: `backend_rust/src/backup/restore.rs` (`Plan`, `plan_for`)
- Modify: `backend_rust/tests/backup_test.rs`

**Interfaces:**
- Consumes: `detail_from_bundle`, `owned_detail` (Task 1, Task 2).
- Produces: `Plan { schema_version, mode, delete: PlanSide, insert: PlanSide, prices, warnings }`.

- [ ] **Step 1: Write the failing test**

Append to `backend_rust/tests/backup_test.rs`, in the preview section:

```rust
#[tokio::test]
async fn preview_names_the_portfolios_and_assets_it_will_insert() {
    let (pool, uid) = db_with_user("preview-detail-insert").await;
    let _ = uid;
    let client = Client::tracked(build_rocket(pool)).await.unwrap();
    let body = bundle_with_size(2, 3, 4);

    let res = client
        .post("/api/backup/preview?mode=copy")
        .header(common::session_header())
        .header(rocket::http::ContentType::JSON)
        .body(body)
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let plan: Value = res.into_json().await.unwrap();

    assert_eq!(plan["insert"]["counts"]["portfolios"], 2);
    assert_eq!(plan["insert"]["shown"].as_array().unwrap().len(), 2);
    let first = &plan["insert"]["shown"][0];
    assert_eq!(first["shown"].as_array().unwrap().len(), 3);
    assert!(first["name"].is_string());
    assert!(first["asset_flags"].is_object());
    // Copy deletes nothing, and says so with an empty side rather than a missing one.
    assert_eq!(plan["delete"]["counts"]["portfolios"], 0);
    assert!(plan["delete"]["shown"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn replace_preview_names_what_it_will_delete() {
    let (pool, uid) = db_with_user("preview-detail-delete").await;
    let pid = seed_portfolio(&pool, uid, "Doomed", "EUR").await;
    let aid = seed_asset(&pool, pid, "DOOM", "Doomed One").await;
    seed_transaction(&pool, aid, "BUY", 1.0, 4.0, 0.0).await;

    let client = Client::tracked(build_rocket(pool)).await.unwrap();
    let res = client
        .post("/api/backup/preview?mode=replace")
        .header(common::session_header())
        .header(rocket::http::ContentType::JSON)
        .body(bundle_with_size(1, 1, 1))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let plan: Value = res.into_json().await.unwrap();

    assert_eq!(plan["delete"]["counts"]["portfolios"], 1);
    assert_eq!(plan["delete"]["shown"][0]["name"], "Doomed");
    assert_eq!(plan["delete"]["shown"][0]["currency"], "EUR");
    assert_eq!(plan["delete"]["shown"][0]["transactions"], 1);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test backup_test preview_`
Expected: FAIL — `plan["insert"]["counts"]` is null, because `insert` is still a flat `Counts`.

- [ ] **Step 3: Implement**

In `backend_rust/src/backup/restore.rs`, change the `Plan` fields and `plan_for`'s tail:

```rust
#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub schema_version: i64,
    pub mode: String,
    /// What the caller owns today, in the same shape as `insert` so one component
    /// renders both. Was a bare `Counts`.
    pub delete: super::detail::PlanSide,
    /// What the bundle holds.
    pub insert: super::detail::PlanSide,
    pub prices: usize,
    pub warnings: Vec<String>,
}
```

and at the end of `plan_for`:

```rust
    // One clock reading for both sides, so a preview that straddles midnight cannot
    // call the same transaction future-dated on one side and not on the other.
    let today = chrono::Utc::now().date_naive();
    let insert = super::detail::detail_from_bundle(bundle, today);
    let delete = match mode {
        Mode::Copy => super::detail::PlanSide::empty(),
        Mode::Replace => owned_detail(pool, user_id, today).await?,
    };

    Ok(Plan {
        schema_version: bundle.schema_version,
        mode: mode.as_str().to_string(),
        delete,
        insert,
        prices: bundle.prices.len(),
        warnings,
    })
```

`plan_for`'s existing warnings block is unchanged; it reads `delete.portfolios`, which becomes `delete.counts.portfolios`. Then **delete `owned_counts`** (`restore.rs:74`): `plan_for` was its only caller — `grep -rn owned_counts src/ tests/` returns the definition and that one call — and leaving a second, narrower way to count the caller's rows is how the two drift apart later.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --test backup_test`
Expected: PASS. Existing preview tests that read `plan["insert"]["portfolios"]` must be updated to `plan["insert"]["counts"]["portfolios"]` — update them in this step and say so in the commit body.

- [ ] **Step 5: Commit**

```bash
git add backend_rust/src/backup/restore.rs backend_rust/tests/backup_test.rs
git commit -m "feat(backup): the plan names what it inserts and what it deletes

Both sides of the plan become a PlanSide: exact counts plus a bounded listing.
Existing preview assertions move from plan.insert.portfolios to
plan.insert.counts.portfolios."
```

---

### Task 4: The restore reports what it deleted

**Files:**
- Modify: `backend_rust/src/backup/restore.rs` (`RestoreOutcome`, `apply`)
- Modify: `backend_rust/tests/backup_test.rs`

**Interfaces:**
- Consumes: the existing `apply` transaction body.
- Produces: `RestoreOutcome { inserted: Counts, deleted: Counts, portfolio_ids: Vec<i64>, prices_inserted: usize }`.

- [ ] **Step 1: Write the failing test**

```rust
/// A plan is a statement about a moment. If rows appeared between the preview and the
/// restore, `replace` deletes what is there now — so the outcome has to report the
/// deletes it actually made, not repeat the plan's number.
#[tokio::test]
async fn restore_reports_the_rows_it_actually_deleted() {
    let (pool, uid) = db_with_user("restore-deleted-counts").await;
    let pid = seed_portfolio(&pool, uid, "Existing", "USD").await;
    let aid = seed_asset(&pool, pid, "EX", "Existing").await;
    seed_transaction(&pool, aid, "BUY", 1.0, 3.0, 0.0).await;
    seed_transaction(&pool, aid, "BUY", 1.0, 4.0, 0.0).await;

    let client = Client::tracked(build_rocket(pool)).await.unwrap();
    let res = client
        .post("/api/backup/restore?mode=replace")
        .header(common::session_header())
        .header(rocket::http::ContentType::JSON)
        .body(bundle_with_size(1, 1, 1))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let out: Value = res.into_json().await.unwrap();

    assert_eq!(out["deleted"]["portfolios"], 1);
    assert_eq!(out["deleted"]["assets"], 1);
    assert_eq!(out["deleted"]["transactions"], 2);
    assert_eq!(out["inserted"]["portfolios"], 1);
}

#[tokio::test]
async fn copy_mode_reports_nothing_deleted() {
    let (pool, uid) = db_with_user("restore-copy-no-delete").await;
    seed_portfolio(&pool, uid, "Kept", "USD").await;

    let client = Client::tracked(build_rocket(pool)).await.unwrap();
    let res = client
        .post("/api/backup/restore?mode=copy")
        .header(common::session_header())
        .header(rocket::http::ContentType::JSON)
        .body(bundle_with_size(1, 1, 1))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let out: Value = res.into_json().await.unwrap();

    assert_eq!(out["deleted"]["portfolios"], 0);
    assert_eq!(out["deleted"]["assets"], 0);
    assert_eq!(out["deleted"]["transactions"], 0);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test backup_test restore_reports_the_rows`
Expected: FAIL — `deleted` is absent from the response.

- [ ] **Step 3: Implement**

Add the field to `RestoreOutcome`:

```rust
    /// Rows the `replace` deletes actually removed, from `rows_affected`, not from
    /// the plan. A plan describes a moment and the database may have moved.
    pub deleted: Counts,
```

In `apply`, capture the three delete counts. Each existing statement becomes:

```rust
        let txns = sqlx::query!(
            "DELETE FROM transactions WHERE asset_id IN \
             (SELECT id FROM assets WHERE portfolio_id IN \
              (SELECT id FROM portfolios WHERE owner_id = $1))",
            user_id
        )
        .execute(&mut *tx)
        .await
        .map_err(classify)?;
        let assets = sqlx::query!(
            "DELETE FROM assets WHERE portfolio_id IN \
             (SELECT id FROM portfolios WHERE owner_id = $1)",
            user_id
        )
        .execute(&mut *tx)
        .await
        .map_err(classify)?;
        let ports = sqlx::query!("DELETE FROM portfolios WHERE owner_id = $1", user_id)
            .execute(&mut *tx)
            .await
            .map_err(classify)?;
        deleted = Counts {
            portfolios: ports.rows_affected() as usize,
            assets: assets.rows_affected() as usize,
            transactions: txns.rows_affected() as usize,
        };
```

Declare `let mut deleted = Counts::default();` before the `if mode == Mode::Replace` block and pass `deleted` into the returned `RestoreOutcome`. The `split_sync` delete stays uncounted — it is not one of the three tables a bundle has rows in, and the plan's warning already names it.

- [ ] **Step 4: Refresh the SQLx cache**

Run: `cargo sqlx prepare -- --all-targets`
Expected: the three cached `DELETE` entries change. Skipping this is not caught here — it surfaces at Task 10 as a `check_sqlx_cache.sh` failure with no pointer back to this task.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --test backup_test --test restore_concurrency_test`
Expected: PASS. `restore_concurrency_test.rs` constructs `RestoreOutcome` literals if so, add `deleted: Counts::default()` there.

- [ ] **Step 6: Commit**

```bash
git add backend_rust/src/backup/restore.rs backend_rust/tests/ backend_rust/.sqlx
git commit -m "feat(backup): the restore outcome reports the rows it actually deleted"
```

---

### Task 5: The readback route

**Files:**
- Modify: `backend_rust/src/api_routes/backup.rs`
- Modify: `backend_rust/src/lib.rs` (routes list), `backend_rust/src/openapi.rs` (paths list)
- Modify: `backend_rust/tests/route_coverage_test.rs`
- Modify: `backend_rust/tests/backup_detail_test.rs`

**Interfaces:**
- Consumes: `owned_detail`.
- Produces: `GET /api/backup/owned` → `Json<PlanSide>`.

- [ ] **Step 1: Write the failing test**

Append to `backend_rust/tests/backup_detail_test.rs`:

```rust
/// The route takes no id, so the only thing scoping it is `owner_id` inside
/// `owned_detail`. Two users in one database — see the note on the isolation test in
/// Task 2 for why a second `db_with_user` would not do.
#[tokio::test]
async fn owned_route_returns_the_callers_side() {
    let (pool, uid) = db_with_user("owned-route-isolation").await;
    let theirs = seed_user(&pool, "owned-route-theirs").await;
    let their_pid = seed_portfolio(&pool, theirs, "Theirs", "USD").await;
    seed_portfolio(&pool, uid, "Mine", "USD").await;

    let client = Client::tracked(build_rocket(pool)).await.unwrap();
    let res = client
        .get("/api/backup/owned")
        .header(common::session_header())
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let side: Value = res.into_json().await.unwrap();
    assert_eq!(side["counts"]["portfolios"], 1);
    assert_eq!(side["shown"][0]["name"], "Mine");
    assert!(their_pid > 0);
}

#[tokio::test]
async fn owned_route_needs_a_session() {
    let (pool, _uid) = db_with_user("owned-route-anon").await;
    let client = Client::tracked(build_rocket_oidc(pool)).await.unwrap();
    let res = client.get("/api/backup/owned").dispatch().await;
    assert_eq!(res.status(), Status::Unauthorized);
}

/// The claim the readback rests on: what the preview promised to insert is what the
/// database holds afterwards. If the two producers drift, this is where it shows.
#[tokio::test]
async fn after_a_restore_the_readback_matches_what_the_preview_promised() {
    let (pool, uid) = db_with_user("owned-route-roundtrip").await;
    let bundle = bundle_with_size(2, 3, 4);
    let parsed: Bundle = serde_json::from_str(&bundle).unwrap();
    let promised = detail_from_bundle(&parsed, today());

    let client = Client::tracked(build_rocket(pool.clone())).await.unwrap();
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
    // disagree on what a code means would pass every assertion above.
    assert_eq!(after.shown[0].asset_flags, promised.shown[0].asset_flags);
    assert_eq!(after.shown[0].flags, promised.shown[0].flags);
}
```

Add to the file's `use` block: `build_rocket`, `build_rocket_oidc`, `bundle_with_size` from
`common`; `backend_rust::backup::Bundle`; `backend_rust::backup::detail::{detail_from_bundle,
flag, owned_detail, MAX_ASSETS_SHOWN}`; `rocket::local::asynchronous::Client`;
`rocket::http::Status`; `serde_json::Value`. The `db_with_user` helper gives each test its own
database, which is why the two-user test above can seed both sides without interference.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test backup_detail_test`
Expected: FAIL — 404 on `/api/backup/owned`.

- [ ] **Step 3: Implement the route**

In `backend_rust/src/api_routes/backup.rs`:

```rust
/// The caller's own current state, in the shape the preview uses for both sides.
///
/// It exists for the panel's post-restore readback, and it is the same function
/// `preview` calls for its delete side — one implementation, two callers. It takes
/// no id, so there is nothing for a `require*` call to scope-check; the `owner_id`
/// filter inside `owned_detail` is the isolation, and `tests/backup_detail_test.rs`
/// pins it with two users. Rule C of the access gate is satisfied by `AuthedUser`.
#[utoipa::path(
    get,
    path = "/api/backup/owned",
    responses(
        (status = 200, description = "The caller's portfolios, assets and transaction spans", body = serde_json::Value),
        (status = 401, description = "No valid session", body = serde_json::Value),
        (status = 500, description = "The read failed", body = serde_json::Value),
        (status = 503, description = "The database is busy", body = serde_json::Value),
    )
)]
#[get("/backup/owned")]
pub async fn owned(
    user: AuthedUser,
    pool: &State<DbPool>,
) -> Result<Json<super::detail::PlanSide>, BackupError> {
    let today = chrono::Utc::now().date_naive();
    crate::backup::detail::owned_detail(pool.inner(), user.id, today)
        .await
        .map(Json)
        .map_err(|e| match e {
            DbFailure::Lock => lock_response(),
            DbFailure::Missing => fail(Status::NotFound, "portfolio not found"),
            DbFailure::Other => fail(DbFailure::Other.status(), "could not read the current data"),
        })
}
```

Register it in `src/lib.rs` next to the other backup routes (`api_routes::backup::owned`) and in `src/openapi.rs`'s paths list (`crate::api_routes::backup::owned`). Update the preview's `utoipa` response description to name the new shape: `"The plan: mode, delete side, insert side, prices, warnings"`.

- [ ] **Step 4: Add it to the route inventory**

In `backend_rust/tests/route_coverage_test.rs`, add to `PROTECTED_ROUTES` beside the other backup entries:

```rust
    // Reads the caller's own portfolios and nothing else; scoped by `owner_id`
    // inside `owned_detail`, pinned by two users in `tests/backup_detail_test.rs`.
    ("GET", "/api/backup/owned", None),
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --test backup_detail_test --test route_coverage_test`
Expected: PASS. If `route_coverage_test` fails on the published document, the `openapi.rs` registration is missing.

- [ ] **Step 6: Commit**

```bash
git add backend_rust/src/api_routes/backup.rs backend_rust/src/lib.rs backend_rust/src/openapi.rs backend_rust/tests/
git commit -m "feat(backup): a route for what the caller owns, so the panel can read back a restore"
```

---

### Task 6: The detail component

**Files:**
- Create: `frontend/src/components/BackupPlanDetail.tsx`
- Create: `frontend/src/test/components/BackupPlanDetail.test.tsx`

**Interfaces:**
- Consumes: nothing.
- Produces: `PlanSideView({ title, side })`, `countList(counts)`, `isCounts(value)`, `isPlanSide(value): value is PlanSide`, and the exported types `Counts`, `PlanAsset`, `PlanPortfolio`, `PlanSide`.

- [ ] **Step 1: Write the failing tests**

Create `frontend/src/test/components/BackupPlanDetail.test.tsx`:

```tsx
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { PlanSideView, isPlanSide, type PlanSide } from '../../components/BackupPlanDetail';

const asset = (over: Record<string, unknown> = {}) => ({
  symbol: 'AAA', name: 'Alpha', currency: 'USD', transactions: 2,
  first_date: '2026-01-05', last_date: '2026-02-05', zero_price: 0, flags: [], ...over,
});

const side = (over: Record<string, unknown> = {}): PlanSide => ({
  counts: { portfolios: 1, assets: 1, transactions: 2 },
  shown: [{
    name: 'Alpha Fund', currency: 'USD', assets: 1, transactions: 2,
    shown: [asset()], more_assets: 0, asset_flags: {}, flags: [],
  }],
  more_portfolios: 0,
  transactions_in_shown: 2,
  ...over,
} as PlanSide);

describe('PlanSideView', () => {
  it('names the portfolio and its counts', () => {
    render(<PlanSideView title="Will insert" side={side()} />);
    expect(screen.getByText(/Will insert 1 portfolio, 1 asset, 2 transactions/)).toBeInTheDocument();
    expect(screen.getByText(/Alpha Fund/)).toBeInTheDocument();
    expect(screen.getByText(/AAA/)).toBeInTheDocument();
  });

  it('says what is not listed, without changing what is counted', () => {
    // 25 shown against 1,204 counted. The cap line has to name both, and the counts
    // line has to stay the server's total rather than the sample's.
    const shown25 = Array.from({ length: 25 }, (_, i) => ({ ...side().shown[0], name: `P${i}` }));
    render(
      <PlanSideView
        title="Will insert"
        side={side({
          counts: { portfolios: 1204, assets: 9120, transactions: 9120 },
          shown: shown25,
          more_portfolios: 1179,
          transactions_in_shown: 400,
        })}
      />,
    );
    expect(screen.getByText(/showing 25 of 1204 portfolios/)).toBeInTheDocument();
    expect(screen.getByText(/the portfolios shown hold 400 of 9120 transactions/)).toBeInTheDocument();
  });

  it('collapses a portfolio with more than five assets', () => {
    const many = Array.from({ length: 6 }, (_, i) => asset({ symbol: `S${i}` }));
    render(
      <PlanSideView
        title="Will insert"
        side={side({
          shown: [{ name: 'Wide', currency: 'USD', assets: 6, transactions: 12, shown: many, more_assets: 0, asset_flags: {}, flags: [] }],
        })}
      />,
    );
    const block = document.querySelector('details[data-testid="plan-portfolio"]');
    expect(block).not.toBeNull();
    expect(block?.hasAttribute('open')).toBe(false);
  });

  it('leaves a small portfolio open', () => {
    render(<PlanSideView title="Will insert" side={side()} />);
    expect(document.querySelector('details[data-testid="plan-portfolio"]')).toBeNull();
  });

  /** The collapse rule would otherwise lose this: a flag must be visible without
   * expanding anything, or it was never raised. */
  it('shows aggregated asset flags on a collapsed header', () => {
    const many = Array.from({ length: 6 }, (_, i) => asset({ symbol: `S${i}` }));
    render(
      <PlanSideView
        title="Will delete"
        side={side({
          shown: [{
            name: 'Wide', currency: 'USD', assets: 318, transactions: 9120, shown: many,
            more_assets: 312, asset_flags: { zero_price: 3, future_dated: 1 }, flags: [],
          }],
        })}
      />,
    );
    const summary = screen.getByText(/Wide/).closest('summary') ?? document.body;
    expect(summary.textContent).toContain('zero-price ×3');
    expect(summary.textContent).toContain('future-dated ×1');
  });

  it('renders a flag code it does not know instead of throwing', () => {
    render(
      <PlanSideView
        title="Will insert"
        side={side({ shown: [{ ...side().shown[0], flags: ['made_up_flag'] }] })}
      />,
    );
    expect(screen.getByText(/made_up_flag/)).toBeInTheDocument();
  });

  it('marks a portfolio with no assets', () => {
    render(
      <PlanSideView
        title="Will insert"
        side={side({
          counts: { portfolios: 1, assets: 0, transactions: 0 },
          shown: [{ name: 'Hollow', currency: 'USD', assets: 0, transactions: 0, shown: [], more_assets: 0, asset_flags: {}, flags: ['no_assets'] }],
          transactions_in_shown: 0,
        })}
      />,
    );
    expect(screen.getByText(/no assets/)).toBeInTheDocument();
  });
});

describe('isPlanSide', () => {
  it('accepts a real side', () => {
    expect(isPlanSide(side())).toBe(true);
  });

  it('refuses a body that is not a side', () => {
    expect(isPlanSide(null)).toBe(false);
    expect(isPlanSide({})).toBe(false);
    expect(isPlanSide({ ...side(), shown: 'nope' })).toBe(false);
    expect(isPlanSide({ ...side(), shown: [{ name: 'x' }] })).toBe(false);
    expect(isPlanSide({ ...side(), counts: { portfolios: 1 } })).toBe(false);
  });
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npm test -- BackupPlanDetail`
Expected: FAIL — cannot resolve `../../components/BackupPlanDetail`.

- [ ] **Step 3: Implement**

Create `frontend/src/components/BackupPlanDetail.tsx`:

```tsx
import type { ReactNode } from 'react';

/** The three counts a plan side carries. Mirrors `backup::restore::Counts`. */
export type Counts = { portfolios: number; assets: number; transactions: number };

export type PlanAsset = {
  symbol: string;
  name: string;
  currency: string;
  transactions: number;
  first_date: string | null;
  last_date: string | null;
  zero_price: number;
  flags: string[];
};

export type PlanPortfolio = {
  name: string;
  currency: string;
  /** Exact, over every asset — not `shown.length`. */
  assets: number;
  transactions: number;
  shown: PlanAsset[];
  more_assets: number;
  /** Flag counts over every asset, so a collapsed header cannot hide one. */
  asset_flags: Record<string, number>;
  flags: string[];
};

export type PlanSide = {
  counts: Counts;
  shown: PlanPortfolio[];
  more_portfolios: number;
  transactions_in_shown: number;
};

/** Above this many assets a portfolio starts collapsed. A display choice, not a
 * contract: 25 portfolios x 50 assets is a wall, and a wall is not verification. */
const COLLAPSE_ABOVE = 5;

/** English for a flag code. An unknown code renders as itself, so a flag added
 * server-side degrades into something readable rather than a crash. */
const FLAG_LABELS: Record<string, string> = {
  no_assets: 'no assets',
  no_transactions: 'no transactions',
  zero_price: 'zero-price',
  future_dated: 'future-dated',
  currency_mismatch: 'currency mismatch',
  duplicate_symbol: 'duplicate symbol',
};

export function flagLabel(code: string, count?: number): string {
  const label = FLAG_LABELS[code] ?? code;
  return count === undefined ? label : `${label} ×${count}`;
}

/** `{n} {noun}` with the plural spelled out. The counts are the server's; only the
 * English is here. */
function countList(counts: Counts): string {
  const named = (n: number, noun: string) => `${n} ${noun}${n === 1 ? '' : 's'}`;
  return [
    named(counts.portfolios, 'portfolio'),
    named(counts.assets, 'asset'),
    named(counts.transactions, 'transaction'),
  ].join(', ');
}

export function isCounts(value: unknown): value is Counts {
  const c = value as Partial<Counts> | null;
  return !!c && typeof c === 'object'
    && Number.isFinite(c.portfolios) && Number.isFinite(c.assets) && Number.isFinite(c.transactions);
}

function isAsset(value: unknown): value is PlanAsset {
  const a = value as Partial<PlanAsset> | null;
  return !!a && typeof a === 'object' && typeof a.symbol === 'string'
    && typeof a.name === 'string' && typeof a.currency === 'string'
    && Number.isFinite(a.transactions) && Number.isFinite(a.zero_price)
    && (a.first_date === null || typeof a.first_date === 'string')
    && (a.last_date === null || typeof a.last_date === 'string')
    && Array.isArray(a.flags) && a.flags.every((f) => typeof f === 'string');
}

function isPortfolio(value: unknown): value is PlanPortfolio {
  const p = value as Partial<PlanPortfolio> | null;
  return !!p && typeof p === 'object' && typeof p.name === 'string' && typeof p.currency === 'string'
    && Number.isFinite(p.assets) && Number.isFinite(p.transactions) && Number.isFinite(p.more_assets)
    && Array.isArray(p.shown) && p.shown.every(isAsset)
    && Array.isArray(p.flags) && p.flags.every((f) => typeof f === 'string')
    && !!p.asset_flags && typeof p.asset_flags === 'object'
    && Object.values(p.asset_flags).every((n) => Number.isFinite(n));
}

/** A plan side off an untrusted body, or `null`. The reason it refuses rather than
 * casts is the reason `asPlan` exists in `BackupPanel`: a thrown render is a blank
 * page, and a blank page is the worst possible answer at the moment someone is
 * deciding whether to delete their data. */
export function isPlanSide(value: unknown): value is PlanSide {
  const s = value as Partial<PlanSide> | null;
  return !!s && typeof s === 'object' && isCounts(s.counts)
    && Array.isArray(s.shown) && s.shown.every(isPortfolio)
    && Number.isFinite(s.more_portfolios) && Number.isFinite(s.transactions_in_shown);
}

function FlagList({ codes, counts }: { codes: string[]; counts?: Record<string, number> }) {
  if (codes.length === 0) return null;
  return (
    <span data-testid="plan-flags" className="text-muted">
      {codes.map((c) => flagLabel(c, counts?.[c])).join(' · ')}
    </span>
  );
}

function AssetRow({ asset }: { asset: PlanAsset }) {
  const span = asset.first_date && asset.last_date
    ? `${asset.first_date} – ${asset.last_date}`
    : 'no transactions';
  return (
    <div data-testid="plan-asset" style={{ paddingLeft: '12px' }}>
      <strong>{asset.symbol}</strong>
      {` ${asset.name} · ${asset.currency} · ${asset.transactions} txn`}
      {` · ${span}`}
      {' '}
      <FlagList codes={asset.flags} counts={{ zero_price: asset.zero_price }} />
    </div>
  );
}

function PortfolioBlock({ portfolio }: { portfolio: PlanPortfolio }) {
  const one = (n: number, noun: string) => `${n} ${noun}${n === 1 ? '' : 's'}`;
  const header = (
    <span data-testid="plan-portfolio-header">
      <strong>{portfolio.name}</strong>
      {` · ${portfolio.currency} · ${one(portfolio.assets, 'asset')} · ${one(portfolio.transactions, 'transaction')}`}
      {' '}
      <FlagList codes={portfolio.flags} />
      <FlagList
        codes={Object.keys(portfolio.asset_flags)}
        counts={portfolio.asset_flags}
      />
    </span>
  );

  const rows: ReactNode[] = portfolio.shown.map((a, i) => <AssetRow key={`${a.symbol}-${i}`} asset={a} />);
  const more = portfolio.more_assets > 0
    ? <div className="text-muted">{`and ${portfolio.more_assets} more asset${portfolio.more_assets === 1 ? '' : 's'}`}</div>
    : null;

  if (portfolio.shown.length <= COLLAPSE_ABOVE) {
    return (
      <div data-testid="plan-portfolio" style={{ marginBottom: '8px' }}>
        <div>{header}</div>
        {rows}
        {more}
      </div>
    );
  }

  return (
    <details data-testid="plan-portfolio" style={{ marginBottom: '8px' }}>
      <summary>{header}</summary>
      {rows}
      {more}
    </details>
  );
}

/** One side of a plan — what a bundle holds, or what the database holds. Rendered
 * three times by `BackupPanel`, which is the whole reason it is a component. */
export function PlanSideView({ title, side }: { title: string; side: PlanSide }) {
  return (
    <div data-testid="plan-side">
      <p style={{ marginBottom: '4px' }}>{`${title} ${countList(side.counts)}.`}</p>
      {side.more_portfolios > 0 && (
        <p className="text-muted" data-testid="plan-cap">
          {`showing ${side.shown.length} of ${side.counts.portfolios} portfolios · `
            + `the portfolios shown hold ${side.transactions_in_shown} of ${side.counts.transactions} transactions`}
        </p>
      )}
      {side.shown.map((p, i) => <PortfolioBlock key={`${p.name}-${i}`} portfolio={p} />)}
    </div>
  );
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `npm test -- BackupPlanDetail`
Expected: PASS, 9 tests.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/BackupPlanDetail.tsx frontend/src/test/components/BackupPlanDetail.test.tsx
git commit -m "feat(ui): one component for both halves of a backup plan"
```

---

### Task 7: Wire the panel to the new plan

**Files:**
- Modify: `frontend/src/components/BackupPanel.tsx`
- Modify: `frontend/src/test/components/BackupPanel.test.tsx`

**Interfaces:**
- Consumes: `PlanSideView`, `isPlanSide`, `countList`, types from `BackupPlanDetail.tsx`; `Plan.delete`/`Plan.insert` as `PlanSide` (Task 3).
- Produces: the panel's `Plan` type with `PlanSide` halves; `data-testid="preview-delete"` and `data-testid="preview-insert"`.

- [ ] **Step 1: Write the failing tests**

Append to `frontend/src/test/components/BackupPanel.test.tsx`, using the file's existing
`fetchMock` / `jsonResponse` / `bundleFile()` / `portfolios` fixtures:

```tsx
/** The body both halves of the plan now carry. Built here rather than imported from
 * the component's test helper so a change to the server shape lands as a failing
 * assertion, not as a shared fixture that quietly moves with it. */
const sideFixture = (name = 'Alpha Fund') => ({
  counts: { portfolios: 1, assets: 1, transactions: 2 },
  shown: [{
    name, currency: 'USD', assets: 1, transactions: 2,
    shown: [{ symbol: 'AAA', name: 'Alpha', currency: 'USD', transactions: 2,
              first_date: '2026-01-05', last_date: '2026-02-05', zero_price: 0, flags: [] }],
    more_assets: 0, asset_flags: {}, flags: [],
  }],
  more_portfolios: 0, transactions_in_shown: 2,
})
const emptySide = () => ({
  counts: { portfolios: 0, assets: 0, transactions: 0 },
  shown: [], more_portfolios: 0, transactions_in_shown: 0,
})

it('names the portfolios and the securities on both sides', async () => {
  fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
    if (String(input).includes('/backup/preview')) {
      return jsonResponse({ schema_version: 1, mode: 'copy', prices: 0, warnings: [],
        insert: sideFixture(), delete: emptySide() })
    }
    return jsonResponse({}, 404)
  })
  render(<BackupPanel portfolios={portfolios} />)
  await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())

  await waitFor(() => expect(screen.getByTestId('preview-insert')).toBeInTheDocument())
  expect(screen.getByTestId('preview-insert')).toHaveTextContent('Alpha Fund')
  expect(screen.getByTestId('preview-insert')).toHaveTextContent('AAA')
  expect(screen.getByTestId('preview-delete')).toBeInTheDocument()
})

/** A 200 that is not a plan — a proxy envelope, a truncated body, a renamed field.
 * Rendering half of one means reading `.map` off `undefined`, and with no error
 * boundary in this app a throw during render is a blank page at the worst possible
 * moment. The existing guard catches this; this test keeps it there. */
it('refuses a 200 whose body is not a plan, and does not arm the write', async () => {
  fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
    if (String(input).includes('/backup/preview')) {
      return jsonResponse({ data: { message: 'ok' } })
    }
    return jsonResponse({}, 404)
  })
  render(<BackupPanel portfolios={portfolios} />)
  await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())

  await waitFor(() => expect(screen.getByTestId('restore-error'))
    .toHaveTextContent('not a plan this app could read'))
  expect(screen.queryByTestId('preview-insert')).not.toBeInTheDocument()
  expect(screen.getByTestId('restore-button')).not.toBeEnabled()
})

it('describes price history as an operator flow, not a missing feature', async () => {
  fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
    if (String(input).includes('/backup/preview')) {
      return jsonResponse({ schema_version: 1, mode: 'copy', prices: 1234, warnings: [],
        insert: sideFixture(), delete: emptySide() })
    }
    return jsonResponse({}, 404)
  })
  render(<BackupPanel portfolios={portfolios} />)
  await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())

  expect(await screen.findByTestId('preview-prices')).toHaveTextContent(
    'restored by an operator, not from this panel')
  expect(screen.queryByText(/will be left alone/)).not.toBeInTheDocument()
})
```

The existing assertion `'Will delete 2 portfolios, 4 assets, 6 transactions.'` keeps
passing unchanged: `PlanSideView`'s first line is `${title} ${countList(side.counts)}.`,
which is the same sentence. That is deliberate — the detail is added underneath the
summary, not instead of it.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npm test -- BackupPanel`
Expected: FAIL — `preview-insert` not found, and the prices copy is the old sentence.

- [ ] **Step 3: Implement**

In `BackupPanel.tsx`:

```tsx
import { PlanSideView, isPlanSide, isCounts, countList, type Counts, type PlanSide } from './BackupPlanDetail';
```

Delete the local `countList` and `isCounts` — they now live in the component file, and
`isCounts` is still needed in `runRestore` for the `inserted` field of the outcome.

and change the `Plan` type:

```tsx
type Plan = {
  schema_version: number;
  mode: RestoreMode;
  delete: PlanSide;
  insert: PlanSide;
  prices: number;
  warnings: string[];
};
```

`asPlan` keeps its shape but delegates the halves:

```tsx
function asPlan(body: unknown, askedFor: RestoreMode): Plan | null {
  const p = body as Partial<Plan> | null;
  if (!p || typeof p !== 'object') return null;
  if (p.mode !== askedFor) return null;
  if (!isPlanSide(p.delete) || !isPlanSide(p.insert)) return null;
  if (!Number.isFinite(p.prices)) return null;
  if (!Array.isArray(p.warnings) || !p.warnings.every((w) => typeof w === 'string')) return null;
  return p as Plan;
}
```

Replace the plan block's two `<p>` sentences with:

```tsx
<div data-testid="preview-delete">
  {plan.mode === 'replace'
    ? <PlanSideView title="Will delete" side={plan.delete} />
    : <p>Will delete nothing.</p>}
</div>
<div data-testid="preview-insert">
  <PlanSideView title="Will insert" side={plan.insert} />
</div>
```

and the prices sentence:

```tsx
{plan.prices > 0 && (
  <p data-testid="preview-prices" className="text-muted">
    {`This bundle carries ${plan.prices} price row${plan.prices === 1 ? '' : 's'}. `
      + 'Price history is shared market data and is restored by an operator, not from this panel.'}
  </p>
)}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `npm test -- BackupPanel`
Expected: PASS. Any existing assertion on the old `Will insert 1 portfolio…` string moves to the `preview-insert` element.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/BackupPanel.tsx frontend/src/test/components/BackupPanel.test.tsx
git commit -m "feat(ui): the preview names the portfolios and securities on both sides

Price history is described as the operator flow it is, not as something this
panel is one checkbox short of."
```

---

### Task 8: Read back after the restore

**Files:**
- Modify: `frontend/src/components/BackupPanel.tsx`
- Modify: `frontend/src/test/components/BackupPanel.test.tsx`

**Interfaces:**
- Consumes: `GET /api/backup/owned` (Task 5), `isPlanSide`.
- Produces: `data-testid="restore-owned"`, `data-testid="restore-owned-failed"`.

- [ ] **Step 1: Write the failing tests**

```tsx
it('shows what the database holds after a restore', async () => {
  fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
    if (String(input).includes('/backup/restore')) {
      return jsonResponse({ inserted: { portfolios: 1, assets: 1, transactions: 2 },
        deleted: { portfolios: 0, assets: 0, transactions: 0 },
        portfolio_ids: [7], prices_inserted: 0 })
    }
    if (String(input).includes('/backup/owned')) {
      return jsonResponse(sideFixture('Renamed After Restore'))
    }
    return jsonResponse({}, 404)
  })
  render(<BackupPanel portfolios={portfolios} />)
  await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
  await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())
  await userEvent.click(screen.getByTestId('restore-button'))

  await waitFor(() => expect(screen.getByTestId('restore-owned'))
    .toHaveTextContent('Renamed After Restore'))
})

/** A failed readback is not a failed restore. The write committed, and a line saying
 * otherwise is how someone restores the same bundle twice. */
it('keeps a successful restore successful when the readback fails', async () => {
  fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
    if (String(input).includes('/backup/restore')) {
      return jsonResponse({ inserted: { portfolios: 1, assets: 1, transactions: 2 },
        deleted: { portfolios: 0, assets: 0, transactions: 0 },
        portfolio_ids: [7], prices_inserted: 0 })
    }
    if (String(input).includes('/backup/owned')) {
      return jsonResponse({ error: 'the database is busy' }, 503)
    }
    return jsonResponse({}, 404)
  })
  render(<BackupPanel portfolios={portfolios} />)
  await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
  await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())
  await userEvent.click(screen.getByTestId('restore-button'))

  await waitFor(() => expect(screen.getByTestId('restore-result')).toBeInTheDocument())
  expect(screen.getByTestId('restore-owned-failed')).toHaveTextContent(/could not be read back/)
  expect(screen.queryByTestId('restore-error')).not.toBeInTheDocument()
})
```

Existing restore tests mock every unmatched path as a 404, so their readback fails and
they will start rendering `restore-owned-failed`. That is correct behaviour, not a
regression — but add an `/backup/owned` branch to any existing restore test that asserts
on the result block, so it is testing the readback it means to.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npm test -- BackupPanel`
Expected: FAIL — `restore-owned` not found.

- [ ] **Step 3: Implement**

Add state beside the existing `restoreResult`:

```tsx
const [owned, setOwned] = useState<PlanSide | null>(null);
const [ownedFailed, setOwnedFailed] = useState(false);
```

After `setRestoreResult({ inserted: written })` in `runRestore`:

```tsx
      // The write is committed and the panel is now describing it, so the readback is
      // decoration: if it fails, the counts the restore itself returned stay on
      // screen with one honest line beside them. Nothing here may make a committed
      // write look like a failure — that is how the same bundle gets restored twice.
      try {
        const back = await apiFetch(`${API_BASE}/backup/owned`);
        const body = back.ok ? (await back.json()) as unknown : null;
        if (isPlanSide(body)) setOwned(body);
        else setOwnedFailed(true);
      } catch {
        setOwnedFailed(true);
      }
```

Clear both where `setRestoreResult(null)` is already called. Render after the result line:

```tsx
{restoreResult && owned && (
  <div data-testid="restore-owned" style={{ marginTop: '12px' }}>
    <PlanSideView title="Your portfolios now" side={owned} />
  </div>
)}
{restoreResult && ownedFailed && (
  <p data-testid="restore-owned-failed" className="text-muted" style={{ marginTop: '8px' }}>
    The restore succeeded, but your data could not be read back to confirm it. Reload to see what is there.
  </p>
)}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `npm test -- BackupPanel`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/BackupPanel.tsx frontend/src/test/components/BackupPanel.test.tsx
git commit -m "feat(ui): read the database back after a restore, and fail the readback quietly"
```

---

### Task 9: Document and check what the plan now carries

**Files:**
- Modify: `docs/openapi/openapi.json`
- Check only: `backend_rust/src/backup/restore.rs`, `backend_rust/src/api_routes/backup.rs`

**Interfaces:**
- Consumes: everything above.
- Produces: a published document that names the new route; a recorded answer to "does anything log names".

- [ ] **Step 1: Regenerate the document**

```bash
cd backend_rust
cargo run --bin backend_rust -- --print-openapi > ../docs/openapi/openapi.json
```

Expected: the diff adds `/api/backup/owned` and touches the preview description. It must not touch the bundle schema.

- [ ] **Step 2: Check nothing logs names**

```bash
cd backend_rust
grep -rn "eprintln!\|println!" src/backup/ src/api_routes/backup.rs
```

Expected: the only lines are the existing `apply` version log (counts and mode only) and any startup warnings. A plan or `RestoreOutcome` serialised into a log line would put portfolio and asset names in the container log, where they outlive the request. If one exists, remove the names from it and add a test asserting the log line contains counts and not names.

- [ ] **Step 3: Commit**

```bash
git add docs/openapi/openapi.json
git commit -m "docs(openapi): publish the owned route"
```

---

### Task 10: Full gate

**Files:** none new.

- [ ] **Step 1: Backend**

```bash
cd backend_rust
cargo test
bash scripts/check_access_gate.sh
bash scripts/check_pricing_seam.sh
bash scripts/check_backup_portability.sh
bash scripts/check_upload_path.sh
bash scripts/check_migrations.sh
bash scripts/check_sqlx_cache.sh
cargo clippy --all-targets
```

Expected: `cargo test` green across every binary; every gate prints PASS; clippy adds no warning naming `detail.rs` or the new tests.

- [ ] **Step 2: Frontend**

```bash
cd frontend
npm run lint
npm test
```

Expected: lint clean apart from the pre-existing `CsvImportModal` warning; tests green.

- [ ] **Step 3: Prove the new gate assertions bite**

Temporarily change `MAX_ASSETS_SHOWN` to `usize::MAX` and run `cargo test --lib backup::detail`; the truncation tests must fail. Temporarily compute `asset_flags` from `shown` instead of every asset; `asset_flags_are_counted_over_every_asset_not_just_the_shown_ones` and `asset_flags_cover_assets_past_the_cap` must both fail. Revert both.

- [ ] **Step 4: Open the PR**

Branch `feat/import-preview`, one PR covering backend and frontend. The body names: the two-sided plan, the readback route, the flags and their non-blocking status, the caps, and the price-history rewording. No real symbols, counts, dates or hostnames anywhere in it.

## Out of Scope

Price apply in the panel, paging or show-all, transaction-level listing, `negative_quantity`, expected-versus-found reconciliation, async or resumable import, and any change to the bundle format.
