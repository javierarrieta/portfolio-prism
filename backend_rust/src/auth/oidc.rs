//! The OIDC authorization-code-with-PKCE flow, and the database half of login.
//!
//! The network half and the database half are separate on purpose: a signed ID
//! token cannot be produced in a test without a live provider, so every rule that
//! decides *who a login means* ([`provision_or_update`]) takes primitives and is
//! testable without one.
//!
//! The API shapes here were verified by compiling a probe against
//! `openidconnect` 4.1.0 / `oauth2` 5.0.0. Four things differ from the 3.x shape
//! most examples show: there is no `async_http_client` function (`reqwest::Client`
//! implements `AsyncHttpClient`); `Client::new` takes `(client_id, issuer, jwks)`
//! and endpoints are set with setters; JWKS must be fetched explicitly; and
//! `authorize_url` takes the flow, CSRF and nonce generators as three arguments.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow};
use openidconnect::core::{
    CoreClient, CoreIdToken, CoreIdTokenClaims, CoreIdTokenVerifier, CoreJsonWebKeySet,
    CoreResponseType, CoreTokenResponse,
};
use openidconnect::{
    AuthenticationFlow, AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce,
    PkceCodeChallenge, PkceCodeVerifier, ProviderMetadataWithLogout, RedirectUrl, Scope,
};
use sqlx::SqlitePool;

use super::OidcConfig;
use super::guard::AuthedUser;
use super::session::StatePayload;

/// Provider metadata type.
///
/// Not `CoreProviderMetadata`: the logout endpoint lives in *additional* metadata
/// in `openidconnect` 4.1, and `ProviderMetadataWithLogout` is the crate's own
/// type for reading it.
type ProviderMetadata = ProviderMetadataWithLogout;

/// Cached provider metadata, keyed on issuer.
///
/// Cached **only on success**, and keyed rather than a bare `OnceLock` so a test
/// or a future multi-tenant config cannot inherit another issuer's endpoints. A
/// failed discovery is never cached: one Authentik outage during a restart must
/// not disable logins until the next restart.
static PROVIDER_CACHE: Mutex<Option<(String, ProviderMetadata)>> = Mutex::new(None);

/// Cached JWKS, keyed on issuer and **bounded by age**.
///
/// Unbounded, this is a scheduled outage: when Authentik rotates its signing key,
/// every login fails ID-token validation against the stale set until someone
/// restarts the pod, and nothing alarms. The TTL makes rotation self-healing within
/// one window instead of an incident. Key rotation is slow and rare, so a window of
/// this size costs one extra fetch per process per window.
static JWKS_CACHE: Mutex<Option<(String, Instant, CoreJsonWebKeySet)>> = Mutex::new(None);

/// How long a cached provider metadata document or key set stays usable.
const CACHE_TTL: Duration = Duration::from_secs(900);

fn http_client() -> reqwest::Client {
    // `reqwest::Client` implements `oauth2::AsyncHttpClient`, which is how the
    // openidconnect crate performs its requests in 4.x.
    reqwest::Client::new()
}

/// Discover (and cache) the provider metadata. Nothing at startup calls this —
/// that is what keeps a mistyped `OIDC_ISSUER` from crash-looping the pod, and
/// what lets the test suite run with no provider reachable.
pub async fn provider(cfg: &OidcConfig) -> anyhow::Result<ProviderMetadata> {
    if let Some((issuer, md)) = PROVIDER_CACHE.lock().expect("provider cache lock").as_ref()
        && *issuer == cfg.issuer
    {
        return Ok(md.clone());
    }
    let issuer = IssuerUrl::new(cfg.issuer.clone())
        .with_context(|| format!("OIDC_ISSUER={:?} is not a valid issuer URL", cfg.issuer))?;
    let md = ProviderMetadata::discover_async(issuer.clone(), &http_client())
        .await
        .context("OIDC provider discovery failed")?;
    *PROVIDER_CACHE.lock().expect("provider cache lock") = Some((cfg.issuer.clone(), md.clone()));
    Ok(md)
}

async fn jwks(cfg: &OidcConfig, md: &ProviderMetadata) -> anyhow::Result<CoreJsonWebKeySet> {
    if let Some((issuer, at, set)) = JWKS_CACHE.lock().expect("jwks cache lock").as_ref()
        && *issuer == cfg.issuer
        && at.elapsed() < CACHE_TTL
    {
        return Ok(set.clone());
    }
    let set = CoreJsonWebKeySet::fetch_async(md.jwks_uri(), &http_client())
        .await
        .context("could not fetch the OIDC provider JWKS")?;
    *JWKS_CACHE.lock().expect("jwks cache lock") =
        Some((cfg.issuer.clone(), Instant::now(), set.clone()));
    Ok(set)
}

