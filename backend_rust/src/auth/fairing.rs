//! Request-lifecycle enforcement.
//!
//! This is the fairing that makes "every `/api/` request requires an
//! authenticated identity" true even for a handler that forgot to ask who the
//! caller is.
//!
//! Rocket 0.5.1 `Fairing::on_request` returns `()`, so a fairing **cannot**
//! produce a response. What it can do is rewrite the request URI before routing
//! (`on_request` runs in `preprocess_request`, before `route()`), which sends the
//! request to a denial route instead of the real handler. The real handler never
//! runs, so nothing with a side effect happens.

use rocket::fairing::{Fairing, Info, Kind};
use rocket::http::Method;
use rocket::http::uri::Origin;
use rocket::{Data, Request};

use super::AuthMode;
use super::guard::resolve_caller;

/// The sentinel every denied `/api/` request is routed to. Outside `/api`, so it
/// is neither covered by the prefix rule nor part of the public API surface.
pub const DENIED_PATH: &str = "/_auth/denied";

/// The sentinel a request is routed to when the **lookup itself** failed.
///
/// Separate from [`DENIED_PATH`] on purpose. Collapsing a database failure into the
/// `401` made the frontend's `apiFetch` bounce a signed-in user to the identity
/// provider for the duration of an outage, and once there the provider had nothing
/// to fix, so the browser came back and bounced again. A `503` is both the truth
/// and loop-proof: the frontend does not redirect on it.
pub const UNAVAILABLE_PATH: &str = "/_auth/unavailable";

pub struct AuthFairing;

/// A request is protected when its path starts with `/api/` — with the trailing
/// slash, which is why `/api-docs` is not covered and why Task 18 asserts no
/// mounted route resolves to exactly `/api`.
fn is_protected(req: &Request<'_>) -> bool {
    req.uri().path().starts_with("/api/")
}

/// A real CORS preflight: `OPTIONS` **and** `Origin` **and**
/// `Access-Control-Request-Method`. A bare `OPTIONS /api/…` is not a preflight
/// and is denied like any other unauthenticated request.
///
/// The exemption has to exist at all because `rocket_cors`' preflight handling
/// would otherwise answer `OPTIONS` on `/api` paths before authentication.
fn is_cors_preflight(req: &Request<'_>) -> bool {
    req.method() == Method::Options
        && req.headers().get("Origin").next().is_some()
        && req
            .headers()
            .get("Access-Control-Request-Method")
            .next()
            .is_some()
}

#[rocket::async_trait]
impl Fairing for AuthFairing {
    fn info(&self) -> Info {
        Info {
            name: "auth",
            kind: Kind::Request,
        }
    }

    async fn on_request(&self, req: &mut Request<'_>, _data: &mut Data<'_>) {
        if !is_protected(req) || is_cors_preflight(req) {
            return;
        }

        let (Some(mode), Some(pool)) = (
            req.rocket().state::<AuthMode>(),
            req.rocket().state::<sqlx::SqlitePool>(),
        ) else {
            // Not wired up: fail closed, and loudly.
            eprintln!("ERROR: auth fairing ran without AuthMode/SqlitePool managed state");
            deny(req);
            return;
        };

        match resolve_caller(req, mode, pool).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                // Logged before the rewrite, because after it the original path is
                // gone — and "who probed what" is the point of the log line.
                eprintln!(
                    "WARN: unauthenticated {} {} denied",
                    req.method(),
                    req.uri().path()
                );
                deny(req);
            }
            Err(e) => {
                // A database failure is not a denial, but the handler must not run
                // either: it would execute with no caller. Route it somewhere that
                // says "try again", not "sign in".
                eprintln!(
                    "ERROR: authentication lookup failed for {} {}: {e}",
                    req.method(),
                    req.uri().path()
                );
                unavailable(req);
            }
        }
    }
}

fn unavailable(req: &mut Request<'_>) {
    let path = Origin::parse(UNAVAILABLE_PATH).expect("UNAVAILABLE_PATH is a valid origin");
    req.set_uri(path);
}

fn deny(req: &mut Request<'_>) {
    // Safe: the path is a constant with no characters the URI parser can reject.
    let denied = Origin::parse(DENIED_PATH).expect("DENIED_PATH is a valid origin");
    req.set_uri(denied);
}
