use rocket::{State, serde::json::Json, http::Status};
use sqlx::SqlitePool;
use std::collections::HashMap;
use crate::models::{Portfolio as DbPortfolio, Asset as DbAsset, Transaction as DbTransaction};
use crate::schemas::{AssetOut, PortfolioCreate, PortfolioOut, PortfolioUpdate, TransactionOut};
use crate::access::{require, Access};
use crate::auth::guard::AuthedUser;

#[utoipa::path(
    post,
    path = "/api/portfolios/",
    responses(
        (status = 201, description = "Portfolio created", body = PortfolioOut),
        (status = 400, description = "Bad request")
    )
)]
#[post("/", data = "<portfolio>")]
pub async fn create_portfolio(
    user: AuthedUser,
    pool: &State<SqlitePool>,
    portfolio: Json<PortfolioCreate>
) -> Result<Json<PortfolioOut>, Status> {
    // The creator is the owner. There is no other write path for owner_id in this
    // change, which is what makes ownership trustworthy.
    let res = sqlx::query_as::<_, DbPortfolio>(
        "INSERT INTO portfolios (name, description, currency, base_currency, owner_id) 
         VALUES (?, ?, ?, ?, ?) RETURNING *"
    )
    .bind(&portfolio.name)
    .bind(&portfolio.description)
    .bind(&portfolio.currency)
    .bind(&portfolio.currency) // Using currency as base_currency default
    .bind(user.id as i64)
    .fetch_one(pool.inner())
    .await;

    match res {
        Ok(p) => {
            let assets = fetch_assets_for_portfolio(pool.inner(), p.id).await
                .map_err(|_| Status::InternalServerError)?;
            Ok(Json(PortfolioOut {
                id: p.id,
                name: p.name,
                description: p.description,
                currency: p.currency,
                assets,
                splits_last_synced_at: split_sync_age(pool.inner(), p.id).await
                    .map_err(|_| Status::InternalServerError)?,
            }))
        },
        Err(_) => Err(Status::InternalServerError),
    }
}

/// Oldest split-sync timestamp across a portfolio's assets, or `None` when the
/// portfolio is not fully covered (#50).
///
/// `None` means "at least one asset here has never had its splits looked up", which
/// covers both the never-synced-at-all portfolio and the one where an asset was added
/// after the last sync. Collapsing those is deliberate: the actionable state is the
/// same, and the alternative — reporting the age of the assets that *were* synced —
/// is exactly the silent staleness this field exists to prevent.
///
/// `MIN` over the covered assets rather than `MAX`: the portfolio is only as fresh as
/// its least recently synced holding.
pub async fn split_sync_age(pool: &sqlx::sqlite::SqlitePool, portfolio_id: i32) -> Result<Option<String>, sqlx::Error> {
    let (total, synced, oldest): (i64, i64, Option<String>) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM assets WHERE portfolio_id = ?),
                (SELECT COUNT(*) FROM assets a JOIN split_sync s ON s.asset_id = a.id
                  WHERE a.portfolio_id = ?),
                (SELECT MIN(s.last_synced_at) FROM assets a JOIN split_sync s ON s.asset_id = a.id
                  WHERE a.portfolio_id = ?)"
    )
    .bind(portfolio_id)
    .bind(portfolio_id)
    .bind(portfolio_id)
    .fetch_one(pool)
    .await?;

    Ok(if total > 0 && synced == total { oldest } else { None })
}

/// Batch form for `list_portfolios`, so the dashboard does not pay one query per
/// portfolio. Same semantics as `split_sync_age`.
pub async fn split_sync_ages(pool: &sqlx::sqlite::SqlitePool) -> Result<HashMap<i32, Option<String>>, sqlx::Error> {
    let rows: Vec<(i32, i64, i64, Option<String>)> = sqlx::query_as(
        "SELECT p.id,
                (SELECT COUNT(*) FROM assets a WHERE a.portfolio_id = p.id),
                (SELECT COUNT(*) FROM assets a JOIN split_sync s ON s.asset_id = a.id
                  WHERE a.portfolio_id = p.id),
                (SELECT MIN(s.last_synced_at) FROM assets a JOIN split_sync s ON s.asset_id = a.id
                  WHERE a.portfolio_id = p.id)
         FROM portfolios p"
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(id, total, synced, oldest)| {
            (id, if total > 0 && synced == total { oldest } else { None })
        })
        .collect())
}

