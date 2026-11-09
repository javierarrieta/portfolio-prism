use rocket::{State, serde::json::Json, http::Status};
use sqlx::SqlitePool;
use crate::models::{Portfolio, Asset, Transaction};
use crate::services::currency_service::CurrencyService;
use crate::engines::tax_engine::TaxLotEngine;
use crate::schemas::SellPnlResponse;
use crate::engines::stats_engine::StatsEngine;
use rust_decimal::Decimal;
use anyhow::Result;
use crate::access::{require, require_transaction, Access};
use crate::auth::guard::AuthedUser;

#[utoipa::path(
    get,
    path = "/api/portfolios/<id>/tax-summary",
    responses(
        (status = 200, description = "Tax summary retrieved", body = serde_json::Value),
        (status = 404, description = "Portfolio not found")
    )
)]
#[get("/<id>/tax-summary?<strategy>&<threshold_days>")]
pub async fn get_portfolio_tax_summary(
    user: AuthedUser,
    id: i32,
    pool: &State<SqlitePool>,
    currency_service: &State<CurrencyService>,
    strategy: Option<String>,
    threshold_days: Option<i64>,
) -> Result<Json<serde_json::Value>, Status> {
    require(pool.inner(), user.id, id, Access::Viewer).await?;

    let portfolio = sqlx::query_as::<_, Portfolio>("SELECT * FROM portfolios WHERE id = ?")
        .bind(id)
        .fetch_optional(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?
        .ok_or(Status::NotFound)?;

    let assets = sqlx::query_as::<_, Asset>("SELECT * FROM assets WHERE portfolio_id = ?")
        .bind(id)
        .fetch_all(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?;

    if assets.is_empty() {
        return Ok(Json(serde_json::json!({
            "assets": [],
            "total_portfolio_value": "0",
            "total_realized_pnl": "0",
            "total_unrealized_pnl": "0"
        })));
    }

    let mut asset_summaries = Vec::new();
    let mut total_value = Decimal::ZERO;
    let mut total_realized = Decimal::ZERO;
    let mut total_unrealized = Decimal::ZERO;

    let strategy_val = strategy.unwrap_or_else(|| "FIFO".to_string());
    let threshold_val = threshold_days.unwrap_or(30);

    for asset in assets {
        // Auto-detect and persist any unrecorded stock splits for this asset
        // before computing tax lots, so they feed into P&L and value.
        let _ = currency_service.sync_splits_for_asset(&asset, pool.inner()).await;

        let transactions = sqlx::query_as::<_, Transaction>("SELECT * FROM transactions WHERE asset_id = ?")
            .bind(asset.id)
            .fetch_all(pool.inner())
            .await
            .map_err(|_| Status::InternalServerError)?;

        let current_price = currency_service.get_price(&asset.symbol, asset.isin.as_deref(), pool.inner()).await;

        let summary = TaxLotEngine::calculate_lots(
            &asset.symbol,
            &asset.asset_type,
            &transactions,
            current_price,
            &asset.currency,
            &portfolio.base_currency,
            currency_service,
            &strategy_val,
            threshold_val,
            pool.inner(),
        ).await.map_err(|_| Status::InternalServerError)?;

        total_value += summary.market_value;
        total_realized += summary.realized_pnl;
        total_unrealized += summary.unrealized_pnl;
        asset_summaries.push(summary);
    }

    Ok(Json(serde_json::json!({
        "assets": asset_summaries,
        "total_portfolio_value": total_value.to_string(),
        "total_realized_pnl": total_realized.to_string(),
        "total_unrealized_pnl": total_unrealized.to_string(),
        "strategy": strategy_val,
        "threshold_days": threshold_val,
        "currency": portfolio.base_currency,
    })))
}

#[utoipa::path(
    get,
    path = "/api/portfolios/<id>/performance",
    responses(
        (status = 200, description = "Performance data retrieved", body = serde_json::Value),
        (status = 404, description = "Portfolio not found")
    )
)]
#[get("/<id>/performance")]
pub async fn get_portfolio_performance(
    user: AuthedUser,
    id: i32,
    pool: &State<SqlitePool>,
    currency_service: &State<CurrencyService>,
) -> Result<Json<serde_json::Value>, Status> {
    // Before the `WHERE asset_id IN (SELECT id FROM assets WHERE portfolio_id = ?)`
    // subquery: that shape is invisible to the gate script's grep, so this call is
    // the only thing keeping it honest.
    require(pool.inner(), user.id, id, Access::Viewer).await?;

    let portfolio = sqlx::query_as::<_, Portfolio>("SELECT * FROM portfolios WHERE id = ?")
        .bind(id)
        .fetch_optional(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?
        .ok_or(Status::NotFound)?;

    let assets = sqlx::query_as::<_, Asset>("SELECT * FROM assets WHERE portfolio_id = ?")
        .bind(id)
        .fetch_all(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?;

    // Auto-detect and persist any unrecorded stock splits so historical
    // portfolio value is scaled correctly.
    for asset in &assets {
        let _ = currency_service.sync_splits_for_asset(asset, pool.inner()).await;
    }

    let transactions = sqlx::query_as::<_, Transaction>(
        "SELECT * FROM transactions WHERE asset_id IN (SELECT id FROM assets WHERE portfolio_id = ?)"
    )
    .bind(id)
    .fetch_all(pool.inner())
    .await
    .map_err(|_| Status::InternalServerError)?;

    let perf = StatsEngine::calculate_portfolio_performance(
        pool.inner(),
        &assets,
        &transactions,
        &portfolio.base_currency,
        currency_service
    ).await.map_err(|_| Status::InternalServerError)?;

    Ok(Json(perf))
}

#[utoipa::path(
    get,
    path = "/api/transactions/<id>/pnl",
    params(
        ("id" = i32, Path, description = "Sell transaction id"),
        ("strategy" = Option<String>, Query, description = "FIFO, LIFO or HYBRID (default FIFO)"),
        ("threshold_days" = Option<i64>, Query, description = "HYBRID short-term threshold (default 30)")
    ),
    responses(
        (status = 200, description = "Realized P&L breakdown for the sell", body = SellPnlResponse),
        (status = 400, description = "Transaction is not a SELL"),
        (status = 404, description = "Transaction not found")
    )
)]
#[get("/transactions/<id>/pnl?<strategy>&<threshold_days>")]
pub async fn get_transaction_sell_pnl(
    user: AuthedUser,
    id: i32,
    pool: &State<SqlitePool>,
    currency_service: &State<CurrencyService>,
    strategy: Option<String>,
    threshold_days: Option<i64>,
) -> Result<Json<SellPnlResponse>, Status> {
    require_transaction(pool.inner(), user.id, id, Access::Viewer).await?;

    // The three lookups below (transaction -> asset -> portfolio) stay: they are the
    // response's data source, not the authorization decision.
    let tx = sqlx::query_as::<_, Transaction>("SELECT * FROM transactions WHERE id = ?")
        .bind(id)
        .fetch_optional(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?
        .ok_or(Status::NotFound)?;

    if tx.r#type.to_uppercase() != "SELL" {
        return Err(Status::BadRequest);
    }

    // The base currency comes from the portfolio that owns the asset, never from
    // a path parameter, so this answer cannot disagree with the portfolio on screen.
    let asset = sqlx::query_as::<_, Asset>("SELECT * FROM assets WHERE id = ?")
        .bind(tx.asset_id)
        .fetch_optional(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?
        .ok_or(Status::NotFound)?;

    let portfolio = sqlx::query_as::<_, Portfolio>("SELECT * FROM portfolios WHERE id = ?")
        .bind(asset.portfolio_id)
        .fetch_optional(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?
        .ok_or(Status::NotFound)?;

    // Normalized once, then used for both the engine call and the response, so
    // the modal can never name a matching method the engine did not use.
    let strategy_val = match strategy.unwrap_or_default().to_uppercase().as_str() {
        "LIFO" => "LIFO".to_string(),
        "HYBRID" => "HYBRID".to_string(),
        _ => "FIFO".to_string(),
    };
    let threshold_val = threshold_days.unwrap_or(30);
    let fx_applied = asset.currency != portfolio.base_currency;

    // Read-only on purpose -- see the review record for this feature. The two other
    // analytics endpoints insert auto-detected SPLIT rows before reading; this one
    // agrees with them because fetchPortfolioData loads tax-summary before the
    // ledger tab is reachable, which pins the ordering. Pinned by
    // test_sell_pnl_sums_to_tax_summary_realized_pnl.
    //
    // Same query and same scope as get_portfolio_tax_summary.
    let transactions = sqlx::query_as::<_, Transaction>("SELECT * FROM transactions WHERE asset_id = ?")
        .bind(asset.id)
        .fetch_all(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?;

    let breakdown = TaxLotEngine::sell_breakdown(
        tx.id,
        &transactions,
        &asset.currency,
        &portfolio.base_currency,
        currency_service.inner(),
        &strategy_val,
        threshold_val,
        pool.inner(),
    )
    .await
    .map_err(|_| Status::InternalServerError)?
    .ok_or(Status::NotFound)?;

    // This list is already loaded, and it is scoped to exactly the asset the replay
    // consumed. The modal reads this instead of scanning whatever list the browser
    // happens to hold, which can be one fetch stale.
    //
    // Scoped to the lots this sell actually consumed, not "any split on the asset":
    // the replay only rescales lots that already exist, so a split predating every
    // matched buy adjusted nothing and the footnote would claim otherwise. A split
    // on the same date as a buy also adjusted nothing -- splits sort first on a tie.
    let earliest_buy = breakdown.matches.iter().map(|m| m.buy_date).min();
    let split_applied = match earliest_buy {
        Some(buy_floor) => transactions.iter().any(|t| {
            t.r#type.to_uppercase() == "SPLIT" && t.date > buy_floor && t.date <= tx.date
        }),
        None => false,
    };

    // Full precision, unquantised, per the review record: no field on this response
    // is rounded. The frontend renders it with toFixed(2), so the digit tail is a
    // wire concern only, and quantising here would make this field the one exception
    // to a rule the rest of the payload obeys.
    let roi = if breakdown.cost_basis > Decimal::ZERO {
        Some(breakdown.realized_pnl / breakdown.cost_basis)
    } else {
        None
    };

    Ok(Json(SellPnlResponse {
        transaction_id: tx.id,
        asset_id: asset.id,
        symbol: asset.symbol.clone(),
        asset_type: asset.asset_type.clone(),
        strategy: strategy_val,
        threshold_days: threshold_val,
        currency: portfolio.base_currency.clone(),
        asset_currency: asset.currency.clone(),
        fx_applied,
        split_applied,
        sell_date: breakdown.sell_date,
        sold_qty: breakdown.sold_qty,
        // As entered, in asset currency.
        price: crate::db_types::str_to_decimal(&tx.price),
        fee: crate::db_types::str_to_decimal(&tx.fee),
        proceeds: breakdown.proceeds,
        cost_basis: breakdown.cost_basis,
        realized_pnl: breakdown.realized_pnl,
        roi,
        matched_qty: breakdown.matched_qty,
        unmatched_qty: breakdown.unmatched_qty,
        matches: breakdown.matches,
    }))
}
