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
//! a second listener would have been, but excluded by one route string in one fairing.
//!
//! What that actually costs, stated precisely, because the loose version of this sentence
//! teaches the wrong lesson: moving `/mcp` under `/api` would **not** hand every tool to a
//! session cookie. This handler requires a bearer unconditionally — `parts.bearer
//! .ok_or(Status::Unauthorized)?` is its first statement, and `pat::lookup` on that string is
//! the only credential it resolves — so an `/api/mcp` mount would answer 401 and every MCP
//! test
//! would answer 404, which is how the mistake announced itself when it happened here. The
//! boundary is the route's own bearer check, not the prefix; the prefix decides who else
//! gets to *see* the route, and `AuthFairing` running on top of a route that already
//! authenticates itself is a second, different authentication on the same request.
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
//! cannot quietly become an MCP credential. Note where the second test runs: in
//! `AuthMode::Oidc`, because `resolve_caller` reads a cookie in that mode only. A cookie
//! test written against `AuthMode::Disabled` cannot fail for the reason it claims to prove.
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
//!
//! ## The Host guard runs here too, and it runs after the token check
//!
//! `StreamableHttpService::handle` validates the `Host` header before it looks at the body,
//! against [`StreamableHttpServerConfig::allowed_hosts`] — whose default is loopback only,
//! because the library assumes a server on the machine that is calling it. This deployment
//! is not that: nginx forwards the public hostname (`proxy_set_header Host $host`), so the
//! untouched default answers **403 `Forbidden: Host header is not allowed`** to every call,
//! and it does so *after* `pat::lookup` has already accepted the bearer. The operator sees a
//! valid token refused, and the response names the Host rather than the missing setting.
//!
//! So the list is set from `AgentConfig::mcp_allowed_hosts` in [`service`] and nowhere else.
//! It is configuration and not a constant for the same reason `AGENT_MCP_MOUNT` is: a lever
//! that needs an image rebuild is a lever nobody pulls.

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use rmcp::transport::streamable_http_server::{
    session::never::NeverSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rocket::data::{Data, ToByteUnit};
use rocket::http::{Header, Status};
use rocket::request::{FromRequest, Outcome, Request};
use rocket::response::Responder;
use sqlx::PgPool;
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

use crate::agent::tools::AgentServer;
use crate::auth::pat;
use crate::services::currency_service::CurrencyService;

/// The body cap for `/mcp`, enforced **by this route** and not by a `Limits` entry.
///
/// There is no global `Limits` in this application (`lib.rs` says so out loud: the bundle
/// upload is capped per route rather than by raising a global limit for one endpoint), and
/// Rocket's default for the `bytes` limit — the one a `Vec<u8>` data guard reads — is
/// **8 KiB** (`rocket::data::Limits::BYTES`). A `Vec<u8>` argument here would therefore
/// have refused every `stage_import` over ~90 rows with a `400` from Rocket, before this
/// handler ran, while `config.max_request_body_bytes` below sat at 1 MiB never being
/// reached. So the handler takes `Data` and opens it under this constant itself, the same
/// shape as `api_routes::backup::preview`.
///
/// The number has to be at least what `staging::MAX_TRANSACTIONS_PER_BATCH` can produce:
/// 500 rows of `{"type":…, "date":…, "quantity":…}` is roughly 50 KB, and a batch that the
/// validator accepts must survive the transport that carries it.
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// How many `/mcp` requests may be in flight in this process at once.
///
/// There was no bound at all, and the surface shares a process, a runtime and a database
/// pool with the interactive app. `db.rs` caps the pool at **5** connections; every `/api`
/// request takes one to resolve its own caller (`guard.rs` documents that it makes two
/// reads), and `staging::apply` holds one for the whole of a multi-hundred-statement
/// transaction. Five unbounded agent calls could therefore take the entire pool and turn
/// the next human click into a 503 from `/_auth/unavailable` — an agent, holding a scoped
/// token, denying the human who issued it.
///
/// Three leaves two connections for `/api`, which is enough for interactive traffic
/// because those reads are short and sequential. Exhaustion answers 503 rather than
/// queueing: an agent that is told to back off knows to back off, and an agent whose
/// request sits in an invisible queue is one whose user is watching a spinner.
const MAX_CONCURRENT_MCP: usize = 3;

/// The wall-clock bound on one `/mcp` request, tool execution included.
///
/// Nothing bounded a request before: `statement_timeout = '30s'` in `db.rs` is **per
/// statement**, so a transaction of six hundred of them is unbounded by construction, and
/// a client that stalls mid-body held its task open indefinitely. This is the outer bound.
///
/// It is deliberately larger than the per-statement timeout, not smaller: `apply_staged`
/// legitimately runs hundreds of statements, so a 30 s request deadline would kill valid
/// work and turn a slow apply into a mystery. A request that needs more than this is not
/// slow, it is stuck.
const MAX_REQUEST_SECONDS: u64 = 120;

/// The bound behind [`MAX_CONCURRENT_MCP`]. `const_new` so there is no lazy-init branch and
/// nothing to poison: a semaphore cannot be poisoned.
static MCP_SLOTS: Semaphore = Semaphore::const_new(MAX_CONCURRENT_MCP);

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
            // literal, and it is now load-bearing in a second way: rmcp matches this string
            // against `allowed_hosts`, and `localhost` is on that list only because the
            // default is loopback. A test that sets its own allowlist and sends no Host
            // header is therefore testing the fallback, not the host it means to test —
            // `tests/mcp_host_test.rs` sends the header explicitly for that reason.
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
    ttl_minutes: i64,
    allowed_hosts: Vec<String>,
) -> StreamableHttpService<AgentServer, NeverSessionManager> {
    let mut config = StreamableHttpServerConfig::default();
    config.json_response = true;
    // The default is TRUE. Left alone, a client negotiating a pre-2026-07-28 protocol
    // version is routed onto the session path and `NeverSessionManager` answers 500.
    config.legacy_session_mode = false;
    // Redundant by design. The route has already refused anything larger than this before
    // rmcp sees a byte (see `MAX_BODY_BYTES`), so this arm never fires — it stays set to the
    // same number so the two caps cannot drift if the route's is ever raised.
    config.max_request_body_bytes = MAX_BODY_BYTES;
    // The library default is loopback only, which is a development default and refuses the
    // real hostname on any deployment. See the module comment: this check runs after the
    // bearer check, so leaving it defaulted looks like a broken token, not like missing
    // configuration. The value is `AgentConfig::mcp_allowed_hosts`, whose own default is
    // this same library list — so an unset variable changes nothing here, and the empty
    // list that disables the guard can only arrive through the explicit `*`.
    config.allowed_hosts = allowed_hosts;
    StreamableHttpService::new(
        move || {
            Ok(AgentServer::new(
                pool.clone(),
                user.clone(),
                pricing.clone(),
                ttl_minutes,
            ))
        },
        Arc::new(NeverSessionManager::default()),
        config,
    )
}

