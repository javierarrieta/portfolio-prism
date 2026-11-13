//! Counting, planning, and applying a bundle. All restore SQL lives here.
//!
//! Two rules hold throughout. **Bundle refs are never database ids** — they are
//! local labels used only to rewire parent/child links, and nothing here matches
//! one against a stored row. **The whole restore is one transaction** — a bundle
//! that fails anywhere writes nothing.
//!
//! Task 5 uses only the counting half: [`plan_for`] reads, and writes nothing. It
//! is what the user is shown before a restore is allowed, so its numbers are the
//! promise the write has to keep.

use serde::Serialize;
use sqlx::SqlitePool;
use crate::backup::Bundle;
use crate::backup::export::{classify, DbFailure};

/// What to do with what the caller already owns. Merge is deliberately absent:
/// "which transaction is a duplicate?" has no clean answer, and a wrong guess
/// silently corrupts a tax ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Copy,
    Replace,
}

impl Mode {
    /// `None` for anything other than exactly `copy` or `replace`, so an
    /// unrecognised mode is a `422` rather than a default.
    pub fn from_query(raw: Option<&str>) -> Option<Mode> {
        match raw.map(str::trim) {
            Some("copy") => Some(Mode::Copy),
            Some("replace") => Some(Mode::Replace),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Copy => "copy",
            Mode::Replace => "replace",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Counts {
    pub portfolios: usize,
    pub assets: usize,
    pub transactions: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub schema_version: i64,
    pub mode: String,
    pub delete: Counts,
    pub insert: Counts,
    pub prices: usize,
    pub warnings: Vec<String>,
}

/// Rows the caller owns today — the exact set `replace` would delete.
///
/// Scoped by `owner_id` and by nothing else: every count a foreign bundle can
/// provoke is a count of the caller's own rows, because the caller's id is the
/// only filter any of these three queries has.
pub async fn owned_counts(pool: &SqlitePool, user_id: i32) -> Result<Counts, DbFailure> {
    let portfolios: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM portfolios WHERE owner_id = ?")
        .bind(user_id as i64)
        .fetch_one(pool)
        .await
        .map_err(classify)?;

    let assets: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM assets WHERE portfolio_id IN \
         (SELECT id FROM portfolios WHERE owner_id = ?)",
    )
    .bind(user_id as i64)
    .fetch_one(pool)
    .await
    .map_err(classify)?;

    let transactions: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM transactions WHERE asset_id IN \
         (SELECT id FROM assets WHERE portfolio_id IN \
          (SELECT id FROM portfolios WHERE owner_id = ?))",
    )
    .bind(user_id as i64)
    .fetch_one(pool)
    .await
    .map_err(classify)?;

    Ok(Counts {
        portfolios: portfolios.0 as usize,
        assets: assets.0 as usize,
        transactions: transactions.0 as usize,
    })
}

/// Counts in the bundle, which is what the UI shows as "will insert".
pub fn bundle_counts(bundle: &Bundle) -> Counts {
    let mut c = Counts {
        portfolios: bundle.portfolios.len(),
        assets: 0,
        transactions: 0,
    };
    for p in &bundle.portfolios {
        c.assets += p.assets.len();
        for a in &p.assets {
            c.transactions += a.transactions.len();
        }
    }
    c
}

/// The plan the user must see before restoring. Reads only; writes nothing.
pub async fn plan_for(
    pool: &SqlitePool,
    user_id: i32,
    bundle: &Bundle,
    mode: Mode,
) -> Result<Plan, DbFailure> {
    let delete = match mode {
        Mode::Copy => Counts::default(),
        Mode::Replace => owned_counts(pool, user_id).await?,
    };

    let mut warnings = Vec::new();
    if mode == Mode::Replace && delete.portfolios > 0 {
        warnings.push(format!(
            "replace will delete all {} portfolio(s) you own, including any not in this bundle",
            delete.portfolios
        ));
    }
    if !bundle.prices.is_empty() {
        warnings.push(
            "this bundle carries price history; it is shared market data, not your data".into(),
        );
    }

    Ok(Plan {
        schema_version: bundle.schema_version,
        mode: mode.as_str().to_string(),
        delete,
        insert: bundle_counts(bundle),
        prices: bundle.prices.len(),
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::{
        BundleAsset, BundlePortfolio, BundleTransaction, CURRENT_SCHEMA_VERSION, FORMAT_TAG,
        row_count,
    };

    #[test]
    fn only_the_two_modes_parse_and_nothing_else_does() {
        // Including `None`: there is no default mode, so a request that named none
        // gets a 422 rather than the mode this build happens to prefer.
        assert_eq!(Mode::from_query(Some("copy")), Some(Mode::Copy));
        assert_eq!(Mode::from_query(Some("replace")), Some(Mode::Replace));
        assert_eq!(Mode::from_query(Some(" copy ")), Some(Mode::Copy));
        for raw in ["", " ", "merge", "COPY", "Replace", "copying"] {
            assert_eq!(Mode::from_query(Some(raw)), None, "{raw:?}");
        }
        assert_eq!(Mode::from_query(None), None);
        assert_eq!(Mode::Copy.as_str(), "copy");
        assert_eq!(Mode::Replace.as_str(), "replace");
    }

    /// One portfolio, two assets, one transaction on the first of them.
    fn bundle() -> Bundle {
        let tx = BundleTransaction {
            r#type: "BUY".into(),
            quantity: "1".into(),
            price: "1".into(),
            fee: "0".into(),
            date: "2024-01-01T00:00:00Z".into(),
        };
        Bundle {
            format: FORMAT_TAG.to_string(),
            schema_version: CURRENT_SCHEMA_VERSION,
            created_at: "2026-11-13T00:00:00Z".to_string(),
            portfolios: vec![BundlePortfolio {
                bundle_ref: "p1".into(),
                name: "P".into(),
                description: None,
                currency: "USD".into(),
                base_currency: "USD".into(),
                assets: vec![
                    BundleAsset {
                        bundle_ref: "a1".into(),
                        symbol: "S".into(),
                        name: "S".into(),
                        asset_type: "STOCK".into(),
                        sector: None,
                        currency: "USD".into(),
                        isin: None,
                        transactions: vec![tx],
                    },
                    BundleAsset {
                        bundle_ref: "a2".into(),
                        symbol: "T".into(),
                        name: "T".into(),
                        asset_type: "STOCK".into(),
                        sector: None,
                        currency: "USD".into(),
                        isin: None,
                        transactions: vec![],
                    },
                ],
            }],
            prices: vec![],
        }
    }

    #[test]
    fn bundle_counts_counts_every_level_of_the_bundle() {
        // The same three levels `row_count` walks on the way in. If the two ever
        // disagree, the plan promises a different number of rows than validation
        // counted, and the user is shown one file and told about another.
        let bundle = bundle();
        let c = bundle_counts(&bundle);
        assert_eq!((c.portfolios, c.assets, c.transactions), (1, 2, 1));
        assert_eq!(
            row_count(&bundle),
            c.portfolios + c.assets + c.transactions,
            "row_count counts a portfolio row for each portfolio; these counts must agree"
        );
    }
}
