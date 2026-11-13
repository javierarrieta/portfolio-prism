//! Reading a caller's rows into a `Bundle`. All export SQL lives here.
//!
//! The queries are transitive by design: only `portfolios` carries `owner_id`, so
//! assets and transactions are reached through `portfolio_id` and `asset_id`. The
//! caller's ids are checked by `access::require` in the handler *before* anything
//! here runs; nothing in this file decides who may see a row.

use chrono::{NaiveDate, Utc};
use sqlx::SqlitePool;
use crate::backup::{Bundle, BundleAsset, BundlePortfolio, BundlePrice, BundleTransaction, FORMAT_TAG};

/// A database failure, kept distinct from a client error so the handler can answer
/// `503` for a lock and `500` for anything else. Collapsing the two is what
/// `src/access.rs` warns against: an outage would look like a bad client.
#[derive(Debug)]
pub enum DbFailure {
    Lock,
    Other,
}

impl DbFailure {
    pub fn status(&self) -> rocket::http::Status {
        match self {
            DbFailure::Lock => rocket::http::Status::ServiceUnavailable,
            DbFailure::Other => rocket::http::Status::InternalServerError,
        }
    }
}

/// Classify a sqlx error as a lock or not.
///
/// sqlx has no `is_locked()` on `DatabaseError`, so this matches on what the
/// drivers actually surface: SQLite reports `SQLITE_BUSY` as code `5` and
/// `SQLITE_LOCKED` as code `6` (`sqlx-sqlite`'s `DatabaseError::code()` formats the
/// raw code), and Postgres — later — reports `40001` serialisation failure and
/// `40P01` deadlock detected. The message check is the belt: it is what a bundled
/// SQLite build actually prints.
pub fn classify(e: sqlx::Error) -> DbFailure {
    if let sqlx::Error::Database(db) = &e {
        let code = db.code().map(|c| c.to_string()).unwrap_or_default();
        let message = db.message().to_lowercase();
        if matches!(code.as_str(), "5" | "6" | "40001" | "40P01")
            || message.contains("database is locked")
            || message.contains("database table is locked")
        {
            return DbFailure::Lock;
        }
    }
    eprintln!("ERROR: backup database failure: {e}");
    DbFailure::Other
}

/// Build a bundle for exactly `portfolio_ids`, which the caller has already been
/// authorised for. Order follows the order requested, so the file matches what the
/// user ticked.
pub async fn build(
    pool: &SqlitePool,
    portfolio_ids: &[i32],
    include_prices: bool,
) -> Result<Bundle, DbFailure> {
    let mut portfolios = Vec::with_capacity(portfolio_ids.len());
    let mut symbols = std::collections::BTreeSet::new();

    for id in portfolio_ids {
        let row: Option<(String, Option<String>, String, String)> = sqlx::query_as(
            "SELECT name, description, currency, base_currency FROM portfolios WHERE id = ?",
        )
        .bind(*id)
        .fetch_optional(pool)
        .await
        .map_err(classify)?;

        let Some((name, description, currency, base_currency)) = row else {
            // `require` already cleared this id; a row vanishing underneath us is a
            // concurrent delete, not a denial.
            return Err(DbFailure::Lock);
        };

        let assets = assets_for(pool, *id).await?;
        for a in &assets {
            symbols.insert(a.symbol.clone());
        }
        portfolios.push(BundlePortfolio {
            bundle_ref: format!("p{id}"),
            name,
            description,
            currency,
            base_currency,
            assets,
        });
    }

    let prices = if include_prices {
        prices_for(pool, &symbols).await?
    } else {
        Vec::new()
    };

    Ok(Bundle {
        format: FORMAT_TAG.to_string(),
        schema_version: 1,
        created_at: Utc::now().to_rfc3339(),
        portfolios,
        prices,
    })
}

/// One `assets` row, in the order `assets_for` names its columns. A named type
/// rather than the inline tuple: seven positional columns is past what a reader can
/// hold in their head at the `for` that destructures it, and clippy's
/// `type_complexity` says so too.
type AssetRow = (i32, String, String, String, Option<String>, String, Option<String>);

/// One `transactions` row, in the order `assets_for` names its columns.
type TransactionRow = (String, String, String, String, chrono::DateTime<Utc>);

async fn assets_for(pool: &SqlitePool, portfolio_id: i32) -> Result<Vec<BundleAsset>, DbFailure> {
    let rows: Vec<AssetRow> = sqlx::query_as(
        "SELECT id, symbol, name, asset_type, sector, currency, isin \
         FROM assets WHERE portfolio_id = ? ORDER BY id",
    )
    .bind(portfolio_id)
    .fetch_all(pool)
    .await
    .map_err(classify)?;

    let mut out = Vec::with_capacity(rows.len());
    for (id, symbol, name, asset_type, sector, currency, isin) in rows {
        let txs: Vec<TransactionRow> = sqlx::query_as(
            "SELECT type, quantity, price, fee, date FROM transactions \
             WHERE asset_id = ? ORDER BY date, id",
        )
        .bind(id)
        .fetch_all(pool)
        .await
        .map_err(classify)?;

        out.push(BundleAsset {
            bundle_ref: format!("a{id}"),
            symbol,
            name,
            asset_type,
            sector,
            currency,
            isin,
            transactions: txs
                .into_iter()
                .map(|(r#type, quantity, price, fee, date)| BundleTransaction {
                    r#type,
                    quantity,
                    price,
                    fee,
                    date: date.to_rfc3339(),
                })
                .collect(),
        });
    }
    Ok(out)
}

async fn prices_for(
    pool: &SqlitePool,
    symbols: &std::collections::BTreeSet<String>,
) -> Result<Vec<BundlePrice>, DbFailure> {
    let mut out = Vec::new();
    // One query per symbol rather than a constructed IN list: the selection is
    // bounded by the portfolio's holdings, and a placeholder built by `format!`
    // is invisible to the access gate and to a reader alike.
    for symbol in symbols {
        let rows: Vec<(String, NaiveDate, String)> = sqlx::query_as(
            "SELECT symbol, date, close_price FROM historical_prices \
             WHERE symbol = ? ORDER BY date",
        )
        .bind(symbol)
        .fetch_all(pool)
        .await
        .map_err(classify)?;

        out.extend(rows.into_iter().map(|(symbol, date, close_price)| BundlePrice {
            symbol,
            date: date.format("%Y-%m-%d").to_string(),
            close_price,
        }));
    }
    Ok(out)
}
