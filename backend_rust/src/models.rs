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

/// The closed set of transaction types the engines understand.
///
/// This enum exists because of a specific failure that was live: `transactions.type`
/// is `TEXT` with no `CHECK`, and the write path did `to_uppercase()` and stored
/// whatever came back. So `"DIVIDEND"` was accepted, stored, and then **silently
/// ignored** — `tax_engine.rs:665-771` and `stats_engine.rs:250-256` are both
/// `if BUY / else if SELL / else if SPLIT` with no terminal `else`. Invisible tax
/// data is worse than a wrong lot, because nothing looks wrong.
///
/// `rename_all = "UPPERCASE"` makes serde match the style names exactly, so the
/// accepted wire values are precisely `BUY`, `SELL`, `SPLIT` and anything else is a
/// serde error the route turns into a 422. There is no `to_uppercase()` left to
/// forget, because a lowercase spelling is a compile error at the call site rather
/// than a string that survives to the database.
///
/// **The read side stays `String`.** [`Transaction`] and `schemas::TransactionOut`
/// are `FromRow` decode targets for rows this migration still accepts in any case —
/// the `CHECK` is `upper(btrim(type)) IN (...)` precisely so legacy `buy` rows do not
/// have to be rewritten. Decoding those into this enum would fail to read every one of
/// them, which turns a validation improvement into an unreadable database.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum TxType {
    Buy,
    Sell,
    Split,
}

impl TxType {
    /// The spelling stored in the database and compared by the engines.
    pub fn as_str(&self) -> &'static str {
        match self {
            TxType::Buy => "BUY",
            TxType::Sell => "SELL",
            TxType::Split => "SPLIT",
        }
    }

    /// The set, in the order the engines handle it. Mirrors
    /// `backup::KNOWN_TRANSACTION_TYPES`; `tests/tx_type_test.rs` asserts the two
    /// agree, because a fourth variant added here and not there would make a bundle
    /// that the app can write and restore refuses to read.
    pub const ALL: [TxType; 3] = [TxType::Buy, TxType::Sell, TxType::Split];
}

impl std::fmt::Display for TxType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
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
