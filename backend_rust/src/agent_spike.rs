//! SPIKE — throwaway code, not the design.
//!
//! Question this answers: can rmcp's `StreamableHttpService` be mounted as an ordinary
//! Rocket route in stateless mode, so that the second listener in the MCP spec is not
//! needed? Concretely: bridge one POST (method, URI, headers, buffered body in; status,
//! headers, bytes out) with `json_response(true)` and `NeverSessionManager`.
//!
//! Deliberately minimal: no auth, no pool, no tools that touch the database.

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use rmcp::handler::server::ServerHandler;
use rmcp::model::{CallToolResult, ContentBlock, ServerCapabilities, ServerConfig};
use rmcp::transport::streamable_http_server::{
    session::never::NeverSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::{tool, tool_router, ErrorData as McpError};
use rocket::http::{Header, Status};
use rocket::request::{FromRequest, Outcome, Request};
use rocket::response::Responder;
use std::io::Cursor;
use std::sync::Arc;

/// Matches the 1 MiB body cap the spec proposes; rmcp enforces it itself.
const MAX_BODY_BYTES: usize = 1024 * 1024;

#[derive(Clone, Default)]
pub struct SpikeServer;

#[tool_router]
impl SpikeServer {
    #[tool(description = "pong")]
    async fn ping(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![ContentBlock::text("pong")]))
    }
}

#[rmcp::tool_handler]
impl ServerHandler for SpikeServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }
}

fn service() -> StreamableHttpService<SpikeServer, NeverSessionManager> {
    // The config type is #[non_exhaustive], so it is built by mutation, not literal.
    let mut config = StreamableHttpServerConfig::default();
    config.json_response = true;
    // The default is TRUE. Without this, a client negotiating a pre-2026-07-28
    // protocol version is routed to the session path and NeverSessionManager 500s.
    config.legacy_session_mode = false;
    config.max_request_body_bytes = MAX_BODY_BYTES;
    StreamableHttpService::new(
        || Ok(SpikeServer),
        Arc::new(NeverSessionManager::default()),
        config,
    )
}

/// The pieces of the Rocket request the bridge needs, captured before the body is read.
pub struct RawParts {
    method: http::Method,
    uri: String,
    headers: Vec<(String, String)>,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for RawParts {
    type Error = std::convert::Infallible;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let headers = request
            .headers()
            .iter()
            .map(|h| (h.name().to_string(), h.value().to_string()))
            .collect();
        Outcome::Success(RawParts {
            method: http::Method::from_bytes(request.method().as_str().as_bytes())
                .unwrap_or(http::Method::POST),
            uri: request.uri().to_string(),
            headers,
        })
    }
}

pub struct McpResponse {
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
            res.set_header(h); // NOTE: set_header replaces repeats; see spike notes
        }
        res.set_sized_body(self.body.len(), Cursor::new(self.body));
        Ok(res)
    }
}

/// The whole adapter: Rocket request in, Rocket response out.
#[rocket::post("/mcp", data = "<body>")]
pub async fn mcp(parts: RawParts, body: Vec<u8>) -> Result<McpResponse, Status> {
    let mut builder = http::Request::builder().method(parts.method).uri(parts.uri);
    for (name, value) in parts.headers {
        builder = builder.header(name, value);
    }
    let http_req = builder
        .body(Full::new(Bytes::from(body)))
        .map_err(|_| Status::InternalServerError)?;

    let response = service().handle(http_req).await;
    let (resp_parts, resp_body) = response.into_parts();
    let out = resp_body
        .collect()
        .await
        .map_err(|_| Status::InternalServerError)?
        .to_bytes();

    let mut headers = Vec::new();
    for (name, value) in resp_parts.headers.iter() {
        if let Ok(v) = value.to_str() {
            headers.push(Header::new(name.as_str().to_string(), v.to_string()));
        }
    }
    Ok(McpResponse {
        status: resp_parts.status.as_u16(),
        headers,
        body: out.to_vec(),
    })
}

/// Rocket mount for the real app, used only to check fairing interaction.
pub fn rocket_routes() -> Vec<rocket::Route> {
    rocket::routes![mcp]
}