async fn fetch_assets_for_portfolio(pool: &sqlx::sqlite::SqlitePool, portfolio_id: i32) -> Result<Vec<AssetOut>, sqlx::Error> {
    let assets = sqlx::query_as::<_, DbAsset>("SELECT * FROM assets WHERE portfolio_id = ?")
        .bind(portfolio_id)
        .fetch_all(pool)
        .await?;

    let asset_ids: Vec<i32> = assets.iter().map(|a| a.id).collect();

    let all_txs = if asset_ids.is_empty() {
        vec![]
    } else {
        let placeholders: Vec<String> = (0..asset_ids.len()).map(|_| "?".to_string()).collect();
        let sql = format!("SELECT * FROM transactions WHERE asset_id IN ({})", placeholders.join(","));
        let mut query = sqlx::query_as::<_, DbTransaction>(&sql);
        for id in &asset_ids {
            query = query.bind(id);
        }
        query.fetch_all(pool).await?
    };

    let mut txs_by_asset: HashMap<i32, Vec<TransactionOut>> = HashMap::new();
    for tx in all_txs {
        txs_by_asset
            .entry(tx.asset_id)
            .or_default()
            .push(TransactionOut {
                id: tx.id,
                asset_id: tx.asset_id,
                r#type: tx.r#type,
                quantity: crate::db_types::str_to_decimal(&tx.quantity),
                price: crate::db_types::str_to_decimal(&tx.price),
                fee: crate::db_types::str_to_decimal(&tx.fee),
                date: tx.date,
            });
    }

    let out = assets.into_iter().map(|a| AssetOut {
        id: a.id,
        portfolio_id: a.portfolio_id,
        symbol: a.symbol,
        name: a.name,
        asset_type: a.asset_type,
        sector: a.sector,
        currency: a.currency,
        isin: a.isin,
        transactions: txs_by_asset.remove(&a.id).unwrap_or_default(),
    }).collect();

    Ok(out)
}

