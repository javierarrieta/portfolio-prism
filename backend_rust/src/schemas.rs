use serde::{Serialize, Deserialize};
use chrono::{DateTime, NaiveDate, Utc};
use utoipa::ToSchema;
use rust_decimal::Decimal;
use crate::db_types::{decimal_json, decimal_opt_json};
use crate::models::{Asset, Transaction};

// --- Transaction ---
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TransactionCreate {
    pub r#type: String,
    #[serde(with = "decimal_json")]
    pub quantity: Decimal,
    #[serde(with = "decimal_json")]
    pub price: Decimal,
    #[serde(with = "decimal_json")]
    pub fee: Decimal,
    pub date: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TransactionOut {
    pub id: i32,
    pub asset_id: i32,
    pub r#type: String,
    #[serde(with = "decimal_json")]
    pub quantity: Decimal,
    #[serde(with = "decimal_json")]
    pub price: Decimal,
    #[serde(with = "decimal_json")]
    pub fee: Decimal,
    pub date: DateTime<Utc>,
}

// --- Asset ---
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AssetCreate {
    pub symbol: String,
    pub name: String,
    pub asset_type: String,
    pub sector: Option<String>,
    pub currency: String,
    #[serde(default)]
    pub isin: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AssetOut {
    pub id: i32,
    pub portfolio_id: i32,
    pub symbol: String,
    pub name: String,
    pub asset_type: String,
    pub sector: Option<String>,
    pub currency: String,
    pub isin: Option<String>,
    pub transactions: Vec<TransactionOut>,
}

// --- Portfolio ---
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PortfolioCreate {
    pub name: String,
    pub description: Option<String>,
    pub currency: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PortfolioOut {
    pub id: i32,
    pub name: String,
    pub description: Option<String>,
    pub currency: String,
    pub assets: Vec<AssetOut>,
}

// --- Portfolio Update ---
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PortfolioUpdate {
    pub currency: String,
}

// --- Asset Update ---
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AssetUpdate {
    pub currency: String,
}

// --- Tax Lot Out ---
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TaxLot {
    /// The buy that opened this lot. Present so a lot can be joined back to the ledger
    /// row a person is reading.
    pub buy_transaction_id: i32,
    pub buy_date: DateTime<Utc>,
    #[serde(with = "decimal_json")]
    pub buy_price: Decimal,
    #[serde(with = "decimal_json")]
    pub original_qty: Decimal,
    #[serde(with = "decimal_json")]
    pub remaining_qty: Decimal,
    #[serde(with = "decimal_json")]
    pub latent_gain_loss: Decimal,
    #[serde(with = "decimal_json")]
    pub latent_roi: Decimal,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AssetTaxSummary {
    pub symbol: String,
    pub asset_type: String,
    #[serde(with = "decimal_json")]
    pub current_shares: Decimal,
    #[serde(with = "decimal_json")]
    pub average_cost: Decimal,
    #[serde(with = "decimal_json")]
    pub current_price: Decimal,
    #[serde(with = "decimal_json")]
    pub total_cost: Decimal,
    #[serde(with = "decimal_json")]
    pub market_value: Decimal,
    #[serde(with = "decimal_json")]
    pub unrealized_pnl: Decimal,
    #[serde(with = "decimal_json")]
    pub unrealized_roi: Decimal,
    #[serde(with = "decimal_json")]
    pub realized_pnl: Decimal,
    pub tax_lots: Vec<TaxLot>,
}

// --- Sell P&L breakdown ---

/// One row per buy lot consumed by one sell. `buy_price` and `unit_cost` are
/// split-adjusted and expressed in the portfolio base currency; `unit_cost`
/// additionally carries the amortised buy fee.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LotMatch {
    pub buy_transaction_id: i32,
    pub buy_date: DateTime<Utc>,
    #[serde(with = "decimal_json")]
    pub buy_price: Decimal,
    #[serde(with = "decimal_json")]
    pub unit_cost: Decimal,
    #[serde(with = "decimal_json")]
    pub matched_qty: Decimal,
    #[serde(with = "decimal_json")]
    pub lot_qty_before_match: Decimal,
    #[serde(with = "decimal_json")]
    pub cost_basis: Decimal,
    #[serde(with = "decimal_json")]
    pub proceeds: Decimal,
    #[serde(with = "decimal_json")]
    pub pnl: Decimal,
    pub holding_days: i64,
    /// "SHORT" | "LONG", populated for the HYBRID strategy only; null otherwise.
    pub term: Option<String>,
}

/// One sell and every lot it consumed, in consumption order. `unmatched_qty` is
/// the quantity sold with no buy lot behind it, so it carries no cost basis and
/// contributes nothing to `realized_pnl`.
///
/// Engine-internal: not registered in the OpenAPI components, because no
/// response body is of this type.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SellBreakdown {
    pub sell_transaction_id: i32,
    pub sell_date: DateTime<Utc>,
    #[serde(with = "decimal_json")]
    pub sold_qty: Decimal,
    #[serde(with = "decimal_json")]
    pub matched_qty: Decimal,
    #[serde(with = "decimal_json")]
    pub unmatched_qty: Decimal,
    #[serde(with = "decimal_json")]
    pub proceeds: Decimal,
    #[serde(with = "decimal_json")]
    pub cost_basis: Decimal,
    #[serde(with = "decimal_json")]
    pub realized_pnl: Decimal,
    pub matches: Vec<LotMatch>,
}

/// One lot still open at `end`, valued at the requested date. The report's OPEN_LOT row
/// is this struct; `unrealized_roi` is `None` rather than zero when the lot has no cost
/// basis, so a consumer can tell "not applicable" from "0%".
#[derive(Debug, Clone)]
pub struct DatedLot {
    pub buy_transaction_id: i32,
    pub buy_date: DateTime<Utc>,
    /// Split-adjusted purchase price per unit, **in the portfolio's base currency** —
    /// converted at the buy's own date, not the asset's own currency. The value is
    /// `tx_price * get_rate(asset, base, buy_date)`, which is what the undated `TaxLot`
    /// and `LotMatch.buy_price` also carry, so the three agree by construction.
    /// Anything rendering this must label it base currency; a CSV column that says
    /// "in asset currency" while printing this value would mix two currencies on one row.
    pub buy_price: Decimal,
    /// The quantity originally bought, split-adjusted: a 100-share lot after a 2:1 split
    /// is 200 whether or not any of it has been sold.
    pub original_qty: Decimal,
    pub remaining_qty: Decimal,
    /// Fee-amortised cost per unit, in the portfolio's base currency.
    pub unit_cost: Decimal,
    pub market_value: Decimal,
    pub unrealized_gain_loss: Decimal,
    pub unrealized_roi: Option<Decimal>,
}

/// One dated replay: the open lots and every sell's lot attribution, from a single pass
/// over the transactions with `date <= end`.
#[derive(Debug, Clone)]
pub struct DatedReplay {
    pub open_lots: Vec<DatedLot>,
    pub sell_breakdowns: Vec<SellBreakdown>,
    /// The base-currency price the open lots were valued at. The report sums lot values
    /// and never multiplies this by a quantity of its own.
    pub current_price_base: Decimal,
    /// The date whose close priced the market value, or `None` when no conversion
    /// happened. Compared case-insensitively, matching `get_rate`'s own identity rule —
    /// an asset stored as `"usd"` in a `"USD"` portfolio needs no conversion and must not
    /// report one. Reports the date of the row that actually produced the rate, which
    /// `select_rate` may answer with an earlier date than the one requested.
    pub fx_rate_date_used: Option<NaiveDate>,
}

/// Endpoint payload: the breakdown flattened next to the context needed to read
/// it honestly — which strategy produced it, which currency the money is in,
/// whether FX or a split touched it. `price`/`fee` are the raw as-entered values
/// in `asset_currency`; every other money field is in `currency`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SellPnlResponse {
    pub transaction_id: i32,
    pub asset_id: i32,
    pub symbol: String,
    pub asset_type: String,
    pub strategy: String,
    pub threshold_days: i64,
    pub currency: String,
    pub asset_currency: String,
    pub fx_applied: bool,
    /// A SPLIT on this asset dated on or before the sell: buy prices in the
    /// matches are split-adjusted rather than as entered.
    pub split_applied: bool,
    pub sell_date: DateTime<Utc>,
    #[serde(with = "decimal_json")]
    pub sold_qty: Decimal,
    #[serde(with = "decimal_json")]
    pub price: Decimal,
    #[serde(with = "decimal_json")]
    pub fee: Decimal,
    #[serde(with = "decimal_json")]
    pub proceeds: Decimal,
    #[serde(with = "decimal_json")]
    pub cost_basis: Decimal,
    #[serde(with = "decimal_json")]
    pub realized_pnl: Decimal,
    /// Null when cost basis is zero — dividing there would invent a number.
    #[serde(with = "decimal_opt_json")]
    pub roi: Option<Decimal>,
    #[serde(with = "decimal_json")]
    pub matched_qty: Decimal,
    #[serde(with = "decimal_json")]
    pub unmatched_qty: Decimal,
    pub matches: Vec<LotMatch>,
}

// --- Dated tax report -------------------------------------------------------------

/// How the dated report priced an asset's close. Recorded so a regenerated file can
/// state where its numbers came from (Goal 4), and so a database-answered report can be
/// told from a provider-answered one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PriceSource {
    Database,
    Provider,
}

