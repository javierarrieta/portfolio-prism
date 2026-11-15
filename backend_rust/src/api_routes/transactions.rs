use rocket::{State, serde::json::Json, http::Status};
use crate::db::DbPool;
use crate::models::{Asset, Transaction};
use crate::schemas::{AssetCreate, AssetOut, AssetUpdate, TransactionCreate, TransactionOut};
use crate::api_routes::lookup::is_valid_isin;
use crate::services::currency_service::CurrencyService;
use crate::access::{require, require_asset, require_asset_in, require_transaction, Access};
use crate::auth::guard::AuthedUser;
use rust_decimal::Decimal;

#[utoipa::path(
    post,
    path = "/api/portfolios/<portfolio_id>/assets",
    responses(
        (status = 201, description = "Asset created", body = AssetOut),
        (status = 400, description = "Asset already exists"),
        (status = 404, description = "Portfolio not found")
    )
)]
#[post("/portfolios/<portfolio_id>/assets", data = "<asset>")]
pub async fn create_asset(
    user: AuthedUser,
    portfolio_id: i64,
    pool: &State<DbPool>,
    _currency_service: &State<CurrencyService>,
    asset: Json<AssetCreate>
) -> Result<Json<AssetOut>, Status> {
    // This also fixes a pre-existing hole: the insert used to accept any
    // `portfolio_id`, creating an asset pointing at a portfolio that does not
    // exist. A nonexistent portfolio is now 404 instead of 201-with-a-dangling-row.
    require(pool.inner(), user.id, portfolio_id, Access::Editor).await?;

    let existing = sqlx::query_as!(
        Asset,
        "SELECT * FROM assets WHERE portfolio_id = $1 AND symbol = $2",
        portfolio_id,
        asset.symbol.to_uppercase()
    )
    .fetch_optional(pool.inner())
    .await
    .map_err(|_| Status::InternalServerError)?;

    if existing.is_some() {
        return Err(Status::BadRequest);
    }

    if !asset.isin.is_empty() {
        let isin_upper = asset.isin.to_uppercase();
        if !is_valid_isin(&isin_upper) {
            return Err(Status::BadRequest);
        }
        // `assets.id` is BIGINT; reading it as `i32` is a runtime mismatched-types
        // error under sqlx-postgres, not a compile error. The value is only tested for
        // presence, but the row still has to decode.
        let isin_exists: Option<i64> = sqlx::query_scalar!(
            "SELECT id FROM assets WHERE portfolio_id = $1 AND isin = $2",
            portfolio_id,
            &isin_upper
        )
        .fetch_optional(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?;
        if isin_exists.is_some() {
            return Err(Status::Conflict);
        }
    }

    let resolved_currency = if asset.currency.is_empty() {
        CurrencyService::detect_currency(&asset.symbol)
    } else {
        asset.currency.clone()
    };

    let res = sqlx::query_as!(
        Asset,
        "INSERT INTO assets (portfolio_id, symbol, name, asset_type, sector, currency, isin)
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING *",
        portfolio_id,
        asset.symbol.to_uppercase(),
        &asset.name,
        asset.asset_type.to_uppercase(),
        asset.sector.as_deref(),
        &resolved_currency,
        if asset.isin.is_empty() { None } else { Some(asset.isin.to_uppercase()) }
    )
    .fetch_one(pool.inner())
    .await
    .map_err(|_| Status::InternalServerError)?;

    Ok(Json(AssetOut {
        id: res.id,
        portfolio_id: res.portfolio_id,
        symbol: res.symbol,
        name: res.name,
        asset_type: res.asset_type,
        sector: res.sector,
        currency: res.currency,
        isin: res.isin,
        transactions: vec![],
    }))
}

#[utoipa::path(
    patch,
    path = "/api/portfolios/<portfolio_id>/assets/<asset_id>",
    responses(
        (status = 200, description = "Asset updated", body = AssetOut),
        (status = 404, description = "Asset not found")
    )
)]
#[patch("/portfolios/<portfolio_id>/assets/<asset_id>", data = "<update>")]
pub async fn update_asset(
    user: AuthedUser,
    portfolio_id: i64,
    asset_id: i64,
    pool: &State<DbPool>,
    update: Json<AssetUpdate>
) -> Result<Json<AssetOut>, Status> {
    require_asset_in(pool.inner(), user.id, portfolio_id, asset_id, Access::Editor).await?;

    // The `AND portfolio_id = ?` predicate below stays on purpose. The guard and the
    // write are two statements; the predicate is what makes the write itself
    // incapable of crossing portfolios if the guard is ever refactored away.
    sqlx::query!(
        "UPDATE assets SET currency = $1 WHERE id = $2 AND portfolio_id = $3",
        &update.currency,
        asset_id,
        portfolio_id
    )
    .execute(pool.inner())
    .await
    .map_err(|_| Status::InternalServerError)?;

    let asset = sqlx::query_as!(
        Asset,
        "SELECT * FROM assets WHERE id = $1 AND portfolio_id = $2",
        asset_id,
        portfolio_id
    )
    .fetch_optional(pool.inner())
    .await
    .map_err(|_| Status::InternalServerError)?;

    match asset {
        Some(a) => Ok(Json(AssetOut {
            id: a.id,
            portfolio_id: a.portfolio_id,
            symbol: a.symbol,
            name: a.name,
            asset_type: a.asset_type,
            sector: a.sector,
            currency: a.currency,
            isin: a.isin,
            transactions: vec![],
        })),
        None => Err(Status::NotFound),
    }
}

