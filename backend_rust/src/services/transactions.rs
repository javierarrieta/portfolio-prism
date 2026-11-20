use rust_decimal::Decimal;
use sqlx::PgConnection;

use crate::models::{Transaction, TxType};
use crate::schemas::TransactionCreate;
use crate::services::error::WriteError;

/// Insert one transaction row and return it.
///
/// This is the single place a transaction row is written. The HTTP handler and the
/// staged-import apply path both call it, so the normalization and the checks
/// below cannot be bypassed by whichever caller arrives second.
///
/// **Authorization is not this function's job.** The caller has already proved the
/// caller may write to `asset_id`, through `access.rs`. Nothing here consults
/// ownership, and a future caller must not read that as an oversight to fix by
/// adding a check: the chokepoint stays in one layer, on purpose.
///
/// It takes a connection rather than a pool so that a caller can wrap several
/// writes in one transaction: the staged-import apply path passes `&mut *tx`,
/// because a batch must land as one unit or not at all. The HTTP handler checks a
/// connection out of the pool and passes that, which is the same autocommit
/// single-statement behaviour it had before this function existed.
///
/// The type arrives as [`TxType`], so it is already one of the three the engines
/// handle — there is no `to_uppercase()` here and nothing to forget. That is the
/// caller's problem to have solved, and it is solved at deserialisation for the HTTP
/// route; a future caller that builds a `TransactionCreate` in Rust gets the same
/// guarantee from the type system. The database enforces it a third time with the
/// `CHECK` in `0002_transaction_type_check.sql`.
pub async fn insert(
    conn: &mut PgConnection,
    asset_id: i64,
    create: &TransactionCreate,
) -> Result<Transaction, WriteError> {
    let tx_type = create.r#type;
    if tx_type == TxType::Split
        && (create.quantity <= Decimal::ZERO || create.price <= Decimal::ZERO)
    {
        return Err(WriteError::InvalidSplit);
    }
    // A split is a ratio, not a trade: whatever fee the caller sent is discarded
    // rather than stored, exactly as before the extraction.
    let tx_fee = if tx_type == TxType::Split {
        Decimal::ZERO
    } else {
        create.fee
    };

    let tx_date = create.date.date_naive();
    let min_date = chrono::NaiveDate::from_ymd_opt(1900, 1, 1).ok_or(WriteError::DateOutOfRange)?;
    let max_date = chrono::NaiveDate::from_ymd_opt(2100, 1, 1).ok_or(WriteError::DateOutOfRange)?;
    if tx_date < min_date || tx_date > max_date {
        return Err(WriteError::DateOutOfRange);
    }

    // `to_string()` on the decimals is what keeps `f64` out of the wire format:
    // the value goes to Postgres as a decimal string, never through a float.
    sqlx::query_as!(
        Transaction,
        "INSERT INTO transactions (asset_id, type, quantity, price, fee, date)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING *",
        asset_id,
        tx_type.as_str(),
        create.quantity.to_string(),
        create.price.to_string(),
        tx_fee.to_string(),
        create.date
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(|_| WriteError::Database)
}