/// Build the client for this configuration.
///
/// A macro rather than a function because `Client`'s type parameters change as
/// each endpoint is set (`EndpointNotSet` → `EndpointSet`), so the built type
/// cannot be named by a return signature without spelling out sixteen parameters.
macro_rules! core_client {
    ($cfg:expr, $md:expr) => {{
        let issuer = IssuerUrl::new($cfg.issuer.clone())?;
        let token_uri = $md
            .token_endpoint()
            .cloned()
            .ok_or_else(|| anyhow!("provider metadata has no token_endpoint"))?;
        CoreClient::new(
            ClientId::new($cfg.client_id.clone()),
            issuer,
            jwks($cfg, $md).await?,
        )
        .set_client_secret(ClientSecret::new($cfg.client_secret.clone()))
        .set_auth_uri($md.authorization_endpoint().clone())
        .set_token_uri(token_uri)
        .set_redirect_uri(RedirectUrl::new($cfg.redirect_uri.clone())?)
    }};
}

/// Start a login: build the authorization URL and the state to remember across
/// the redirect.
pub async fn login_url(cfg: &OidcConfig) -> anyhow::Result<(String, StatePayload)> {
    let md = provider(cfg).await?;
    let client = core_client!(cfg, &md);

    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let (auth_url, csrf_token, nonce) = client
        .authorize_url(
            AuthenticationFlow::<CoreResponseType>::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        // `openid` is requested explicitly even though the client adds it by
        // default, so the scope the provider sees is visible in this file.
        .add_scope(Scope::new("openid profile email".to_string()))
        .set_pkce_challenge(pkce_challenge)
        .url();

    Ok((
        auth_url.to_string(),
        StatePayload {
            state: csrf_token.secret().clone(),
            nonce: nonce.secret().clone(),
            verifier: pkce_verifier.secret().clone(),
        },
    ))
}

/// Exchange the authorization code for tokens.
///
/// Returns the raw token response; the ID token is validated separately by
/// [`validate_id_token`] so the trust check can be reasoned about on its own.
pub async fn exchange_code(
    cfg: &OidcConfig,
    code: &str,
    state: &StatePayload,
) -> anyhow::Result<CoreTokenResponse> {
    let md = provider(cfg).await?;
    let client = core_client!(cfg, &md);
    // `exchange_code` returns the request directly in 4.1 (not a `Result`).
    let response = client
        .exchange_code(AuthorizationCode::new(code.to_string()))
        .set_pkce_verifier(PkceCodeVerifier::new(state.verifier.clone()))
        .request_async(&http_client())
        .await
        .context("token exchange failed")?;
    Ok(response)
}

/// Validate an ID token: signature against the provider JWKS, plus `iss`, `aud`,
/// `exp` and the `nonce` we issued.
///
/// The nonce check is what stops a token minted for a different login from being
/// replayed into this one.
pub async fn validate_id_token(
    cfg: &OidcConfig,
    id_token: &CoreIdToken,
    nonce: &str,
) -> anyhow::Result<CoreIdTokenClaims> {
    let md = provider(cfg).await?;
    let issuer = IssuerUrl::new(cfg.issuer.clone())?;
    let verifier = CoreIdTokenVerifier::new_confidential_client(
        ClientId::new(cfg.client_id.clone()),
        ClientSecret::new(cfg.client_secret.clone()),
        issuer,
        jwks(cfg, &md).await?,
    );
    id_token
        .claims(&verifier, &Nonce::new(nonce.to_string()))
        .context("ID token validation failed")
        .cloned()
}

/// The three claims this application cares about, as plain values.
///
/// `email` is stored and **never compared**: with self-registration enabled, an
/// `email` claim is a value any registrant can choose, so making it decide
/// ownership would let one user claim another's portfolios. Ownership is keyed on
/// `(issuer, sub)` alone.
pub fn identity_from(claims: &CoreIdTokenClaims) -> (String, Option<String>, Option<String>) {
    let sub = claims.subject().to_string();
    let email = claims.email().map(|e| e.to_string());
    let name = claims
        .name()
        .and_then(|localized| localized.get(None))
        .map(|n| n.to_string());
    (sub, email, name)
}

/// Turn an identity into the application's user row, creating or updating it.
///
/// Takes primitives rather than a claims type so the handover rules below are
/// testable without a signed token from a live provider.
///
/// Order matters:
/// 1. A row already matching `(issuer, sub)` wins, and refreshes `email`/`name`.
/// 2. Otherwise, if `sub` is the designated bootstrap subject **and** the
///    placeholder row still exists, that row is updated in place — preserving its
///    `id`, which is what keeps pre-existing portfolios attached to it.
/// 3. Otherwise a new row is provisioned.
pub async fn provision_or_update(
    pool: &SqlitePool,
    issuer: &str,
    sub: &str,
    email: Option<&str>,
    name: Option<&str>,
    bootstrap_subj: Option<&str>,
) -> anyhow::Result<AuthedUser> {
    let now = chrono::Utc::now().to_rfc3339();

    // 1. Known identity.
    let existing: Option<crate::auth::guard::UserRow> = sqlx::query_as(
        "SELECT id, email, name, subj, is_active, token_version FROM users \
         WHERE issuer = ? AND subj = ?",
    )
    .bind(issuer)
    .bind(sub)
    .fetch_optional(pool)
    .await?;

    if let Some((id, _, _, _, _, token_version)) = existing {
        sqlx::query("UPDATE users SET email = ?, name = ?, last_login_at = ? WHERE id = ?")
            .bind(email)
            .bind(name)
            .bind(&now)
            .bind(id)
            .execute(pool)
            .await?;
        return Ok(AuthedUser {
            id: id as i32,
            email: email.map(str::to_string),
            name: name.map(str::to_string),
            sub: sub.to_string(),
            token_version,
        });
    }

    // 2. Bootstrap handover. Fires only on an exact match with the configured
    // subject *and* the placeholder's existence — never on row existence alone,
    // which is what makes the handover impossible to repeat.
    // A blank configured subject counts as unset, matching the backfill rule.
    let bootstrap = bootstrap_subj.map(str::trim).filter(|s| !s.is_empty());
    if bootstrap == Some(sub) {
        let placeholder: Option<(i64,)> =
            sqlx::query_as("SELECT id FROM users WHERE issuer = 'local' AND subj = 'bootstrap'")
                .fetch_optional(pool)
                .await?;

        if let Some((placeholder_id,)) = placeholder {
            let handed: (i64,) = sqlx::query_as(
                "UPDATE users SET issuer = ?, subj = ?, email = ?, name = ?, last_login_at = ? \
                 WHERE id = ? RETURNING token_version",
            )
            .bind(issuer)
            .bind(sub)
            .bind(email)
            .bind(name)
            .bind(&now)
            .bind(placeholder_id)
            .fetch_one(pool)
            .await?;
            eprintln!(
                "INFO: bootstrap handover — placeholder user {placeholder_id} is now \
                 issuer={issuer} sub={sub}; portfolios it owned came with it"
            );
            return Ok(AuthedUser {
                id: placeholder_id as i32,
                email: email.map(str::to_string),
                name: name.map(str::to_string),
                sub: sub.to_string(),
                token_version: handed.0,
            });
        }
    }

    // 3. New identity. ON CONFLICT DO NOTHING plus a re-select makes two concurrent
    // first logins for the same subject safe: one of them loses the insert and
    // still gets the row, instead of failing a whole login on the unique index.
    sqlx::query(
        "INSERT INTO users (issuer, subj, email, name, created_at, last_login_at) \
         VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT (issuer, subj) DO NOTHING",
    )
    .bind(issuer)
    .bind(sub)
    .bind(email)
    .bind(name)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    let row: (i64, Option<String>, Option<String>, i64) = sqlx::query_as(
        "SELECT id, email, name, token_version FROM users WHERE issuer = ? AND subj = ?",
    )
    .bind(issuer)
    .bind(sub)
    .fetch_one(pool)
    .await?;

    let (id, email, name, token_version) = row;
    eprintln!("INFO: provisioned user id={id} issuer={issuer} sub={sub}");
    Ok(AuthedUser {
        id: id as i32,
        email,
        name,
        sub: sub.to_string(),
        token_version,
    })
}

/// Bump the caller's `token_version`, invalidating every session issued to them.
pub async fn bump_token_version(pool: &SqlitePool, user_id: i32) -> anyhow::Result<()> {
    sqlx::query("UPDATE users SET token_version = token_version + 1 WHERE id = ?")
        .bind(user_id as i64)
        .execute(pool)
        .await?;
    Ok(())
}

/// The provider's front-channel logout URL, or `None` when the provider does not
/// advertise one (in which case logging out is local-only).
pub async fn logout_url(cfg: &OidcConfig) -> anyhow::Result<Option<String>> {
    let md = provider(cfg).await?;
    let Some(endpoint) = md.additional_metadata().end_session_endpoint.clone() else {
        return Ok(None);
    };
    let mut url = endpoint.url().clone();
    url.query_pairs_mut()
        .append_pair("post_logout_redirect_uri", &cfg.post_logout_redirect_uri);
    Ok(Some(url.to_string()))
}