impl PriceSource {
    /// The token the CSV column carries. One spelling per concept.
    pub fn as_str(self) -> &'static str {
        match self {
            PriceSource::Database => "database",
            PriceSource::Provider => "provider",
        }
    }
}

/// Which selection a row is. Exhaustive on purpose: both renderers switch on it, and the
/// CSV's "this column totals, that one does not" rule is expressed through it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RowKind {
    SoldLot,
    OpenLot,
    Unmatched,
}

impl RowKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RowKind::SoldLot => "SOLD_LOT",
            RowKind::OpenLot => "OPEN_LOT",
            RowKind::Unmatched => "UNMATCHED",
        }
    }
}

/// What a dated request was answered about. `ReportContext` is the single authority for
/// `strategy` and `threshold_days`: the response does not repeat them at top level.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReportContext {
    pub portfolio_id: i32,
    pub portfolio_name: String,
    pub base_currency: String,
    pub strategy: String,
    pub threshold_days: i64,
    /// The window actually applied, not the parameters as typed. `requested_to` is the
    /// call's `to`, or today when `to` was absent; `requested_from` is the call's `from`,
    /// or the portfolio's first transaction on or before `requested_to`, or `None` when
    /// the portfolio has no transaction at all. An open end is visible in the artifact
    /// rather than inferred by its reader.
    pub requested_from: Option<NaiveDate>,
    pub requested_to: NaiveDate,
    /// Splits are as recorded when this was generated; the dated path deliberately does
    /// not sync them (issue #50), so a split recorded after the fact changes past
    /// figures. A constant string in the payload rather than prose in each renderer,
    /// because the CSV carries it as a cell (the file's first row is the header row).
    pub splits_note: String,
}

