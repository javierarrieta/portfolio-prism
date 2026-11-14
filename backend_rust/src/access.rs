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
use crate::db::DbPool;

// Every id in this module is `i64`, the width of every id column in
// `migrations/0001_init.sql`. That matters more here than anywhere else in the crate:
// a narrowing on these parameters is not a wrong number in a response body, it is an
// authorization decision made about a *different row* than the one the caller named.
// `resolve` compares against `portfolios.owner_id` directly, so no id may arrive here
// having been squeezed through `i32`.

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
/// lock or pool timeout into a `404` hides an outage behind what looks like a
/// denied client, and defeats the point of making denials distinguishable from
/// real 500s.
async fn resolve(
    pool: &DbPool,
    user_id: i64,
    portfolio_id: i64,
) -> Result<Option<Access>, sqlx::Error> {
    let row: Option<(Option<i64>,)> =
        sqlx::query_as("SELECT owner_id FROM portfolios WHERE id = $1")
            .bind(portfolio_id)
            .fetch_optional(pool)
            .await?;

    let Some((Some(owner_id),)) = row else {
        return Ok(None);
    };

    if owner_id != user_id {
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
fn deny(what: &str, id: i64, user_id: i64) -> Status {
    eprintln!("WARN: access denied: user {user_id} has no access to {what} {id}");
    Status::NotFound
}

/// Map a resolution onto a handler-facing result, keeping database failures
/// distinguishable from denials.
fn outcome(
    resolved: Result<Option<Access>, sqlx::Error>,
    what: &str,
    id: i64,
    user_id: i64,
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
    pool: &DbPool,
    user_id: i64,
    portfolio_id: i64,
    min: Access,
) -> Result<Access, Status> {
    outcome(
        resolve(pool, user_id, portfolio_id).await,
        "portfolio",
        portfolio_id,
        user_id,
        min,
    )
}

/// Check access to an asset by id, via its portfolio.
pub async fn require_asset(
    pool: &DbPool,
    user_id: i64,
    asset_id: i64,
    min: Access,
) -> Result<Access, Status> {
    let portfolio_id: Option<(i64,)> =
        sqlx::query_as("SELECT portfolio_id FROM assets WHERE id = $1")
            .bind(asset_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| {
                eprintln!("ERROR: could not look up asset {asset_id} for user {user_id}: {e}");
                Status::InternalServerError
            })?;

    let Some((portfolio_id,)) = portfolio_id else {
        return Err(deny("asset", asset_id, user_id));
    };

    match resolve(pool, user_id, portfolio_id).await {
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
    pool: &DbPool,
    user_id: i64,
    path_portfolio_id: i64,
    asset_id: i64,
    min: Access,
) -> Result<Access, Status> {
    let row: Option<(i64,)> = sqlx::query_as("SELECT portfolio_id FROM assets WHERE id = $1")
        .bind(asset_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| {
            eprintln!("ERROR: could not look up asset {asset_id} for user {user_id}: {e}");
            Status::InternalServerError
        })?;

    let Some((real_portfolio,)) = row else {
        return Err(deny("asset", asset_id, user_id));
    };

    if real_portfolio != path_portfolio_id {
        // The path and the row disagree: deny without revealing which is wrong,
        // and without a second query to find out.
        return Err(deny("asset", asset_id, user_id));
    }

    require(pool, user_id, path_portfolio_id, min).await
}

/// Check access to a transaction by id, via its asset and portfolio.
pub async fn require_transaction(
    pool: &DbPool,
    user_id: i64,
    tx_id: i64,
    min: Access,
) -> Result<Access, Status> {
    let asset: Option<(i64,)> = sqlx::query_as("SELECT asset_id FROM transactions WHERE id = $1")
        .bind(tx_id)
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
    // not reachable through the API: `transactions.asset_id` is a foreign key, so the
    // API cannot leave a transaction pointing at a missing asset —
    // `deleting_an_asset_that_has_transactions_fails_today` pins that. A hand-edited
    // database still can, and this branch must not grant on it;
    // `a_transaction_whose_asset_is_gone_is_not_found` constructs that case by
    // dropping the constraint, because nothing else can.
    require_asset(pool, user_id, asset_id, min).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_db;

    /// The migrated schema, in a database this test owns alone.
    ///
    /// Unlike the SQLite fixture this replaces, it is the real schema — the one
    /// `migrations/0001_init.sql` builds, foreign keys included. The one place that
    /// needs a shape the real schema forbids says so where it does it
    /// ([`a_transaction_whose_asset_is_gone_is_not_found`]).
    async fn pool_with_schema() -> DbPool {
        test_db::test_pool(&test_db::next_caller("access")).await
    }

    /// A real `users` row, because `portfolios.owner_id` is a foreign key here and
    /// will not hold an id that does not exist.
    async fn seed_user(pool: &DbPool, subj: &str) -> i64 {
        sqlx::query_as::<_, (i64,)>(
            "INSERT INTO users (issuer, subj, created_at) \
             VALUES ('https://auth.test', $1, now()) RETURNING id",
        )
        .bind(subj)
        .fetch_one(pool)
        .await
        .unwrap()
        .0
    }

    /// `owner` is a user id from [`seed_user`], or `None` for an unowned portfolio.
    async fn seed(pool: &DbPool, owner: Option<i64>) -> (i64, i64, i64) {
        let pid: (i64,) = sqlx::query_as(
            "INSERT INTO portfolios (name, currency, base_currency, owner_id) \
             VALUES ('p', 'USD', 'USD', $1) RETURNING id",
        )
        .bind(owner)
        .fetch_one(pool)
        .await
        .unwrap();
        let aid: (i64,) = sqlx::query_as(
            "INSERT INTO assets (portfolio_id, symbol, name, asset_type, currency) \
             VALUES ($1, 'A', 'A', 'stock', 'USD') RETURNING id",
        )
        .bind(pid.0)
        .fetch_one(pool)
        .await
        .unwrap();
        let tid: (i64,) = sqlx::query_as(
            "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) \
             VALUES ($1, 'buy', '1', '1', '0', '2024-01-01T00:00:00Z') RETURNING id",
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
        let owner = seed_user(&pool, "owner").await;
        let (pid, _, _) = seed(&pool, Some(owner)).await;
        for min in [Access::Viewer, Access::Editor, Access::Owner] {
            assert_eq!(resolve(&pool, owner, pid).await.unwrap(), Some(Access::Owner), "{min:?}");
            assert!(require(&pool, owner, pid, min).await.is_ok(), "{min:?}");
        }
    }

    #[tokio::test]
    async fn another_user_is_denied_at_every_level() {
        let pool = pool_with_schema().await;
        let owner = seed_user(&pool, "owner").await;
        let other = seed_user(&pool, "other").await;
        let (pid, _, _) = seed(&pool, Some(owner)).await;
        for min in [Access::Viewer, Access::Editor, Access::Owner] {
            assert_eq!(resolve(&pool, other, pid).await.unwrap(), None, "{min:?}");
            assert_eq!(
                require(&pool, other, pid, min).await.unwrap_err(),
                Status::NotFound,
                "{min:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_null_owner_is_nobody_s() {
        let pool = pool_with_schema().await;
        let owner = seed_user(&pool, "owner").await;
        let (pid, _, _) = seed(&pool, None).await;
        assert_eq!(resolve(&pool, owner, pid).await.unwrap(), None);
        assert_eq!(require(&pool, owner, pid, Access::Viewer).await.unwrap_err(), Status::NotFound);
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
        let owner = seed_user(&pool, "owner").await;
        let other = seed_user(&pool, "other").await;
        let (_, aid, tid) = seed(&pool, Some(owner)).await;
        assert!(require_asset(&pool, owner, aid, Access::Viewer).await.is_ok());
        assert_eq!(
            require_asset(&pool, other, aid, Access::Viewer).await.unwrap_err(),
            Status::NotFound
        );
        assert!(require_transaction(&pool, owner, tid, Access::Editor).await.is_ok());
        assert_eq!(
            require_transaction(&pool, other, tid, Access::Editor).await.unwrap_err(),
            Status::NotFound
        );
    }

    #[tokio::test]
    async fn require_asset_in_rejects_a_path_that_disagrees_with_the_row() {
        let pool = pool_with_schema().await;
        let owner = seed_user(&pool, "owner").await;
        let (pid, aid, _) = seed(&pool, Some(owner)).await;
        let (other_pid, _, _) = seed(&pool, Some(owner)).await;
        // Right asset, wrong portfolio in the path.
        assert_eq!(
            require_asset_in(&pool, owner, other_pid, aid, Access::Editor)
                .await
                .unwrap_err(),
            Status::NotFound
        );
        // Right asset, right portfolio.
        assert!(require_asset_in(&pool, owner, pid, aid, Access::Editor).await.is_ok());
        // The same asset with no path to disagree with is fine.
        assert!(require_asset(&pool, owner, aid, Access::Editor).await.is_ok());
    }

    #[tokio::test]
    async fn a_transaction_whose_asset_is_gone_is_not_found() {
        let pool = pool_with_schema().await;
        let owner = seed_user(&pool, "owner").await;
        let (_, _, tid) = seed(&pool, Some(owner)).await;
        // The real schema forbids this row: `transactions.asset_id` is a foreign
        // key, so no sequence of API calls can leave a transaction pointing at a
        // missing asset. Dropping the constraint is the hand-edited database that
        // `require_transaction`'s last branch exists for, and it is now stated as
        // such instead of being a property of a hand-written test schema.
        sqlx::query("ALTER TABLE transactions DROP CONSTRAINT transactions_asset_id_fkey")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM assets").execute(&pool).await.unwrap();
        assert_eq!(
            require_transaction(&pool, owner, tid, Access::Viewer).await.unwrap_err(),
            Status::NotFound
        );
    }
}
