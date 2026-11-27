//! The human half of the approval gate.
//!
//! Three routes, and between them they are the only way anything in this application turns
//! a staged proposal into a write. That is the point: the agent can stage and it can ask,
//! but `staged -> approved` is a state transition only a session cookie can cause, so it
//! lives here and not under `/mcp`. A scope would have been the wrong shape — a token that
//! holds a scope can use it whenever it likes, which is the hole a scope-shaped control
//! leaves open.
//!
//! The web UI never writes a portfolio row. Its only write is the approval.
//!
//! All three are ordinary `/api` routes, so `AuthFairing` covers them, `check_access_gate.sh`
//! sees them, and a leaked agent token cannot reach any of them — which is the same
//! property that makes `POST /api/tokens` safe to exist.

use rocket::http::Status;
use rocket::serde::json::Json;
use rocket::State;
use serde::Serialize;
use utoipa::ToSchema;
use serde_json::{json, Value};

use crate::access::{require_own_staged_batch, Access};
use crate::agent::staging;
use crate::auth::guard::AuthedUser;
use crate::db::DbPool;

/// One row of the list. No payload and no preview: the list is a menu, and the detail route
/// is where the bytes are.
#[derive(Debug, Serialize, ToSchema)]
pub struct BatchOut {
    pub handle: String,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub approved_at: Option<chrono::DateTime<chrono::Utc>>,
    pub applied_at: Option<chrono::DateTime<chrono::Utc>>,
    pub asset_count: i64,
    pub transaction_count: i64,
    /// Computed here rather than stored, because "is this still approvable" is a fact about
    /// the clock and not about the row: an `approved` batch past `expires_at` can never be
    /// applied, and showing it as approvable invites a human to approve something that will
    /// then be refused.
    pub expired: bool,
}

impl From<staging::BatchSummary> for BatchOut {
    fn from(s: staging::BatchSummary) -> Self {
        let expired = s.expires_at <= chrono::Utc::now();
        Self {
            handle: s.handle,
            status: s.status,
            created_at: s.created_at,
            expires_at: s.expires_at,
            approved_at: s.approved_at,
            applied_at: s.applied_at,
            asset_count: s.asset_count,
            transaction_count: s.transaction_count,
            expired,
        }
    }
}

/// Every error this module answers with is a status *and* a JSON body.
///
/// A bare `Status` as a handler's error type renders Rocket's default error page, and
/// there is no `404` catcher in this application — only the `401` one, which is why the
/// unauthenticated sweep can assert a JSON body on a `401` and nothing else. A panel that
/// has to parse HTML to explain "that batch is gone" is a panel that shows the user
/// nothing, so the shape is stated once here and used by all three routes.
fn reject(status: Status, code: &'static str, detail: &str) -> (Status, Json<Value>) {
    (
        status,
        Json(json!({"error": code, "detail": detail})),
    )
}

/// List the caller's staged batches, newest first.
///
/// No caller-supplied id: the caller *is* the selector, so there is nothing for
/// `access.rs` to resolve.
// access-gate: global-config (no caller-supplied id)
#[utoipa::path(
    get,
    path = "/api/staged-batches",
    responses(
        (status = 200, description = "This user's staged batches, newest first", body = [BatchOut]),
        (status = 401, description = "Not authenticated")
    )
)]
#[get("/staged-batches")]
pub async fn list_batches(
    user: AuthedUser,
    pool: &State<DbPool>,
) -> Result<Json<Vec<BatchOut>>, (Status, Json<Value>)> {
    let rows = staging::list(pool.inner(), user.id)
        .await
        .map_err(|_| reject(Status::InternalServerError, "database", "could not list batches"))?;
    Ok(Json(rows.into_iter().map(BatchOut::from).collect()))
}