/// One asset's dated totals. Every field is computed in Rust from the same replay that
/// produced the rows; no renderer sums anything.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AssetTotals {
    pub asset_id: i32,
    pub symbol: String,
    pub asset_type: String,
    pub asset_currency: String,
    #[serde(with = "decimal_json")]
    pub realized_in_window: Decimal,
    #[serde(with = "decimal_json")]
    pub realized_cumulative: Decimal,
    #[serde(with = "decimal_json")]
    pub unrealized_at_end: Decimal,
    #[serde(with = "decimal_json")]
    pub market_value: Decimal,
    #[serde(with = "decimal_json")]
    pub cost_basis: Decimal,
    /// The close that priced this asset, and the date it actually came from. Null when
    /// the asset has no transaction at or before the window end, so no close was needed.
    pub price_date_used: Option<NaiveDate>,
    pub fx_rate_date_used: Option<NaiveDate>,
    pub price_source: Option<PriceSource>,
    pub fx_applied: bool,
    /// A SPLIT dated on or before the window end was applied to the lots in scope.
    pub split_applied: bool,
}

/// The portfolio rollup: exactly the five per-asset figures, summed across assets.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReportTotals {
    #[serde(with = "decimal_json")]
    pub realized_in_window: Decimal,
    #[serde(with = "decimal_json")]
    pub realized_cumulative: Decimal,
    #[serde(with = "decimal_json")]
    pub unrealized_at_end: Decimal,
    #[serde(with = "decimal_json")]
    pub market_value: Decimal,
    #[serde(with = "decimal_json")]
    pub cost_basis: Decimal,
}

