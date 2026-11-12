use rocket::{State, serde::json::Json, http::{ContentType, Status}};
use rocket::request::Request;
use rocket::response::{Responder, Response};
use sqlx::SqlitePool;
use std::io::Cursor;
use std::time::{Duration, Instant};
use chrono::{NaiveDate, Utc};
use crate::models::{Portfolio, Asset, Transaction};
use crate::services::currency_service::{
    fx_symbol, select_rate, stored_window_is_fresh, CurrencyService, PricingError,
    FX_LOOKBACK_DAYS,
};
use crate::engines::tax_engine::TaxLotEngine;
use crate::report_csv;
use crate::schemas::{
    AssetTotals, ErrorBody, MissingRate, PriceSource, ReportContext, ReportResponse, ReportRow,
    ReportTotals, SellPnlResponse,
};
use crate::engines::stats_engine::StatsEngine;
use rust_decimal::Decimal;
use anyhow::Result;
use crate::access::{require, require_transaction, Access};
use crate::auth::guard::AuthedUser;

/// The tax summary, or — with `from` and/or `to` — the dated tax report.
///
/// Same route, same access level, two shapes. Without a window the response is the
/// summary object this endpoint has always returned, which carries `assets` with the
/// open lots the Holdings tab renders; it is not described here, and its shape is
/// deliberately unchanged (the printable statement and `PortfolioDetail` both read it).
/// With a window the response is `ReportResponse`.
///
/// No `params(...)` block, deliberately: the parameters come from the handler's own
/// arguments, which is why `strategy`, `threshold_days`, `from`, `to` and `format` are
/// all in the committed document without one. `?format=csv` (dated requests only) answers
/// the same bytes the JSON payload carries, as a server-generated file: the CSV is a
/// projection of `ReportResponse`, so no number in it is re-derived. It is described here
/// rather than as a second content type for status 200, which utoipa's response map —
/// keyed by status — cannot express.
#[utoipa::path(
    get,
    path = "/api/portfolios/<id>/tax-summary",
    responses(
        (status = 200, description = "Dated tax report when a window is given; the legacy summary object otherwise", body = ReportResponse),
        (status = 404, description = "Portfolio not found", body = ErrorBody),
        (status = 422, description = "Invalid window, or a price or rate does not exist at or before the window end", body = ErrorBody),
        (status = 503, description = "A pricing provider is unreachable, or the report exceeded its elapsed deadline", body = ErrorBody)
    )
)]
#[allow(clippy::too_many_arguments)]
#[get("/<id>/tax-summary?<strategy>&<threshold_days>&<from>&<to>&<format>")]
pub async fn get_portfolio_tax_summary(
    user: AuthedUser,
    id: i32,
    pool: &State<SqlitePool>,
    currency_service: &State<CurrencyService>,
    strategy: Option<String>,
    threshold_days: Option<i64>,
    from: Option<String>,
    to: Option<String>,
    format: Option<String>,
) -> Result<TaxSummaryResponse, ReportError> {
    // First statement in the body on purpose. Note that Rule B of `check_access_gate.sh`
    // no longer sees this handler's portfolio lookup at all -- it moved into
    // `load_portfolio`, past the awk block's column-0 `}` terminator -- so the ordering
    // rule is vacuous here and this call is what actually gates the route. The real check
    // is `tests/isolation_test.rs`, which exercises the dated variant end to end.
    require(pool.inner(), user.id, id, Access::Viewer)
        .await
        .map_err(ReportError::from_status)?;

    let pool = pool.inner();
    let today = Utc::now().date_naive();
    let window = parse_window(from.as_deref(), to.as_deref(), today)?;
    let csv = parse_format(format.as_deref(), window.is_some())?;

    // Strict on the dated path, permissive on the undated one (decision D6).
    let strategy_val = match &window {
        Some(_) => strict_strategy(strategy.as_deref())?,
        None => normalize_strategy(strategy.as_deref()),
    };
    let threshold_val = threshold_days.unwrap_or(30);
    if window.is_some() && threshold_val < 0 {
        return Err(ReportError::validation("threshold_days", "must not be negative"));
    }

    let portfolio = load_portfolio(pool, id).await?;
    let assets = load_assets(pool, id).await?;

    match window {
        Some(window) => {
            let report = build_tax_report(
                pool, currency_service.inner(), &portfolio, &assets,
                &strategy_val, threshold_val, &window, REPORT_DEADLINE,
            )
            .await?;
            if csv {
                // `inception` when `requested_from` is None — the portfolio has no
                // transaction at or before the window end. The filename names the window
                // the file actually covers, not the parameters as typed.
                let filename = format!(
                    "tax-report-{}-{}-{}.csv",
                    report.context.portfolio_id,
                    report.context.requested_from.map(|d| d.to_string()).unwrap_or_else(|| "inception".to_string()),
                    report.context.requested_to,
                );
                Ok(TaxSummaryResponse::Csv { body: report_csv::render(&report), filename })
            } else {
                Ok(TaxSummaryResponse::Report(Json(report)))
            }
        }
        None => {
            let summary = undated_summary(pool, currency_service.inner(), &portfolio, &assets, &strategy_val, threshold_val).await?;
            Ok(TaxSummaryResponse::Legacy(Json(summary)))
        }
    }
}

