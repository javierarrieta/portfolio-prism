//! The tools an agent may call, and the scope each one needs.
//!
//! A tool here is a thin shell: it checks a scope, calls the same function an `/api` route
//! calls, and renders the answer as JSON. Anything smarter than that belongs in `staging`,
//! `access` or the engines, because a rule that lives in a tool is a rule an `/api` caller
//! does not have to follow.
//!
//! Two shapes worth naming:
//!
//! **A refused tool call is a tool result, not a protocol error.** `not_approved`,
//! `expired`, `scope_missing` — these are answers, and an agent reads them. A protocol
//! error means the call was malformed and the model may retry it identically forever. So
//! domain refusals come back as `CallToolResult::error` carrying a JSON body with a `code`
//! field, and only genuinely broken arguments become `McpError`.
//!
//! **The scope is checked here, at the only door.** `require_scope` is the same function the
//! `/api` token routes use, so a token that lacks `import` gets the same answer from both
//! surfaces. `Credential::Session` passes every scope — which is why the mount refuses
//! sessions before a tool is ever reached, and why that refusal is a required test rather
//! than an assumption.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, ServerCapabilities, ServerConfig};
use rmcp::handler::server::ServerHandler;
use rmcp::{tool, tool_router, ErrorData as McpError};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::access::{require_scope, Access};
use crate::agent::protocol::Proposal;
use crate::agent::staging;
use crate::auth::guard::AuthedUser;
use crate::auth::pat::Scope;

/// The agent's view of the application, bound to one caller for one request.
///
/// Built per request by `mount`, after the bearer token has been resolved. There is no
/// shared mutable state here and that is deliberate: the mount runs stateless, so nothing
/// survives a request and nothing is pinned to a process.
#[derive(Clone)]
pub struct AgentServer {
    pool: PgPool,
    user: AuthedUser,
}

/// A domain refusal, rendered the way every refusal in this module is rendered.
fn refused(code: &str, detail: impl ToString) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(
        json!({ "error": code, "detail": detail.to_string() }).to_string(),
    )])
}

fn ok(value: Value) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(value.to_string())])
}

/// Turn a `StageError`/`ApplyError` into the same shape, so an agent learns one convention.
fn refused_from(code: &str, detail: &str, field: Option<&str>) -> CallToolResult {
    let mut body = json!({ "error": code, "detail": detail });
    if let Some(f) = field {
        body["field"] = json!(f);
    }
    CallToolResult::error(vec![ContentBlock::text(body.to_string())])
}

#[tool_router]
impl AgentServer {
    pub fn new(pool: PgPool, user: AuthedUser) -> Self {
        Self { pool, user }
    }

    /// The scope gate, expressed once. A tool that forgets to call it is a tool that
    /// ignores scopes, which is the failure mode this whole module exists to avoid.
    /// `Some(refusal)` when the token does not hold the scope, `None` when it does.
    ///
    /// An `Option` rather than a `Result` because the refusal is a *result* the agent reads,
    /// not an error the protocol surfaces: `Err(McpError)` invites a client to retry the
    /// identical call forever, and a missing scope will never fix itself.
    fn gate(&self, scope: Scope) -> Option<CallToolResult> {
        require_scope(&self.user, scope)
            .err()
            .map(|_| refused("scope_missing", format!(
                "this token does not hold the '{}' scope",
                scope.as_str()
            )))
    }

    // --- read ----------------------------------------------------------------------