#[utoipa::path(
    delete,
    path = "/api/assets/<id>",
    responses(
        (status = 204, description = "Asset deleted"),
        (status = 404, description = "Asset not found")
    )
)]
#[delete("/assets/<id>")]
pub async fn delete_asset(user: AuthedUser, id: i64, pool: &State<DbPool>) -> Result<Status, Status> {
    require_asset(pool.inner(), user.id, id, Access::Owner).await?;

    // Both deletes are one transaction. `split_sync.asset_id` references `assets(id)`
    // and `0001_init.sql` gives it no `ON DELETE CASCADE`, so the throttle row has to
    // go first. The SQLite table had no foreign key here at all
    // (`migrate_split_sync_key`, lib.rs:530 at the merge base), so the old order —
    // asset first, throttle row second — did work, and worked reliably; it was simply
    // unchecked. The constraint arrived with the Postgres schema and the ordering
    // became load-bearing, which is what `tests/foreign_keys_test.rs` pins. And the
    // throttle row has to go first *inside* this transaction: if it went first outside
    // one, a failed asset delete would still have destroyed it.
    let mut tx = pool.begin().await.map_err(|_| Status::InternalServerError)?;

    // Drop the asset's split-sync throttle row with it. Not a correctness
    // requirement — `assets.id` is an identity column, so the id is never handed to a
    // later asset and a stale row cannot mis-throttle anything. This just stops the
    // throttle table keeping one permanent row per asset the user ever deleted.
    sqlx::query!("DELETE FROM split_sync WHERE asset_id = $1", id)
        .execute(&mut *tx)
        .await
        .map_err(|_| Status::InternalServerError)?;

    let res = sqlx::query!("DELETE FROM assets WHERE id = $1", id)
        .execute(&mut *tx)
        .await
        .map_err(|_| Status::InternalServerError)?;

    if res.rows_affected() == 0 {
        return Err(Status::NotFound);
    }

    tx.commit().await.map_err(|_| Status::InternalServerError)?;
    Ok(Status::NoContent)
}

