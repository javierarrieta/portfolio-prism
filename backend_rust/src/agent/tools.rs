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
use chrono::Utc;
use rmcp::handler::server::ServerHandler;
use rmcp::{tool, tool_router, ErrorData as McpError};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::access::{require_scope, Access};
use crate::agent::protocol::Proposal;
use crate::agent::staging;
use crate::api_routes::analytics;
use crate::api_routes::lookup;
use crate::auth::guard::AuthedUser;
use crate::auth::pat::Scope;
use crate::engines::stats_engine::StatsEngine;
use crate::engines::tax_engine::TaxLotEngine;
use crate::services::currency_service::CurrencyService;
use std::sync::Arc;

/// Page size for every listing tool, per *Limits*: 100 default, 500 max.
///
/// The ceiling is not politeness. A read response that exceeds a client's truncation limit
/// is a response the model silently sees half of, and half a transaction list looks exactly
/// like a complete one.
/// One row of `list_transactions`, in the order the SELECT lists its columns.
type TxRow = (i64, i64, String, i64, String, String, String, String, String);

const DEFAULT_PAGE_SIZE: i64 = 100;
const MAX_PAGE_SIZE: i64 = 500;

/// The agent's view of the application, bound to one caller for one request.
///
/// Built per request by `mount`, after the bearer token has been resolved. There is no
/// shared mutable state here and that is deliberate: the mount runs stateless, so nothing
/// survives a request and nothing is pinned to a process.
#[derive(Clone)]
pub struct AgentServer {
    pool: PgPool,
    user: AuthedUser,
    /// Shared with every request, because the pricing cache and the rate limiter live in
    /// it. A second instance would double the request rate against a rate-limited provider.
    pricing: Arc<CurrencyService>,
}

/// A domain refusal, rendered the way every refusal in this module is rendered.
fn refused(code: &str, detail: impl ToString) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(
        json!({ "error": code, "detail": detail.to_string() }).to_string(),
    )])
}

/// A `ReportError` already carries a status, a machine code and a retryable flag. The
/// status is a Rocket HTTP concept and there is no HTTP status in a tool result, so the
/// body is passed through and the status is dropped rather than invented.
fn refused_report(e: &analytics::ReportError) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(
        serde_json::to_value(&e.body)
            .unwrap_or_else(|_| json!({"error": "report_failed", "detail": e.status.to_string()}))
            .to_string(),
    )])
}

