//! The backup bundle: its format, its versioning, and the code that reads and
//! writes it.
//!
//! Everything SQL in this module is written to the portability discipline in
//! `docs/superpowers/specs/2026-11-13-backup-export-restore-design.md`
//! (*Portability discipline*) so that the engine-selection work is a mechanical
//! change here rather than a rewrite. `scripts/check_backup_portability.sh`
//! enforces it statically; do not weaken that script to make a statement fit.

pub mod export;
pub mod restore;
pub mod schema;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The literal `format` every bundle must carry. A file that is not a bundle
/// fails here, with a clear message, rather than at the first missing field.
pub const FORMAT_TAG: &str = "portfolio-prism-backup";

/// The schema versions this build accepts. `v1.json` is never edited: a format
/// change adds `v2.json` plus a `v1 -> v2` upgrader and appends `2` here.
pub const ACCEPTED_VERSIONS: &[i64] = &[1];

/// Application-level ceiling on an uploaded bundle. Exceeding it is a `422` that
/// names the limit, not a `413` from the framework. Rocket's own `data` limit is
/// deliberately larger (see `lib.rs`) so this check is the one that answers.
pub const MAX_BUNDLE_BYTES: usize = 8 * 1024 * 1024;

/// Ceiling on `portfolios + assets + transactions` in one bundle.
pub const MAX_BUNDLE_ROWS: usize = 50_000;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub format: String,
    pub schema_version: i64,
    /// RFC 3339 UTC, informational. Excluded from the round-trip equality test.
    pub created_at: String,
    pub portfolios: Vec<BundlePortfolio>,
    /// Absent unless requested at export time. Flat, because price rows are
    /// global and belong to no portfolio.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prices: Vec<BundlePrice>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BundlePortfolio {
    /// Bundle-local label (`p1`, `p2`), unique within the bundle, used only to
    /// rewire parent/child links during restore. Never a database id, never
    /// persisted. `ref` is a Rust keyword, hence the rename.
    #[serde(rename = "ref")]
    pub bundle_ref: String,
    pub name: String,
    pub description: Option<String>,
    pub currency: String,
    pub base_currency: String,
    pub assets: Vec<BundleAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BundleAsset {
    #[serde(rename = "ref")]
    pub bundle_ref: String,
    pub symbol: String,
    pub name: String,
    pub asset_type: String,
    pub sector: Option<String>,
    pub currency: String,
    pub isin: Option<String>,
    pub transactions: Vec<BundleTransaction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BundleTransaction {
    pub r#type: String,
    /// Decimal as a string, matching the TEXT column and `models::Transaction`.
    pub quantity: String,
    pub price: String,
    pub fee: String,
    /// RFC 3339 UTC datetime, matching `models::Transaction::date`.
    pub date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BundlePrice {
    pub symbol: String,
    /// `YYYY-MM-DD`, matching `models::HistoricalPrice::date`.
    pub date: String,
    pub close_price: String,
}

/// The asset types the tax engine recognises. An unknown value is a validation
/// error, not a row that silently never matches.
pub const KNOWN_ASSET_TYPES: &[&str] = &["STOCK", "FUND", "ETF", "BOND", "CASH"];

/// The transaction types the tax engine replays.
pub const KNOWN_TRANSACTION_TYPES: &[&str] = &["BUY", "SELL", "SPLIT", "DIVIDEND"];

/// Total authored rows in a bundle, against `MAX_BUNDLE_ROWS`.
pub fn row_count(bundle: &Bundle) -> usize {
    bundle.portfolios.iter().fold(0usize, |acc, p| {
        acc + 1 + p.assets.iter().fold(0usize, |a, ast| a + 1 + ast.transactions.len())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle(portfolios: Vec<BundlePortfolio>) -> Bundle {
        Bundle {
            format: FORMAT_TAG.to_string(),
            schema_version: 1,
            created_at: "2026-11-13T00:00:00Z".to_string(),
            portfolios,
            prices: vec![],
        }
    }

    #[test]
    fn accepted_versions_are_exactly_v1() {
        assert_eq!(ACCEPTED_VERSIONS, &[1]);
        assert_eq!(FORMAT_TAG, "portfolio-prism-backup");
    }

    #[test]
    fn row_count_counts_every_level() {
        let b = bundle(vec![BundlePortfolio {
            bundle_ref: "p1".into(),
            name: "Main".into(),
            description: None,
            currency: "USD".into(),
            base_currency: "USD".into(),
            assets: vec![
                BundleAsset {
                    bundle_ref: "a1".into(),
                    symbol: "AAPL".into(),
                    name: "Apple".into(),
                    asset_type: "STOCK".into(),
                    sector: None,
                    currency: "USD".into(),
                    isin: None,
                    transactions: vec![BundleTransaction {
                        r#type: "BUY".into(),
                        quantity: "1".into(),
                        price: "1".into(),
                        fee: "0".into(),
                        date: "2024-01-01T00:00:00Z".into(),
                    }],
                },
                BundleAsset {
                    bundle_ref: "a2".into(),
                    symbol: "MSFT".into(),
                    name: "Microsoft".into(),
                    asset_type: "STOCK".into(),
                    sector: None,
                    currency: "USD".into(),
                    isin: None,
                    transactions: vec![],
                },
            ],
        }]);
        // 1 portfolio + 2 assets + 1 transaction
        assert_eq!(row_count(&b), 4);
    }

    #[test]
    fn an_empty_prices_section_is_not_serialised() {
        let json = serde_json::to_string(&bundle(vec![])).unwrap();
        assert!(
            !json.contains("\"prices\""),
            "an absent price section must not appear as an empty array: {json}"
        );
    }

    #[test]
    fn an_unknown_field_is_rejected_rather_than_ignored() {
        // Review Focus #1: a hand-edited bundle must not be able to smuggle a
        // database id in, so unknown keys are an error, not a shrug.
        let raw = r#"{"format":"portfolio-prism-backup","schema_version":1,
            "created_at":"2026-11-13T00:00:00Z","portfolios":[],"prices":[],"id":7}"#;
        assert!(serde_json::from_str::<Bundle>(raw).is_err());
    }
}
