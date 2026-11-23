use sqlx::PgConnection;

use crate::api_routes::lookup::is_valid_isin;
use crate::models::Asset;
use crate::schemas::AssetCreate;
use crate::services::currency_service::CurrencyService;
use crate::services::error::WriteError;

/// Insert one asset row and return it.
///
/// The companion to [`crate::services::transactions::insert`], and the reason the
/// two exist together: a staged batch can create an asset and the transactions
/// that reference it in the same database transaction, which is only possible if
/// both writes go through a function that accepts a caller-supplied connection.
///
/// Authorization stays with the caller, as it does for transactions — the handler
/// has already proved the portfolio is writable through `access.rs`. The
/// pre-existing guarantee that a nonexistent `portfolio_id` cannot gain an asset
/// comes from that `require()` in the handler, not from anything here; a caller
/// that skips it would insert a dangling row, and no constraint would stop it.
///
/// Symbol and ISIN are uppercased here rather than in the handler so that every
/// caller stores the same canonical form. The uniqueness checks are per-portfolio.
///
/// **Symbol uniqueness is constrained; ISIN uniqueness is not.**
///
/// Migration `0006_asset_symbol_unique.sql` added `uq_assets_portfolio_symbol`, unique on
/// `(portfolio_id, upper(symbol))`, so the `SELECT`-then-`INSERT` below is now a friendly
/// answer in front of a real constraint rather than the whole rule. Two callers racing here
/// still both pass the `SELECT`, but exactly one `INSERT` lands and the other gets a database
/// error — which is the correct outcome and the reason the index exists.
///
/// `isin` has the same shape and no index: the check below is a `SELECT`-then-`INSERT` with a
/// window, and two concurrent writers can each store a row for one ISIN in one portfolio.
/// It is left that way on purpose rather than by omission — closing it needs the same
/// audit-first migration, and the audit script reports existing ISIN collisions so the
/// decision is made with the data in view. Do not read the symbol index as covering it.
///
/// A note for whoever adds the ISIN one: do not normalize symbols in a migration to make the
/// data match a constraint. `historical_prices` is keyed by the symbol string, not by
/// `asset_id`, so rewriting an asset's symbol silently detaches its price history. Step (2)
/// of `0006` says all this at length; the fix there was to change the lookup, not the rows.
pub async fn insert(
    conn: &mut PgConnection,
    portfolio_id: i64,
    create: &AssetCreate,
) -> Result<Asset, WriteError> {
    let symbol = create.symbol.to_uppercase();

    let existing = sqlx::query_as!(
        Asset,
        // `upper(symbol)`, not `symbol`. Migration 0006 puts the uniqueness constraint on
        // `(portfolio_id, upper(symbol))`, and this check has to agree with the constraint it
        // is standing in front of: comparing the bare column would miss a row stored as `aapl`
        // — by restore, by hand, or by an older build — let the insert through, and the insert
        // would then fail on the index as a raw database error instead of returning
        // `SymbolExists`. The bind is already upper-cased above, and the index leads with
        // `portfolio_id`, so this is the shape the index serves.
        "SELECT * FROM assets WHERE portfolio_id = $1 AND upper(symbol) = $2",
        portfolio_id,
        &symbol
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(|_| WriteError::Database)?;

    if existing.is_some() {
        return Err(WriteError::SymbolExists);
    }

    let isin = if create.isin.is_empty() {
        None
    } else {
        let isin_upper = create.isin.to_uppercase();
        if !is_valid_isin(&isin_upper) {
            return Err(WriteError::IsinInvalid);
        }
        // `assets.id` is BIGINT; reading it as `i32` is a runtime mismatched-types
        // error under sqlx-postgres, not a compile error. The value is only tested
        // for presence, but the row still has to decode.
        let isin_exists: Option<i64> = sqlx::query_scalar!(
            "SELECT id FROM assets WHERE portfolio_id = $1 AND isin = $2",
            portfolio_id,
            &isin_upper
        )
        .fetch_optional(&mut *conn)
        .await
        .map_err(|_| WriteError::Database)?;
        if isin_exists.is_some() {
            return Err(WriteError::IsinExists);
        }
        Some(isin_upper)
    };

    let resolved_currency = if create.currency.is_empty() {
        CurrencyService::detect_currency(&symbol)
    } else {
        create.currency.clone()
    };

    sqlx::query_as!(
        Asset,
        "INSERT INTO assets (portfolio_id, symbol, name, asset_type, sector, currency, isin)
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING *",
        portfolio_id,
        &symbol,
        &create.name,
        create.asset_type.to_uppercase(),
        create.sector.as_deref(),
        &resolved_currency,
        isin.as_deref()
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(|_| WriteError::Database)
}
