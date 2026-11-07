//! The chokepoint: the only module permitted to resolve a portfolio, asset or
//! transaction from a caller-supplied id.
//!
//! Everything else in `api_routes` handles business logic and takes the caller as
//! given. Concentrating ownership resolution here is what makes the rule
//! checkable: `scripts/check_access_gate.sh` (see plan deviation D-7) fails a
//! handler that queries `portfolios`/`assets`/`transactions` by id without going
//! through one of the `require*` functions below.
//!
//! `require*` answers **"may this caller touch this id"** and nothing else. It
//! never returns the row: handlers keep their own `SELECT` for the data they
//! serialise. Do not "simplify" that duplication away — the gate script is written
//! to clear a handler that calls `require*` and still reads the row, because that
//! is the intended shape.

use rocket::http::Status;
use sqlx::SqlitePool;

/// What a caller needs in order to perform an operation.
///
/// There is no membership table in this change, so every level is satisfied by
/// ownership; the level argument is still threaded through so that adding
/// sharing later is a change to `resolve` and not to 15 call sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Access {
    Viewer,
    Editor,
    Owner,
}

/// Resolve what access `user_id` has to `portfolio_id`.
///
/// `Ok(None)` means no access, including a portfolio whose `owner_id` is `NULL`
/// (an unowned row is nobody's, and must not be readable by "the first user").
/// `Err` is a database failure and is never collapsed into "no access": turning a
/// `SQLITE_BUSY` into a `404` hides an outage behind what looks like a denied
/// client, and defeats the point of making denials distinguishable from real 500s.
async fn resolve(
    pool: &SqlitePool,
    user_id: i32,
    portfolio_id: i32,
) -> Result<Option<Access>, sqlx::Error> {
    let row: Option<(Option<i64>,)> =
        sqlx::query_as("SELECT owner_id FROM portfolios WHERE id = ?")
            .bind(portfolio_id as i64)
            .fetch_optional(pool)
            .await?;

    let Some((Some(owner_id),)) = row else {
        return Ok(None);
    };

    if owner_id != user_id as i64 {
        return Ok(None);
    }
    // Ownership is the only level this change can grant.
    Ok(Some(Access::Owner))
}

/// Grant `granted` if it satisfies `min`, else deny.
fn grade(granted: Access, min: Access) -> Result<Access, Status> {
    if granted >= min {
        Ok(granted)
    } else {
        Err(Status::NotFound)
    }
}

/// Deny a caller-supplied id as **not found**, never forbidden: a `403` confirms
/// the resource exists, which is the oracle this rule removes.
fn deny(what: &str, id: i32, user_id: i32) -> Status {
    eprintln!("WARN: access denied: user {user_id} has no access to {what} {id}");
    Status::NotFound
}

/// Map a resolution onto a handler-facing result, keeping database failures
/// distinguishable from denials.
fn outcome(
    resolved: Result<Option<Access>, sqlx::Error>,
    what: &str,
    id: i32,
    user_id: i32,
    min: Access,
) -> Result<Access, Status> {
    match resolved {
        Ok(Some(granted)) => grade(granted, min).map_err(|_| deny(what, id, user_id)),
        Ok(None) => Err(deny(what, id, user_id)),
        Err(e) => {
            eprintln!("ERROR: could not check {what} {id} for user {user_id}: {e}");
            Err(Status::InternalServerError)
        }
    }
}

/// Check the caller's access to a portfolio by id.
pub async fn require(
    pool: &SqlitePool,
    user_id: i32,
    portfolio_id: i32,
    min: Access,
) -> Result<Access, Status> {
    outcome(resolve(pool, user_id, portfolio_id).await, "portfolio", portfolio_id, user_id, min)
}

/// Check access to an asset by id, via its portfolio.
pub async fn require_asset(
    pool: &SqlitePool,
    user_id: i32,
    asset_id: i32,
    min: Access,
) -> Result<Access, Status> {
    let portfolio_id: Option<(i64,)> =
        sqlx::query_as("SELECT portfolio_id FROM assets WHERE id = ?")
            .bind(asset_id as i64)
            .fetch_optional(pool)
            .await
            .map_err(|e| {
                eprintln!("ERROR: could not look up asset {asset_id} for user {user_id}: {e}");
                Status::InternalServerError
            })?;

    let Some((portfolio_id,)) = portfolio_id else {
        return Err(deny("asset", asset_id, user_id));
    };

    match resolve(pool, user_id, portfolio_id as i32).await {
        Ok(Some(granted)) => grade(granted, min).map_err(|_| deny("asset", asset_id, user_id)),
        Ok(None) => Err(deny("asset", asset_id, user_id)),
        Err(e) => {
            eprintln!("ERROR: could not resolve portfolio {portfolio_id} of asset {asset_id}: {e}");
            Err(Status::InternalServerError)
        }
    }
}

