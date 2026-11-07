//! The request guard that hands a handler its caller.
//!
//! [`AuthedUser`] is what a handler asks for. It is *not* the enforcement
//! mechanism — [`crate::auth::fairing::AuthFairing`] is, because a guard only
//! protects the handlers that remember to ask for it. The guard's job is to give
//! a handler a typed caller and a single vocabulary for "who is this".

use std::io::Cursor;

use rocket::http::{ContentType, Status};
use rocket::request::{self, FromRequest, Outcome};
use rocket::response::Responder;
use rocket::{Request, Response};
use sqlx::SqlitePool;

use super::session;
use super::AuthMode;

/// The authenticated caller.
#[derive(Debug, Clone)]
pub struct AuthedUser {
    pub id: i32,
    pub email: Option<String>,
    pub name: Option<String>,
    pub sub: String,
}

/// The one definition of the unauthenticated response body.
///
/// Both the guard (which answers `/auth/me`) and the `/_auth/denied` sentinel
/// (which answers `/api/…`) return this, so the two cannot drift apart. A bare
/// `Status::Unauthorized` would give Rocket's default body and the two paths
/// would disagree about what a `401` looks like.
pub struct Unauthenticated;

impl std::fmt::Debug for Unauthenticated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Unauthenticated")
    }
}

/// Body shared by every `401` this application produces.
pub(crate) const UNAUTHENTICATED_BODY: &str = r#"{"error":"unauthenticated"}"#;

impl Responder<'_, 'static> for Unauthenticated {
    fn respond_to(self, _: &Request<'_>) -> Result<Response<'static>, Status> {
        Response::build()
            .status(Status::Unauthorized)
            .header(ContentType::JSON)
            .sized_body(UNAUTHENTICATED_BODY.len(), Cursor::new(UNAUTHENTICATED_BODY))
            .ok()
    }
}

/// The columns `resolve_caller` reads, in select order.
pub(crate) type UserRow = (i64, Option<String>, Option<String>, String, i64, i64);

/// Resolve the caller, or explain why there is none.
///
/// `Ok(None)` means "no valid session" — deny. `Err(db)` means the database
/// failed, which is a 500 and not a denial: collapsing a `SQLITE_BUSY` into a
/// `401` would hide an outage behind what looks like an unauthenticated client,
/// and would make the fairing rewrite a request that a working database would
/// have served.
///
/// This signature is the contract for the fairing (Task 6) and the guard below;
/// both must handle `Err` distinctly from `Ok(None)`.
pub async fn resolve_caller(
    req: &Request<'_>,
    mode: &AuthMode,
    pool: &SqlitePool,
) -> Result<Option<AuthedUser>, sqlx::Error> {
    // `expected_tv` is `None` in development mode, where there is no session to
    // compare a token version against.
    let (uid, expected_tv) = match mode {
        AuthMode::Oidc(_) => {
            let Some(payload) = session::read(req.cookies()) else {
                return Ok(None);
            };
            if payload.exp <= chrono::Utc::now().timestamp() {
                // Server-side expiry. The cookie is still well-formed and
                // correctly signed; it has simply outlived its lifetime.
                return Ok(None);
            }
            (payload.uid, Some(payload.tv))
        }
        #[cfg(debug_assertions)]
        AuthMode::Disabled { as_user } => (*as_user as i64, None),
    };

    // (id, email, name, subj, is_active, token_version)
    let row: Option<UserRow> = sqlx::query_as(
        "SELECT id, email, name, subj, is_active, token_version FROM users WHERE id = ?",
    )
    .bind(uid)
    .fetch_optional(pool)
    .await?;

    let Some((id, email, name, sub, is_active, token_version)) = row else {
        return Ok(None);
    };

    // A deactivated account, or a session minted before a `logout-all`, is
    // denied the same way a missing cookie is.
    if is_active == 0 {
        return Ok(None);
    }
    if let Some(tv) = expected_tv {
        // A session minted before a `logout-all` carries the old version.
        if tv != token_version {
            return Ok(None);
        }
    }

    Ok(Some(AuthedUser { id: id as i32, email, name, sub }))
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AuthedUser {
    type Error = Unauthenticated;

    async fn from_request(req: &'r Request<'_>) -> request::Outcome<Self, Self::Error> {
        // Both pieces of state are read from the request, so there is exactly one
        // copy of each and no fairing owns a duplicate.
        let (Some(mode), Some(pool)) = (
            req.rocket().state::<AuthMode>(),
            req.rocket().state::<SqlitePool>(),
        ) else {
            // Not wired up: a programming error, not a client error.
            return Outcome::Error((Status::InternalServerError, Unauthenticated));
        };

        // The fairing already resolved this caller to decide about the URI
        // rewrite. Rocket 0.5.1 has no request-local state to hand the result
        // down, so the session is resolved a second time: two primary-key reads
        // of a local SQLite row per request. Do NOT replace the second read with
        // a request-header marker — request headers are client-controlled, so a
        // marker would be an authentication bypass.
        match resolve_caller(req, mode, pool).await {
            // An `Authorization` header is never consulted, only ignored: a
            // bearer token is not an accepted credential in this change, and
            // accepting one here would silently widen the authentication surface.
            Ok(Some(user)) => Outcome::Success(user),
            Ok(None) => Outcome::Error((Status::Unauthorized, Unauthenticated)),
            Err(e) => {
                eprintln!("ERROR: /api request could not be authenticated: {e}");
                Outcome::Error((Status::InternalServerError, Unauthenticated))
            }
        }
    }
}