async fn load_portfolio(pool: &SqlitePool, id: i32) -> Result<Portfolio, ReportError> {
    sqlx::query_as::<_, Portfolio>("SELECT * FROM portfolios WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| ReportError::internal(e.to_string()))?
        .ok_or_else(ReportError::not_found)
}

async fn load_assets(pool: &SqlitePool, portfolio_id: i32) -> Result<Vec<Asset>, ReportError> {
    sqlx::query_as::<_, Asset>("SELECT * FROM assets WHERE portfolio_id = ?")
        .bind(portfolio_id)
        .fetch_all(pool)
        .await
        .map_err(|e| ReportError::internal(e.to_string()))
}

/// The undated response, moved verbatim out of the handler: same numbers, same shape,
/// same keys, including the empty-portfolio early return and the splits sync this
/// endpoint has always performed before reading. The dated path deliberately does not
/// sync splits (issue #50). `strategy` is the normalized value the handler chose, so
/// this response can never name a matching method the engine did not use.
async fn undated_summary(
    pool: &SqlitePool,
    currency_service: &CurrencyService,
    portfolio: &Portfolio,
    assets: &[Asset],
    strategy: &str,
    threshold_days: i64,
) -> Result<serde_json::Value, ReportError> {
    if assets.is_empty() {
        return Ok(serde_json::json!({
            "assets": [],
            "total_portfolio_value": "0",
            "total_realized_pnl": "0",
            "total_unrealized_pnl": "0"
        }));
    }

    let mut asset_summaries = Vec::new();
    let mut total_value = Decimal::ZERO;
    let mut total_realized = Decimal::ZERO;
    let mut total_unrealized = Decimal::ZERO;

    for asset in assets {
        // Auto-detect and persist any unrecorded stock splits for this asset before
        // computing tax lots, so they feed into P&L and value. A failure must not stop
        // the report, but it must not be invisible either: a missed split is a wrong
        // cost basis, and every failure path in the sync returns Ok or an Err that
        // used to be discarded here.
        if let Err(e) = currency_service.sync_splits_for_asset(asset, pool).await {
            eprintln!(
                "WARN: split sync failed for asset {} ({}): {}",
                asset.id, asset.symbol, e
            );
        }

        let transactions = sqlx::query_as::<_, Transaction>("SELECT * FROM transactions WHERE asset_id = ?")
            .bind(asset.id)
            .fetch_all(pool)
            .await
            .map_err(|e| ReportError::internal(e.to_string()))?;

        // A pricing failure is still exactly what it was on this path: a zero, and a 200
        // response. This endpoint is undated, so the type change must not move a number
        // here; a dated report is where an `Err` becomes an error instead.
        let current_price = match currency_service
            .get_price(&asset.symbol, asset.isin.as_deref(), pool)
            .await
        {
            Ok(price) => price,
            Err(e) => {
                eprintln!("WARN: no price for {}: {}", asset.symbol, e);
                Decimal::ZERO
            }
        };

        let summary = TaxLotEngine::calculate_lots(
            &asset.symbol,
            &asset.asset_type,
            &transactions,
            current_price,
            &asset.currency,
            &portfolio.base_currency,
            currency_service,
            strategy,
            threshold_days,
            pool,
        )
        .await
        .map_err(|e| ReportError::internal(e.to_string()))?;

        total_value += summary.market_value;
        total_realized += summary.realized_pnl;
        total_unrealized += summary.unrealized_pnl;
        asset_summaries.push(summary);
    }

    Ok(serde_json::json!({
        "assets": asset_summaries,
        "total_portfolio_value": total_value.to_string(),
        "total_realized_pnl": total_realized.to_string(),
        "total_unrealized_pnl": total_unrealized.to_string(),
        "strategy": strategy,
        "threshold_days": threshold_days,
        "currency": portfolio.base_currency.clone(),
    }))
}

#[utoipa::path(
    get,
    path = "/api/portfolios/<id>/performance",
    responses(
        (status = 200, description = "Performance data retrieved", body = serde_json::Value),
        (status = 404, description = "Portfolio not found")
    )
)]
#[get("/<id>/performance")]
pub async fn get_portfolio_performance(
    user: AuthedUser,
    id: i32,
    pool: &State<SqlitePool>,
    currency_service: &State<CurrencyService>,
) -> Result<Json<serde_json::Value>, Status> {
    // Before the `WHERE asset_id IN (SELECT id FROM assets WHERE portfolio_id = ?)`
    // subquery: that shape is invisible to the gate script's grep, so this call is
    // the only thing keeping it honest.
    require(pool.inner(), user.id, id, Access::Viewer).await?;

    let portfolio = sqlx::query_as::<_, Portfolio>("SELECT * FROM portfolios WHERE id = ?")
        .bind(id)
        .fetch_optional(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?
        .ok_or(Status::NotFound)?;

    let assets = sqlx::query_as::<_, Asset>("SELECT * FROM assets WHERE portfolio_id = ?")
        .bind(id)
        .fetch_all(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?;

    // Auto-detect and persist any unrecorded stock splits so historical
    // portfolio value is scaled correctly. Logged rather than discarded for the same
    // reason as the tax-summary path above.
    for asset in &assets {
        if let Err(e) = currency_service.sync_splits_for_asset(asset, pool.inner()).await {
            eprintln!(
                "WARN: split sync failed for asset {} ({}): {}",
                asset.id, asset.symbol, e
            );
        }
    }

    let transactions = sqlx::query_as::<_, Transaction>(
        "SELECT * FROM transactions WHERE asset_id IN (SELECT id FROM assets WHERE portfolio_id = ?)"
    )
    .bind(id)
    .fetch_all(pool.inner())
    .await
    .map_err(|_| Status::InternalServerError)?;

    let perf = StatsEngine::calculate_portfolio_performance(
        pool.inner(),
        &assets,
        &transactions,
        &portfolio.base_currency,
        currency_service
    ).await.map_err(|_| Status::InternalServerError)?;

    Ok(Json(perf))
}

#[utoipa::path(
    get,
    path = "/api/transactions/<id>/pnl",
    params(
        ("id" = i32, Path, description = "Sell transaction id"),
        ("strategy" = Option<String>, Query, description = "FIFO, LIFO or HYBRID (default FIFO)"),
        ("threshold_days" = Option<i64>, Query, description = "HYBRID short-term threshold (default 30)")
    ),
    responses(
        (status = 200, description = "Realized P&L breakdown for the sell", body = SellPnlResponse),
        (status = 400, description = "Transaction is not a SELL"),
        (status = 404, description = "Transaction not found")
    )
)]
#[get("/transactions/<id>/pnl?<strategy>&<threshold_days>")]
pub async fn get_transaction_sell_pnl(
    user: AuthedUser,
    id: i32,
    pool: &State<SqlitePool>,
    currency_service: &State<CurrencyService>,
    strategy: Option<String>,
    threshold_days: Option<i64>,
) -> Result<Json<SellPnlResponse>, Status> {
    require_transaction(pool.inner(), user.id, id, Access::Viewer).await?;

    // The three lookups below (transaction -> asset -> portfolio) stay: they are the
    // response's data source, not the authorization decision.
    let tx = sqlx::query_as::<_, Transaction>("SELECT * FROM transactions WHERE id = ?")
        .bind(id)
        .fetch_optional(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?
        .ok_or(Status::NotFound)?;

    if tx.r#type.to_uppercase() != "SELL" {
        return Err(Status::BadRequest);
    }

    // The base currency comes from the portfolio that owns the asset, never from
    // a path parameter, so this answer cannot disagree with the portfolio on screen.
    let asset = sqlx::query_as::<_, Asset>("SELECT * FROM assets WHERE id = ?")
        .bind(tx.asset_id)
        .fetch_optional(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?
        .ok_or(Status::NotFound)?;

    let portfolio = sqlx::query_as::<_, Portfolio>("SELECT * FROM portfolios WHERE id = ?")
        .bind(asset.portfolio_id)
        .fetch_optional(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?
        .ok_or(Status::NotFound)?;

    // Normalized once, then used for both the engine call and the response, so
    // the modal can never name a matching method the engine did not use.
    let strategy_val = match strategy.unwrap_or_default().to_uppercase().as_str() {
        "LIFO" => "LIFO".to_string(),
        "HYBRID" => "HYBRID".to_string(),
        _ => "FIFO".to_string(),
    };
    let threshold_val = threshold_days.unwrap_or(30);
    // Case-insensitive, deliberately, to match the two places that actually decide whether a
    // conversion is needed: `get_rate` uppercases (`currency_service.rs:401-403`) and
    // `replay_at` uses `eq_ignore_ascii_case` (`tax_engine.rs:940`). A case-sensitive `!=`
    // here takes the FX branch for an asset stored `usd` in a `USD` portfolio, demands a
    // `USDUSD=X` series that does not exist, and refuses with 422/503 a report the undated
    // path serves without complaint.
    let fx_applied = !asset.currency.eq_ignore_ascii_case(&portfolio.base_currency);

    // Read-only on purpose -- see the review record for this feature. The two other
    // analytics endpoints insert auto-detected SPLIT rows before reading; this one
    // agrees with them because fetchPortfolioData loads tax-summary before the
    // ledger tab is reachable, which pins the ordering. Pinned by
    // test_sell_pnl_sums_to_tax_summary_realized_pnl.
    //
    // Same query and same scope as get_portfolio_tax_summary.
    let transactions = sqlx::query_as::<_, Transaction>("SELECT * FROM transactions WHERE asset_id = ?")
        .bind(asset.id)
        .fetch_all(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?;

    let breakdown = TaxLotEngine::sell_breakdown(
        tx.id,
        &transactions,
        &asset.currency,
        &portfolio.base_currency,
        currency_service.inner(),
        &strategy_val,
        threshold_val,
        pool.inner(),
    )
    .await
    .map_err(|_| Status::InternalServerError)?
    .ok_or(Status::NotFound)?;

    // This list is already loaded, and it is scoped to exactly the asset the replay
    // consumed. The modal reads this instead of scanning whatever list the browser
    // happens to hold, which can be one fetch stale.
    //
    // Scoped to the lots this sell actually consumed, not "any split on the asset":
    // the replay only rescales lots that already exist, so a split predating every
    // matched buy adjusted nothing and the footnote would claim otherwise. A split
    // on the same date as a buy also adjusted nothing -- splits sort first on a tie.
    let earliest_buy = breakdown.matches.iter().map(|m| m.buy_date).min();
    let split_applied = match earliest_buy {
        Some(buy_floor) => transactions.iter().any(|t| {
            t.r#type.to_uppercase() == "SPLIT" && t.date > buy_floor && t.date <= tx.date
        }),
        None => false,
    };

    // Full precision, unquantised, per the review record: no field on this response
    // is rounded. The frontend renders it with toFixed(2), so the digit tail is a
    // wire concern only, and quantising here would make this field the one exception
    // to a rule the rest of the payload obeys.
    let roi = if breakdown.cost_basis > Decimal::ZERO {
        Some(breakdown.realized_pnl / breakdown.cost_basis)
    } else {
        None
    };

    Ok(Json(SellPnlResponse {
        transaction_id: tx.id,
        asset_id: asset.id,
        symbol: asset.symbol.clone(),
        asset_type: asset.asset_type.clone(),
        strategy: strategy_val,
        threshold_days: threshold_val,
        currency: portfolio.base_currency.clone(),
        asset_currency: asset.currency.clone(),
        fx_applied,
        split_applied,
        sell_date: breakdown.sell_date,
        sold_qty: breakdown.sold_qty,
        // As entered, in asset currency.
        price: crate::db_types::str_to_decimal(&tx.price),
        fee: crate::db_types::str_to_decimal(&tx.fee),
        proceeds: breakdown.proceeds,
        cost_basis: breakdown.cost_basis,
        realized_pnl: breakdown.realized_pnl,
        roi,
        matched_qty: breakdown.matched_qty,
        unmatched_qty: breakdown.unmatched_qty,
        matches: breakdown.matches,
    }))
}

// ---------------------------------------------------------------------------
// The dated tax report
// ---------------------------------------------------------------------------

/// Wall-clock bound on one report.
///
/// Checked between outbound calls, not between assets: one asset can cost two calls (a
/// close and a currency pair), so a check placed between assets can be followed by 66 s
/// of work — past the ingress' 60 s default, which turns the designed 503 into an HTML
/// 504 with no typed body and no `retryable`. It cannot preempt a call already in flight;
/// it refuses the next one. A cold portfolio can legitimately exceed this once and
/// succeed on retry, because the first attempt warmed the per-pair series.
pub const REPORT_DEADLINE: Duration = Duration::from_secs(20);

/// The caps on the two missing-item lists (settled value: 20 symbols, 10 pairs). The list
/// is for a human to read; a portfolio with 200 unpriced symbols needs the count, not the
/// list, and `missing_*_total` is what keeps truncation from being silent.
const MISSING_PRICES_CAP: usize = 20;
const MISSING_RATES_CAP: usize = 10;

/// The run context's splits statement. A constant in the payload rather than prose in each
/// renderer, because the CSV carries it as a cell and the file's first row is the header
/// row.
pub const SPLITS_NOTE: &str =
    "Split-adjusted history is as recorded when this report was generated; a split recorded later changes past figures.";

/// The window a dated request asks about, after defaults are applied.
pub struct ReportWindow {
    /// `None` means inception — or "the portfolio has no transaction at all".
    pub from: Option<NaiveDate>,
    pub to: NaiveDate,
}

/// Pricing failures collected across every asset, so one response can name all of them
/// instead of failing on the first (Goal 5).
///
/// `pub` so `tests/tax_report_test.rs` can pin the caps: the collection itself cannot be
/// exercised hermetically (see Step 9), and the cap is the part of that contract that can.
#[derive(Default)]
pub struct MissingItems {
    prices: Vec<String>,
    rates: Vec<MissingRate>,
}

impl MissingItems {
    pub fn new() -> Self { Self::default() }

    /// Once per symbol, in the order first seen.
    pub fn add_price(&mut self, symbol: &str) {
        if !self.prices.iter().any(|s| s == symbol) {
            self.prices.push(symbol.to_string());
        }
    }

    /// Once per pair. A missing rate is a pair, not a symbol, and `missing_rates` says so.
    pub fn add_rate(&mut self, from: &str, to: &str) {
        if !self.rates.iter().any(|r| r.from == from && r.to == to) {
            self.rates.push(MissingRate { from: from.to_string(), to: to.to_string() });
        }
    }

    pub fn is_empty(&self) -> bool { self.prices.is_empty() && self.rates.is_empty() }

    /// Totals are counted before truncation, so a client can tell a short list from a
    /// complete one.
    pub fn into_error_body(self, error: &str) -> ErrorBody {
        let prices_total = self.prices.len();
        let rates_total = self.rates.len();
        ErrorBody {
            error: error.to_string(),
            missing_prices: self.prices.into_iter().take(MISSING_PRICES_CAP).collect(),
            missing_rates: self.rates.into_iter().take(MISSING_RATES_CAP).collect(),
            missing_prices_total: prices_total,
            missing_rates_total: rates_total,
            retryable: false,
        }
    }
}

/// The route's one error type, in the style of the auth `Unavailable` responder
/// (`auth/routes.rs:33-52`): a responder, so `Result<..>` needs no conversion layer, and
/// public with public fields so a test can assert a failure it produced directly.
pub struct ReportError {
    pub status: Status,
    pub body: ErrorBody,
}

impl ReportError {
    /// A parameter the client can fix. 422, never retryable.
    pub fn validation(param: &str, detail: &str) -> Self {
        Self::plain(Status::UnprocessableEntity, format!("{param}: {detail}"), false)
    }

    /// Prices or rates that do not exist at or before the requested date. 422: asking
    /// again changes nothing.
    pub fn no_data(missing: MissingItems) -> Self {
        Self {
            status: Status::UnprocessableEntity,
            body: missing.into_error_body("no price or rate exists at or before the requested date"),
        }
    }

    /// A provider we could not ask. 503: the same request may succeed later.
    pub fn provider(detail: String) -> Self {
        Self::plain(Status::ServiceUnavailable, format!("pricing provider unavailable: {detail}"), true)
    }

    /// The elapsed deadline. Same 503 as an outage, and fail-closed: no partial payload.
    pub fn deadline() -> Self {
        Self::plain(
            Status::ServiceUnavailable,
            format!("the report exceeded its {} second deadline", REPORT_DEADLINE.as_secs()),
            true,
        )
    }

    pub fn not_found() -> Self {
        Self::plain(Status::NotFound, "portfolio not found".to_string(), false)
    }

    pub fn internal(detail: String) -> Self {
        Self::plain(Status::InternalServerError, detail, false)
    }

    /// `require` answers `NotFound` for a portfolio the caller cannot see, and
    /// `InternalServerError` when the access check itself failed (`access.rs:78-93`).
    pub fn from_status(status: Status) -> Self {
        // `==` rather than a match: Rocket's `Status` is not usable in patterns (it is a
        // non-structural struct of a code and a reason).
        if status == Status::NotFound {
            Self::not_found()
        } else {
            Self::internal(format!("access check failed: {}", status.reason().unwrap_or("unknown")))
        }
    }

    fn plain(status: Status, error: String, retryable: bool) -> Self {
        Self {
            status,
            body: ErrorBody {
                error,
                missing_prices: Vec::new(),
                missing_rates: Vec::new(),
                missing_prices_total: 0,
                missing_rates_total: 0,
                retryable,
            },
        }
    }
}

impl std::fmt::Debug for ReportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ReportError({}, {})", self.status, self.body.error)
    }
}

impl<'r> Responder<'r, 'static> for ReportError {
    fn respond_to(self, _: &'r Request<'_>) -> rocket::response::Result<'static> {
        // Infallible in practice; the fallback is a valid body rather than a second
        // failure mode.
        let body = serde_json::to_string(&self.body)
            .unwrap_or_else(|_| r#"{"error":"internal","retryable":false}"#.to_string());
        Response::build()
            .status(self.status)
            .header(ContentType::JSON)
            .sized_body(body.len(), Cursor::new(body))
            .ok()
    }
}

