//! The `/mcp` route: an authenticated door onto [`AgentServer`]'s tools.
//!
//! Shape taken from the spike (`spike/mcp-in-rocket`, `f167dfb`), which answered the spec's
//! open question: rmcp's `StreamableHttpService` can be served by an ordinary Rocket route,
//! so the second listener is not needed. `rmcp::handle` takes any `http_body::Body` and
//! returns `Response<BoxBody<Bytes, Infallible>>`; nothing from axum, hyper or tower-http
//! enters the tree.
//!
//! ## This route authenticates itself, and that is a load-bearing sentence
//!
//! `AuthFairing` protects by the `/api/` prefix (`auth/fairing.rs:40`). `/mcp` is outside
//! that prefix, so it is **unauthenticated by default** — not structurally excluded the way
//! a second listener would have been, but excluded by one route string in one fairing. If
//! someone later "tidies" that prefix, or mounts `/mcp` under `/api`, the failure is silent
//! and total: every tool would run as whoever holds a session cookie.
//!
//! So two assertions live here and both are tested:
//!
//! 1. a request with no bearer token is refused, and
//! 2. a request carrying a **session cookie and no bearer token is refused**.
//!
//! The second is the one that matters. A session cookie is a human's whole authority —
//! `Credential::Session` passes every scope — so if `/mcp` ever accepted one, the entire
//! scope model would be decorative. The check is written as "a bearer token is required and
//! nothing else is examined", not as "reject cookies", so adding a new cookie path elsewhere
//! cannot quietly become an MCP credential.
//!
//! ## Not in the OpenAPI document
//!
//! `/mcp` is deliberately absent from `docs/openapi/openapi.json`, and the route-coverage
//! gate allows it because it filters on the `/api/` prefix. That is not an oversight:
//! OpenAPI describes a surface of distinct paths and methods, and MCP is one path whose
//! methods are chosen by a JSON field in the body. The tool list is published by the
//! protocol itself, through `tools/list`, which is where a client reads it from.
//!
//! ## Stateless, on purpose
//!
//! `NeverSessionManager` with `json_response = true`, so every request is independent, no
//! `Mcp-Session-Id` is minted, and nothing is pinned to a process. `legacy_session_mode`
//! defaults to **true**, and leaving it there routes any client negotiating a pre-
//! `2026-07-28` protocol version onto the session path, where `NeverSessionManager` answers
//! 500. The spike found that by being hit by it.

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use rmcp::transport::streamable_http_server::{
    session::never::NeverSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rocket::http::{Header, Status};
use rocket::request::{FromRequest, Outcome, Request};
use rocket::response::Responder;
use sqlx::PgPool;
use std::io::Cursor;
use std::sync::Arc;

use crate::agent::tools::AgentServer;
use crate::auth::pat;
use crate::services::currency_service::CurrencyService;

/// Matches the body cap in *Limits*. rmcp enforces it itself; naming it here keeps the two
/// documents from drifting apart silently.
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// The parts of the Rocket request the bridge needs, captured before the body is consumed.
pub(crate) struct RawParts {
    method: http::Method,
    uri: String,
    headers: Vec<(String, String)>,
    bearer: Option<String>,
    host: String,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for RawParts {
    type Error = std::convert::Infallible;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let headers: Vec<(String, String)> = request
            .headers()
            .iter()
            .map(|h| (h.name().to_string(), h.value().to_string()))
            .collect();

        // Exactly one accepted credential form, read here and nowhere else.
        //
        // `Authorization: Bearer pp_...` is the only thing that can authenticate this
        // route. A session cookie is deliberately not consulted: it carries every scope, so
        // honouring it would make the scope model a suggestion. See the module comment.
        let bearer = request
            .headers()
            .get_one("Authorization")
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        // rmcp builds its own `http::Request` and rejects one without a Host. Rocket's
        // local client does not send one, and a reverse proxy may strip it, so the host is
        // taken from the header when present and from the request URI otherwise. Synthesizing
        // it is not inventing a fact: Rocket has already decided which host it is answering.
        let host = headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case("host"))
            .map(|(_, v)| v.clone())
            // Rocket's `Request` carries the origin-form URI only, so there is no host to
            // read here; a local client sends no `Host` header at all. The fallback is a
            // literal, and that is fine: rmcp requires *a* Host to build its `http::Request`
            // and nothing in this application branches on its value. Anyone who later makes
            // a routing decision on it must revisit this line.
            .unwrap_or_else(|| "localhost".to_string());

        Outcome::Success(RawParts {
            method: http::Method::from_bytes(request.method().as_str().as_bytes())
                .unwrap_or(http::Method::POST),
            uri: request.uri().to_string(),
            headers,
            bearer,
            host,
        })
    }
}

