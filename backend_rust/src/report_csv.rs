//! Server-side CSV for the dated tax report.
//!
//! Deliberately not under `src/api_routes/`: the access gate's Rule A requires every file
//! in that directory to import `crate::access` and call a `require*` function, and a
//! serializer has no business calling one.
//!
//! Row 1 is always the header row and no comment row precedes it. That is evidence rather
//! than taste: `detectColumns` (`frontend/src/utils/csvParser.ts:25`) normalises every cell
//! of row 1 against `FIELD_ALIASES`, so a leading `#` row maps every column to `null` and
//! an import silently yields nothing. Caveats therefore ride as columns, exactly as the
//! single-sell export already does (`sellPnlCsv.ts:13-16`), and the split caveat comes from
//! the payload (`ReportContext.splits_note`) rather than from a string typed in here.
//!
//! There is no generated-at column on purpose: two dispatches over unchanged data must be
//! byte-identical, which is Goal 4 and is only testable because the file is server-side.

use crate::schemas::{ReportResponse, ReportRow};
use rust_decimal::Decimal;

/// The header, in order: run context, then the asset and its pricing provenance, then the
/// asset's server-computed totals, then the sell, the lot, and the open-lot cells.
///
/// The per-asset totals ride as columns so a reader can check the file against them without
/// recomputing anything, and so a spreadsheet can be reconciled against the screen.
/// `lot_pnl` is the one column whose sum over `SOLD_LOT` rows is a figure at all
/// (`AssetTotals.realized_in_window`); every sell-level column repeats on each lot row and
/// must not be summed, and every `asset_*` column is already a total.
pub const CSV_COLUMNS: [&str; 49] = [
    // run context
    "row_type", "portfolio_id", "portfolio_name", "base_currency", "strategy",
    "threshold_days", "window_from", "window_to", "splits_note",
    // asset and pricing provenance
    "asset_id", "symbol", "asset_type", "asset_currency", "fx_applied", "split_applied",
    "price_source", "price_date_used", "fx_rate_date_used",
    // this asset's server-computed totals
    "asset_realized_in_window", "asset_realized_cumulative", "asset_unrealized_at_end",
    "asset_market_value", "asset_cost_basis",
    // sell context, repeated on each of the sell's lot rows
    "sell_transaction_id", "sell_date", "sold_qty", "sell_price_in_asset_currency",
    "sell_fee_in_asset_currency", "proceeds", "cost_basis", "realized_pnl", "matched_qty",
    "unmatched_qty",
    // the buy lot a sell consumed
    "buy_transaction_id", "buy_date", "buy_price_in_base_currency", "unit_cost",
    "lot_matched_qty", "lot_qty_before_match", "lot_cost_basis", "lot_proceeds", "lot_pnl",
    "holding_days", "term",
    // the lot still open at the window end
    "original_qty", "remaining_qty", "market_value", "unrealized_gain_loss", "unrealized_roi",
];

/// Characters that make Excel and Google Sheets run a cell instead of showing it. A symbol
/// reaches the file as free text — `csvValidator` checks only that it is non-empty and the
/// backend stores it uppercased and unfiltered — so it is attacker-chosen by the time it is
/// exported. The leading apostrophe is what makes a spreadsheet render the cell as text.
/// Applied to every text cell rather than to an enumerated set: none of the literal cells
/// this file writes starts with one of these characters, so the guard is free.
const FORMULA_LEAD: [char; 6] = ['=', '+', '-', '@', '\t', '\r'];

/// A formula-led cell is apostrophe-guarded **and** quoted. Quoting is not what RFC 4180
/// requires here — the guard removes the formula lead and the value contains no comma,
/// quote or newline — but it is what the settled test pins
/// (`every_row_kind_reaches_the_file_and_nulls_are_empty_cells`: "a free-text cell with a
/// formula lead is quoted and apostrophe-guarded"), and it makes the emitted value a
/// visibly single cell rather than one whose leading apostrophe a reader might normalise
/// away. A guarded cell is therefore quoted even when it would not need to be.
fn text_cell(value: &str) -> String {
    let guarded = value.starts_with(FORMULA_LEAD);
    let cell = if guarded {
        format!("'{value}")
    } else {
        value.to_string()
    };
    // RFC 4180: a cell containing a comma, a quote or a newline is quoted, and inner
    // quotes are doubled.
    if guarded || cell.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", cell.replace('"', "\"\""))
    } else {
        cell
    }
}