#[utoipa::path(
    post,
    path = "/api/portfolios/<portfolio_id>/assets/<asset_id>/transactions",
    responses(
        (status = 201, description = "Transaction created", body = TransactionOut),
        (status = 404, description = "Asset not found")
    )
)]
#[post("/portfolios/<portfolio_id>/assets/<asset_id>/transactions", data = "<tx>")]
pub async fn create_transaction(
    user: AuthedUser,
    portfolio_id: i64,
    asset_id: i64,
    pool: &State<DbPool>,
    tx: Json<TransactionCreate>
) -> Result<Json<TransactionOut>, Status> {
    require_asset_in(pool.inner(), user.id, portfolio_id, asset_id, Access::Editor).await?;

    // Kept: this row is the data the handler validates against (asset type, symbol),
    // not the authorization decision.
    let asset = sqlx::query_as!(
        Asset,
        "SELECT * FROM assets WHERE id = $1 AND portfolio_id = $2",
        asset_id,
        portfolio_id
    )
    .fetch_optional(pool.inner())
    .await
    .map_err(|_| Status::InternalServerError)?;

    if asset.is_none() {
        return Err(Status::NotFound);
    }

    let tx_type = tx.r#type.to_uppercase();
    if tx_type == "SPLIT" {
        if tx.quantity <= Decimal::ZERO || tx.price <= Decimal::ZERO {
            return Err(Status::BadRequest);
        }
    }
    let tx_fee = if tx_type == "SPLIT" { Decimal::ZERO } else { tx.fee };

    // Validate date is within reasonable range
    let tx_date = tx.date.date_naive();
    let min_date = chrono::NaiveDate::from_ymd_opt(1900, 1, 1).ok_or(Status::BadRequest)?;
    let max_date = chrono::NaiveDate::from_ymd_opt(2100, 1, 1).ok_or(Status::BadRequest)?;
    if tx_date < min_date || tx_date > max_date {
        return Err(Status::BadRequest);
    }

    let res = sqlx::query_as!(
        Transaction,
        "INSERT INTO transactions (asset_id, type, quantity, price, fee, date)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING *",
        asset_id,
        &tx_type,
        tx.quantity.to_string(),
        tx.price.to_string(),
        tx_fee.to_string(),
        tx.date
    )
    .fetch_one(pool.inner())
    .await
    .map_err(|_| Status::InternalServerError)?;

    Ok(Json(TransactionOut {
        id: res.id,
        asset_id: res.asset_id,
        r#type: res.r#type,
        quantity: crate::db_types::str_to_decimal(&res.quantity),
        price: crate::db_types::str_to_decimal(&res.price),
        fee: crate::db_types::str_to_decimal(&res.fee),
        date: res.date,
    }))
}

#[utoipa::path(
    get,
    path = "/api/portfolios/<portfolio_id>/transactions",
    responses(
        (status = 200, description = "List of transactions", body = [TransactionOut]),
        (status = 404, description = "Portfolio not found")
    )
)]
#[get("/portfolios/<portfolio_id>/transactions")]
pub async fn list_portfolio_transactions(user: AuthedUser, portfolio_id: i64, pool: &State<DbPool>) -> Result<Json<Vec<TransactionOut>>, Status> {
    require(pool.inner(), user.id, portfolio_id, Access::Viewer).await?;
    let asset_ids = sqlx::query_scalar!("SELECT id FROM assets WHERE portfolio_id = $1 ORDER BY id", portfolio_id)
        .fetch_all(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?;

    if asset_ids.is_empty() {
        return Ok(Json(vec![]));
    }

    let mut all_txs = Vec::new();
    for id in asset_ids {
        let txs = sqlx::query_as!(
            Transaction,
            // `ORDER BY id` is what makes the sort below total. The handler re-sorts by
            // date descending with a *stable* sort, so same-day transactions keep
            // whatever order the database returned — and with no `ORDER BY` that was
            // whatever the planner produced, which moves when a row is updated. `id` is
            // insertion order, which is the order the file-backed build returned.
            "SELECT * FROM transactions WHERE asset_id = $1 ORDER BY id",
            id
        )
        .fetch_all(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?;
        all_txs.extend(txs);
    }
    all_txs.sort_by(|a, b| b.date.cmp(&a.date));

    let out = all_txs.into_iter().map(|t| TransactionOut {

        id: t.id,
        asset_id: t.asset_id,
        r#type: t.r#type,
        quantity: crate::db_types::str_to_decimal(&t.quantity),
        price: crate::db_types::str_to_decimal(&t.price),
        fee: crate::db_types::str_to_decimal(&t.fee),
        date: t.date,
    }).collect();

    Ok(Json(out))
}

#[utoipa::path(
    delete,
    path = "/api/transactions/<id>",
    responses(
        (status = 204, description = "Transaction deleted"),
        (status = 404, description = "Transaction not found")
    )
)]
#[delete("/transactions/<id>")]
pub async fn delete_transaction(user: AuthedUser, id: i64, pool: &State<DbPool>) -> Result<Status, Status> {
    require_transaction(pool.inner(), user.id, id, Access::Editor).await?;
    let res = sqlx::query!("DELETE FROM transactions WHERE id = $1", id)
        .execute(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?;

    if res.rows_affected() == 0 {
        return Err(Status::NotFound);
    }

    Ok(Status::NoContent)
}