/// rmcp's response, copied back into a Rocket one.
pub(crate) struct McpResponse {
    status: u16,
    headers: Vec<Header<'static>>,
    body: Vec<u8>,
}

#[rocket::async_trait]
impl<'r> Responder<'r, 'static> for McpResponse {
    fn respond_to(self, _: &'r Request<'_>) -> rocket::response::Result<'static> {
        let mut res = rocket::Response::new();
        res.set_status(Status::new(self.status));
        for h in self.headers {
            // `set_header` replaces a repeated name rather than appending. Harmless for
            // everything rmcp sends today (content type, and nothing else in stateless
            // mode), and recorded here because a future header that legitimately repeats
            // would be silently dropped by this line.
            res.set_header(h);
        }
        res.set_sized_body(self.body.len(), Cursor::new(self.body));
        Ok(res)
    }
}

fn service(
    pool: PgPool,
    user: crate::auth::guard::AuthedUser,
    pricing: Arc<CurrencyService>,
) -> StreamableHttpService<AgentServer, NeverSessionManager> {
    let mut config = StreamableHttpServerConfig::default();
    config.json_response = true;
    // The default is TRUE. Left alone, a client negotiating a pre-2026-07-28 protocol
    // version is routed onto the session path and `NeverSessionManager` answers 500.
    config.legacy_session_mode = false;
    config.max_request_body_bytes = MAX_BODY_BYTES;
    StreamableHttpService::new(
        move || Ok(AgentServer::new(pool.clone(), user.clone(), pricing.clone())),
        Arc::new(NeverSessionManager::default()),
        config,
    )
}

/// The whole door: authenticate, then hand the request to rmcp unchanged.
#[rocket::post("/mcp", data = "<body>")]
pub(crate) async fn mcp(
    parts: RawParts,
    body: Vec<u8>,
    pool: &rocket::State<PgPool>,
    pricing: &rocket::State<std::sync::Arc<CurrencyService>>,
) -> Result<McpResponse, Status> {
    let secret = parts.bearer.ok_or(Status::Unauthorized)?;

    let user = pat::lookup(pool.inner(), &secret)
        .await
        .map_err(|_| Status::InternalServerError)?
        // A dead, revoked or expired token is indistinguishable from a wrong one, and the
        // response carries no hint about which. That is the point: the difference tells an
        // attacker that a token id was real.
        .ok_or(Status::Unauthorized)?;

    // The bearer is consumed and never reaches rmcp: the tool server holds the resolved
    // user, so a tool cannot accidentally forward a credential it has no business seeing.
    let mut builder = http::Request::builder()
        .method(parts.method)
        .uri(parts.uri)
        .header("host", parts.host);
    for (name, value) in parts.headers {
        if name.eq_ignore_ascii_case("authorization") || name.eq_ignore_ascii_case("host") {
            continue;
        }
        builder = builder.header(name, value);
    }
    let http_req = builder
        .body(Full::new(Bytes::from(body)))
        .map_err(|_| Status::InternalServerError)?;

    let response = service(pool.inner().clone(), user, pricing.inner().clone())
        .handle(http_req)
        .await;
    let (parts_out, resp_body) = response.into_parts();
    let out = resp_body
        .collect()
        .await
        .map_err(|_| Status::InternalServerError)?
        .to_bytes();

    let mut headers = Vec::new();
    for (name, value) in parts_out.headers.iter() {
        if let Ok(v) = value.to_str() {
            headers.push(Header::new(name.as_str().to_string(), v.to_string()));
        }
    }
    Ok(McpResponse {
        status: parts_out.status.as_u16(),
        headers,
        body: out.to_vec(),
    })
}

#[allow(dead_code)]
pub(crate) fn routes() -> Vec<rocket::Route> {
    rocket::routes![mcp]
}