/// The status a pricing failure maps to. Pure so the mapping is testable without a
/// provider: `PricingError::NoData` is a fact about the world (422), and
/// `ProviderUnavailable` is a fact about this attempt (503).
pub fn pricing_failure_status(error: &PricingError) -> Status {
    match error {
        PricingError::NoData => Status::UnprocessableEntity,
        PricingError::ProviderUnavailable(_) => Status::ServiceUnavailable,
    }
}

/// The route's three shapes. The enum is hand-written rather than Rocket's `Either`: that
/// type is re-exported `#[doc(hidden)]` (`rocket-0.5.1/src/lib.rs:186`).
// The arms differ in size because one carries the legacy `Value` and another the typed
// payload. Boxing the report would change the shape the CSV arm consumes, and this value
// exists for the length of one request.
#[allow(clippy::large_enum_variant)]
pub enum TaxSummaryResponse {
    Legacy(Json<serde_json::Value>),
    Report(Json<ReportResponse>),
    /// The dated payload as a server-generated file: `render` projects `ReportResponse`,
    /// so the file's numbers are the route's numbers and nothing is re-derived here.
    Csv { body: String, filename: String },
}

impl<'r> Responder<'r, 'static> for TaxSummaryResponse {
    fn respond_to(self, request: &'r Request<'_>) -> rocket::response::Result<'static> {
        match self {
            TaxSummaryResponse::Legacy(json) => json.respond_to(request),
            TaxSummaryResponse::Report(json) => json.respond_to(request),
            TaxSummaryResponse::Csv { body, filename } => Response::build()
                .status(Status::Ok)
                // `ContentType::new` rather than a constant: Rocket has no CSV constant
                // (`rocket_http-0.5.1/src/header/content_type.rs:230`).
                .header(ContentType::new("text", "csv"))
                .raw_header("Content-Disposition", format!("attachment; filename=\"{filename}\""))
                .sized_body(body.len(), Cursor::new(body))
                .ok(),
        }
    }
}

