use crate::api_routes::lookup::AssetLookupResult;
use crate::models;
use crate::schemas;
use utoipa::OpenApi;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityRequirement, SecurityScheme};
use utoipa::openapi::{Components, RefOr, Response};

#[derive(OpenApi)]
#[openapi(
    paths(
        crate::api_routes::portfolios::create_portfolio,
        crate::api_routes::portfolios::list_portfolios,
        crate::api_routes::portfolios::get_portfolio,
        crate::api_routes::portfolios::delete_portfolio,
        crate::api_routes::portfolios::update_portfolio,
        crate::api_routes::transactions::create_asset,
        crate::api_routes::transactions::update_asset,
        crate::api_routes::transactions::delete_asset,
        crate::api_routes::transactions::create_transaction,
        crate::api_routes::transactions::list_portfolio_transactions,
        crate::api_routes::transactions::delete_transaction,
        crate::api_routes::analytics::get_portfolio_tax_summary,
        crate::api_routes::analytics::sync_portfolio_splits,
        crate::api_routes::analytics::get_portfolio_performance,
        crate::api_routes::analytics::get_transaction_sell_pnl,
        crate::api_routes::lookup::lookup_isin,
        crate::api_routes::backup::capabilities,
    ),
    components(schemas(
        models::Portfolio,
        models::Asset,
        models::Transaction,
        schemas::PortfolioCreate,
        schemas::PortfolioOut,
        schemas::SyncSplitsOut,
        schemas::SyncSplitFailure,
        schemas::PortfolioUpdate,
        schemas::AssetCreate,
        schemas::AssetOut,
        schemas::TransactionCreate,
        schemas::TransactionOut,
        schemas::TaxLot,
        schemas::AssetTaxSummary,
        schemas::LotMatch,
        schemas::SellPnlResponse,
        schemas::ReportContext,
        schemas::AssetTotals,
        schemas::ReportTotals,
        schemas::ReportRow,
        schemas::RowKind,
        schemas::PriceSource,
        schemas::ReportResponse,
        schemas::ErrorBody,
        schemas::MissingRate,
        AssetLookupResult,
    ))
)]
pub struct ApiDoc;

/// Name of the security scheme the whole API is protected by.
pub const SESSION_SECURITY_SCHEME: &str = "pp_session";

/// The one document this project publishes.
///
/// Every consumer — the `/api-docs` handler, `--print-openapi`, the committed
/// `docs/openapi/openapi.json`, and the route-coverage test — goes through this
/// function, so the served document and the committed one cannot disagree.
///
/// The security scheme and the `401` responses are added here rather than as
/// derive attributes on 15 handlers: one code path, so a handler cannot be
/// forgotten. `bearerAuth` is deliberately **not** declared — that scheme belongs
/// to the API-keys spec, and declaring a scheme nothing accepts is worse than
/// declaring nothing.
pub fn document() -> utoipa::openapi::OpenApi {
    let mut doc = ApiDoc::openapi();

    let components = doc.components.get_or_insert_with(Components::new);
    components.add_security_scheme(
        SESSION_SECURITY_SCHEME,
        SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::with_description(
            SESSION_SECURITY_SCHEME,
            "Session cookie issued by /auth/callback. There is no bearer token in this API: the browser never sees a credential.",
        ))),
    );
    doc.security = Some(vec![SecurityRequirement::new(
        SESSION_SECURITY_SCHEME,
        std::iter::empty::<&str>(),
    )]);

    for (path, item) in doc.paths.paths.iter_mut() {
        // Only the protected surface. `/` and `/api-docs/openapi.json` stay public
        // and must not advertise a `401`.
        // The same rule the fairing enforces, not a bare `/api` prefix: `/api-docs`
        // is a public route and must not be documented as requiring a session.
        if !(path == "/api" || path.starts_with("/api/")) {
            continue;
        }
        for operation in item.operations.values_mut() {
            operation.responses.responses.insert(
                "401".to_string(),
                RefOr::T(Response::new("Not authenticated")),
            );
        }
    }

    doc
}

/// Serialise [`document`] the one way the committed file is produced. Both the
/// handler and `--print-openapi` use this, because the committed file is compared
/// byte-for-byte in CI and key order differs between serialisers.
pub fn document_json() -> String {
    // Infallible in practice: the document is built from serialisable values.
    serde_json::to_string_pretty(&document()).expect("openapi document serialises")
}
