//! Personal access token routes — mint, list, revoke.
//!
//! These are ordinary `/api` routes over a session cookie. That is deliberate and it
//! is the whole security argument for the shape of this feature: **a token cannot
//! mint a token.** The routes that create and destroy credentials are reachable only
//! from a browser session, so a leaked agent token can read and stage, but it cannot
//! escalate itself into a new long-lived credential. It also means `AuthFairing`
//! covers them and `scripts/check_access_gate.sh` sees them, with no special case.
//!
//! No SQL here beyond what `crate::auth::pat` does. Handlers authenticate, authorise,
//! translate, and hand off.

use chrono::{DateTime, Duration, Utc};
use rocket::http::Status;
use rocket::serde::json::Json;
use rocket::State;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::access::require_own_token;
use crate::auth::guard::AuthedUser;
use crate::auth::pat::{self, PatError};
use crate::db::DbPool;

/// What the owner sends to create a token.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TokenCreate {
    /// Free text, shown in the list. The only thing that makes a token
    /// distinguishable once it is out in the world, so it is required.
    pub label: String,
    /// Scope names as the client spells them; `pat::mint` is what decides whether
    /// they exist.
    pub scopes: Vec<String>,
    /// Omitted or `null` means no expiry — the default, and the honest one.
    #[serde(default)]
    pub expires_in_days: Option<i64>,
}

/// A minted token. `secret` appears in this response and nowhere else, ever.
#[derive(Debug, Serialize, ToSchema)]
pub struct TokenMinted {
    pub id: i64,
    /// Shown once. There is no endpoint that can show it again, which is why the UI
    /// has to say so out loud rather than offer a reveal button later.
    pub secret: String,
    pub label: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// A token as listed. No secret, no hash — the query does not select either.
#[derive(Debug, Serialize, ToSchema)]
pub struct TokenOut {
    pub id: i64,
    pub label: String,
    pub scopes: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    /// Idle for 90 days or more (or minted then and never used). The UI renders this
    /// as a nudge; the threshold itself lives in the list query, not here.
    pub stale: bool,
    /// Past its expiry by the server's clock, so the list cannot show a token as live
    /// that `lookup` would refuse.
    pub expired: bool,
}

impl From<PatError> for Status {
    fn from(err: PatError) -> Status {
        match err {
            PatError::InvalidInput => Status::BadRequest,
            PatError::Entropy | PatError::Database(_) => Status::InternalServerError,
        }
    }
}

/// Upper bound on a self-chosen expiry. Not a security boundary — a user who wants a
/// token for a decade can mint one every morning — but it stops "never" being
/// spelled as a nine-digit number, and it keeps `expires_at` inside the range every
/// consumer already handles.
const MAX_EXPIRY_DAYS: i64 = 3650;

#[utoipa::path(
    post,
    path = "/api/tokens",
    request_body = TokenCreate,
    responses(
        (status = 201, description = "Token created; the secret is shown here for the first and last time", body = TokenMinted),
        (status = 400, description = "Blank label, no scopes, an unknown scope name, or an out-of-range expiry")
    )
)]
// No marker here, and it cannot have one: the marker's cross-check fails closed on
// any `<...>` in the route attribute, and `data = "<body>"` is one. `restore` in
// `backup.rs` is in the same position and for the same reason. This handler clears
// Rule A the way that file does — through `revoke_token`'s `require_own_token` call
// in this file — and Rule C by taking `AuthedUser`. It genuinely resolves no
// caller-supplied id: the only thing it reads is its own body.
#[post("/tokens", data = "<body>")]
pub async fn create_token(
    user: AuthedUser,
    pool: &State<DbPool>,
    body: Json<TokenCreate>,
) -> Result<(Status, Json<TokenMinted>), Status> {
    let expires_at = match body.expires_in_days {
        Some(days) if days <= 0 => return Err(Status::BadRequest),
        Some(days) if days > MAX_EXPIRY_DAYS => return Err(Status::BadRequest),
        Some(days) => Some(Utc::now() + Duration::days(days)),
        None => None,
    };

    let minted = pat::mint(
        pool.inner(),
        user.id,
        &body.label,
        &body.scopes,
        expires_at,
    )
    .await
    .map_err(Status::from)?;

    // Rocket answers 200 for a bare `Json`, so a created resource has to say so.
    // The `#[utoipa::path]` above already promises 201; several older routes in this
    // crate document a status they do not return, and this one is not joining them.
    Ok((
        Status::Created,
        Json(TokenMinted {
            id: minted.id,
            secret: minted.secret,
            label: minted.label,
            scopes: minted.scopes.iter().map(|s| s.as_str().to_string()).collect(),
            expires_at: minted.expires_at,
            created_at: minted.created_at,
        }),
    ))
}

#[utoipa::path(
    get,
    path = "/api/tokens",
    responses(
        (status = 200, description = "This user's tokens, newest first, without secrets", body = [TokenOut])
    )
)]
// access-gate: global-config (no caller-supplied id)
#[get("/tokens")]
pub async fn list_tokens(
    user: AuthedUser,
    pool: &State<DbPool>,
) -> Result<Json<Vec<TokenOut>>, Status> {
    let rows = pat::list(pool.inner(), user.id)
        .await
        .map_err(Status::from)?;

    Ok(Json(
        rows.into_iter()
            .map(|t| TokenOut {
                id: t.id,
                label: t.label,
                scopes: t.scopes.iter().map(|s| s.as_str().to_string()).collect(),
                created_at: t.created_at,
                expires_at: t.expires_at,
                last_used_at: t.last_used_at,
                revoked_at: t.revoked_at,
                stale: t.stale,
                expired: t.expired,
            })
            .collect(),
    ))
}

#[utoipa::path(
    delete,
    path = "/api/tokens/<id>",
    responses(
        (status = 204, description = "Token revoked"),
        (status = 404, description = "No such token, or not yours")
    )
)]
#[delete("/tokens/<id>")]
pub async fn revoke_token(
    user: AuthedUser,
    pool: &State<DbPool>,
    id: i64,
) -> Result<Status, Status> {
    // The id is caller-supplied, so it resolves in `access.rs` and nowhere else.
    require_own_token(pool.inner(), user.id, id, crate::access::Access::Owner).await?;

    // A token that is already revoked answers 204 as well. Revocation is
    // idempotent from the owner's point of view, and a second DELETE that reports
    // 404 would make a retried request look like a failure.
    pat::revoke(pool.inner(), user.id, id)
        .await
        .map_err(Status::from)?;

    Ok(Status::NoContent)
}