#[utoipa::path(
    get,
    path = "/api/portfolios/",
    responses(
        (status = 200, description = "List of portfolios", body = [PortfolioOut])
    )
)]
#[get("/")]
pub async fn list_portfolios(
    user: AuthedUser,
    pool: &State<SqlitePool>) -> Result<Json<Vec<PortfolioOut>>, Status> {
    // Both queries are scoped. Filtering only the portfolios would still return
    // every other user's asset rows, because the assets query had no filter at
    // all and the handler groups them by portfolio_id afterwards.
    let portfolios = sqlx::query_as::<_, DbPortfolio>("SELECT * FROM portfolios WHERE owner_id = ?")
        .bind(user.id as i64)
        .fetch_all(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?;

    let all_assets = sqlx::query_as::<_, DbAsset>(
        "SELECT * FROM assets WHERE portfolio_id IN (SELECT id FROM portfolios WHERE owner_id = ?)",
    )
        .bind(user.id as i64)
        .fetch_all(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?;

    let mut by_portfolio: HashMap<i32, Vec<AssetOut>> = all_assets
        .into_iter()
        .map(|a| AssetOut {
            id: a.id,
            portfolio_id: a.portfolio_id,
            symbol: a.symbol,
            name: a.name,
            asset_type: a.asset_type,
            sector: a.sector,
            currency: a.currency,
            isin: a.isin,
            transactions: vec![],
        })
        .fold(HashMap::new(), |mut map, asset| {
            map.entry(asset.portfolio_id).or_default().push(asset);
            map
        });

    // One grouped query for every portfolio, so the dashboard's staleness badges do
    // not cost a query per row.
    let ages = split_sync_ages(pool.inner()).await
        .map_err(|_| Status::InternalServerError)?;

    let out = portfolios.into_iter().map(|p| PortfolioOut {
        id: p.id,
        name: p.name,
        description: p.description,
        currency: p.currency,
        assets: by_portfolio.remove(&p.id).unwrap_or_default(),
        splits_last_synced_at: ages.get(&p.id).cloned().flatten(),
    }).collect();

    Ok(Json(out))
}

#[utoipa::path(
    get,
    path = "/api/portfolios/<id>",
    responses(
        (status = 200, description = "Portfolio found", body = PortfolioOut),
        (status = 404, description = "Portfolio not found")
    )
)]
#[get("/<id>")]
pub async fn get_portfolio(user: AuthedUser, id: i32, pool: &State<SqlitePool>) -> Result<Json<PortfolioOut>, Status> {
    require(pool.inner(), user.id, id, Access::Viewer).await?;

    // Kept deliberately: `require` answers whether the caller may touch this id and
    // never returns the row, so the read below is the response body, not a check.
    let p = sqlx::query_as::<_, DbPortfolio>("SELECT * FROM portfolios WHERE id = ?")
        .bind(id)
        .fetch_optional(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?;

    match p {
        Some(p) => {
            let assets = fetch_assets_for_portfolio(pool.inner(), p.id).await
                .map_err(|_| Status::InternalServerError)?;
            Ok(Json(PortfolioOut {
                id: p.id,
                name: p.name,
                description: p.description,
                currency: p.currency,
                assets,
                splits_last_synced_at: split_sync_age(pool.inner(), p.id).await
                    .map_err(|_| Status::InternalServerError)?,
            }))
        },
        None => Err(Status::NotFound),
    }
}

#[utoipa::path(
    patch,
    path = "/api/portfolios/<id>",
    responses(
        (status = 200, description = "Portfolio updated", body = PortfolioOut),
        (status = 404, description = "Portfolio not found")
    )
)]
#[patch("/<id>", data = "<update>")]
pub async fn update_portfolio(
    user: AuthedUser,
    id: i32,
    pool: &State<SqlitePool>,
    update: Json<PortfolioUpdate>
) -> Result<Json<PortfolioOut>, Status> {
    require(pool.inner(), user.id, id, Access::Owner).await?;

    let new_currency = &update.currency;

    sqlx::query("UPDATE portfolios SET currency = ?, base_currency = ? WHERE id = ?")
        .bind(new_currency)
        .bind(new_currency)
        .bind(id)
        .execute(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?;

    let p = sqlx::query_as::<_, DbPortfolio>("SELECT * FROM portfolios WHERE id = ?")
        .bind(id)
        .fetch_optional(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?;

    match p {
        Some(p) => {
            let assets = fetch_assets_for_portfolio(pool.inner(), p.id).await
                .map_err(|_| Status::InternalServerError)?;
            Ok(Json(PortfolioOut {
                id: p.id,
                name: p.name,
                description: p.description,
                currency: p.currency,
                assets,
                splits_last_synced_at: split_sync_age(pool.inner(), p.id).await
                    .map_err(|_| Status::InternalServerError)?,
            }))
        },
        None => Err(Status::NotFound),
    }
}

#[utoipa::path(
    delete,
    path = "/api/portfolios/<id>",
    responses(
        (status = 204, description = "Portfolio deleted"),
        (status = 404, description = "Portfolio not found")
    )
)]
#[delete("/<id>")]
pub async fn delete_portfolio(user: AuthedUser, id: i32, pool: &State<SqlitePool>) -> Result<Status, Status> {
    // Before the asset-count check: otherwise a non-owner could tell "exists and is
    // empty" (409) from "does not exist" (404) and use the difference as an
    // existence oracle.
    require(pool.inner(), user.id, id, Access::Owner).await?;

    let portfolio = sqlx::query_as::<_, DbPortfolio>("SELECT * FROM portfolios WHERE id = ?")
        .bind(id)
        .fetch_optional(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?;

    match portfolio {
        Some(_) => {},
        None => return Err(Status::NotFound),
    }

    let asset_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM assets WHERE portfolio_id = ?")
        .bind(id)
        .fetch_one(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?;

    if asset_count.0 > 0 {
        return Err(Status::Conflict);
    }

    let res = sqlx::query("DELETE FROM portfolios WHERE id = ?")
        .bind(id)
        .execute(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?;

    if res.rows_affected() == 0 {
        return Err(Status::NotFound);
    }

    Ok(Status::NoContent)
}
