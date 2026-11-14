//! The request guard that hands a handler its caller.
//!
//! [`AuthedUser`] is what a handler asks for. It is *not* the enforcement
//! mechanism — [`crate::auth::fairing::AuthFairing`] is, because a guard only
//! protects the handlers that remember to ask for it. The guard's job is to give
//! a handler a typed caller and a single vocabulary for "who is this".

use std::io::Cursor;

use crate::db::DbPool;
use rocket::http::{ContentType, Status};
use rocket::request::{self, FromRequest, Outcome};
use rocket::response::Responder;
use rocket::{Request, Response};

use super::AuthMode;
use super::session;

/// The authenticated caller.
#[derive(Debug, Clone)]
pub struct AuthedUser {
    /// `users.id` is `BIGINT GENERATED ALWAYS AS IDENTITY`, so this is `i64` and not
    /// one bit narrower. Narrowing it is an authorization bug either way, and the two
    /// halves depend on the size of the id: one past `i32::MAX` but below `2^32`
    /// truncates to a negative number that matches no `users.id`, so the caller is
    /// spuriously *denied*; only at `2^32` and above can the low 32 bits land on a row
    /// that belongs to someone else, which is being authorized as *a different user, or
    /// as nobody at all*. Every read of `users.id` already decodes `i64`; this is where
    /// that width used to be thrown away.
    pub id: i64,
    pub email: Option<String>,
    pub name: Option<String>,
    pub sub: String,
    /// The row's current `token_version`, carried so a caller that mints a session
    /// (the OIDC callback) issues it at the version the guard is about to demand.
    /// Issuing a literal `0` instead locks a user out permanently after their first
    /// `logout-all`, because the guard rejects `tv != token_version` and nothing
    /// ever lowers the column again.
    pub token_version: i64,
}

impl AuthedUser {
    /// The `(name, value)` pair of a fresh session for this identity. The version
    /// comes from the row the identity was read from, and this is the **only** place
    /// that chooses it: two places picking a token version is how the `logout-all`
    /// lockout shipped.
    pub fn session_pair(&self) -> (&'static str, String) {
        super::session::pair(self.id, self.token_version)
    }

    /// Mint a session for this identity, attaching exactly what [`Self::session_pair`]
    /// derived.
    pub fn issue_session(&self, jar: &rocket::http::CookieJar<'_>) {
        let (name, value) = self.session_pair();
        super::session::attach(jar, name, &value);
    }
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
            .sized_body(
                UNAUTHENTICATED_BODY.len(),
                Cursor::new(UNAUTHENTICATED_BODY),
            )
            .ok()
    }
}

/// The columns `resolve_caller` reads, in select order.
///
/// Position 5 is `bool`, not `i64`: `users.is_active` is `BOOLEAN` in Postgres and
/// was `INTEGER` in SQLite, and `sqlx-postgres` will not decode `BOOL` into an
/// integer. Position 1 and position 6 are `i64` because both columns are `BIGINT`
/// — `token_version` deliberately so, see `migrations/0001_init.sql`.
pub(crate) type UserRow = (i64, Option<String>, Option<String>, String, bool, i64);

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
    pool: &DbPool,
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
        AuthMode::Disabled { as_user } => (*as_user, None),
    };

    // (id, email, name, subj, is_active, token_version)
    let row: Option<UserRow> = sqlx::query_as(
        "SELECT id, email, name, subj, is_active, token_version FROM users WHERE id = $1",
    )
    .bind(uid)
    .fetch_optional(pool)
    .await?;

    let Some((id, email, name, sub, is_active, token_version)) = row else {
        return Ok(None);
    };

    // A deactivated account, or a session minted before a `logout-all`, is
    // denied the same way a missing cookie is.
    if !is_active {
        return Ok(None);
    }
    if let Some(tv) = expected_tv {
        // A session minted before a `logout-all` carries the old version.
        if tv != token_version {
            return Ok(None);
        }
    }

    Ok(Some(AuthedUser {
        id,
        email,
        name,
        sub,
        token_version,
    }))
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AuthedUser {
    type Error = Unauthenticated;

    async fn from_request(req: &'r Request<'_>) -> request::Outcome<Self, Self::Error> {
        // Both pieces of state are read from the request, so there is exactly one
        // copy of each and no fairing owns a duplicate.
        let (Some(mode), Some(pool)) = (
            req.rocket().state::<AuthMode>(),
            req.rocket().state::<DbPool>(),
        ) else {
            // Not wired up: a programming error, not a client error.
            return Outcome::Error((Status::InternalServerError, Unauthenticated));
        };

        // The fairing already resolved this caller to decide about the URI
        // rewrite. Rocket 0.5.1 has no request-local state to hand the result
        // down, so the session is resolved a second time: two primary-key reads
        // of the caller's own `users` row per request. Do NOT replace the second read with
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