    #[tool(
        description = "List the portfolios this credential can see, with asset and transaction counts.",
        annotations(read_only_hint = true)
    )]
    async fn list_portfolios(&self) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.gate(Scope::Read) {

            return Ok(refusal);

        }
        let rows: Vec<(i64, String, String, String, i64, i64)> = sqlx::query_as(
            "SELECT p.id, p.name, p.currency, p.base_currency,
                    (SELECT count(*) FROM assets a WHERE a.portfolio_id = p.id)::bigint,
                    (SELECT count(*) FROM transactions t
                       JOIN assets a ON a.id = t.asset_id
                      WHERE a.portfolio_id = p.id)::bigint
               FROM portfolios p WHERE p.owner_id = $1 ORDER BY p.id",
        )
        .bind(self.user.id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(ok(json!(rows
            .into_iter()
            .map(|(id, name, currency, base_currency, assets, transactions)| json!({
                "id": id,
                "name": name,
                "currency": currency,
                "base_currency": base_currency,
                "asset_count": assets,
                "transaction_count": transactions,
            }))
            .collect::<Vec<_>>())))
    }

    #[tool(
        description = "One portfolio with its assets: symbol, ISIN, currency, type. Resolves a caller-supplied portfolio id, so it is refused for a portfolio that is not yours.",
        annotations(read_only_hint = true)
    )]
    async fn get_portfolio(
        &self,
        Parameters(args): Parameters<PortfolioArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.gate(Scope::Read) {

            return Ok(refusal);

        }
        // The chokepoint, not a local check: a portfolio id is a caller-supplied id and this
        // module is not allowed to resolve one itself.
        if require_portfolio(&self.pool, self.user.id, args.portfolio_id, Access::Owner)
            .await
            .is_err()
        {
            return Ok(refused(
                "portfolio_not_found",
                "no such portfolio for this user",
            ));
        }

        let assets: Vec<(i64, String, Option<String>, String, String)> = sqlx::query_as(
            "SELECT id, symbol, isin, asset_type, currency FROM assets
              WHERE portfolio_id = $1 ORDER BY symbol",
        )
        .bind(args.portfolio_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(ok(json!({
            "portfolio_id": args.portfolio_id,
            "assets": assets
                .into_iter()
                .map(|(id, symbol, isin, asset_type, currency)| json!({
                    "id": id,
                    "symbol": symbol,
                    "isin": isin,
                    "asset_type": asset_type,
                    "currency": currency,
                }))
                .collect::<Vec<_>>(),
        })))
    }

    // --- import --------------------------------------------------------------------

    #[tool(
        description = "Stage a batch of rows for a human to approve. Writes nothing. Money is a decimal string, never a JSON number; dates are YYYY-MM-DD. A single row is a batch of one.",
        annotations(read_only_hint = false)
    )]
    async fn stage_import(
        &self,
        Parameters(args): Parameters<StageImportArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.gate(Scope::Import) {

            return Ok(refusal);

        }

        // The proposal arrives as a nested object and is deserialized here rather than by
        // the tool macro, so that a bad value can be reported with the path to it.
        let proposal: Proposal = match serde_json::from_value(args.payload) {
            Ok(p) => p,
            Err(e) => {
                return Ok(refused("bad_payload", format!(
                    "the proposal could not be read: {e}"
                )))
            }
        };

        match staging::stage(&self.pool, self.user.id, &proposal, staging::DEFAULT_TTL_MINUTES)
            .await
        {
            Ok(s) => Ok(ok(json!({
                "handle": s.handle,
                "status": "staged",
                "expires_at": s.expires_at.to_rfc3339(),
                "preview": s.preview,
                "next": "a human must approve this handle at status 'staged' before apply_staged will write anything",
            }))),
            Err(e) => Ok(refused_from(e.code(), &format!("{e:?}"), e.field())),
        }
    }

    #[tool(
        description = "Write a staged batch that a human has already approved. Returns not_approved until a session approves it, and already_applied if it has been written once.",
        annotations(read_only_hint = false)
    )]
    async fn apply_staged(
        &self,
        Parameters(args): Parameters<HandleArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.gate(Scope::Import) {

            return Ok(refusal);

        }
        match staging::apply(&self.pool, self.user.id, &args.handle).await {
            Ok(a) => Ok(ok(json!({
                "status": "applied",
                "portfolio_id": a.portfolio_id,
                "asset_ids": a.asset_ids,
                "transaction_count": a.transaction_count,
                "skipped_duplicates": a.skipped_duplicates,
            }))),
            Err(e) => Ok(refused_from(e.code(), &e.message(), None)),
        }
    }

    #[tool(
        description = "Throw a staged batch away without writing it. No approval needed to discard.",
        annotations(read_only_hint = false)
    )]
    async fn discard_staged(
        &self,
        Parameters(args): Parameters<HandleArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.gate(Scope::Import) {

            return Ok(refusal);

        }
        let done = staging::discard(&self.pool, self.user.id, &args.handle)
            .await
            .map_err(|e| McpError::internal_error(e.code().to_string(), None))?;
        if done {
            Ok(ok(json!({ "status": "discarded" })))
        } else {
            Ok(refused("stale_handle", "no staged batch with that handle"))
        }
    }

    #[tool(
        description = "Re-render the stored preview for a handle, so you can report what a human actually approved rather than what you staged.",
        annotations(read_only_hint = true)
    )]
    async fn get_staged(
        &self,
        Parameters(args): Parameters<HandleArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.gate(Scope::Import) {

            return Ok(refusal);

        }
        match staging::preview_of(&self.pool, self.user.id, &args.handle).await {
            Ok(Some((status, preview, expires_at))) => Ok(ok(json!({
                "status": status,
                "expires_at": expires_at.to_rfc3339(),
                "expired": expires_at <= chrono::Utc::now(),
                "preview": preview,
            }))),
            Ok(None) => Ok(refused("stale_handle", "no staged batch with that handle")),
            Err(e) => Ok(refused(e.code(), "could not read the batch")),
        }
    }
}

#[rmcp::tool_handler]
impl ServerHandler for AgentServer {
    fn get_info(&self) -> ServerConfig {
        // The capability list is one line today and will not stay that way; naming tools
        // only says what this server does, which is all it should claim.
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }
}

/// Free function rather than a method, so the tool bodies read as one call to the
/// chokepoint instead of one to a wrapper around it.
async fn require_portfolio(
    pool: &PgPool,
    user_id: i64,
    portfolio_id: i64,
    min: Access,
) -> Result<Access, rocket::http::Status> {
    crate::access::require(pool, user_id, portfolio_id, min).await
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PortfolioArgs {
    pub portfolio_id: i64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct HandleArgs {
    pub handle: String,
}

/// The proposal is passed through as a `Value` and deserialized by hand.
///
/// Deriving a schema for `Proposal` would advertise a shape to clients, which is worth
/// doing — but the deserializer cannot name a path when a value is wrong, and the path is
/// the half an agent can act on. So the schema stays loose here and `staging::validate`
/// produces `assets[0].transactions[1].quantity`, which is what the agent actually needs.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StageImportArgs {
    pub payload: Value,
}
