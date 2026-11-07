//! The `/auth/*` routes, and the `/_auth/denied` sentinel the fairing rewrites to.

use std::io::Cursor;

use rocket::http::{ContentType, Status};
use rocket::request::{self, FromRequest, Outcome};
use rocket::response::{Redirect, Responder};
use rocket::serde::json::Json;
use openidconnect::TokenResponse;
use rocket::{Request, Response, State};
use rocket::http::CookieJar;
use serde_json::json;
use sqlx::SqlitePool;

use super::guard::{AuthedUser, Unauthenticated};
use super::{oidc, session, AuthMode};

/// Body for "the OIDC exchange failed", which is a different fact from
/// "you have no session" and must not be conflated with it.
pub(crate) const AUTH_FAILED_BODY: &str = r#"{"error":"authentication_failed"}"#;

/// The fixed message the frontend shows for a failed sign-in. Nothing from the
/// provider — no error string, no description, no URI — is ever reflected back.
pub(crate) const AUTH_FAILED_REDIRECT: &str = "/?auth_error=auth_failed";

// --------------------------------------------------------------------------------------
// Denial sentinel (one handler per method, so no method can answer 405 where the
// requirement is 401)
// --------------------------------------------------------------------------------------

/// Registered as a catcher so that **every** `401` answers with the same body,
/// whichever path produced it.
///
/// Required, not cosmetic: Rocket prefers a registered catcher over a request
/// guard's error responder, and with no catcher it falls back to its own HTML error
/// page. Without this, a guard rejection on `/auth/me` rendered HTML and the shared
/// JSON body was never seen by a client.
#[catch(401)]
pub fn unauthenticated_catcher() -> Unauthenticated {
    Unauthenticated
}

pub fn catchers() -> Vec<rocket::Catcher> {
    rocket::catchers![unauthenticated_catcher]
}

#[get("/denied")]
pub fn denied_get() -> Unauthenticated {
    Unauthenticated
}
#[post("/denied")]
pub fn denied_post() -> Unauthenticated {
    Unauthenticated
}
#[put("/denied")]
pub fn denied_put() -> Unauthenticated {
    Unauthenticated
}
#[delete("/denied")]
pub fn denied_delete() -> Unauthenticated {
    Unauthenticated
}
#[patch("/denied")]
pub fn denied_patch() -> Unauthenticated {
    Unauthenticated
}
#[head("/denied")]
pub fn denied_head() -> Unauthenticated {
    Unauthenticated
}
#[options("/denied")]
pub fn denied_options() -> Unauthenticated {
    Unauthenticated
}

// --------------------------------------------------------------------------------------
// /auth/*
// --------------------------------------------------------------------------------------

/// What a login attempt can end as. One type so the browser and API-client
/// branches cannot drift into two different handlers.
///
/// `large_enum_variant` is silenced rather than boxing: `Redirect` is a few strings,
/// boxing it would allocate on every redirect to save a fixed amount of stack in a
/// type that exists once per request, and the enum is not on a hot path.
#[allow(clippy::large_enum_variant)]
pub enum LoginOutcome {
    /// Session issued; go to the app.
    Success(Redirect),
    /// A browser navigated into a broken login; show a fixed message.
    BrowserFailure,
    /// A non-browser client hit a broken login.
    ApiFailure,
}

impl Responder<'_, 'static> for LoginOutcome {
    fn respond_to(self, req: &Request<'_>) -> Result<Response<'static>, Status> {
        match self {
            LoginOutcome::Success(r) => r.respond_to(req),
            LoginOutcome::BrowserFailure => Redirect::to(AUTH_FAILED_REDIRECT).respond_to(req),
            LoginOutcome::ApiFailure => Response::build()
                .status(Status::Unauthorized)
                .header(ContentType::JSON)
                .sized_body(AUTH_FAILED_BODY.len(), Cursor::new(AUTH_FAILED_BODY))
                .ok(),
        }
    }
}

/// Did the client ask for HTML? Rocket 0.5 ships no `Accept` guard, so this reads
/// the header once, as a guard, rather than in every handler that branches on it.
pub struct WantsHtml(pub bool);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for WantsHtml {
    type Error = std::convert::Infallible;