/// The whole door: authenticate, then hand the request to rmcp unchanged.
#[rocket::post("/mcp", data = "<body>")]
pub(crate) async fn mcp(
    parts: RawParts,
    body: Data<'_>,
    pool: &rocket::State<PgPool>,
    pricing: &rocket::State<std::sync::Arc<CurrencyService>>,
    cfg: &rocket::State<crate::agent::AgentConfig>,
) -> Result<McpResponse, Status> {
    // Taken before authentication on purpose: the bound exists to protect the process, and
    // an unauthenticated flood is exactly the load that needs bounding. The permit is held
    // for the whole request, including the response body collection below.
    let _slot = MCP_SLOTS
        .try_acquire()
        .map_err(|_| Status::ServiceUnavailable)?;
    let secret = parts.bearer.ok_or(Status::Unauthorized)?;

    let user = pat::lookup(pool.inner(), &secret)
        .await
        .map_err(|_| Status::InternalServerError)?
        // A dead, revoked or expired token is indistinguishable from a wrong one, and the
        // response carries no hint about which. That is the point: the difference tells an
        // attacker that a token id was real.
        .ok_or(Status::Unauthorized)?;

    // Read the body under the route's own cap. `Data` rather than `Vec<u8>` because the
    // `Vec<u8>` guard reads the `bytes` limit (8 KiB by default, and no `Limits` is
    // configured here) and answers `400` on truncation — see `MAX_BODY_BYTES`. A body over
    // the cap is refused with `413` rather than handed to rmcp short, which would come back
    // as a JSON parse error and send an agent off to fix a payload that was fine.
    let uploaded = body
        .open(MAX_BODY_BYTES.bytes())
        .into_bytes()
        .await
        .map_err(|_| Status::BadRequest)?;
    if !uploaded.is_complete() {
        return Err(Status::PayloadTooLarge);
    }
    let body: Vec<u8> = uploaded.value;

    // The bearer is consumed and never reaches rmcp: the tool server holds the resolved
    // user, so a tool cannot accidentally forward a credential it has no business seeing.
    let mut builder = http::Request::builder()
        .method(parts.method)
        .uri(parts.uri)
        .header("host", parts.host);
    for (name, value) in parts.headers {
        // `authorization` is consumed at the door and `host` was already set from the
        // request URI. `content-length` and `transfer-encoding` are dropped because Rocket
        // has already decoded the body: what is handed to rmcp is the decoded bytes, so the
        // client's framing headers describe a body that is no longer the one being sent. A
        // `Transfer-Encoding: chunked` forwarded verbatim tells hyper to expect framing it
        // will never receive, and a stale `Content-Length` contradicts the payload.
        //
        // `cookie` goes for the same reason `authorization` does, and it is the more
        // dangerous of the two: a browser pointed at `/mcp` sends the `pp_session` cookie
        // unprompted, and `Credential::Session` passes every scope. The route does not
        // *authenticate* on it — `pat::lookup` on the bearer is the only credential this
        // route honours, and `mcp_refuses_a_session_cookie` pins that — but leaving a
        // full-authority session credential sitting in the request object the tool server
        // receives is how a future header log, or an error that echoes its request, leaks
        // one. Nothing in rmcp needs a cookie; nothing here should hand it one.
        if name.eq_ignore_ascii_case("authorization")
            || name.eq_ignore_ascii_case("cookie")
            || name.eq_ignore_ascii_case("host")
            || name.eq_ignore_ascii_case("content-length")
            || name.eq_ignore_ascii_case("transfer-encoding")
        {
            continue;
        }
        builder = builder.header(name, value);
    }
    let http_req = builder
        .body(Full::new(Bytes::from(body)))
        .map_err(|_| Status::InternalServerError)?;

    let response = tokio::time::timeout(
        Duration::from_secs(MAX_REQUEST_SECONDS),
        service(
            pool.inner().clone(),
            user,
            pricing.inner().clone(),
            cfg.batch_ttl_minutes,
            cfg.mcp_allowed_hosts.clone(),
        )
        .handle(http_req),
    )
    .await
    .map_err(|_| {
        // Logged rather than surfaced in detail: the client sees a 504, the operator sees
        // which bound fired. A tool that hits this is stuck, not slow.
        eprintln!("WARN: /mcp request exceeded {MAX_REQUEST_SECONDS}s and was abandoned");
        Status::GatewayTimeout
    })?;
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