/// One report row. Three selections share one shape: the kind-specific cells are null on
/// the rows where they do not apply, never omitted and never zero.
///
/// The asset-level pricing facts repeat on every row of that asset — one close per asset,
/// not one per lot — so the CSV is a projection of this vector and needs no join.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReportRow {
    pub row_type: RowKind,
    pub asset_id: i32,
    pub symbol: String,
    pub asset_type: String,
    pub asset_currency: String,
    pub price_date_used: Option<NaiveDate>,
    pub fx_rate_date_used: Option<NaiveDate>,
    pub price_source: Option<PriceSource>,

    // Sell-level, mirrored from `SellBreakdown` and the sell transaction. Repeated on
    // every lot row of that sell so the file pivots, exactly as `sellPnlCsv.ts` does.
    pub sell_transaction_id: Option<i32>,
    pub sell_date: Option<DateTime<Utc>>,
    #[serde(with = "decimal_opt_json")]
    pub sold_qty: Option<Decimal>,
    /// As entered, in `asset_currency`.
    #[serde(with = "decimal_opt_json")]
    pub sell_price: Option<Decimal>,
    /// As entered, in `asset_currency` — same rule as `sell_price`, and the only other
    /// field on this row that is not in the portfolio's base currency.
    #[serde(with = "decimal_opt_json")]
    pub sell_fee: Option<Decimal>,
    /// Sell-level, **matched quantity only**, in the portfolio's base currency. Summed
    /// from the per-match figures (`tax_engine.rs:761-763`), so it never includes the
    /// unmatched tail and must not be added to the `lot_proceeds` of the sell's rows.
    #[serde(with = "decimal_opt_json")]
    pub proceeds: Option<Decimal>,
    /// Sell-level, **matched quantity only**, in the portfolio's base currency. Summed
    /// from the per-match figures; excludes the unmatched tail.
    #[serde(with = "decimal_opt_json")]
    pub cost_basis: Option<Decimal>,
    /// Sell-level, **matched quantity only**, in the portfolio's base currency:
    /// `proceeds - cost_basis` over the matched lots. This is the whole sell's figure and
    /// repeats on every `SOLD_LOT` row of that sell — the column to SUM is `lot_pnl`, not
    /// this one.
    #[serde(with = "decimal_opt_json")]
    pub realized_pnl: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub matched_qty: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub unmatched_qty: Option<Decimal>,

    // Buy-lot cells: present on SOLD_LOT and OPEN_LOT.
    pub buy_transaction_id: Option<i32>,
    pub buy_date: Option<DateTime<Utc>>,
    /// Split-adjusted, in the portfolio's **base** currency — converted at the buy's own
    /// date. NOT `asset_currency`: `DatedLot.buy_price` and `LotMatch.buy_price` are both
    /// `tx_price * get_rate(asset, base, buy_date)`, and the frontend already renders
    /// `LotMatch.buy_price` with the base currency (`SellPnlModal.tsx:255`). The sell-side
    /// price on the same row IS asset currency, so the two columns must not share a label.
    #[serde(with = "decimal_opt_json")]
    pub buy_price: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub unit_cost: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub lot_matched_qty: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub lot_qty_before_match: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub lot_cost_basis: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub lot_proceeds: Option<Decimal>,
    /// The column that totals. `LotMatch.pnl` under the name the CSV uses.
    #[serde(with = "decimal_opt_json")]
    pub lot_pnl: Option<Decimal>,
    pub holding_days: Option<i64>,
    /// "SHORT" | "LONG" for HYBRID only; null otherwise, and never claimed on an open lot.
    pub term: Option<String>,

    // Open-lot cells, on OPEN_LOT only.
    #[serde(with = "decimal_opt_json")]
    pub original_qty: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub remaining_qty: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub market_value: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub unrealized_gain_loss: Option<Decimal>,
    #[serde(with = "decimal_opt_json")]
    pub unrealized_roi: Option<Decimal>,
}