/// The window a request asked about, or `None` for the legacy undated summary.
///
/// `None` is the whole of "no window": it is the only shape that still carries
/// `assets[].tax_lots`, `current_shares` and `total_portfolio_value`, which the printable
/// statement and `PortfolioDetail` read. `from` alone means through today and `to` alone
/// means from inception — both-or-neither was rejected because it forces every caller to
/// invent a start date, and an invented start silently changes the number.
fn parse_window(
    from: Option<&str>,
    to: Option<&str>,
    today: NaiveDate,
) -> Result<Option<ReportWindow>, ReportError> {
    if from.is_none() && to.is_none() {
        return Ok(None);
    }
    let (Some(from), Some(to)) = (from, to) else {
        let parsed_from = from.map(|raw| parse_date_param("from", raw)).transpose()?;
        let parsed_to = to.map(|raw| parse_date_param("to", raw)).transpose()?.unwrap_or(today);
        if let Some(from) = parsed_from {
            // Future check FIRST. With `to` absent, `parsed_to` is today, so a `from > parsed_to`
            // test would fire first and answer "must not be after `to`" to a caller that never
            // sent `to` — naming a parameter it did not supply and sending it looking for one.
            // With `to` present, `from > today` implies `to >= from > today`, so the `to`
            // future check below would have swallowed it too. Reordered, both are reachable.
            if from > today {
                return Err(ReportError::validation("from", "is in the future"));
            }
            if from > parsed_to {
                return Err(ReportError::validation("from", "must not be after `to`"));
            }
        }
        if parsed_to > today {
            return Err(ReportError::validation("to", "is in the future"));
        }
        return Ok(Some(ReportWindow { from: parsed_from, to: parsed_to }));
    };
    // Both are present: one window, validated the same way.
    let parsed_from = parse_date_param("from", from)?;
    let parsed_to = parse_date_param("to", to)?;
    if parsed_from > parsed_to {
        return Err(ReportError::validation("from", "must not be after `to`"));
    }
    if parsed_from > today {
        return Err(ReportError::validation("from", "is in the future"));
    }
    if parsed_to > today {
        return Err(ReportError::validation("to", "is in the future"));
    }
    Ok(Some(ReportWindow { from: Some(parsed_from), to: parsed_to }))
}