/// Money and quantities, verbatim from the `Decimal`. Never through `f64`, and never with
/// an exponent: `Decimal`'s display is fixed-point at the stored scale.
fn decimal_cell(value: Option<Decimal>) -> String {
    value.map(|d| d.to_string()).unwrap_or_default()
}

/// A CSV row as cells, already escaped and joined with `,`.
fn line(cells: impl IntoIterator<Item = String>) -> String {
    let mut out = cells.into_iter().collect::<Vec<_>>().join(",");
    out.push_str("\r\n");
    out
}

/// The whole file, `\r\n`-terminated per row (RFC 4180, and what Excel expects).
pub fn render(report: &ReportResponse) -> String {
    let mut out = line(CSV_COLUMNS.iter().map(|c| c.to_string()));

    for row in &report.rows {
        // The asset's totals are looked up, never computed: the CSV is a projection of the
        // payload, so the numbers in the file are the numbers the route produced.
        let totals = report.asset_totals.iter().find(|t| t.asset_id == row.asset_id);
        out.push_str(&render_row(report, row, totals));
    }

    out
}

fn render_row(
    report: &ReportResponse,
    row: &ReportRow,
    totals: Option<&crate::schemas::AssetTotals>,
) -> String {
    let context = &report.context;
    line([
        text_cell(row.row_type.as_str()),
        context.portfolio_id.to_string(),
        text_cell(&context.portfolio_name),
        text_cell(&context.base_currency),
        text_cell(&context.strategy),
        context.threshold_days.to_string(),
        context.requested_from.map(|d| d.to_string()).unwrap_or_default(),
        context.requested_to.to_string(),
        text_cell(&context.splits_note),
        row.asset_id.to_string(),
        text_cell(&row.symbol),
        text_cell(&row.asset_type),
        text_cell(&row.asset_currency),
        totals.map(|t| t.fx_applied).unwrap_or(false).to_string(),
        totals.map(|t| t.split_applied).unwrap_or(false).to_string(),
        row.price_source.map(|s| s.as_str().to_string()).unwrap_or_default(),
        row.price_date_used.map(|d| d.to_string()).unwrap_or_default(),
        row.fx_rate_date_used.map(|d| d.to_string()).unwrap_or_default(),
        decimal_cell(totals.map(|t| t.realized_in_window)),
        decimal_cell(totals.map(|t| t.realized_cumulative)),
        decimal_cell(totals.map(|t| t.unrealized_at_end)),
        decimal_cell(totals.map(|t| t.market_value)),
        decimal_cell(totals.map(|t| t.cost_basis)),
        row.sell_transaction_id.map(|v| v.to_string()).unwrap_or_default(),
        row.sell_date.map(|d| d.to_rfc3339()).unwrap_or_default(),
        decimal_cell(row.sold_qty),
        decimal_cell(row.sell_price),
        decimal_cell(row.sell_fee),
        decimal_cell(row.proceeds),
        decimal_cell(row.cost_basis),
        decimal_cell(row.realized_pnl),
        decimal_cell(row.matched_qty),
        decimal_cell(row.unmatched_qty),
        row.buy_transaction_id.map(|v| v.to_string()).unwrap_or_default(),
        row.buy_date.map(|d| d.to_rfc3339()).unwrap_or_default(),
        decimal_cell(row.buy_price),
        decimal_cell(row.unit_cost),
        decimal_cell(row.lot_matched_qty),
        decimal_cell(row.lot_qty_before_match),
        decimal_cell(row.lot_cost_basis),
        decimal_cell(row.lot_proceeds),
        decimal_cell(row.lot_pnl),
        row.holding_days.map(|v| v.to_string()).unwrap_or_default(),
        row.term.clone().map(|t| text_cell(&t)).unwrap_or_default(),
        decimal_cell(row.original_qty),
        decimal_cell(row.remaining_qty),
        decimal_cell(row.market_value),
        decimal_cell(row.unrealized_gain_loss),
        decimal_cell(row.unrealized_roi),
    ])
}