impl ReportRow {
    /// The empty row every constructor starts from: every kind-specific cell null.
    fn of(kind: RowKind, asset_id: i32, symbol: &str, asset_type: &str, asset_currency: &str,
          price_source: Option<PriceSource>) -> Self {
        ReportRow {
            row_type: kind,
            asset_id,
            symbol: symbol.to_string(),
            asset_type: asset_type.to_string(),
            asset_currency: asset_currency.to_string(),
            price_date_used: None,
            fx_rate_date_used: None,
            price_source,
            sell_transaction_id: None, sell_date: None, sold_qty: None, sell_price: None,
            sell_fee: None, proceeds: None, cost_basis: None, realized_pnl: None,
            matched_qty: None, unmatched_qty: None,
            buy_transaction_id: None, buy_date: None, buy_price: None, unit_cost: None,
            lot_matched_qty: None, lot_qty_before_match: None, lot_cost_basis: None,
            lot_proceeds: None, lot_pnl: None, holding_days: None, term: None,
            original_qty: None, remaining_qty: None, market_value: None,
            unrealized_gain_loss: None, unrealized_roi: None,
        }
    }

    /// One buy lot a sell consumed, with the sell's context repeated.
    #[allow(clippy::too_many_arguments)]
    pub fn sold_lot(
        asset: &Asset,
        price_date_used: Option<NaiveDate>,
        fx_rate_date_used: Option<NaiveDate>,
        price_source: Option<PriceSource>,
        sell: &Transaction,
        breakdown: &SellBreakdown,
        matched: &LotMatch,
    ) -> Self {
        let mut row = ReportRow::of(RowKind::SoldLot, asset.id, &asset.symbol, &asset.asset_type, &asset.currency, price_source);
        row.price_date_used = price_date_used;
        row.fx_rate_date_used = fx_rate_date_used;
        row.sell_transaction_id = Some(sell.id);
        row.sell_date = Some(sell.date);
        row.sold_qty = Some(breakdown.sold_qty);
        row.sell_price = Some(crate::db_types::str_to_decimal(&sell.price));
        row.sell_fee = Some(crate::db_types::str_to_decimal(&sell.fee));
        row.proceeds = Some(breakdown.proceeds);
        row.cost_basis = Some(breakdown.cost_basis);
        row.realized_pnl = Some(breakdown.realized_pnl);
        row.matched_qty = Some(breakdown.matched_qty);
        row.unmatched_qty = Some(breakdown.unmatched_qty);
        row.buy_transaction_id = Some(matched.buy_transaction_id);
        row.buy_date = Some(matched.buy_date);
        row.buy_price = Some(matched.buy_price);
        row.unit_cost = Some(matched.unit_cost);
        row.lot_matched_qty = Some(matched.matched_qty);
        row.lot_qty_before_match = Some(matched.lot_qty_before_match);
        row.lot_cost_basis = Some(matched.cost_basis);
        row.lot_proceeds = Some(matched.proceeds);
        row.lot_pnl = Some(matched.pnl);
        row.holding_days = Some(matched.holding_days);
        row.term = matched.term.clone();
        row
    }

    /// A sell's unmatched tail: the quantity no buy lot covers. The sell's **computed
    /// money allocations** (`proceeds`, `cost_basis`, `realized_pnl`, every `lot_*`) stay
    /// null, because there is no matched quantity for them to describe — a zero would read
    /// as a realized figure of nothing. What the row does carry is the sell's identity, its
    /// quantities, and the **as-entered** `sell_price` and `sell_fee`, which are facts
    /// about the transaction rather than an allocation of it.
    pub fn unmatched(
        asset: &Asset,
        price_date_used: Option<NaiveDate>,
        fx_rate_date_used: Option<NaiveDate>,
        price_source: Option<PriceSource>,
        sell: &Transaction,
        breakdown: &SellBreakdown,
    ) -> Self {
        let mut row = ReportRow::of(RowKind::Unmatched, asset.id, &asset.symbol, &asset.asset_type, &asset.currency, price_source);
        row.price_date_used = price_date_used;
        row.fx_rate_date_used = fx_rate_date_used;
        row.sell_transaction_id = Some(sell.id);
        row.sell_date = Some(sell.date);
        row.sold_qty = Some(breakdown.sold_qty);
        row.sell_price = Some(crate::db_types::str_to_decimal(&sell.price));
        row.sell_fee = Some(crate::db_types::str_to_decimal(&sell.fee));
        row.matched_qty = Some(breakdown.matched_qty);
        row.unmatched_qty = Some(breakdown.unmatched_qty);
        row
    }