/// The stored preview, rendered verbatim — plus three envelope fields.
///
/// The UI renders this JSON and does not recompute it. That is not a rendering preference:
/// if the preview were derived at render time, what a human read and what the agent was
/// shown could differ, and the approval would be an approval of something else.
///
/// "Verbatim" is about the preview object, not about the response body: `status`,
/// `expires_at` and `expired` are added to it below, because the panel needs the state to
/// decide whether to offer the approve button and the preview itself is stateless. The
/// preview's own keys are untouched and none of the three collides with one.
#[utoipa::path(
    get,
    path = "/api/staged-batches/<handle>",
    responses(
        (status = 200, description = "The stored preview verbatim, plus `status`, `expires_at` and `expired`", body = Value),
        (status = 401, description = "Not authenticated"),
        (status = 404, description = "No such batch for this user")
    )
)]
#[get("/staged-batches/<handle>")]
pub async fn get_batch(
    user: AuthedUser,
    pool: &State<DbPool>,
    handle: &str,
) -> Result<Json<Value>, (Status, Json<Value>)> {
    require_own_staged_batch(pool.inner(), user.id, handle, Access::Owner)
        .await
        .map_err(|_| reject(Status::NotFound, "not_found", "no such staged batch"))?;

    let found = staging::preview_of(pool.inner(), user.id, handle)
        .await
        .map_err(|_| reject(Status::InternalServerError, "database", "could not read batch"))?;

    match found {
        Some((status, preview, expires_at)) => {
            let mut body = preview;
            // Attached to the same object rather than wrapped in an envelope, so the UI
            // renders `body` and cannot accidentally render the wrapper instead of the
            // preview it was told to render.
            if let Some(obj) = body.as_object_mut() {
                obj.insert("status".into(), json!(status));
                obj.insert("expires_at".into(), json!(expires_at.to_rfc3339()));
                obj.insert("expired".into(), json!(expires_at <= chrono::Utc::now()));
            }
            Ok(Json(body))
        }
        // The chokepoint already passed, so this is a race (the batch was swept between the
        // two statements) rather than a permission answer. Same 404 either way, because the
        // two must stay indistinguishable to a caller.
        None => Err(reject(
            Status::NotFound,
            "not_found",
            "no such staged batch",
        )),
    }
}

/// `staged -> approved`. The human act, and the only write this UI performs.
///
/// A conditional update inside `staging::approve`, so two tabs cannot both succeed; the
/// loser gets 409 naming the current status rather than a silent second success.
#[utoipa::path(
    post,
    path = "/api/staged-batches/<handle>/approve",
    responses(
        (status = 200, description = "Approved", body = Value),
        (status = 401, description = "Not authenticated"),
        (status = 404, description = "No such batch for this user"),
        (status = 409, description = "Not in a state that can be approved")
    )
)]
#[post("/staged-batches/<handle>/approve")]
pub async fn approve_batch(
    user: AuthedUser,
    pool: &State<DbPool>,
    handle: &str,
) -> Result<(Status, Json<Value>), (Status, Json<Value>)> {
    // `Status` is not a structural type, so this cannot be a `match` on the variant; a
    // comparison is the only way to ask. The distinction is worth keeping: 404 is the
    // chokepoint's answer and must stay a 404, while anything else it ever learns to
    // return should pass through rather than be relabelled.
    require_own_staged_batch(pool.inner(), user.id, handle, Access::Owner)
        .await
        .map_err(|_| reject(Status::NotFound, "not_found", "no such staged batch"))?;

    let approved = staging::approve(pool.inner(), user.id, handle)
        .await
        .map_err(|_| {
            reject(
                Status::InternalServerError,
                "database",
                "could not record the approval",
            )
        })?;

    if approved {
        return Ok((
            Status::Ok,
            Json(json!({"status": "approved", "detail": "an agent may now apply this batch"})),
        ));
    }

    // The update refused, so say why with the state the row is actually in. Reading it back
    // is not a check-then-act: the decision has already been made by the conditional update,
    // and this only describes the outcome.
    //
    // `expired` is part of the answer because `status` alone would lie. An expired batch is
    // still `staged`, and "only a batch at status 'staged' can be approved" read to a human
    // as though clicking again would work. It would not: expiry is in the same predicate now.
    let current = staging::preview_of(pool.inner(), user.id, handle)
        .await
        .ok()
        .flatten();
    let (current_status, expired) = match current {
        Some((status, _, expires_at)) => (status, expires_at <= chrono::Utc::now()),
        None => ("gone".to_string(), false),
    };

    Err((
        Status::Conflict,
        Json(json!({
            "error": "not_approvable",
            "current_status": current_status,
            "expired": expired,
            "detail": if expired {
                "this batch has expired; it can no longer be approved or applied"
            } else {
                "only a batch at status 'staged' can be approved"
            },
        })),
    ))
}
