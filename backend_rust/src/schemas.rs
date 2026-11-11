use serde::{Serialize, Deserialize};
use chrono::{DateTime, NaiveDate, Utc};
use utoipa::ToSchema;
use rust_decimal::Decimal;
use crate::db_types::{decimal_json, decimal_opt_json};

// --- Transaction ---
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TransactionCreate {
    pub r#type: String,
    #[serde(with = "decimal_json")]
    pub quantity: Decimal,
    #[serde(with = "decimal_json")]
    pub price: Decimal,
    #[serde(with = "decimal_json")]
    pub fee: Decimal,
    pub date: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TransactionOut {
    pub id: i32,
    pub asset_id: i32,
    pub r#type: String,
    #[serde(with = "decimal_json")]
    pub quantity: Decimal,
    #[serde(with = "decimal_json")]
    pub price: Decimal,
    #[serde(with = "decimal_json")]
    pub fee: Decimal,
    pub date: DateTime<Utc>,
}

// --- Asset ---
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AssetCreate {
    pub symbol: String,
    pub name: String,
    pub asset_type: String,
    pub sector: Option<String>,
    pub currency: String,
    #[serde(default)]
    pub isin: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AssetOut {
    pub id: i32,
    pub portfolio_id: i32,
    pub symbol: String,
    pub name: String,
    pub asset_type: String,
    pub sector: Option<String>,
    pub currency: String,
    pub isin: Option<String>,
    pub transactions: Vec<TransactionOut>,
}

// --- Portfolio ---
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PortfolioCreate {
    pub name: String,
    pub description: Option<String>,
    pub currency: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PortfolioOut {
    pub id: i32,
    pub name: String,
    pub description: Option<String>,
    pub currency: String,
    pub assets: Vec<AssetOut>,
}

// --- Portfolio Update ---
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PortfolioUpdate {
    pub currency: String,
}

// --- Asset Update ---
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AssetUpdate {
    pub currency: String,
}

// --- Tax Lot Out ---
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TaxLot {
    /// The buy that opened this lot. Present so a lot can be joined back to the ledger
    /// row a person is reading.
    pub buy_transaction_id: i32,
    pub buy_date: DateTime<Utc>,
    #[serde(with = "decimal_json")]
    pub buy_price: Decimal,
    #[serde(with = "decimal_json")]
    pub original_qty: Decimal,
    #[serde(with = "decimal_json")]
    pub remaining_qty: Decimal,
    #[serde(with = "decimal_json")]
    pub latent_gain_loss: Decimal,
    #[serde(with = "decimal_json")]
    pub latent_roi: Decimal,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AssetTaxSummary {
    pub symbol: String,
    pub asset_type: String,
    #[serde(with = "decimal_json")]
    pub current_shares: Decimal,
    #[serde(with = "decimal_json")]
    pub average_cost: Decimal,
    #[serde(with = "decimal_json")]
    pub current_price: Decimal,
    #[serde(with = "decimal_json")]
    pub total_cost: Decimal,
    #[serde(with = "decimal_json")]
    pub market_value: Decimal,
    #[serde(with = "decimal_json")]
    pub unrealized_pnl: Decimal,
    #[serde(with = "decimal_json")]
    pub unrealized_roi: Decimal,
    #[serde(with = "decimal_json")]
    pub realized_pnl: Decimal,
    pub tax_lots: Vec<TaxLot>,
}

// --- Sell P&L breakdown ---

/// One row per buy lot consumed by one sell. `buy_price` and `unit_cost` are
/// split-adjusted and expressed in the portfolio base currency; `unit_cost`
/// additionally carries the amortised buy fee.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LotMatch {
    pub buy_transaction_id: i32,
    pub buy_date: DateTime<Utc>,
    #[serde(with = "decimal_json")]
    pub buy_price: Decimal,
    #[serde(with = "decimal_json")]
    pub unit_cost: Decimal,
    #[serde(with = "decimal_json")]
    pub matched_qty: Decimal,
    #[serde(with = "decimal_json")]
    pub lot_qty_before_match: Decimal,
    #[serde(with = "decimal_json")]
    pub cost_basis: Decimal,
    #[serde(with = "decimal_json")]
    pub proceeds: Decimal,
    #[serde(with = "decimal_json")]
    pub pnl: Decimal,
    pub holding_days: i64,
    /// "SHORT" | "LONG", populated for the HYBRID strategy only; null otherwise.
    pub term: Option<String>,
}

/// One sell and every lot it consumed, in consumption order. `unmatched_qty` is
/// the quantity sold with no buy lot behind it, so it carries no cost basis and
/// contributes nothing to `realized_pnl`.
///
/// Engine-internal: not registered in the OpenAPI components, because no
/// response body is of this type.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SellBreakdown {
    pub sell_transaction_id: i32,
    pub sell_date: DateTime<Utc>,
    #[serde(with = "decimal_json")]
    pub sold_qty: Decimal,
    #[serde(with = "decimal_json")]
    pub matched_qty: Decimal,
    #[serde(with = "decimal_json")]
    pub unmatched_qty: Decimal,
    #[serde(with = "decimal_json")]
    pub proceeds: Decimal,
    #[serde(with = "decimal_json")]
    pub cost_basis: Decimal,
    #[serde(with = "decimal_json")]
    pub realized_pnl: Decimal,
    pub matches: Vec<LotMatch>,
}

/// One lot still open at `end`, valued at the requested date. The report's OPEN_LOT row
/// is this struct; `unrealized_roi` is `None` rather than zero when the lot has no cost
/// basis, so a consumer can tell "not applicable" from "0%".
#[derive(Debug, Clone)]
pub struct DatedLot {
    pub buy_transaction_id: i32,
    pub buy_date: DateTime<Utc>,
    /// Split-adjusted purchase price per unit, in the asset's own currency.
    pub buy_price: Decimal,
    /// The quantity originally bought, split-adjusted: a 100-share lot after a 2:1 split
    /// is 200 whether or not any of it has been sold.
    pub original_qty: Decimal,
    pub remaining_qty: Decimal,
    /// Fee-amortised cost per unit, in the portfolio's base currency.
    pub unit_cost: Decimal,
    pub market_value: Decimal,
    pub unrealized_gain_loss: Decimal,
    pub unrealized_roi: Option<Decimal>,
}

/// One dated replay: the open lots and every sell's lot attribution, from a single pass
/// over the transactions with `date <= end`.
#[derive(Debug, Clone)]
pub struct DatedReplay {
    pub open_lots: Vec<DatedLot>,
    pub sell_breakdowns: Vec<SellBreakdown>,
    /// The base-currency price the open lots were valued at. The report sums lot values
    /// and never multiplies this by a quantity of its own.
    pub current_price_base: Decimal,
    /// The date whose close priced the market value, or `None` when the asset and the
    /// base currency are the same and no conversion happened.
    pub fx_rate_date_used: Option<NaiveDate>,
}

/// Endpoint payload: the breakdown flattened next to the context needed to read
/// it honestly — which strategy produced it, which currency the money is in,
/// whether FX or a split touched it. `price`/`fee` are the raw as-entered values
/// in `asset_currency`; every other money field is in `currency`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SellPnlResponse {
    pub transaction_id: i32,
    pub asset_id: i32,
    pub symbol: String,
    pub asset_type: String,
    pub strategy: String,
    pub threshold_days: i64,
    pub currency: String,
    pub asset_currency: String,
    pub fx_applied: bool,
    /// A SPLIT on this asset dated on or before the sell: buy prices in the
    /// matches are split-adjusted rather than as entered.
    pub split_applied: bool,
    pub sell_date: DateTime<Utc>,
    #[serde(with = "decimal_json")]
    pub sold_qty: Decimal,
    #[serde(with = "decimal_json")]
    pub price: Decimal,
    #[serde(with = "decimal_json")]
    pub fee: Decimal,
    #[serde(with = "decimal_json")]
    pub proceeds: Decimal,
    #[serde(with = "decimal_json")]
    pub cost_basis: Decimal,
    #[serde(with = "decimal_json")]
    pub realized_pnl: Decimal,
    /// Null when cost basis is zero — dividing there would invent a number.
    #[serde(with = "decimal_opt_json")]
    pub roi: Option<Decimal>,
    #[serde(with = "decimal_json")]
    pub matched_qty: Decimal,
    #[serde(with = "decimal_json")]
    pub unmatched_qty: Decimal,
    pub matches: Vec<LotMatch>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use std::str::FromStr;

    fn sample_datetime() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2025-01-15T10:30:00Z").unwrap().with_timezone(&Utc)
    }

    #[test]
    fn test_transaction_create_roundtrip() {
        let tx = TransactionCreate {
            r#type: "BUY".to_string(),
            quantity: Decimal::from_str("100.0").unwrap(),
            price: Decimal::from_str("150.5").unwrap(),
            fee: Decimal::from_str("9.99").unwrap(),
            date: sample_datetime(),
        };
        let json = serde_json::to_string(&tx).unwrap();
        let deserialized: TransactionCreate = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.r#type, "BUY");
        assert_eq!(deserialized.quantity, Decimal::from_str("100.0").unwrap());
        assert_eq!(deserialized.price, Decimal::from_str("150.5").unwrap());
        assert_eq!(deserialized.fee, Decimal::from_str("9.99").unwrap());
    }

    #[test]
    fn test_transaction_out_roundtrip() {
        let tx = TransactionOut {
            id: 42,
            asset_id: 7,
            r#type: "SELL".to_string(),
            quantity: Decimal::from_str("25.0").unwrap(),
            price: Decimal::from_str("200.0").unwrap(),
            fee: Decimal::from_str("5.0").unwrap(),
            date: sample_datetime(),
        };
        let json = serde_json::to_string(&tx).unwrap();
        let deserialized: TransactionOut = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, 42);
        assert_eq!(deserialized.asset_id, 7);
        assert_eq!(deserialized.r#type, "SELL");
    }

    #[test]
    fn test_asset_create_roundtrip() {
        let a = AssetCreate {
            symbol: "TSLA".to_string(),
            name: "Tesla Inc".to_string(),
            asset_type: "STOCK".to_string(),
            sector: Some("Auto".to_string()),
            currency: "USD".to_string(),
            isin: "US88160R1014".to_string(),
        };
        let json = serde_json::to_string(&a).unwrap();
        let deserialized: AssetCreate = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.symbol, "TSLA");
        assert_eq!(deserialized.asset_type, "STOCK");
        assert_eq!(deserialized.currency, "USD");
        assert_eq!(deserialized.isin, "US88160R1014");
        assert_eq!(deserialized.sector, Some("Auto".to_string()));
    }

    #[test]
    fn test_portfolio_create_with_none_description() {
        let p = PortfolioCreate {
            name: "My Portfolio".to_string(),
            description: None,
            currency: "USD".to_string(),
        };
        let json = serde_json::to_string(&p).unwrap();
        let deserialized: PortfolioCreate = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "My Portfolio");
        assert_eq!(deserialized.description, None);
        assert_eq!(deserialized.currency, "USD");
    }

    #[test]
    fn test_portfolio_out_roundtrip() {
        let p = PortfolioOut {
            id: 1,
            name: "Test".to_string(),
            description: Some("A test portfolio".to_string()),
            currency: "EUR".to_string(),
            assets: vec![],
        };
        let json = serde_json::to_string(&p).unwrap();
        let deserialized: PortfolioOut = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, 1);
        assert_eq!(deserialized.name, "Test");
        assert_eq!(deserialized.description, Some("A test portfolio".to_string()));
    }

    #[test]
    fn test_asset_out_roundtrip() {
        let a = AssetOut {
            id: 1,
            portfolio_id: 1,
            symbol: "AAPL".to_string(),
            name: "Apple Inc".to_string(),
            asset_type: "STOCK".to_string(),
            sector: Some("Technology".to_string()),
            currency: "USD".to_string(),
            isin: Some("US0378331005".to_string()),
            transactions: vec![],
        };
        let json = serde_json::to_string(&a).unwrap();
        let deserialized: AssetOut = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.symbol, "AAPL");
        assert_eq!(deserialized.currency, "USD");
        assert_eq!(deserialized.isin, Some("US0378331005".to_string()));
        assert_eq!(deserialized.sector, Some("Technology".to_string()));
    }

    #[test]
    fn test_tax_lot_roundtrip() {
        let lot = TaxLot {
            buy_transaction_id: 7,
            buy_date: sample_datetime(),
            buy_price: Decimal::from_str("150.0").unwrap(),
            original_qty: Decimal::from_str("100.0").unwrap(),
            remaining_qty: Decimal::from_str("75.0").unwrap(),
            latent_gain_loss: Decimal::from_str("500.0").unwrap(),
            latent_roi: Decimal::from_str("0.0444").unwrap(),
        };
        let json = serde_json::to_string(&lot).unwrap();
        let deserialized: TaxLot = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.remaining_qty, Decimal::from_str("75.0").unwrap());
        assert_eq!(deserialized.latent_gain_loss, Decimal::from_str("500.0").unwrap());
    }

    #[test]
    fn test_asset_tax_summary_roundtrip() {
        let summary = AssetTaxSummary {
            symbol: "AAPL".to_string(),
            asset_type: "STOCK".to_string(),
            current_shares: Decimal::from_str("100.0").unwrap(),
            average_cost: Decimal::from_str("150.0").unwrap(),
            current_price: Decimal::from_str("160.0").unwrap(),
            total_cost: Decimal::from_str("15000.0").unwrap(),
            market_value: Decimal::from_str("16000.0").unwrap(),
            unrealized_pnl: Decimal::from_str("1000.0").unwrap(),
            unrealized_roi: Decimal::from_str("0.0667").unwrap(),
            realized_pnl: Decimal::from_str("200.0").unwrap(),
            tax_lots: vec![],
        };
        let json = serde_json::to_string(&summary).unwrap();
        let deserialized: AssetTaxSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.symbol, "AAPL");
        assert_eq!(deserialized.unrealized_pnl, Decimal::from_str("1000.0").unwrap());
    }

    #[test]
    fn test_portfolio_update_roundtrip() {
        let u = PortfolioUpdate {
            currency: "EUR".to_string(),
        };
        let json = serde_json::to_string(&u).unwrap();
        let deserialized: PortfolioUpdate = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.currency, "EUR");
    }

    #[test]
    fn test_asset_update_roundtrip() {
        let u = AssetUpdate {
            currency: "GBP".to_string(),
        };
        let json = serde_json::to_string(&u).unwrap();
        let deserialized: AssetUpdate = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.currency, "GBP");
    }

    #[test]
    fn test_lot_match_roundtrip() {
        let m = LotMatch {
            buy_transaction_id: 11,
            buy_date: sample_datetime(),
            buy_price: Decimal::from_str("100.0").unwrap(),
            unit_cost: Decimal::from_str("100.5").unwrap(),
            matched_qty: Decimal::from_str("60.0").unwrap(),
            lot_qty_before_match: Decimal::from_str("100.0").unwrap(),
            cost_basis: Decimal::from_str("6030.0").unwrap(),
            proceeds: Decimal::from_str("7800.0").unwrap(),
            pnl: Decimal::from_str("1770.0").unwrap(),
            holding_days: 335,
            term: Some("SHORT".to_string()),
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: LotMatch = serde_json::from_str(&json).unwrap();
        assert_eq!(back.buy_transaction_id, 11);
        assert_eq!(back.unit_cost, Decimal::from_str("100.5").unwrap());
        assert_eq!(back.holding_days, 335);
        assert_eq!(back.term, Some("SHORT".to_string()));
    }

    #[test]
    fn test_sell_breakdown_roundtrip_and_decimals_are_strings() {
        let b = SellBreakdown {
            sell_transaction_id: 42,
            sell_date: sample_datetime(),
            sold_qty: Decimal::from_str("80.0").unwrap(),
            matched_qty: Decimal::from_str("80.0").unwrap(),
            unmatched_qty: Decimal::ZERO,
            proceeds: Decimal::from_str("10400.0").unwrap(),
            cost_basis: Decimal::from_str("8400.0").unwrap(),
            realized_pnl: Decimal::from_str("2000.0").unwrap(),
            matches: vec![],
        };
        let json = serde_json::to_string(&b).unwrap();
        assert!(json.contains(r#""realized_pnl":"2000.0""#), "decimals must serialize as strings: {json}");
        let back: SellBreakdown = serde_json::from_str(&json).unwrap();
        assert_eq!(back.realized_pnl, Decimal::from_str("2000.0").unwrap());
        assert!(back.matches.is_empty());
    }

    #[test]
    fn test_sell_pnl_response_roi_null_when_no_cost_basis() {
        let r = SellPnlResponse {
            transaction_id: 42, asset_id: 7, symbol: "AAPL".to_string(),
            asset_type: "STOCK".to_string(), strategy: "FIFO".to_string(), threshold_days: 30,
            currency: "USD".to_string(), asset_currency: "USD".to_string(),
            fx_applied: false, split_applied: false,
            sell_date: sample_datetime(),
            sold_qty: Decimal::from_str("50.0").unwrap(),
            price: Decimal::from_str("120.0").unwrap(),
            fee: Decimal::ZERO,
            proceeds: Decimal::ZERO, cost_basis: Decimal::ZERO, realized_pnl: Decimal::ZERO,
            roi: None,
            matched_qty: Decimal::ZERO, unmatched_qty: Decimal::from_str("50.0").unwrap(),
            matches: vec![],
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains(r#""roi":null"#), "roi must be null, not 0: {json}");
        let back: SellPnlResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.roi, None);
        assert_eq!(back.unmatched_qty, Decimal::from_str("50.0").unwrap());
    }
}