    /// One lot still open at the window end, valued at the requested date.
    pub fn open_lot(
        asset: &Asset,
        price_date_used: Option<NaiveDate>,
        fx_rate_date_used: Option<NaiveDate>,
        price_source: Option<PriceSource>,
        lot: &DatedLot,
    ) -> Self {
        let mut row = ReportRow::of(RowKind::OpenLot, asset.id, &asset.symbol, &asset.asset_type, &asset.currency, price_source);
        row.price_date_used = price_date_used;
        row.fx_rate_date_used = fx_rate_date_used;
        row.buy_transaction_id = Some(lot.buy_transaction_id);
        row.buy_date = Some(lot.buy_date);
        row.buy_price = Some(lot.buy_price);
        row.unit_cost = Some(lot.unit_cost);
        row.original_qty = Some(lot.original_qty);
        row.remaining_qty = Some(lot.remaining_qty);
        row.market_value = Some(lot.market_value);
        row.unrealized_gain_loss = Some(lot.unrealized_gain_loss);
        row.unrealized_roi = lot.unrealized_roi;
        row
    }
}

impl AssetTotals {
    /// An asset with nothing dated at or before the window end: no realized figure, no
    /// open lot, and no close to report.
    pub fn empty(asset: &Asset, fx_applied: bool, split_applied: bool) -> Self {
        AssetTotals {
            asset_id: asset.id,
            symbol: asset.symbol.clone(),
            asset_type: asset.asset_type.clone(),
            asset_currency: asset.currency.clone(),
            realized_in_window: Decimal::ZERO,
            realized_cumulative: Decimal::ZERO,
            unrealized_at_end: Decimal::ZERO,
            market_value: Decimal::ZERO,
            cost_basis: Decimal::ZERO,
            price_date_used: None,
            fx_rate_date_used: None,
            price_source: None,
            fx_applied,
            split_applied,
        }
    }
}

impl ReportTotals {
    pub fn zero() -> Self {
        ReportTotals {
            realized_in_window: Decimal::ZERO,
            realized_cumulative: Decimal::ZERO,
            unrealized_at_end: Decimal::ZERO,
            market_value: Decimal::ZERO,
            cost_basis: Decimal::ZERO,
        }
    }
}

/// The dated report: the run context, per-asset totals, the portfolio rollup and every row.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReportResponse {
    pub context: ReportContext,
    pub asset_totals: Vec<AssetTotals>,
    pub totals: ReportTotals,
    pub rows: Vec<ReportRow>,
}

/// One currency pair a rate could not be found for. A missing rate is a pair, not a
/// symbol, and the body says which it is.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MissingRate {
    pub from: String,
    pub to: String,
}