/// `csv` or nothing. The default is JSON, and JSON is the documented surface: the OpenAPI
/// document describes the typed payload, and `?format=csv` is the same bytes in another
/// envelope, described in the operation's summary rather than as a second content type for
/// status 200 (which utoipa's response map, keyed by status, cannot express).
fn parse_format(format: Option<&str>, dated: bool) -> Result<bool, ReportError> {
    match format {
        None => Ok(false),
        // Both names are matched case-insensitively, and `json` is a no-op: it is what the
        // route already returns, so a client that always appends `format=json` must not be
        // refused for asking for the thing it would have got. The committed document
        // advertises `format` as a free-form string and there is deliberately no
        // `params(...)` block to attach an enum to (see this module's header note), so
        // refusing the default was the only way the mismatch could bite -- and it bit.
        Some(f) if f.eq_ignore_ascii_case("json") => Ok(false),
        Some(f) if f.eq_ignore_ascii_case("csv") && dated => Ok(true),
        Some(f) if f.eq_ignore_ascii_case("csv") => {
            Err(ReportError::validation("format", "`csv` needs `from` or `to`"))
        }
        Some(other) => Err(ReportError::validation(
            "format",
            &format!("`{other}` is not a known format; `csv` and `json` are the accepted values"),
        )),
    }
}