    async fn from_request(req: &'r Request<'_>) -> request::Outcome<Self, Self::Error> {
        Outcome::Success(WantsHtml(
            req.headers().get("Accept").any(|value| value.contains("text/html")),
        ))
    }
}

fn mode_config(mode: &AuthMode) -> Option<&super::OidcConfig> {
    match mode {
        AuthMode::Oidc(cfg) => Some(cfg),
        #[cfg(debug_assertions)]
        AuthMode::Disabled { .. } => None,
    }
}

/// Start the login: stash `state`/`nonce`/PKCE verifier in a short-lived private
/// cookie and redirect to the provider.
#[get("/login")]
pub async fn login(mode: &State<AuthMode>, cookies: &CookieJar<'_>) -> LoginOutcome {
    let Some(cfg) = mode_config(mode) else {
        // `AUTH_MODE=disabled` has no provider to redirect to; the guard already
        // serves every request as the placeholder user.
        return LoginOutcome::Success(Redirect::to("/"));
    };
    match oidc::login_url(cfg).await {
        Ok((url, state)) => {
            session::issue_state(cookies, &state);
            LoginOutcome::Success(Redirect::to(url))
        }
        Err(e) => {
            eprintln!("ERROR: could not start OIDC login: {e}");
            LoginOutcome::BrowserFailure
        }
    }
}

/// The provider's redirect target.
#[get("/callback?<code>&<state>&<error>")]
pub async fn callback(
    wants_html: WantsHtml,
    mode: &State<AuthMode>,
    pool: &State<SqlitePool>,
    cookies: &CookieJar<'_>,
    code: Option<&str>,
    state: Option<&str>,
    error: Option<&str>,
) -> LoginOutcome {
    // A person navigating gets a page; a script gets a status code it can branch on.
    let browser = wants_html.0;
    let fail = |reason: &str| -> LoginOutcome {
        // The provider's own `error`/`error_description` are logged, never sent
        // back: reflecting them turns the login page into a channel for the
        // provider's text.
        eprintln!("WARN: OIDC login failed: {reason}");
        if browser {
            LoginOutcome::BrowserFailure
        } else {
            LoginOutcome::ApiFailure
        }
    };

    let Some(cfg) = mode_config(mode) else {
        return fail("login attempted while authentication is disabled");
    };
    if let Some(provider_error) = error {
        return fail(&format!("provider returned error={provider_error}"));
    }
    let (Some(code), Some(state)) = (code, state) else {
        return fail("callback carried no code/state");
    };
    // The state cookie is consumed whatever happens next, so a replay of the same
    // callback cannot be replayed.
    let Some(expected) = session::read_state(cookies) else {
        return fail("no state cookie to compare against");
    };
    session::clear_state(cookies);
    if expected.state != state {
        return fail("state does not match the state cookie");
    }

    let outcome = async {
        let tokens = oidc::exchange_code(cfg, code, &expected).await?;
        let id_token = tokens
            .id_token()
            .ok_or_else(|| anyhow::anyhow!("token response carried no id_token"))?
            .clone();
        let claims = oidc::validate_id_token(cfg, &id_token, &expected.nonce).await?;
        let (sub, email, name) = oidc::identity_from(&claims);
        anyhow::Ok(oidc::provision_or_update(
            pool.inner(),
            &cfg.issuer,
            &sub,
            email.as_deref(),
            name.as_deref(),
            cfg.bootstrap_subj.as_deref(),
        )
        .await?)
    }
    .await;

    match outcome {
        Ok(user) => {
            session::issue(cookies, user.id as i64, 0);
            LoginOutcome::Success(Redirect::to("/"))
        }
        Err(e) => fail(&format!("identity could not be established: {e}")),
    }
}

/// Local logout, plus the provider's front-channel logout when it advertises one.
#[get("/logout")]
pub async fn logout(mode: &State<AuthMode>, cookies: &CookieJar<'_>) -> Redirect {
    session::clear(cookies);
    let url = match mode_config(mode) {
        Some(cfg) => oidc::logout_url(cfg).await.unwrap_or(None),
        None => None,
    };
    // `None` means the provider has no `end_session_endpoint`: logout is local-only.
    Redirect::to(url.unwrap_or_else(|| "/".to_string()))
}

/// Invalidate every session this user has, then this cookie.
#[post("/logout-all")]
pub async fn logout_all(
    user: AuthedUser,
    pool: &State<SqlitePool>,
    cookies: &CookieJar<'_>,
) -> Result<Redirect, Status> {
    oidc::bump_token_version(pool.inner(), user.id)
        .await
        .map_err(|e| {
            eprintln!("ERROR: could not bump token_version for user {}: {e}", user.id);
            Status::InternalServerError
        })?;
    session::clear(cookies);
    Ok(Redirect::to("/"))
}

/// Who am I? Mounted in **both** modes, so the frontend can always ask.
///
/// Under `AUTH_MODE=disabled` this reports the placeholder `sub` — the literal
/// string `bootstrap`, which is a marker meaning "no real identity", not a
/// subject any provider issues.
#[get("/me")]
pub fn me(user: AuthedUser) -> Json<serde_json::Value> {
    Json(json!({
        "id": user.id,
        "email": user.email,
        "name": user.name,
        "sub": user.sub,
    }))
}