/// A date the caller can be told about. Rejected on shape, not on parse: chrono's
/// `%Y-%m-%d` accepts `2024-3-1`, and a date that loose would be silently reflowed by
/// Postgres rather than refused here.
fn exact_date(value: &str) -> Option<chrono::NaiveDate> {
    let bytes = value.as_bytes();
    let digits = |i: usize| bytes[i].is_ascii_digit();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-'
        || !(0..10).filter(|&i| i != 4 && i != 7).all(digits)
    {
        return None;
    }
    chrono::NaiveDate::from_ymd_opt(
        value[0..4].parse().ok()?,
        value[5..7].parse().ok()?,
        value[8..10].parse().ok()?,
    )
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
    pub fn new(pool: PgPool, user: AuthedUser, pricing: Arc<CurrencyService>) -> Self {
        Self { pool, user, pricing }
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

    #[tool(
        description = "Transactions, newest first, filterable by portfolio, asset, type and date range. Paginated: check total against the page you got, because a truncated list is indistinguishable from a complete one.",
        annotations(read_only_hint = true)
    )]
    async fn list_transactions(
        &self,
        Parameters(args): Parameters<ListTransactionsArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.gate(Scope::Read) {
            return Ok(refusal);
        }
        let limit = args.limit.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE);
        let offset = args.offset.unwrap_or(0).max(0);

        let from = match args.from.as_deref().map(exact_date) {
            Some(None) => return Ok(refused("bad_date", "from must be YYYY-MM-DD")),
            Some(d) => d,
            None => None,
        };
        let to = match args.to.as_deref().map(exact_date) {
            Some(None) => return Ok(refused("bad_date", "to must be YYYY-MM-DD")),
            Some(d) => d,
            None => None,
        };

        // One statement, every filter a `($n IS NULL OR ...)` arm: the owner predicate is
        // unconditional, so no combination of omitted filters can widen the result set.
        // `quantity`, `price` and `fee` are TEXT columns (`0001_init.sql:64-66`), so the
        // `::text` casts are no-ops today. They stay because the rule is about the wire, not
        // the column: if a migration ever narrows those to a numeric type, the tool still
        // emits a string rather than a JSON number that rounds a tax lot.
        let rows: Vec<TxRow> = sqlx::query_as(
            "SELECT t.id, t.asset_id, a.symbol, a.portfolio_id,
                    ((t.date AT TIME ZONE 'UTC')::date)::text, t.type,
                    t.quantity::text, t.price::text, t.fee::text
               FROM transactions t
               JOIN assets a ON a.id = t.asset_id
               JOIN portfolios p ON p.id = a.portfolio_id
              WHERE p.owner_id = $1
                AND ($2::bigint IS NULL OR a.portfolio_id = $2)
                AND ($3::bigint IS NULL OR t.asset_id = $3)
                AND ($4::text IS NULL OR upper(t.type) = upper($4))
                AND ($5::date IS NULL OR (t.date AT TIME ZONE 'UTC')::date >= $5)
                AND ($6::date IS NULL OR (t.date AT TIME ZONE 'UTC')::date <= $6)
              ORDER BY t.date DESC, t.id DESC
              LIMIT $7 OFFSET $8",
        )
        .bind(self.user.id)
        .bind(args.portfolio_id)
        .bind(args.asset_id)
        .bind(args.r#type.as_deref())
        .bind(from)
        .bind(to)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let (total,): (i64,) = sqlx::query_as(
            "SELECT count(*)::bigint
               FROM transactions t
               JOIN assets a ON a.id = t.asset_id
               JOIN portfolios p ON p.id = a.portfolio_id
              WHERE p.owner_id = $1
                AND ($2::bigint IS NULL OR a.portfolio_id = $2)
                AND ($3::bigint IS NULL OR t.asset_id = $3)
                AND ($4::text IS NULL OR upper(t.type) = upper($4))
                AND ($5::date IS NULL OR (t.date AT TIME ZONE 'UTC')::date >= $5)
                AND ($6::date IS NULL OR (t.date AT TIME ZONE 'UTC')::date <= $6)",
        )
        .bind(self.user.id)
        .bind(args.portfolio_id)
        .bind(args.asset_id)
        .bind(args.r#type.as_deref())
        .bind(from)
        .bind(to)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(ok(json!({
            "total": total,
            "limit": limit,
            "offset": offset,
            "transactions": rows
                .into_iter()
                .map(|(id, asset_id, symbol, portfolio_id, date, tx_type, quantity, price, fee)| json!({
                    "id": id, "asset_id": asset_id, "symbol": symbol,
                    "portfolio_id": portfolio_id, "date": date, "type": tx_type,
                    "quantity": quantity, "price": price, "fee": fee,
                }))
                .collect::<Vec<_>>(),
        })))
    }

    #[tool(
        description = "Realized gain for a portfolio: the tax summary, or a dated report when from and to are both given. Strategy is FIFO, LIFO or HYBRID.",
        annotations(read_only_hint = true)
    )]
    async fn get_tax_summary(
        &self,
        Parameters(args): Parameters<TaxSummaryArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.gate(Scope::Read) {
            return Ok(refusal);
        }
        // Same helpers in the same order as `GET /api/portfolios/<id>/tax-summary`,
        // including the strict-on-dated / permissive-on-undated split. A tool that applied
        // its own defaults would answer a question differently than the UI does.
        // Gate first, then load. Nothing here would leak — no data is emitted before the
        // gate returns — but a stranger's id should cost one query and a refusal, not two
        // queries and a refusal, and gate-first is the ordering every other route in this
        // crate uses.
        if require_portfolio(&self.pool, self.user.id, args.portfolio_id, Access::Viewer)
            .await
            .is_err()
        {
            return Ok(refused("portfolio_not_found", "no such portfolio for this user"));
        }
        let portfolio = match analytics::load_portfolio(&self.pool, args.portfolio_id).await {
            Ok(p) => p,
            Err(e) => return Ok(refused_report(&e)),
        };
        let assets = match analytics::load_assets(&self.pool, args.portfolio_id).await {
            Ok(a) => a,
            Err(e) => return Ok(refused_report(&e)),
        };

        let window = match analytics::parse_window(
            args.from.as_deref(),
            args.to.as_deref(),
            Utc::now().date_naive(),
        ) {
            Ok(w) => w,
            Err(e) => return Ok(refused_report(&e)),
        };

        let strategy = match &window {
            Some(_) => match analytics::strict_strategy(args.strategy.as_deref()) {
                Ok(s) => s,
                Err(e) => return Ok(refused_report(&e)),
            },
            None => analytics::normalize_strategy(args.strategy.as_deref()),
        };
        let threshold = args.threshold_days.unwrap_or(30);

        let value = match window {
            Some(window) => {
                match analytics::build_tax_report(
                    &self.pool,
                    &self.pricing,
                    &portfolio,
                    &assets,
                    &strategy,
                    threshold,
                    &window,
                    analytics::REPORT_DEADLINE,
                )
                .await
                {
                    Ok(report) => match serde_json::to_value(&report) {
                        Ok(v) => v,
                        Err(e) => return Ok(refused("serialize", e.to_string())),
                    },
                    Err(e) => return Ok(refused_report(&e)),
                }
            }
            None => {
                match analytics::undated_summary(
                    &self.pool,
                    &self.pricing,
                    &portfolio,
                    &assets,
                    &strategy,
                    threshold,
                )
                .await
                {
                    Ok(summary) => match serde_json::to_value(&summary) {
                        Ok(v) => v,
                        Err(e) => return Ok(refused("serialize", e.to_string())),
                    },
                    Err(e) => return Ok(refused_report(&e)),
                }
            }
        };

        Ok(ok(value))
    }

    #[tool(
        description = "Portfolio value series and cumulative TWR. WARNING: volatility, sharpe_ratio and beta in the metrics object are NOT measurements — the engine returns the constants \"0.0\", \"0.0\" and \"1.0\" on every path. Do not report them as findings. Realized gain comes from get_tax_summary; this tool does not carry realized or unrealized P&L at all.",
        annotations(read_only_hint = true)
    )]
    async fn get_performance(
        &self,
        Parameters(args): Parameters<PortfolioArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.gate(Scope::Read) {
            return Ok(refusal);
        }
        if require_portfolio(&self.pool, self.user.id, args.portfolio_id, Access::Viewer)
            .await
            .is_err()
        {
            return Ok(refused("portfolio_not_found", "no such portfolio for this user"));
        }

        let portfolio = match sqlx::query_as::<_, crate::models::Portfolio>(
            "SELECT id, name, description, currency, base_currency FROM portfolios WHERE id = $1",
        )
        .bind(args.portfolio_id)
        .fetch_optional(&self.pool)
        .await
        {
            Ok(Some(p)) => p,
            Ok(None) => return Ok(refused("portfolio_not_found", "no such portfolio")),
            Err(e) => return Ok(refused("database", e.to_string())),
        };
        let assets = match sqlx::query_as::<_, crate::models::Asset>(
            "SELECT * FROM assets WHERE portfolio_id = $1 ORDER BY id",
        )
        .bind(args.portfolio_id)
        .fetch_all(&self.pool)
        .await
        {
            Ok(a) => a,
            Err(e) => return Ok(refused("database", e.to_string())),
        };
        let transactions = match sqlx::query_as::<_, crate::models::Transaction>(
            "SELECT * FROM transactions WHERE asset_id IN (SELECT id FROM assets WHERE portfolio_id = $1) ORDER BY id",
        )
        .bind(args.portfolio_id)
        .fetch_all(&self.pool)
        .await
        {
            Ok(t) => t,
            Err(e) => return Ok(refused("database", e.to_string())),
        };

        let mut perf = match StatsEngine::calculate_portfolio_performance(
            &self.pool,
            &assets,
            &transactions,
            &portfolio.base_currency,
            &self.pricing,
        )
        .await
        {
            Ok(v) => v,
            Err(e) => return Ok(refused("pricing", e.to_string())),
        };

        // Stripped rather than described-away. The engine returns `realized_pnl` and
        // `unrealized_pnl` as constants, and a field that is present is a field a model
        // will quote; a description that says "ignore that number you can see" loses to the
        // number. Removing it is the only instruction that holds.
        if let Some(metrics) = perf.get_mut("metrics").and_then(|m| m.as_object_mut()) {
            metrics.remove("realized_pnl");
            metrics.remove("unrealized_pnl");
        }

        Ok(ok(perf))
    }

    #[tool(
        description = "The realized P&L breakdown for one SELL transaction: which buys it consumed, at what cost basis. Refused for a transaction that is not a SELL.",
        annotations(read_only_hint = true)
    )]
    async fn get_transaction_pnl(
        &self,
        Parameters(args): Parameters<PnlArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.gate(Scope::Read) {
            return Ok(refusal);
        }
        // The chokepoint again, at the transaction level: this resolves a caller-supplied
        // sell id, and the chain sell -> asset -> portfolio is data, not authorization.
        if crate::access::require_transaction(&self.pool, self.user.id, args.transaction_id, Access::Viewer)
            .await
            .is_err()
        {
            return Ok(refused("transaction_not_found", "no such transaction for this user"));
        }

        let tx = match sqlx::query_as::<_, crate::models::Transaction>(
            "SELECT * FROM transactions WHERE id = $1",
        )
        .bind(args.transaction_id)
        .fetch_optional(&self.pool)
        .await
        {
            Ok(Some(t)) => t,
            Ok(None) => return Ok(refused("transaction_not_found", "no such transaction")),
            Err(e) => return Ok(refused("database", e.to_string())),
        };
        if tx.r#type.to_uppercase() != "SELL" {
            return Ok(refused("not_a_sell", "P&L is defined for a SELL only"));
        }

        let asset = match sqlx::query_as::<_, crate::models::Asset>(
            "SELECT * FROM assets WHERE id = $1",
        )
        .bind(tx.asset_id)
        .fetch_optional(&self.pool)
        .await
        {
            Ok(Some(a)) => a,
            Ok(None) => return Ok(refused("asset_not_found", "the sell's asset is gone")),
            Err(e) => return Ok(refused("database", e.to_string())),
        };
        let portfolio = match sqlx::query_as::<_, crate::models::Portfolio>(
            "SELECT id, name, description, currency, base_currency FROM portfolios WHERE id = $1",
        )
        .bind(asset.portfolio_id)
        .fetch_optional(&self.pool)
        .await
        {
            Ok(Some(p)) => p,
            Ok(None) => return Ok(refused("portfolio_not_found", "no such portfolio")),
            Err(e) => return Ok(refused("database", e.to_string())),
        };
        let transactions = match sqlx::query_as::<_, crate::models::Transaction>(
            "SELECT * FROM transactions WHERE asset_id = $1 ORDER BY id",
        )
        .bind(asset.id)
        .fetch_all(&self.pool)
        .await
        {
            Ok(t) => t,
            Err(e) => return Ok(refused("database", e.to_string())),
        };

        // Normalized the same way the route normalizes, so a typo cannot select a different
        // method than the one the answer names.
        let strategy = match args.strategy.unwrap_or_default().to_uppercase().as_str() {
            "LIFO" => "LIFO",
            "HYBRID" => "HYBRID",
            _ => "FIFO",
        }
        .to_string();

        match TaxLotEngine::sell_breakdown(
            tx.id,
            &transactions,
            &asset.currency,
            &portfolio.base_currency,
            &self.pricing,
            &strategy,
            args.threshold_days.unwrap_or(30),
            &self.pool,
        )
        .await
        {
            Ok(Some(breakdown)) => Ok(ok(serde_json::to_value(breakdown).unwrap_or(Value::Null))),
            Ok(None) => Ok(refused("no_breakdown", "this sell matches no buys")),
            Err(e) => Ok(refused("pricing", e.to_string())),
        }
    }

    #[tool(
        description = "Resolve an ISIN to a symbol and asset type, for normalizing a broker statement before staging it. Same two-provider order as the UI: OpenFIGI, then Yahoo.",
        annotations(read_only_hint = true)
    )]
    async fn lookup_asset(
        &self,
        Parameters(args): Parameters<LookupArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(refusal) = self.gate(Scope::Read) {
            return Ok(refusal);
        }
        // Shape check shared with the route, so a malformed ISIN is refused here for the
        // same reason it is refused there and neither surface reaches the network with it.
        if !lookup::is_valid_isin(&args.isin) {
            return Ok(refused("bad_isin", "an ISIN is 12 characters: 2 letters, then 10 alphanumerics"));
        }
        match lookup::resolve_isin(&args.isin.to_uppercase()).await {
            Some(r) => Ok(ok(json!({
                "symbol": r.symbol, "name": r.name, "asset_type": r.asset_type,
            }))),
            None => Ok(refused("not_found", "no record for that ISIN at either provider")),
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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListTransactionsArgs {
    #[serde(default)]
    pub portfolio_id: Option<i64>,
    #[serde(default)]
    pub asset_id: Option<i64>,
    #[serde(default)]
    pub r#type: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TaxSummaryArgs {
    pub portfolio_id: i64,
    #[serde(default)]
    pub strategy: Option<String>,
    #[serde(default)]
    pub threshold_days: Option<i64>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PnlArgs {
    pub transaction_id: i64,
    #[serde(default)]
    pub strategy: Option<String>,
    #[serde(default)]
    pub threshold_days: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LookupArgs {
    pub isin: String,
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
