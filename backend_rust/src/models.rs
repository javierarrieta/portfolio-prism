use sqlx::FromRow;
use chrono::{DateTime, Utc, NaiveDate};
use serde::{Serialize, Deserialize};
use utoipa::ToSchema;

// Every id here is `i64` because every id in the schema is `BIGINT GENERATED ALWAYS
// AS IDENTITY` (`migrations/0001_init.sql`). `sqlx-postgres` decides compatibility by
// comparing declared type OIDs, so an `int8` column will not decode into an `i32` —
// and it decides at run time, when the row is read, not at compile time. The API
// types in `schemas.rs` are the narrowing point, not these.
//
// The same width rule governs *binds*, and there the failure is worse than a decode
// error. `sqlx::Pool` caches prepared statements by SQL text, so two call sites that
// write the same query with different Rust widths share one server-side statement,
// and whichever prepared it first fixes `$1`'s type. The other then sends the wrong
// number of bytes and Postgres answers `22P03 incorrect binary data format in bind
// parameter 1` (or `08P01 insufficient data left in message`) — a 500 that depends on
// which route ran first. Path parameters are `i64` as well, so every bind of one of
// these ids is `i64` at the point it is bound and no call site can prepare the same
// SQL text at a narrower width. Do not reintroduce a narrowing here to "keep the OpenAPI
// surface still": the surface is already `int64` and the bind failure is not.
#[derive(Debug, FromRow, Serialize, Deserialize, ToSchema)]
pub struct Portfolio {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub currency: String,
    pub base_currency: String,
}

#[derive(Debug, FromRow, Serialize, Deserialize, ToSchema)]
pub struct Asset {
    pub id: i64,
    pub portfolio_id: i64,
    pub symbol: String,
    pub name: String,
    pub asset_type: String,
    pub sector: Option<String>,
    pub currency: String,
    pub isin: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, ToSchema)]
pub struct Transaction {
    pub id: i64,
    pub asset_id: i64,
    pub r#type: String,
    pub quantity: String,
    pub price: String,
    pub fee: String,
    pub date: DateTime<Utc>,
}

#[derive(Debug, FromRow, Serialize, Deserialize, ToSchema)]
pub struct HistoricalPrice {
    pub symbol: String,
    pub date: NaiveDate,
    pub close_price: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc, NaiveDate};

    fn sample_dt() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2025-01-15T10:30:00Z").unwrap().with_timezone(&Utc)
    }

    #[test]
    fn test_portfolio_roundtrip() {
        let p = Portfolio {
            id: 1,
            name: "Test".to_string(),
            description: Some("desc".to_string()),
            currency: "USD".to_string(),
            base_currency: "USD".to_string(),
        };
        let json = serde_json::to_string(&p).unwrap();
        let deserialized: Portfolio = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, 1);
        assert_eq!(deserialized.currency, "USD");
    }

    #[test]
    fn test_asset_roundtrip() {
        let a = Asset {
            id: 1,
            portfolio_id: 1,
            symbol: "AAPL".to_string(),
            name: "Apple".to_string(),
            asset_type: "STOCK".to_string(),
            sector: Some("Tech".to_string()),
            currency: "USD".to_string(),
            isin: Some("US0378331005".to_string()),
        };
        let json = serde_json::to_string(&a).unwrap();
        let deserialized: Asset = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.symbol, "AAPL");
        assert_eq!(deserialized.isin, Some("US0378331005".to_string()));
    }

    #[test]
    fn test_transaction_roundtrip() {
        let tx = Transaction {
            id: 1,
            asset_id: 1,
            r#type: "BUY".to_string(),
            quantity: "50.0".to_string(),
            price: "200.0".to_string(),
            fee: "5.0".to_string(),
            date: sample_dt(),
        };
        let json = serde_json::to_string(&tx).unwrap();
        let deserialized: Transaction = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.r#type, "BUY");
        assert_eq!(deserialized.quantity, "50.0");
    }

    #[test]
    fn test_historical_price_roundtrip() {
        let hp = HistoricalPrice {
            symbol: "SPY".to_string(),
            date: NaiveDate::from_ymd_opt(2025, 1, 15).unwrap(),
            close_price: "480.5".to_string(),
        };
        let json = serde_json::to_string(&hp).unwrap();
        let deserialized: HistoricalPrice = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.symbol, "SPY");
        assert_eq!(deserialized.close_price, "480.5");
    }
}