/// Check access to an asset **through a portfolio path**, and reject when the
/// asset's real portfolio is not the one named in the path.
///
/// This is what stops `PUT /api/portfolios/<mine>/assets/<theirs>` from writing
/// into somebody else's asset.
pub async fn require_asset_in(
    pool: &SqlitePool,
    user_id: i32,
    path_portfolio_id: i32,
    asset_id: i32,
    min: Access,
) -> Result<Access, Status> {
    let row: Option<(i64,)> = sqlx::query_as("SELECT portfolio_id FROM assets WHERE id = ?")
        .bind(asset_id as i64)
        .fetch_optional(pool)
        .await
        .map_err(|e| {
            eprintln!("ERROR: could not look up asset {asset_id} for user {user_id}: {e}");
            Status::InternalServerError
        })?;

    let Some((real_portfolio,)) = row else {
        return Err(deny("asset", asset_id, user_id));
    };

    if real_portfolio != path_portfolio_id as i64 {
        // The path and the row disagree: deny without revealing which is wrong,
        // and without a second query to find out.
        return Err(deny("asset", asset_id, user_id));
    }

    require(pool, user_id, path_portfolio_id, min).await
}

/// Check access to a transaction by id, via its asset and portfolio.
pub async fn require_transaction(
    pool: &SqlitePool,
    user_id: i32,
    tx_id: i32,
    min: Access,
) -> Result<Access, Status> {
    let asset: Option<(i64,)> = sqlx::query_as("SELECT asset_id FROM transactions WHERE id = ?")
        .bind(tx_id as i64)
        .fetch_optional(pool)
        .await
        .map_err(|e| {
            eprintln!("ERROR: could not look up transaction {tx_id} for user {user_id}: {e}");
            Status::InternalServerError
        })?;

    let Some((asset_id,)) = asset else {
        return Err(deny("transaction", tx_id, user_id));
    };

    // A transaction whose asset no longer exists resolves to not-found. Defensive,
    // not reachable through the API: sqlx turns SQLite foreign keys ON by default, so
    // the API cannot leave a transaction pointing at a missing asset —
    // `deleting_an_asset_that_has_transactions_fails_today` pins that. A hand-edited
    // database still can, and this branch must not grant on it. The schema built in
    // `access_test.rs` has no foreign keys, which is what makes the case constructible
    // there and nowhere else.
    require_asset(pool, user_id, asset_id as i32, min).await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool_with_schema() -> SqlitePool {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE portfolios (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, \
             description TEXT, currency TEXT, base_currency TEXT, owner_id INTEGER)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE assets (id INTEGER PRIMARY KEY AUTOINCREMENT, portfolio_id INTEGER, \
             symbol TEXT, name TEXT, asset_type TEXT, sector TEXT, currency TEXT, isin TEXT)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE transactions (id INTEGER PRIMARY KEY AUTOINCREMENT, asset_id INTEGER, \
             type TEXT, quantity TEXT, price TEXT, fee TEXT, date TEXT)",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    async fn seed(pool: &SqlitePool, owner: Option<i64>) -> (i64, i64, i64) {
        let pid: (i64,) = sqlx::query_as(
            "INSERT INTO portfolios (name, currency, base_currency, owner_id) \
             VALUES ('p', 'USD', 'USD', ?) RETURNING id",
        )
        .bind(owner)
        .fetch_one(pool)
        .await
        .unwrap();
        let aid: (i64,) = sqlx::query_as(
            "INSERT INTO assets (portfolio_id, symbol, name, asset_type, currency) \
             VALUES (?, 'A', 'A', 'stock', 'USD') RETURNING id",
        )
        .bind(pid.0)
        .fetch_one(pool)
        .await
        .unwrap();
        let tid: (i64,) = sqlx::query_as(
            "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) \
             VALUES (?, 'buy', '1', '1', '0', '2024-01-01') RETURNING id",
        )
        .bind(aid.0)
        .fetch_one(pool)
        .await
        .unwrap();
        (pid.0, aid.0, tid.0)
    }

    #[tokio::test]
    async fn owner_resolves_and_every_level_is_granted() {
        let pool = pool_with_schema().await;
        let (pid, _, _) = seed(&pool, Some(1)).await;
        for min in [Access::Viewer, Access::Editor, Access::Owner] {
            assert_eq!(resolve(&pool, 1, pid as i32).await.unwrap(), Some(Access::Owner), "{min:?}");
            assert!(require(&pool, 1, pid as i32, min).await.is_ok(), "{min:?}");
        }
    }

    #[tokio::test]
    async fn another_user_is_denied_at_every_level() {
        let pool = pool_with_schema().await;
        let (pid, _, _) = seed(&pool, Some(1)).await;
        for min in [Access::Viewer, Access::Editor, Access::Owner] {
            assert_eq!(resolve(&pool, 2, pid as i32).await.unwrap(), None, "{min:?}");
            assert_eq!(
                require(&pool, 2, pid as i32, min).await.unwrap_err(),
                Status::NotFound,
                "{min:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_null_owner_is_nobody_s() {
        let pool = pool_with_schema().await;
        let (pid, _, _) = seed(&pool, None).await;
        assert_eq!(resolve(&pool, 1, pid as i32).await.unwrap(), None);
        assert_eq!(require(&pool, 1, pid as i32, Access::Viewer).await.unwrap_err(), Status::NotFound);
    }

    #[tokio::test]
    async fn a_missing_portfolio_is_not_found() {
        let pool = pool_with_schema().await;
        assert_eq!(require(&pool, 1, 999, Access::Viewer).await.unwrap_err(), Status::NotFound);
    }

    #[tokio::test]
    async fn access_orders_viewer_below_editor_below_owner() {
        assert!(Access::Viewer < Access::Editor);
        assert!(Access::Editor < Access::Owner);
    }

    #[tokio::test]
    async fn grading_a_level_below_the_minimum_denies() {
        assert_eq!(grade(Access::Owner, Access::Owner), Ok(Access::Owner));
        assert_eq!(grade(Access::Viewer, Access::Editor), Err(Status::NotFound));
    }

    #[tokio::test]
    async fn asset_and_transaction_follow_the_portfolio_owner() {
        let pool = pool_with_schema().await;
        let (_, aid, tid) = seed(&pool, Some(1)).await;
        assert!(require_asset(&pool, 1, aid as i32, Access::Viewer).await.is_ok());
        assert_eq!(
            require_asset(&pool, 2, aid as i32, Access::Viewer).await.unwrap_err(),
            Status::NotFound
        );
        assert!(require_transaction(&pool, 1, tid as i32, Access::Editor).await.is_ok());
        assert_eq!(
            require_transaction(&pool, 2, tid as i32, Access::Editor).await.unwrap_err(),
            Status::NotFound
        );
    }

    #[tokio::test]
    async fn require_asset_in_rejects_a_path_that_disagrees_with_the_row() {
        let pool = pool_with_schema().await;
        let (pid, aid, _) = seed(&pool, Some(1)).await;
        let (other_pid, _, _) = seed(&pool, Some(1)).await;
        // Right asset, wrong portfolio in the path.
        assert_eq!(
            require_asset_in(&pool, 1, other_pid as i32, aid as i32, Access::Editor)
                .await
                .unwrap_err(),
            Status::NotFound
        );
        // Right asset, right portfolio.
        assert!(require_asset_in(&pool, 1, pid as i32, aid as i32, Access::Editor).await.is_ok());
        // The same asset with no path to disagree with is fine.
        assert!(require_asset(&pool, 1, aid as i32, Access::Editor).await.is_ok());
    }

    #[tokio::test]
    async fn a_transaction_whose_asset_is_gone_is_not_found() {
        let pool = pool_with_schema().await;
        let (_, _, tid) = seed(&pool, Some(1)).await;
        sqlx::query("DELETE FROM assets").execute(&pool).await.unwrap();
        assert_eq!(
            require_transaction(&pool, 1, tid as i32, Access::Viewer).await.unwrap_err(),
            Status::NotFound
        );
    }
}
