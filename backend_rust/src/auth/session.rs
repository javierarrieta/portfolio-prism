//! Session and OAuth-state cookies.
//!
//! Both cookies are written through Rocket's **private** cookie jar, which holds
//! the [`rocket::config::SecretKey`] internally. That is a deliberate
//! consequence of `SecretKey` exposing no accessor for its bytes: a
//! hand-rolled `encode(&SecretKey, …)` cannot be written against Rocket 0.5, and
//! the jar gives the same authenticated-encryption guarantee for free.
//!
//! The pure [`value`]/[`parse`] pair exists so the payload format is unit-testable
//! without a Rocket instance.

use rocket::http::Cookie;
use rocket::http::SameSite;
use rocket::time::Duration;
use rocket::http::CookieJar;
use serde::{Deserialize, Serialize};

/// Name of the session cookie.
pub const SESSION_COOKIE: &str = "pp_session";
/// Name of the short-lived cookie holding the OAuth `state`/`nonce`/PKCE verifier.
pub const STATE_COOKIE: &str = "pp_oauth_state";

/// Session lifetime. A user deactivated in the IdP keeps access until their
/// cookie expires, and this constant is that bound — see [`SessionPayload::exp`].
pub const SESSION_MAX_AGE: Duration = Duration::hours(8);
/// The login round-trip is a single redirect; ten minutes is generous.
pub const STATE_MAX_AGE: Duration = Duration::minutes(10);

/// What the session cookie holds.
///
/// `exp` is checked server-side. A Rocket private cookie is otherwise valid
/// forever — `Max-Age` is only advertised to the browser and a client can ignore
/// it — so without `exp` a stolen cookie would be usable indefinitely and nothing
/// in this change writes `is_active`, leaving no revocation path short of
/// rotating `ROCKET_SECRET_KEY`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPayload {
    pub uid: i64,
    pub tv: i64,
    pub exp: i64,
}

/// What the state cookie holds across the redirect to the IdP and back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatePayload {
    pub state: String,
    pub nonce: String,
    pub verifier: String,
}

/// Serialise a session payload. Pure; the encryption is the jar's job.
pub fn value(uid: i64, tv: i64, exp: i64) -> String {
    serde_json::to_string(&SessionPayload { uid, tv, exp }).expect("payload serialises")
}

/// Parse a session payload. `None` for anything that is not exactly one.
pub fn parse(raw: &str) -> Option<SessionPayload> {
    serde_json::from_str(raw).ok()
}

/// Unix seconds at which a session issued `now` expires.
pub fn expiry_now() -> i64 {
    chrono::Utc::now().timestamp() + SESSION_MAX_AGE.whole_seconds()
}

/// One place that builds the shared cookie attributes, so the two cookies cannot
/// drift.
fn build(name: &str, value: &str, max_age: Duration) -> Cookie<'static> {
    Cookie::build((name.to_string(), value.to_string()))
        // `Secure` is unconditional on purpose. TLS terminates at the ingress, so
        // nginx sees plain HTTP and keying this flag on the request scheme would
        // drop it in production, where the browser then refuses to send the
        // cookie back over the https origin it came from.
        .secure(true)
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(max_age)
        .build()
}

/// Issue a session cookie for `uid` at token version `tv`.
pub fn issue(jar: &CookieJar, uid: i64, tv: i64) {
    jar.add_private(build(SESSION_COOKIE, &value(uid, tv, expiry_now()), SESSION_MAX_AGE));
}

/// Read and decrypt the session cookie. Does **not** check `exp` — the caller
/// ([`crate::auth::guard::resolve_caller`]) compares it against the row it loads
/// anyway and can distinguish "expired" from "no cookie".
pub fn read(jar: &CookieJar) -> Option<SessionPayload> {
    jar.get_private(SESSION_COOKIE).and_then(|c| parse(c.value()))
}

/// Expire the session cookie in the browser and forget it here.
pub fn clear(jar: &CookieJar) {
    jar.remove_private(Cookie::build(SESSION_COOKIE));
}

/// Store the OAuth `state`/`nonce`/PKCE verifier for the duration of the redirect.
pub fn issue_state(jar: &CookieJar, payload: &StatePayload) {
    match serde_json::to_string(payload) {
        Ok(serialised) => jar.add_private(build(STATE_COOKIE, &serialised, STATE_MAX_AGE)),
        Err(e) => eprintln!("ERROR: could not serialise the OAuth state cookie: {e}"),
    }
}

/// Read the OAuth state cookie.
pub fn read_state(jar: &CookieJar) -> Option<StatePayload> {
    jar.get_private(STATE_COOKIE).and_then(|c| serde_json::from_str(c.value()).ok())
}

/// Forget the OAuth state cookie.
pub fn clear_state(jar: &CookieJar) {
    jar.remove_private(Cookie::build(STATE_COOKIE));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_and_parse_round_trip() {
        let raw = value(7, 3, 1_800_000_000);
        assert_eq!(
            parse(&raw),
            Some(SessionPayload { uid: 7, tv: 3, exp: 1_800_000_000 })
        );
    }

    #[test]
    fn parse_rejects_anything_that_is_not_the_payload_shape() {
        // A tampered or foreign cookie must fail closed, not default to a user.
        for raw in ["", "not json", "{}", r#"{"uid":1}"#, r#"{"uid":1,"tv":2}"#, "7", "null"] {
            assert_eq!(parse(raw), None, "raw={raw:?}");
        }
    }

    #[test]
    fn expiry_is_the_stated_lifetime_ahead_of_now() {
        let now = chrono::Utc::now().timestamp();
        let exp = expiry_now();
        assert!((exp - now - SESSION_MAX_AGE.whole_seconds()).abs() <= 2);
    }

    #[test]
    fn session_cookie_carries_the_hardening_attributes() {
        let c = build(SESSION_COOKIE, "x", SESSION_MAX_AGE);
        assert_eq!(c.http_only(), Some(true));
        assert_eq!(c.secure(), Some(true));
        assert_eq!(c.same_site(), Some(SameSite::Lax));
        assert_eq!(c.path(), Some("/"));
        assert_eq!(c.max_age(), Some(SESSION_MAX_AGE));
    }
}