fn parse_date_param(param: &str, raw: &str) -> Result<NaiveDate, ReportError> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map_err(|_| ReportError::validation(param, "expected YYYY-MM-DD"))
}

/// Strict on the dated path: a present-but-unrecognized strategy is a 422 naming the
/// accepted values, rather than the silent fallback to FIFO that both this endpoint and
/// the single-sell route used to run. Dated-path only (decision D6).
fn strict_strategy(strategy: Option<&str>) -> Result<String, ReportError> {
    match strategy {
        None => Ok("FIFO".to_string()),
        Some(raw) => match raw.to_uppercase().as_str() {
            "FIFO" => Ok("FIFO".to_string()),
            "LIFO" => Ok("LIFO".to_string()),
            "HYBRID" => Ok("HYBRID".to_string()),
            other => Err(ReportError::validation(
                "strategy",
                &format!("`{other}` is not one of FIFO, LIFO, HYBRID"),
            )),
        },
    }
}

/// The undated path's rule, which is the sell-P&L route's (`analytics.rs:216-222`):
/// normalize once and use the normalized value for both the engine call and the response,
/// so the response can never name a matching method the engine did not use. It still
/// falls back to FIFO rather than rejecting: "absent parameters reproduce today's
/// behaviour" holds for an unrecognized one too.
fn normalize_strategy(strategy: Option<&str>) -> String {
    match strategy.unwrap_or_default().to_uppercase().as_str() {
        "LIFO" => "LIFO".to_string(),
        "HYBRID" => "HYBRID".to_string(),
        _ => "FIFO".to_string(),
    }
}

/// Why one asset's close could not be resolved.
enum CloseFailure {
    /// No close exists at or before the date. A 422 item, not a provider outage.
    NoData,
    /// We could not ask. A 503.
    Provider(String),
    /// The database itself failed. A 500.
    Database(String),
}

