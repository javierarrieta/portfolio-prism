use utoipa::OpenApi;
use crate::schemas;
use crate::models;
use crate::api_routes::lookup::AssetLookupResult;

#[derive(OpenApi)]
#[openapi(
    paths(
        crate::api_routes::portfolios::create_portfolio,
        crate::api_routes::portfolios::list_portfolios,
        crate::api_routes::portfolios::get_portfolio,
        crate::api_routes::portfolios::delete_portfolio,
        crate::api_routes::portfolios::update_portfolio,
        crate::api_routes::transactions::create_asset,
        crate::api_routes::transactions::delete_asset,
        crate::api_routes::transactions::create_transaction,
        crate::api_routes::transactions::list_portfolio_transactions,
        crate::api_routes::transactions::delete_transaction,
        crate::api_routes::analytics::get_portfolio_tax_summary,
        crate::api_routes::analytics::get_portfolio_performance,
        crate::api_routes::analytics::get_transaction_sell_pnl,
        crate::api_routes::lookup::lookup_isin,
    ),

    components(
        schemas(
            models::Portfolio,
            models::Asset,
            models::Transaction,
            schemas::PortfolioCreate,
            schemas::PortfolioOut,
            schemas::PortfolioUpdate,
            schemas::AssetCreate,
            schemas::AssetOut,
            schemas::TransactionCreate,
            schemas::TransactionOut,
            schemas::TaxLot,
            schemas::AssetTaxSummary,
            schemas::LotMatch,
            schemas::SellPnlResponse,
            AssetLookupResult,
        )
    )
)]
pub struct ApiDoc;