/// The one body every failure returns. Validation failures carry an empty pair of lists
/// and name the offending parameter in `error`; a pricing failure populates them.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ErrorBody {
    pub error: String,
    pub missing_prices: Vec<String>,
    pub missing_rates: Vec<MissingRate>,
    /// How many were found, before the lists were capped. Truncation is never silent.
    pub missing_prices_total: usize,
    pub missing_rates_total: usize,
    pub retryable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use std::str::FromStr;

    fn sample_datetime() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2025-01-15T10:30:00Z").unwrap().with_timezone(&Utc)
    }

    #[test]
    fn test_transaction_create_roundtrip() {
        let tx = TransactionCreate {
            r#type: "BUY".to_string(),
            quantity: Decimal::from_str("100.0").unwrap(),
            price: Decimal::from_str("150.5").unwrap(),
            fee: Decimal::from_str("9.99").unwrap(),
            date: sample_datetime(),
        };
        let json = serde_json::to_string(&tx).unwrap();
        let deserialized: TransactionCreate = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.r#type, "BUY");
        assert_eq!(deserialized.quantity, Decimal::from_str("100.0").unwrap());
        assert_eq!(deserialized.price, Decimal::from_str("150.5").unwrap());
        assert_eq!(deserialized.fee, Decimal::from_str("9.99").unwrap());
    }

    #[test]
    fn test_transaction_out_roundtrip() {
        let tx = TransactionOut {
            id: 42,
            asset_id: 7,
            r#type: "SELL".to_string(),
            quantity: Decimal::from_str("25.0").unwrap(),
            price: Decimal::from_str("200.0").unwrap(),
            fee: Decimal::from_str("5.0").unwrap(),
            date: sample_datetime(),
        };
        let json = serde_json::to_string(&tx).unwrap();
        let deserialized: TransactionOut = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, 42);
        assert_eq!(deserialized.asset_id, 7);
        assert_eq!(deserialized.r#type, "SELL");
    }

    #[test]
    fn test_asset_create_roundtrip() {
        let a = AssetCreate {
            symbol: "TSLA".to_string(),
            name: "Tesla Inc".to_string(),
            asset_type: "STOCK".to_string(),
            sector: Some("Auto".to_string()),
            currency: "USD".to_string(),
            isin: "US88160R1014".to_string(),
        };
        let json = serde_json::to_string(&a).unwrap();
        let deserialized: AssetCreate = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.symbol, "TSLA");
        assert_eq!(deserialized.asset_type, "STOCK");
        assert_eq!(deserialized.currency, "USD");
        assert_eq!(deserialized.isin, "US88160R1014");
        assert_eq!(deserialized.sector, Some("Auto".to_string()));
    }

    #[test]
    fn test_portfolio_create_with_none_description() {
        let p = PortfolioCreate {
            name: "My Portfolio".to_string(),
            description: None,
            currency: "USD".to_string(),
        };
        let json = serde_json::to_string(&p).unwrap();
        let deserialized: PortfolioCreate = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "My Portfolio");
        assert_eq!(deserialized.description, None);
        assert_eq!(deserialized.currency, "USD");
    }

    #[test]
    fn test_portfolio_out_roundtrip() {
        let p = PortfolioOut {
            id: 1,
            name: "Test".to_string(),
            description: Some("A test portfolio".to_string()),
            currency: "EUR".to_string(),
            assets: vec![],
        };
        let json = serde_json::to_string(&p).unwrap();
        let deserialized: PortfolioOut = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, 1);
        assert_eq!(deserialized.name, "Test");
        assert_eq!(deserialized.description, Some("A test portfolio".to_string()));
    }

    #[test]
    fn test_asset_out_roundtrip() {
        let a = AssetOut {
            id: 1,
            portfolio_id: 1,
            symbol: "AAPL".to_string(),
            name: "Apple Inc".to_string(),
            asset_type: "STOCK".to_string(),
            sector: Some("Technology".to_string()),
            currency: "USD".to_string(),
            isin: Some("US0378331005".to_string()),
            transactions: vec![],
        };
        let json = serde_json::to_string(&a).unwrap();
        let deserialized: AssetOut = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.symbol, "AAPL");
        assert_eq!(deserialized.currency, "USD");
        assert_eq!(deserialized.isin, Some("US0378331005".to_string()));
        assert_eq!(deserialized.sector, Some("Technology".to_string()));
    }

    #[test]
    fn test_tax_lot_roundtrip() {
        let lot = TaxLot {
            buy_transaction_id: 7,
            buy_date: sample_datetime(),
            buy_price: Decimal::from_str("150.0").unwrap(),
            original_qty: Decimal::from_str("100.0").unwrap(),
            remaining_qty: Decimal::from_str("75.0").unwrap(),
            latent_gain_loss: Decimal::from_str("500.0").unwrap(),
            latent_roi: Decimal::from_str("0.0444").unwrap(),
        };
        let json = serde_json::to_string(&lot).unwrap();
        let deserialized: TaxLot = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.remaining_qty, Decimal::from_str("75.0").unwrap());
        assert_eq!(deserialized.latent_gain_loss, Decimal::from_str("500.0").unwrap());
    }

    #[test]
    fn test_asset_tax_summary_roundtrip() {
        let summary = AssetTaxSummary {
            symbol: "AAPL".to_string(),
            asset_type: "STOCK".to_string(),
            current_shares: Decimal::from_str("100.0").unwrap(),
            average_cost: Decimal::from_str("150.0").unwrap(),
            current_price: Decimal::from_str("160.0").unwrap(),
            total_cost: Decimal::from_str("15000.0").unwrap(),
            market_value: Decimal::from_str("16000.0").unwrap(),
            unrealized_pnl: Decimal::from_str("1000.0").unwrap(),
            unrealized_roi: Decimal::from_str("0.0667").unwrap(),
            realized_pnl: Decimal::from_str("200.0").unwrap(),
            tax_lots: vec![],
        };
        let json = serde_json::to_string(&summary).unwrap();
        let deserialized: AssetTaxSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.symbol, "AAPL");
        assert_eq!(deserialized.unrealized_pnl, Decimal::from_str("1000.0").unwrap());
    }

    #[test]
    fn test_portfolio_update_roundtrip() {
        let u = PortfolioUpdate {
            currency: "EUR".to_string(),
        };
        let json = serde_json::to_string(&u).unwrap();
        let deserialized: PortfolioUpdate = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.currency, "EUR");
    }

    #[test]
    fn test_asset_update_roundtrip() {
        let u = AssetUpdate {
            currency: "GBP".to_string(),
        };
        let json = serde_json::to_string(&u).unwrap();
        let deserialized: AssetUpdate = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.currency, "GBP");
    }

    #[test]
    fn test_lot_match_roundtrip() {
        let m = LotMatch {
            buy_transaction_id: 11,
            buy_date: sample_datetime(),
            buy_price: Decimal::from_str("100.0").unwrap(),
            unit_cost: Decimal::from_str("100.5").unwrap(),
            matched_qty: Decimal::from_str("60.0").unwrap(),
            lot_qty_before_match: Decimal::from_str("100.0").unwrap(),
            cost_basis: Decimal::from_str("6030.0").unwrap(),
            proceeds: Decimal::from_str("7800.0").unwrap(),
            pnl: Decimal::from_str("1770.0").unwrap(),
            holding_days: 335,
            term: Some("SHORT".to_string()),
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: LotMatch = serde_json::from_str(&json).unwrap();
        assert_eq!(back.buy_transaction_id, 11);
        assert_eq!(back.unit_cost, Decimal::from_str("100.5").unwrap());
        assert_eq!(back.holding_days, 335);
        assert_eq!(back.term, Some("SHORT".to_string()));
    }

    #[test]
    fn test_sell_breakdown_roundtrip_and_decimals_are_strings() {
        let b = SellBreakdown {
            sell_transaction_id: 42,
            sell_date: sample_datetime(),
            sold_qty: Decimal::from_str("80.0").unwrap(),
            matched_qty: Decimal::from_str("80.0").unwrap(),
            unmatched_qty: Decimal::ZERO,
            proceeds: Decimal::from_str("10400.0").unwrap(),
            cost_basis: Decimal::from_str("8400.0").unwrap(),
            realized_pnl: Decimal::from_str("2000.0").unwrap(),
            matches: vec![],
        };
        let json = serde_json::to_string(&b).unwrap();
        assert!(json.contains(r#""realized_pnl":"2000.0""#), "decimals must serialize as strings: {json}");
        let back: SellBreakdown = serde_json::from_str(&json).unwrap();
        assert_eq!(back.realized_pnl, Decimal::from_str("2000.0").unwrap());
        assert!(back.matches.is_empty());
    }

    #[test]
    fn test_sell_pnl_response_roi_null_when_no_cost_basis() {
        let r = SellPnlResponse {
            transaction_id: 42, asset_id: 7, symbol: "AAPL".to_string(),
            asset_type: "STOCK".to_string(), strategy: "FIFO".to_string(), threshold_days: 30,
            currency: "USD".to_string(), asset_currency: "USD".to_string(),
            fx_applied: false, split_applied: false,
            sell_date: sample_datetime(),
            sold_qty: Decimal::from_str("50.0").unwrap(),
            price: Decimal::from_str("120.0").unwrap(),
            fee: Decimal::ZERO,
            proceeds: Decimal::ZERO, cost_basis: Decimal::ZERO, realized_pnl: Decimal::ZERO,
            roi: None,
            matched_qty: Decimal::ZERO, unmatched_qty: Decimal::from_str("50.0").unwrap(),
            matches: vec![],
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains(r#""roi":null"#), "roi must be null, not 0: {json}");
        let back: SellPnlResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.roi, None);
        assert_eq!(back.unmatched_qty, Decimal::from_str("50.0").unwrap());
    }
}