/// The close for `symbol` on or before `to`, with the date that actually priced it and
/// which source answered.
///
/// The read order is the pricing layer's, reused rather than re-invented:
/// `stored_window_is_fresh` is the *same* predicate `get_historical_prices` applies
/// (`currency_service.rs:713-717`), so the route's rule and the layer's cannot drift, and
/// the provider is asked only when the stored window stops short of the requested date.
/// The one thing this adds over calling `get_historical_prices` alone is the source flag
/// Goal 4 requires: the layer returns a series, not where it came from.
async fn resolve_close(
    currency_service: &CurrencyService,
    pool: &SqlitePool,
    symbol: &str,
    to: NaiveDate,
) -> Result<(Decimal, NaiveDate, PriceSource), CloseFailure> {
    let window_start = to - chrono::Duration::days(FX_LOOKBACK_DAYS);

    let stored = currency_service
        .get_historical_prices_from_db(symbol, window_start, to, pool)
        .await
        .map_err(|e| CloseFailure::Database(e.to_string()))?;
    let newest = stored.iter().map(|(date, _)| *date).max();
    if stored_window_is_fresh(newest, to)
        && let Some((date_used, close)) = select_rate(&stored, to, FX_LOOKBACK_DAYS)
    {
        return Ok((close, date_used, PriceSource::Database));
    }

    match currency_service.get_historical_prices(symbol, window_start, to, pool).await {
        Ok(series) => match select_rate(&series, to, FX_LOOKBACK_DAYS) {
            Some((date_used, close)) => Ok((close, date_used, PriceSource::Provider)),
            None => Err(CloseFailure::NoData),
        },
        Err(PricingError::NoData) => Err(CloseFailure::NoData),
        Err(PricingError::ProviderUnavailable(detail)) => Err(CloseFailure::Provider(detail)),
    }
}

