//! The chokepoint: the only module permitted to resolve a portfolio, asset, transaction,
//! API token or staged batch from a caller-supplied id.
//!
//! Tokens and staged batches widen that remit beyond the three portfolio tables, and
//! deliberately: a token id and a batch handle are caller-supplied identifiers naming rows
//! whose whole purpose is to become authority or to become a write, so each needs the same
//! single place that answers "may this caller touch this one". Both differ from the
//! originals in one respect — the id is a string, not an `i64` — and both answer 404 for
//! "does not exist" and "not yours" alike, so neither becomes an existence oracle.
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
use crate::auth::guard::{AuthedUser, Credential};
use crate::auth::pat::Scope;
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
    let owner: Option<Option<i64>> =
        sqlx::query_scalar!("SELECT owner_id FROM portfolios WHERE id = $1", portfolio_id)
            .fetch_optional(pool)
            .await?;

    let Some(Some(owner_id)) = owner else {
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
    let portfolio_id: Option<i64> =
        sqlx::query_scalar!("SELECT portfolio_id FROM assets WHERE id = $1", asset_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| {
                eprintln!("ERROR: could not look up asset {asset_id} for user {user_id}: {e}");
                Status::InternalServerError
            })?;

    let Some(portfolio_id) = portfolio_id else {
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
    let real_portfolio: Option<i64> =
        sqlx::query_scalar!("SELECT portfolio_id FROM assets WHERE id = $1", asset_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| {
                eprintln!("ERROR: could not look up asset {asset_id} for user {user_id}: {e}");
                Status::InternalServerError
            })?;

    let Some(real_portfolio) = real_portfolio else {
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
    let asset: Option<i64> =
        sqlx::query_scalar!("SELECT asset_id FROM transactions WHERE id = $1", tx_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| {
                eprintln!("ERROR: could not look up transaction {tx_id} for user {user_id}: {e}");
                Status::InternalServerError
            })?;

    let Some(asset_id) = asset else {
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

/// May this caller touch the staged batch this handle names?
///
/// Answers 404 and never 403, for the reason every `require*` here has that rule: a 403
/// would confirm that the handle exists and belongs to somebody else. The handle namespace
/// is 32 random bytes, so guessing is not the threat — but a token that has *seen* a handle
/// elsewhere (a log line, a shared screenshot of the Access panel) must not be able to
/// confirm it is real by probing an endpoint.
///
/// The handle is a string where every other id here is `i64`. That is the one way this
/// function differs from its neighbours, and it is why the comparison is on `user_id`
/// rather than a join through a portfolio: a staged batch belongs to a user directly, not
/// to a portfolio, because it may create the portfolio it is going to fill.
pub async fn require_own_staged_batch(
    pool: &DbPool,
    user_id: i64,
    handle: &str,
    min: Access,
) -> Result<Access, Status> {
    let owner: Option<i64> =
        sqlx::query_scalar!("SELECT user_id FROM agent_staged_batch WHERE handle = $1", handle)
            .fetch_optional(pool)
            .await
            .map_err(|e| {
                eprintln!("ERROR: could not look up a staged batch for user {user_id}: {e}");
                Status::InternalServerError
            })?;

    match owner {
        Some(uid) if uid == user_id => Ok(min),
        // One message for both cases, so the two cannot be told apart from this side of the
        // network either — including in the logs an operator reads.
        _ => {
            eprintln!("DENY: staged batch not available to user {user_id}");
            Err(Status::NotFound)
        }
    }
}

/// Check that a caller-supplied `api_token` id belongs to the caller.
///
/// The same shape as `require_asset_in`, for the same reason: this module is the only
/// place a caller-supplied id resolves, and a token id is exactly such an id. Without
/// it, `DELETE /api/tokens/<id>` would let any signed-in user revoke anyone's token by
/// counting integers.
///
/// A token that is not yours answers through `deny`, i.e. 404 and not 403: the
/// existence of someone else's token id is not information to hand out.
pub async fn require_own_token(
    pool: &DbPool,
    user_id: i64,
    token_id: i64,
    min: Access,
) -> Result<Access, Status> {
    let owner: Option<i64> =
        sqlx::query_scalar!("SELECT user_id FROM api_token WHERE id = $1", token_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| {
                eprintln!("ERROR: could not look up token {token_id} for user {user_id}: {e}");
                Status::InternalServerError
            })?;

    let Some(owner) = owner else {
        return Err(deny("api_token", token_id, user_id));
    };

    if owner != user_id {
        return Err(deny("api_token", token_id, user_id));
    }

    Ok(min)
}

/// Check that a bearer-token caller holds `scope`. A session passes unconditionally.
///
/// The asymmetry is the point: a human in a browser has whatever their own access
/// allows, and asking them to also hold a scope they were never issued would be
/// theatre. A token is a program with a subset, and the subset is the whole point of
/// issuing it.
///
/// This is a ceiling on what a credential may do, not a check on who is behind it.
/// The control that proves a human approved something is the staged-batch approval
/// state, which no scope can substitute for.
pub fn require_scope(user: &AuthedUser, scope: Scope) -> Result<(), Status> {
    match &user.credential {
        Credential::Session => Ok(()),
        Credential::Token { id, scopes } => {
            if scopes.contains(&scope) {
                Ok(())
            } else {
                // Named by token id, never by secret: the id is what a log may carry.
                eprintln!(
                    "WARN: token {id} lacks scope '{}' (has {:?})",
                    scope.as_str(),
                    scopes
                );
                Err(Status::Forbidden)
            }
        }
    }
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
        sqlx::query_scalar!(
            "INSERT INTO users (issuer, subj, created_at) \
             VALUES ('https://auth.test', $1, now()) RETURNING id",
            subj
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// `owner` is a user id from [`seed_user`], or `None` for an unowned portfolio.
    async fn seed(pool: &DbPool, owner: Option<i64>) -> (i64, i64, i64) {
        let pid: i64 = sqlx::query_scalar!(
            "INSERT INTO portfolios (name, currency, base_currency, owner_id) \
             VALUES ('p', 'USD', 'USD', $1) RETURNING id",
            owner
        )
        .fetch_one(pool)
        .await
        .unwrap();
        let aid: i64 = sqlx::query_scalar!(
            "INSERT INTO assets (portfolio_id, symbol, name, asset_type, currency) \
             VALUES ($1, 'A', 'A', 'stock', 'USD') RETURNING id",
            pid
        )
        .fetch_one(pool)
        .await
        .unwrap();
        let tid: i64 = sqlx::query_scalar!(
            "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) \
             VALUES ($1, 'buy', '1', '1', '0', '2024-01-01T00:00:00Z') RETURNING id",
            aid
        )
        .fetch_one(pool)
        .await
        .unwrap();
        (pid, aid, tid)
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
        // macro-exempt: DDL. `PREPARE` accepts only SELECT/INSERT/UPDATE/DELETE/
        // MERGE/VALUES, so no sqlx macro can describe an ALTER TABLE.
        sqlx::query("ALTER TABLE transactions DROP CONSTRAINT transactions_asset_id_fkey")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query!("DELETE FROM assets").execute(&pool).await.unwrap();
        assert_eq!(
            require_transaction(&pool, owner, tid, Access::Viewer).await.unwrap_err(),
            Status::NotFound
        );
    }
}