/// Build the dated report.
///
/// `deadline` is a parameter rather than the constant alone so a test can pass
/// `Duration::ZERO` and prove the fail-closed path without waiting 20 s.
///
/// The deadline bounds **this function's own** outbound calls, and only those. Per asset the
/// order is one pair prefetch, then one close, each preceded by a deadline check.
///
/// The pair prefetch avoids the network only when the stored pair window is already fresh at
/// `to` (`currency_service.rs:712-717` returns stored rows without fetching on that condition).
/// It does **not** check that the stored window reaches back over the transactions, and when it
/// is fresh-but-short `replay_at`'s per-transaction `get_rate` still falls through to the
/// provider for the uncovered dates (`currency_service.rs:434-441`) — after the last deadline
/// check here. So a slow-but-succeeding provider can still push total elapsed past the ingress
/// bound in that state. No route-local fix exists: `get_historical_prices` owns the fetch
/// decision, and coverage of the stored window is the pricing layer's own stated follow-up
/// (`currency_service.rs:392-394`). Fail-closed is unaffected — an in-replay provider failure
/// still returns 503 or 422 rather than a partial payload.
#[allow(clippy::too_many_arguments)]
pub async fn build_tax_report(
    pool: &SqlitePool,
    currency_service: &CurrencyService,
    portfolio: &Portfolio,
    assets: &[Asset],
    strategy: &str,
    threshold_days: i64,
    window: &ReportWindow,
    deadline: Duration,
) -> Result<ReportResponse, ReportError> {
    let start = Instant::now();
    let expired = || start.elapsed() >= deadline;

    // Every asset's transactions first: the effective window's start is the portfolio's
    // earliest transaction, which is not knowable one asset at a time. Borrowed, not
    // cloned: `Asset` is not `Clone` and the caller owns the rows for the whole call.
    let mut loaded: Vec<(&Asset, Vec<Transaction>)> = Vec::with_capacity(assets.len());
    for asset in assets {
        let txs = sqlx::query_as::<_, Transaction>("SELECT * FROM transactions WHERE asset_id = ?")
            .bind(asset.id)
            .fetch_all(pool)
            .await
            .map_err(|e| ReportError::internal(e.to_string()))?;
        loaded.push((asset, txs));
    }

    let first_in_scope = loaded
        .iter()
        .flat_map(|(_, txs)| txs.iter())
        .filter(|tx| tx.date.date_naive() <= window.to)
        .map(|tx| tx.date.date_naive())
        .min();
    let requested_from = window.from.or(first_in_scope);

    let mut missing = MissingItems::new();
    let mut asset_totals = Vec::new();
    let mut rows = Vec::new();
    let mut totals = ReportTotals::zero();

    for (asset, txs) in &loaded {
        // Same case-insensitive identity rule as above and as in the engine.
        let fx_applied = !asset.currency.eq_ignore_ascii_case(&portfolio.base_currency);
        // A split dated after the window end changed nothing inside the window, and the
        // replay's filter is what makes that true of the figures as well as the flag.
        let split_applied = txs.iter().any(|tx| {
            tx.r#type.to_uppercase() == "SPLIT" && tx.date.date_naive() <= window.to
        });

        let in_scope = txs.iter().any(|tx| tx.date.date_naive() <= window.to);
        if !in_scope {
            // Nothing dated at or before the window end: nothing realized, nothing open,
            // and no close to obtain — so no outbound call and no failure either.
            asset_totals.push(AssetTotals::empty(asset, fx_applied, split_applied));
            continue;
        }

        if fx_applied {
            if expired() {
                return Err(ReportError::deadline());
            }
            let pair = fx_symbol(&asset.currency, &portfolio.base_currency);
            let earliest = txs.iter()
                .map(|tx| tx.date.date_naive())
                .min()
                .expect("the asset has at least one transaction");
            let warm_start = earliest - chrono::Duration::days(FX_LOOKBACK_DAYS);
            match currency_service.get_historical_prices(&pair, warm_start, window.to, pool).await {
                Ok(_) => {}
                Err(PricingError::NoData) => {
                    missing.add_rate(&asset.currency, &portfolio.base_currency);
                    continue;
                }
                Err(PricingError::ProviderUnavailable(detail)) => {
                    return Err(ReportError::provider(detail));
                }
            }
        }

        if expired() {
            return Err(ReportError::deadline());
        }
        let (close, price_date_used, price_source) =
            match resolve_close(currency_service, pool, &asset.symbol, window.to).await {
                Ok(found) => found,
                Err(CloseFailure::NoData) => {
                    missing.add_price(&asset.symbol);
                    continue;
                }
                Err(CloseFailure::Provider(detail)) => return Err(ReportError::provider(detail)),
                Err(CloseFailure::Database(detail)) => return Err(ReportError::internal(detail)),
            };

        let replay = match TaxLotEngine::replay_at(
            txs, window.to, window.to, close, &asset.currency, &portfolio.base_currency,
            currency_service, strategy, threshold_days, pool,
        )
        .await
        {
            Ok(replay) => replay,
            Err(e) => {
                // `replay_at` can fail three ways and they are enumerated rather than
                // inferred: the pricing layer's typed errors, a `sqlx` error, and
                // `no_rate_error` — a plain `anyhow!` from that layer with no type tag
                // (`currency_service.rs:277`), which is "this pair has no close near that
                // date". The last one is the only remaining case.
                if let Some(sq) = e.downcast_ref::<sqlx::Error>() {
                    return Err(ReportError::internal(sq.to_string()));
                }
                if let Some(PricingError::ProviderUnavailable(detail)) = e.downcast_ref::<PricingError>() {
                    return Err(ReportError::provider(detail.clone()));
                }
                missing.add_rate(&asset.currency, &portfolio.base_currency);
                continue;
            }
        };

        let in_window = |date: NaiveDate| {
            requested_from.is_none_or(|from| date >= from) && date <= window.to
        };
        let realized_in_window: Decimal = replay.sell_breakdowns.iter()
            .filter(|b| in_window(b.sell_date.date_naive()))
            .map(|b| b.realized_pnl)
            .sum();
        let realized_cumulative: Decimal = replay.sell_breakdowns.iter()
            .map(|b| b.realized_pnl)
            .sum();
        let market_value: Decimal = replay.open_lots.iter().map(|lot| lot.market_value).sum();
        let cost_basis: Decimal = replay.open_lots.iter()
            .map(|lot| lot.remaining_qty * lot.unit_cost)
            .sum();
        // Derived from the two sums rather than by adding the per-lot gains, so Goal 2's
        // equality is a comparison of two independently computed figures.
        let unrealized_at_end = market_value - cost_basis;

        for breakdown in &replay.sell_breakdowns {
            if !in_window(breakdown.sell_date.date_naive()) {
                continue;
            }
            let sell = txs.iter()
                .find(|tx| tx.id == breakdown.sell_transaction_id)
                .expect("every breakdown came from this transaction set");
            for matched in &breakdown.matches {
                rows.push(ReportRow::sold_lot(
                    asset, Some(price_date_used), replay.fx_rate_date_used,
                    Some(price_source), sell, breakdown, matched,
                ));
            }
            if breakdown.unmatched_qty > Decimal::ZERO {
                rows.push(ReportRow::unmatched(
                    asset, Some(price_date_used), replay.fx_rate_date_used,
                    Some(price_source), sell, breakdown,
                ));
            }
        }
        for lot in &replay.open_lots {
            rows.push(ReportRow::open_lot(
                asset, Some(price_date_used), replay.fx_rate_date_used,
                Some(price_source), lot,
            ));
        }

        totals.realized_in_window += realized_in_window;
        totals.realized_cumulative += realized_cumulative;
        totals.unrealized_at_end += unrealized_at_end;
        totals.market_value += market_value;
        totals.cost_basis += cost_basis;

        asset_totals.push(AssetTotals {
            asset_id: asset.id,
            symbol: asset.symbol.clone(),
            asset_type: asset.asset_type.clone(),
            asset_currency: asset.currency.clone(),
            realized_in_window,
            realized_cumulative,
            unrealized_at_end,
            market_value,
            cost_basis,
            price_date_used: Some(price_date_used),
            fx_rate_date_used: replay.fx_rate_date_used,
            price_source: Some(price_source),
            fx_applied,
            split_applied,
        });
    }

    if !missing.is_empty() {
        // Fail closed. A partial tax report is the outcome Goal 5 exists to prevent, so
        // "return what was collected" is not an option.
        return Err(ReportError::no_data(missing));
    }

    Ok(ReportResponse {
        context: ReportContext {
            portfolio_id: portfolio.id,
            portfolio_name: portfolio.name.clone(),
            base_currency: portfolio.base_currency.clone(),
            strategy: strategy.to_string(),
            threshold_days,
            requested_from,
            requested_to: window.to,
            splits_note: SPLITS_NOTE.to_string(),
        },
        asset_totals,
        totals,
        rows,
    })
}
